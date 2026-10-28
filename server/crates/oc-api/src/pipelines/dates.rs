//! Dates in log pipelines: the Java-style patterns grok's `date("…")` matcher takes
//! (`dd/MMM/yyyy:HH:mm:ss Z`, `yyyy-MM-dd'T'HH:mm:ss.SSSZ`, …) and the values the date remapper
//! accepts (epoch numbers, ISO 8601 and a few common log formats). Both yield epoch milliseconds.

use fancy_regex::Regex;
use serde_json::Value;

/// A compiled `date()` pattern.
pub struct DatePattern {
    /// The pattern as a regex without capture groups, for embedding in a grok rule.
    fragment: String,
    /// The pattern with one named group per field, to read the matched text.
    parse: Regex,
    /// The zone used when the text carries none (the matcher's second argument), in seconds east of UTC.
    default_offset: i32,
}

impl DatePattern {
    pub fn compile(pattern: &str, tz: Option<&str>) -> Result<Self, String> {
        let default_offset = match tz {
            Some(z) => zone_offset(z).ok_or_else(|| format!("Unsupported time zone '{z}' (use UTC or an offset such as +02:00)"))?,
            None => 0,
        };
        let (mut fragment, mut named) = (String::new(), String::new());
        let mut seen = std::collections::HashSet::new();
        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\'' {
                let (lit, next) = quoted_literal(&chars, i);
                i = next;
                let esc = fancy_regex::escape(&lit).into_owned();
                fragment.push_str(&esc);
                named.push_str(&esc);
                continue;
            }
            if !c.is_ascii_alphabetic() {
                let esc = fancy_regex::escape(&c.to_string()).into_owned();
                fragment.push_str(&esc);
                named.push_str(&esc);
                i += 1;
                continue;
            }
            let mut n = 1;
            while chars.get(i + n) == Some(&c) {
                n += 1;
            }
            i += n;
            let (group, re) = field(c, n).ok_or_else(|| format!("Unsupported date pattern letter '{c}' in \"{pattern}\""))?;
            fragment.push_str(&format!("(?:{re})"));
            // A field repeated later (rare) is matched but read once.
            if seen.insert(group) {
                named.push_str(&format!("(?P<{group}>{re})"));
            } else {
                named.push_str(&format!("(?:{re})"));
            }
        }
        let parse = Regex::new(&format!("^{named}$")).map_err(|e| format!("Invalid date pattern \"{pattern}\": {e}"))?;
        Ok(DatePattern { fragment, parse, default_offset })
    }

    pub fn fragment(&self) -> &str {
        &self.fragment
    }

    /// Epoch milliseconds of a text the pattern matched; `now_ms` supplies a missing year.
    pub fn parse(&self, text: &str, now_ms: i64) -> Option<i64> {
        let c = self.parse.captures(text).ok()??;
        let num = |g: &str| c.name(g).and_then(|m| m.as_str().parse::<i64>().ok());
        let month = match c.name("MMM") {
            Some(m) => month_number(m.as_str())?,
            None => num("M").unwrap_or(1),
        };
        let day = num("d").unwrap_or(1);
        let mut hour = num("H").unwrap_or(0);
        if let Some(h12) = num("h") {
            let pm = c.name("a").is_some_and(|m| m.as_str().eq_ignore_ascii_case("pm"));
            hour = h12 % 12 + if pm { 12 } else { 0 };
        }
        let (minute, second) = (num("m").unwrap_or(0), num("s").unwrap_or(0));
        // Fractions keep their first three digits (microseconds and nanoseconds are cut to ms).
        let millis = c.name("S").map_or(0, |m| m.as_str().chars().chain("000".chars()).take(3).collect::<String>().parse::<i64>().unwrap_or(0));
        let offset = match c.name("Z").or_else(|| c.name("z")) {
            Some(m) => zone_offset(m.as_str())?,
            None => self.default_offset,
        };
        let at = |year: i64| -> Option<i64> {
            let date = chrono::NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)?;
            let t = date.and_hms_milli_opt(hour as u32, minute as u32, second as u32, millis as u32)?;
            Some(t.and_utc().timestamp_millis() - offset as i64 * 1000)
        };
        if let Some(year) = num("y") {
            return at(year);
        }
        if let Some(yy) = num("yy") {
            return at(2000 + yy);
        }
        // No year: this year, or last year for a date ahead of now (by more than a day).
        let this_year = chrono::DateTime::from_timestamp_millis(now_ms).map(|d| chrono::Datelike::year(&d) as i64)?;
        let ms = at(this_year)?;
        if ms > now_ms + 86_400_000 {
            at(this_year - 1)
        } else {
            Some(ms)
        }
    }
}

