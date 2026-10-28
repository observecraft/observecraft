//! Workflow Automation: schedule triggers as 5-field cron expressions in an IANA time zone.
//!
//! `minute hour day-of-month month day-of-week`, each `*`, a number, a range `a-b`, a step `*/n` or
//! `a-b/n`, or a comma-separated list of those. Months and weekdays also take their English
//! three-letter names (`JAN`, `MON`); weekday 0 and 7 are Sunday. As in classic cron, when both the
//! day of month and the day of week are restricted a day matching either one fires. Times are local
//! to the schedule's zone: a time skipped by a daylight-saving change doesn't fire that day, and a
//! repeated one fires once.

use chrono::{Datelike, Duration, NaiveDate, NaiveTime, TimeZone};
use chrono_tz::Tz;

#[derive(Debug, Clone, PartialEq)]
pub struct Cron {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days: Vec<u32>,
    months: Vec<u32>,
    weekdays: Vec<u32>,
    days_restricted: bool,
    weekdays_restricted: bool,
}

const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
const WEEKDAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

fn value(s: &str, min: u32, max: u32, names: &[&str], name_base: u32, what: &str) -> Result<u32, String> {
    let upper = s.to_ascii_uppercase();
    if let Some(i) = names.iter().position(|n| *n == upper) {
        return Ok(i as u32 + name_base);
    }
    let v: u32 = s.parse().map_err(|_| format!("{s} isn't a valid {what}"))?;
    if v < min || v > max {
        return Err(format!("The {what} {v} is outside {min}–{max}"));
    }
    Ok(v)
}

fn field(s: &str, min: u32, max: u32, names: &[&str], name_base: u32, what: &str) -> Result<Vec<u32>, String> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, st)) => {
                let st: u32 = st.parse().map_err(|_| format!("{st} isn't a valid step in {part}"))?;
                if st == 0 {
                    return Err(format!("The step in {part} must be at least 1"));
                }
                (r, st)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (value(a, min, max, names, name_base, what)?, value(b, min, max, names, name_base, what)?);
            if a > b {
                return Err(format!("The range {range} goes backwards"));
            }
            (a, b)
        } else {
            let a = value(range, min, max, names, name_base, what)?;
            // `5/15` runs from 5 to the end of the field.
            (a, if part.contains('/') { max } else { a })
        };
        out.extend((lo..=hi).step_by(step as usize));
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

impl Cron {
    pub fn parse(expr: &str) -> Result<Cron, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err("A schedule has 5 fields: minute hour day-of-month month day-of-week (e.g. 0 9 * * MON-FRI)".into());
        }
        let mut weekdays = field(parts[4], 0, 7, &WEEKDAYS, 0, "day of the week")?;
        // 7 is Sunday too.
        if weekdays.contains(&7) {
            weekdays.retain(|d| *d != 7);
            if !weekdays.contains(&0) {
                weekdays.insert(0, 0);
            }
        }
        Ok(Cron {
            minutes: field(parts[0], 0, 59, &[], 0, "minute")?,
            hours: field(parts[1], 0, 23, &[], 0, "hour")?,
            days: field(parts[2], 1, 31, &[], 0, "day of the month")?,
            months: field(parts[3], 1, 12, &MONTHS, 1, "month")?,
            weekdays,
            days_restricted: parts[2] != "*",
            weekdays_restricted: parts[4] != "*",
        })
    }

    fn day_matches(&self, d: NaiveDate) -> bool {
        if !self.months.contains(&d.month()) {
            return false;
        }
        let dom = self.days.contains(&d.day());
        let dow = self.weekdays.contains(&d.weekday().num_days_from_sunday());
        match (self.days_restricted, self.weekdays_restricted) {
            (true, true) => dom || dow,
            (true, false) => dom,
            (false, true) => dow,
            (false, false) => true,
        }
    }

    /// The first time strictly after `after_ms` (Unix ms) the schedule fires, in Unix ms.
    pub fn next_after(&self, after_ms: i64, tz: Tz) -> Option<i64> {
        let after = chrono::DateTime::from_timestamp_millis(after_ms)?;
        let local = after.with_timezone(&tz).naive_local();
        let start_day = local.date();
        // Five years covers 29 February on a given weekday.
        for offset in 0..(366 * 5) {
            let day = start_day + Duration::days(offset);
            if !self.day_matches(day) {
                continue;
            }
            for &h in &self.hours {
                for &m in &self.minutes {
                    let Some(t) = NaiveTime::from_hms_opt(h, m, 0) else { continue };
                    // Skipped by a DST change: no fire. Repeated: the first one.
                    let Some(at) = tz.from_local_datetime(&day.and_time(t)).earliest() else { continue };
                    let ms = at.timestamp_millis();
                    if ms > after_ms {
                        return Some(ms);
                    }
                }
            }
        }
        None
    }
}

