//! Answer formats other than JSON: the Prometheus / OpenMetrics text format, as samples
//! `{name, labels, value, type, help, timestamp}` (`format: prometheus`), and CSV with a header row (`format: csv`).

use std::collections::HashMap;

use serde_json::{json, Map, Value};

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `{a="1",b="x \" y"}` → labels, and the rest of the line.
fn labels(s: &str) -> Option<(Map<String, Value>, &str)> {
    let mut out = Map::new();
    let bytes = s.as_bytes();
    let mut i = 1;
    loop {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        if bytes[i] == b'}' {
            return Some((out, &s[i + 1..]));
        }
        let eq = s[i..].find('=')? + i;
        let key = s[i..eq].trim().to_string();
        if bytes.get(eq + 1) != Some(&b'"') {
            return None;
        }
        let mut j = eq + 2;
        while j < bytes.len() && bytes[j] != b'"' {
            if bytes[j] == b'\\' {
                j += 1;
            }
            j += 1;
        }
        if j >= bytes.len() {
            return None;
        }
        out.insert(key, Value::String(unescape(&s[eq + 2..j])));
        i = j + 1;
    }
}

fn value(s: &str) -> Option<f64> {
    match s {
        "+Inf" | "Inf" => Some(f64::INFINITY),
        "-Inf" => Some(f64::NEG_INFINITY),
        "NaN" => None,
        other => other.parse().ok(),
    }
}

/// The samples of a Prometheus text exposition (NaN and infinite values are left out).
pub fn prometheus(text: &str) -> Vec<Value> {
    let mut types: HashMap<String, String> = HashMap::new();
    let mut helps: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let mut parts = rest.trim().splitn(3, ' ');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("TYPE"), Some(name), Some(kind)) => {
                    types.insert(name.to_string(), kind.trim().to_string());
                }
                (Some("HELP"), Some(name), Some(help)) => {
                    helps.insert(name.to_string(), unescape(help.trim()));
                }
                _ => {}
            }
            continue;
        }
        let name_end = line.find(|c: char| c == '{' || c.is_whitespace()).unwrap_or(line.len());
        let name = &line[..name_end];
        let (labels, rest) = if line[name_end..].starts_with('{') {
            match labels(&line[name_end..]) {
                Some(l) => l,
                None => continue,
            }
        } else {
            (Map::new(), &line[name_end..])
        };
        let mut fields = rest.split_whitespace();
        let Some(v) = fields.next().and_then(value).filter(|v| v.is_finite()) else { continue };
        // Prometheus' text format has integer milliseconds, OpenMetrics (fractional) seconds; `epoch_ms` tells them apart.
        let timestamp = fields.next().and_then(|t| t.parse::<i64>().map(Value::from).ok().or_else(|| t.parse::<f64>().ok().filter(|f| f.is_finite()).map(crate::expr::number)));
        // A histogram's or summary's series carry their family's type.
        let family = ["_bucket", "_sum", "_count", "_total", "_created"].iter().find_map(|s| name.strip_suffix(s).filter(|f| types.contains_key(*f))).unwrap_or(name);
        out.push(json!({
            "name": name,
            "family": family,
            "labels": labels,
            "value": crate::expr::number(v),
            "type": types.get(family).cloned().unwrap_or_else(|| "untyped".into()),
            "help": helps.get(family).cloned(),
            "timestamp": timestamp,
        }));
    }
    out
}

/// The rows of a CSV text (RFC 4180: quoted fields with `""`, separators and newlines inside quotes) as objects by the
/// header row's names.
pub fn csv(text: &str) -> Vec<Value> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let (mut row, mut field, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    let mut rows = rows.into_iter().filter(|r| !(r.len() == 1 && r[0].is_empty()));
    let Some(header) = rows.next() else { return vec![] };
    rows.map(|r| Value::Object(header.iter().cloned().zip(r.into_iter().map(Value::String)).collect())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_with_labels_types_and_families() {
        let text = "# HELP pulsar_msg_rate_in Messages in per second\n# TYPE pulsar_msg_rate_in gauge\npulsar_msg_rate_in{cluster=\"shop\",topic=\"persistent://public/default/orders\"} 12.5\n\
                    # TYPE http_request_duration_seconds histogram\nhttp_request_duration_seconds_bucket{le=\"0.5\"} 3\nhttp_request_duration_seconds_count 7 1794301079641\nbad{x=1} 2\nnan_metric NaN\n";
        let s = prometheus(text);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0]["labels"], json!({ "cluster": "shop", "topic": "persistent://public/default/orders" }));
        assert_eq!((s[0]["value"].as_f64(), s[0]["type"].as_str(), s[0]["help"].as_str()), (Some(12.5), Some("gauge"), Some("Messages in per second")));
        assert_eq!((s[1]["family"].as_str(), s[1]["type"].as_str(), s[1]["labels"]["le"].as_str()), (Some("http_request_duration_seconds"), Some("histogram"), Some("0.5")));
        assert_eq!(s[2]["timestamp"], json!(1_794_301_079_641i64));
    }

    #[test]
    fn openmetrics_timestamps_in_fractional_seconds_are_kept() {
        let s = prometheus("# TYPE temporal_cloud_v1_action_limit gauge\ntemporal_cloud_v1_action_limit 500 1794311820.5\n# EOF\n");
        assert_eq!(s[0]["timestamp"].as_f64(), Some(1_794_311_820.5));
    }

    #[test]
    fn csv_rows_become_objects() {
        let rows = csv("\u{feff}\"EVENT_TYPE\",\"USER_ID\",\"URI\"\r\n\"Login\",\"005xx\",\"/a,b\"\r\n\"API\",\"\",\"say \"\"hi\"\"\nthere\"\r\n");
        assert_eq!(rows, vec![json!({ "EVENT_TYPE": "Login", "USER_ID": "005xx", "URI": "/a,b" }), json!({ "EVENT_TYPE": "API", "USER_ID": "", "URI": "say \"hi\"\nthere" })]);
        assert!(csv("").is_empty());
    }
}