/// The text of a `'quoted'` literal starting at `chars[start]` (`''` is a quote) and the index after it.
fn quoted_literal(chars: &[char], start: usize) -> (String, usize) {
    let mut lit = String::new();
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == '\'' {
            if chars.get(i + 1) == Some(&'\'') {
                lit.push('\'');
                i += 2;
                continue;
            }
            break;
        }
        lit.push(chars[i]);
        i += 1;
    }
    if lit.is_empty() {
        lit.push('\'');
    }
    (lit, i + 1)
}

/// The capture group name and regex of a run of `n` pattern letters `c`.
fn field(c: char, n: usize) -> Option<(&'static str, String)> {
    let digits = |min: usize, max: usize| if n >= max { format!(r"\d{{{max}}}") } else { format!(r"\d{{{min},{max}}}") };
    Some(match c {
        'y' | 'Y' | 'u' if n == 2 => ("yy", r"\d{2}".into()),
        'y' | 'Y' | 'u' => ("y", if n >= 4 { r"\d{4}".into() } else { r"\d{1,4}".into() }),
        'M' | 'L' if n >= 4 => ("MMM", "[A-Za-z]+".into()),
        'M' | 'L' if n == 3 => ("MMM", "[A-Za-z]{3}".into()),
        'M' | 'L' => ("M", digits(1, 2)),
        'd' => ("d", digits(1, 2)),
        'H' | 'k' => ("H", digits(1, 2)),
        'h' | 'K' => ("h", digits(1, 2)),
        'm' => ("m", digits(1, 2)),
        's' => ("s", digits(1, 2)),
        'S' => ("S", format!(r"\d{{{n}}}")),
        'a' => ("a", "(?i:AM|PM)".into()),
        'E' => ("E", "[A-Za-z]+".into()),
        'Z' | 'X' | 'x' => ("Z", r"Z|[+-]\d{2}(?::?\d{2})?".into()),
        'z' => ("z", r"[A-Za-z]{1,5}(?:[+-]\d{1,2}(?::?\d{2})?)?|[+-]\d{2}(?::?\d{2})?".into()),
        _ => return None,
    })
}

fn month_number(name: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let head = name.chars().take(3).collect::<String>().to_ascii_lowercase();
    MONTHS.iter().position(|m| *m == head).map(|i| i as i64 + 1)
}

/// Seconds east of UTC for a zone name or offset: `UTC`, `Z`, `+02:00`, `-0500`, `GMT+2`, `CEST`, …
pub fn zone_offset(zone: &str) -> Option<i32> {
    let z = zone.trim();
    let upper = z.to_ascii_uppercase();
    for prefix in ["UTC", "GMT", "UT"] {
        if let Some(rest) = upper.strip_prefix(prefix) {
            if rest.is_empty() {
                return Some(0);
            }
            if rest.starts_with(['+', '-']) {
                return numeric_offset(rest);
            }
        }
    }
    if upper == "Z" {
        return Some(0);
    }
    if z.starts_with(['+', '-']) {
        return numeric_offset(z);
    }
    let minutes = match upper.as_str() {
        "EST" | "CDT" => -300,
        "EDT" => -240,
        "CST" | "MDT" => -360,
        "MST" | "PDT" => -420,
        "PST" | "AKDT" => -480,
        "AKST" => -540,
        "HST" => -600,
        "WET" => 0,
        "WEST" | "BST" | "CET" => 60,
        "CEST" | "EET" => 120,
        "EEST" | "MSK" => 180,
        "JST" | "KST" => 540,
        "AEST" => 600,
        "AEDT" => 660,
        _ => return None,
    };
    Some(minutes * 60)
}

/// `+2`, `+02`, `+0200`, `+02:00`.
fn numeric_offset(s: &str) -> Option<i32> {
    let sign = if s.starts_with('-') { -1 } else { 1 };
    let digits: String = s[1..].chars().filter(|c| *c != ':').collect();
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let split = if digits.len() <= 2 { digits.len() } else { digits.len() - 2 };
    let h: i32 = digits[..split].parse().ok()?;
    let m: i32 = if split < digits.len() { digits[split..].parse().ok()? } else { 0 };
    (h <= 18 && m < 60).then_some(sign * (h * 3600 + m * 60))
}

