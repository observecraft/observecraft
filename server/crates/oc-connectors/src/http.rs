//! What the engine sends and gets back, and the [`Transport`] that carries it: the server's is SSRF-guarded
//! (`oc-api/src/connectors/net.rs`), the replay harness answers from a package's recorded answers
//! (`crate::replay`).

use serde_json::Value;

use crate::manifest::{Format, Method};

/// A client certificate for mutual TLS (PEM).
#[derive(Clone, PartialEq)]
pub struct ClientTls {
    pub certificate: String,
    pub key: String,
}

impl std::fmt::Debug for ClientTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientTls(..)")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub tls: Option<ClientTls>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn set_header(&mut self, name: &str, value: String) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    /// Names in lowercase.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The headers as an object (lowercase names; repeated headers joined with ", ").
    pub fn headers_value(&self) -> Value {
        let mut out = serde_json::Map::new();
        for (k, v) in &self.headers {
            let k = k.to_ascii_lowercase();
            match out.get_mut(&k) {
                Some(Value::String(prev)) => {
                    prev.push_str(", ");
                    prev.push_str(v);
                }
                _ => {
                    out.insert(k, Value::String(v.clone()));
                }
            }
        }
        Value::Object(out)
    }
}

/// Carries requests. Errors are failures to get any answer (refused host, DNS, TLS, timeout, too large).
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String>;
}

/// The answer's body as a value of `format`.
pub fn parse_body(body: &[u8], format: Format) -> Result<Value, String> {
    let text = || String::from_utf8_lossy(body);
    match format {
        Format::Json => {
            if body.iter().all(|b| b.is_ascii_whitespace()) {
                return Ok(Value::Null);
            }
            serde_json::from_slice(body).map_err(|_| "the answer is not JSON".to_string())
        }
        Format::Ndjson => text().lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).map_err(|_| "a line of the answer is not JSON".to_string())).collect::<Result<Vec<_>, _>>().map(Value::Array),
        Format::Prometheus => Ok(Value::Array(crate::formats::prometheus(&text()))),
        Format::Text => Ok(Value::String(text().into_owned())),
    }
}

/// The `rel="next"` URL of a `Link` header.
pub fn link_next(link: &str) -> Option<String> {
    for part in link.split(',') {
        let mut pieces = part.split(';');
        let target = pieces.next()?.trim();
        if pieces.any(|p| {
            let p = p.trim().to_ascii_lowercase();
            p == "rel=\"next\"" || p == "rel=next"
        }) {
            return Some(target.trim_start_matches('<').trim_end_matches('>').to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn link_headers_and_formats() {
        let link = r#"<https://api.example.com/x?page=1>; rel="prev", <https://api.example.com/x?page=3>; rel="next""#;
        assert_eq!(link_next(link).as_deref(), Some("https://api.example.com/x?page=3"));
        assert_eq!(link_next(r#"<https://a.example.com>; rel="last""#), None);
        assert_eq!(parse_body(b"", Format::Json), Ok(Value::Null));
        assert!(parse_body(b"<html>", Format::Json).is_err());
        assert_eq!(parse_body(b"{\"a\":1}\n\n{\"a\":2}\n", Format::Ndjson), Ok(json!([{ "a": 1 }, { "a": 2 }])));
        let r = HttpResponse { status: 200, headers: vec![("Set-Cookie".into(), "a=1".into()), ("set-cookie".into(), "b=2".into())], body: vec![] };
        assert_eq!(r.headers_value(), json!({ "set-cookie": "a=1, b=2" }));
    }
}
