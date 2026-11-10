//! Datadog's HTTP logs intake takes `ddsource`, `service`, `hostname` and `ddtags` as query parameters too
//! (`/api/v2/logs?dd-api-key=…&ddsource=solarwinds`), for senders that can only set a URL, such as vendors'
//! webhooks. They apply to every log of the body: a log's own `ddsource`, `service` and `hostname` win, and the
//! query's tags are added to its `ddtags`. Applied here, before Kafka, so the log ingester sees ordinary logs.

use serde_json::{Map, Value};

/// The query parameters that apply to every log of a request.
#[derive(Debug, Default, PartialEq)]
pub struct LogDefaults {
    pub ddsource: Option<String>,
    pub service: Option<String>,
    pub hostname: Option<String>,
    pub ddtags: Option<String>,
}

impl LogDefaults {
    /// The parameters of a request's query string (URL-decoded; empty values ignored).
    pub fn from_query(query: Option<&str>) -> LogDefaults {
        let mut d = LogDefaults::default();
        for (k, v) in url::form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            let slot = match k.as_ref() {
                "ddsource" => &mut d.ddsource,
                "service" => &mut d.service,
                "hostname" => &mut d.hostname,
                "ddtags" => &mut d.ddtags,
                _ => continue,
            };
            *slot = Some(v.to_string());
        }
        d
    }

    pub fn is_empty(&self) -> bool {
        *self == LogDefaults::default()
    }

    fn apply_to(&self, event: &mut Map<String, Value>) {
        for (key, value) in [("ddsource", &self.ddsource), ("service", &self.service), ("hostname", &self.hostname)] {
            if let Some(v) = value {
                let set = event.get(key).is_some_and(|e| !e.is_null() && e.as_str() != Some(""));
                if !set {
                    event.insert(key.into(), Value::String(v.clone()));
                }
            }
        }
        if let Some(tags) = &self.ddtags {
            let joined = match event.get("ddtags").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
                Some(own) => format!("{own},{tags}"),
                None => tags.clone(),
            };
            event.insert("ddtags".into(), Value::String(joined));
        }
    }

    /// The JSON logs body (an object or an array of objects) with the parameters applied to every log.
    pub fn apply(&self, body: &[u8]) -> Result<Vec<u8>, &'static str> {
        let mut value: Value = serde_json::from_slice(body).map_err(|_| "invalid JSON log body")?;
        match &mut value {
            Value::Object(event) => self.apply_to(event),
            Value::Array(events) => {
                for event in events {
                    self.apply_to(event.as_object_mut().ok_or("JSON logs must be an object or an array of objects")?);
                }
            }
            _ => return Err("JSON logs must be an object or an array of objects"),
        }
        serde_json::to_vec(&value).map_err(|_| "cannot encode JSON logs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn query_parameters_are_read_decoded() {
        let d = LogDefaults::from_query(Some("dd-api-key=ocik_x&ddsource=solarwinds&service=orion&hostname=nms-01&ddtags=env%3Aprod%2Cteam%3Anoc&other=1"));
        assert_eq!(d, LogDefaults { ddsource: Some("solarwinds".into()), service: Some("orion".into()), hostname: Some("nms-01".into()), ddtags: Some("env:prod,team:noc".into()) });
        assert!(LogDefaults::from_query(Some("api_key=x&ddsource=")).is_empty());
        assert!(LogDefaults::from_query(None).is_empty());
    }

    #[test]
    fn every_log_gets_them_unless_it_sets_its_own() {
        let d = LogDefaults::from_query(Some("ddsource=solarwinds&service=orion&hostname=nms-01&ddtags=env:prod"));
        let body = json!([{"alert_name": "Node down"}, {"message": "m", "ddsource": "custom", "service": "", "hostname": null, "ddtags": "team:noc"}]);
        let out: Value = serde_json::from_slice(&d.apply(body.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(out[0], json!({"alert_name": "Node down", "ddsource": "solarwinds", "service": "orion", "hostname": "nms-01", "ddtags": "env:prod"}));
        assert_eq!((&out[1]["ddsource"], &out[1]["service"], &out[1]["hostname"], &out[1]["ddtags"]), (&json!("custom"), &json!("orion"), &json!("nms-01"), &json!("team:noc,env:prod")));
        let one: Value = serde_json::from_slice(&d.apply(br#"{"alert_name":"x"}"#).unwrap()).unwrap();
        assert_eq!(one["ddsource"], "solarwinds");
        for bad in ["[1]", "\"text\"", "{broken"] {
            assert!(d.apply(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}