/// Epoch milliseconds of an epoch number in seconds, milliseconds, microseconds or nanoseconds.
fn epoch_ms(n: f64) -> Option<i64> {
    let a = n.abs();
    let ms = if a > 1e17 {
        n / 1e6
    } else if a > 1e14 {
        n / 1e3
    } else if a > 1e11 {
        n
    } else {
        n * 1e3
    };
    ms.is_finite().then_some(ms.round() as i64)
}

/// Epoch milliseconds of a date attribute (the date remapper): epoch seconds / ms / µs / ns as a
/// number or numeric string, ISO 8601 / RFC 3339, RFC 2822, Apache's `10/Oct/2026:13:55:36 -0700`,
/// `2026-10-25 13:55:36[.123] [UTC]` and syslog's `Oct 25 13:55:36` (current year).
pub fn parse_any(v: &Value, now_ms: i64) -> Option<i64> {
    match v {
        // Integers stay exact (nanoseconds don't fit an f64).
        Value::Number(n) => match n.as_i64() {
            Some(i) => {
                let a = i.unsigned_abs();
                Some(if a > 100_000_000_000_000_000 {
                    i / 1_000_000
                } else if a > 100_000_000_000_000 {
                    i / 1_000
                } else if a > 100_000_000_000 {
                    i
                } else {
                    i * 1_000
                })
            }
            None => epoch_ms(n.as_f64()?),
        },
        Value::String(s) => parse_text(s.trim(), now_ms),
        _ => None,
    }
}

fn parse_text(s: &str, now_ms: i64) -> Option<i64> {
    use chrono::{DateTime, NaiveDateTime};
    if s.is_empty() {
        return None;
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return parse_any(&Value::from(s.parse::<i64>().ok()?), now_ms);
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.timestamp_millis());
    }
    if let Ok(d) = DateTime::parse_from_rfc2822(s) {
        return Some(d.timestamp_millis());
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%d %H:%M:%S%.f %z", "%Y-%m-%d %H:%M:%S%.f%z", "%d/%b/%Y:%H:%M:%S %z"] {
        if let Ok(d) = DateTime::parse_from_str(s, f) {
            return Some(d.timestamp_millis());
        }
    }
    // Without a zone, or with a zone name: UTC unless the name says otherwise.
    let (text, offset) = match s.rsplit_once(' ') {
        Some((head, zone)) if zone.chars().all(|c| c.is_ascii_alphabetic()) => (head, zone_offset(zone)?),
        _ => (s.trim_end_matches('Z'), 0),
    };
    for f in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y/%m/%d %H:%M:%S%.f"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(text, f) {
            return Some(d.and_utc().timestamp_millis() - offset as i64 * 1000);
        }
    }
    // Syslog: no year.
    let syslog = text.split_whitespace().collect::<Vec<_>>().join(" ");
    DatePattern::compile("MMM d HH:mm:ss", None).ok()?.parse(&syslog, now_ms).map(|ms| ms - offset as i64 * 1000)
}