/// Parses an IANA zone name (`UTC`, `Europe/Paris`).
pub fn zone(name: &str) -> Result<Tz, String> {
    let n = name.trim();
    if n.is_empty() {
        return Ok(Tz::UTC);
    }
    n.parse::<Tz>().map_err(|_| format!("{n} isn't a known time zone (use an IANA name such as UTC or Europe/Paris)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
    }

    fn next(expr: &str, after: &str, tz: &str) -> String {
        let at = Cron::parse(expr).unwrap().next_after(ms(after), zone(tz).unwrap()).unwrap();
        chrono::DateTime::from_timestamp_millis(at).unwrap().to_rfc3339()
    }

    #[test]
    fn fields_parse() {
        let c = Cron::parse("*/15 9-17 * * MON-FRI").unwrap();
        assert_eq!(c.minutes, vec![0, 15, 30, 45]);
        assert_eq!(c.hours, (9..=17).collect::<Vec<_>>());
        assert_eq!(c.weekdays, vec![1, 2, 3, 4, 5]);
        assert_eq!(Cron::parse("0 0 1,15 jan,JUL 7").unwrap().weekdays, vec![0]);
        assert_eq!(Cron::parse("5/20 * * * *").unwrap().minutes, vec![5, 25, 45]);
        assert!(Cron::parse("* * * *").unwrap_err().contains("5 fields"));
        assert!(Cron::parse("60 * * * *").unwrap_err().contains("0–59"));
        assert!(Cron::parse("*/0 * * * *").unwrap_err().contains("at least 1"));
        assert!(Cron::parse("0 17-9 * * *").unwrap_err().contains("backwards"));
        assert!(Cron::parse("0 9 * * FUN").unwrap_err().contains("FUN"));
    }

    #[test]
    fn next_run_in_utc() {
        assert_eq!(next("0 9 * * *", "2026-03-02T08:30:00Z", "UTC"), "2026-03-02T09:00:00+00:00");
        assert_eq!(next("0 9 * * *", "2026-03-02T09:00:00Z", "UTC"), "2026-03-03T09:00:00+00:00", "strictly after");
        assert_eq!(next("*/15 * * * *", "2026-03-02T08:31:10Z", "UTC"), "2026-03-02T08:45:00+00:00");
        // Friday evening → Monday morning.
        assert_eq!(next("0 9 * * MON-FRI", "2026-03-06T18:00:00Z", "UTC"), "2026-03-09T09:00:00+00:00");
        assert_eq!(next("0 0 29 2 *", "2026-03-01T00:00:00Z", "UTC"), "2028-02-29T00:00:00+00:00");
    }

    #[test]
    fn day_of_month_or_weekday() {
        // The 1st, or any Monday: 2026-03-01 is a Sunday, so the first Monday (the 2nd) comes first after it.
        assert_eq!(next("0 12 1 * MON", "2026-03-01T13:00:00Z", "UTC"), "2026-03-02T12:00:00+00:00");
        assert_eq!(next("0 12 1 * MON", "2026-03-30T13:00:00Z", "UTC"), "2026-04-01T12:00:00+00:00");
    }

    #[test]
    fn time_zones_and_daylight_saving() {
        assert_eq!(next("0 9 * * *", "2026-01-15T00:00:00Z", "Europe/Paris"), "2026-01-15T08:00:00+00:00");
        assert_eq!(next("0 9 * * *", "2026-07-15T00:00:00Z", "Europe/Paris"), "2026-07-15T07:00:00+00:00");
        // 2026-03-29 02:30 doesn't exist in Paris: that day is skipped.
        assert_eq!(next("30 2 * * *", "2026-03-28T12:00:00Z", "Europe/Paris"), "2026-03-30T00:30:00+00:00");
        // 2026-10-25 02:30 happens twice in Paris: it fires on the first.
        assert_eq!(next("30 2 * * *", "2026-10-24T12:00:00Z", "Europe/Paris"), "2026-10-25T00:30:00+00:00");
        assert!(zone("Mars/Olympus").unwrap_err().contains("Mars/Olympus"));
        assert_eq!(zone("").unwrap(), Tz::UTC);
    }
}