/// RFC 3339 with milliseconds in UTC (`2026-10-25T13:55:36.123Z`), as the indexer writes `timestamp`.
pub fn to_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 2026-10-25T13:00:00Z
    const NOW: i64 = 1_792_933_200_000;

    fn parse(pattern: &str, tz: Option<&str>, text: &str) -> Option<i64> {
        let p = DatePattern::compile(pattern, tz).unwrap();
        let whole = Regex::new(&format!("^(?:{})$", p.fragment())).unwrap();
        assert!(whole.is_match(text).unwrap(), "{pattern} fragment matches {text}");
        p.parse(text, NOW)
    }

    #[test]
    fn java_patterns() {
        assert_eq!(parse("dd/MMM/yyyy:HH:mm:ss Z", None, "13/Jul/2016:10:55:36 +0000"), Some(1_468_407_336_000));
        assert_eq!(parse("dd/MMM/yyyy:HH:mm:ss Z", None, "13/Jul/2016:12:55:36 +0200"), Some(1_468_407_336_000));
        assert_eq!(parse("yyyy-MM-dd'T'HH:mm:ss.SSSZ", None, "2016-07-13T10:55:36.250Z"), Some(1_468_407_336_250));
        assert_eq!(parse("yyyy-MM-dd'T'HH:mm:ss.SSSXXX", None, "2016-07-13T12:55:36.250+02:00"), Some(1_468_407_336_250));
        assert_eq!(parse("yyyy/MM/dd HH:mm:ss", None, "2017/09/26 14:36:50"), Some(1_506_436_610_000));
        // Microseconds keep their milliseconds.
        assert_eq!(parse("yyyy-MM-dd HH:mm:ss.SSSSSS", None, "2020-08-20 18:19:25.769683"), Some(1_597_947_565_769));
        // A zone name in the text, or the matcher's time zone argument.
        assert_eq!(parse("yyyy-MM-dd HH:mm:ss z", None, "2017-11-07 17:30:39 UTC"), Some(1_510_075_839_000));
        assert_eq!(parse("yyyy-MM-dd HH:mm:ss", Some("+01:00"), "2017-11-07 18:30:39"), Some(1_510_075_839_000));
        // 12-hour clock and day names.
        assert_eq!(parse("EEE MMM d hh:mm:ss a yyyy", None, "Mon Jul 4 01:05:00 PM 2016"), Some(1_467_637_500_000));
    }

    #[test]
    fn a_missing_year_is_the_current_one() {
        // klog's "MMdd HH:mm:ss.SSSSSS": 2026-04-24T11:47:41.605Z.
        assert_eq!(parse("MMdd HH:mm:ss.SSSSSS", None, "0424 11:47:41.605188"), Some(1_777_031_261_605));
        // A date after now belongs to last year (logs written in December, read in January).
        assert_eq!(parse("MMM dd HH:mm:ss", None, "Dec 31 23:00:00"), Some(1_767_222_000_000));
    }

    #[test]
    fn unsupported_patterns_and_zones_are_errors() {
        assert!(DatePattern::compile("yyyy-ww", None).is_err());
        assert!(DatePattern::compile("yyyy-MM-dd", Some("Mars/Olympus")).is_err());
    }

    #[test]
    fn zones() {
        assert_eq!(zone_offset("UTC"), Some(0));
        assert_eq!(zone_offset("Z"), Some(0));
        assert_eq!(zone_offset("+02:00"), Some(7200));
        assert_eq!(zone_offset("-0530"), Some(-19800));
        assert_eq!(zone_offset("GMT+2"), Some(7200));
        assert_eq!(zone_offset("PDT"), Some(-25200));
        assert_eq!(zone_offset("Mars/Olympus"), None);
    }

    #[test]
    fn remapper_values() {
        let ms = 1_792_933_200_123;
        assert_eq!(parse_any(&json!(ms), NOW), Some(ms));
        assert_eq!(parse_any(&json!(1_792_933_200), NOW), Some(1_792_933_200_000));
        assert_eq!(parse_any(&json!(1_792_933_200.5), NOW), Some(1_792_933_200_500));
        assert_eq!(parse_any(&json!(1_792_933_200_123_456_789_i64), NOW), Some(ms));
        assert_eq!(parse_any(&json!("1792933200123"), NOW), Some(ms));
        assert_eq!(parse_any(&json!("2026-10-25T13:00:00.123Z"), NOW), Some(ms));
        assert_eq!(parse_any(&json!("2026-10-25T15:00:00.123+02:00"), NOW), Some(ms));
        assert_eq!(parse_any(&json!("2026-10-25T13:00:00.123"), NOW), Some(ms));
        assert_eq!(parse_any(&json!("2026-10-25 13:00:00.123 UTC"), NOW), Some(ms));
        assert_eq!(parse_any(&json!("25/Oct/2026:15:00:00 +0200"), NOW), Some(1_792_933_200_000));
        assert_eq!(parse_any(&json!("Sun, 25 Oct 2026 13:00:00 +0000"), NOW), Some(1_792_933_200_000));
        assert_eq!(parse_any(&json!("Oct 25 12:00:00"), NOW), Some(1_792_929_600_000));
        assert_eq!(parse_any(&json!("yesterday"), NOW), None);
        assert_eq!(parse_any(&json!(true), NOW), None);
    }

    #[test]
    fn rfc3339_output() {
        assert_eq!(to_rfc3339(1_792_933_200_123), "2026-10-25T13:00:00.123Z");
    }
}
