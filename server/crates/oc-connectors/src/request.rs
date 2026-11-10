//! A manifest [`Request`] rendered into an [`HttpRequest`] in a scope (the templates' variables).

use serde_json::{Map, Value};

use crate::expr::text;
use crate::http::HttpRequest;
use crate::manifest::{Method, Request};
use crate::template::{Template, Tree};

/// Renders `req`: the URL, the query (null and "" values left out, lists repeated), the headers (`extra` first, then
/// the request's own), and a JSON or form body.
pub fn build(req: &Request, scope: &Map<String, Value>, extra: &[(String, String)]) -> Result<HttpRequest, String> {
    let url = Template::parse(&req.url)?.text(scope);
    let mut parsed = url::Url::parse(&url).map_err(|e| format!("the URL {url:?} is not valid: {e}"))?;
    if !matches!(parsed.scheme(), "https" | "http") {
        return Err(format!("the URL {url:?} is not http(s)"));
    }
    let mut pairs: Vec<(String, String)> = Vec::new();
    for (k, v) in &req.query {
        match Tree::parse(v)?.render(scope, false) {
            Value::Array(items) => pairs.extend(items.iter().map(text).filter(|s| !s.is_empty()).map(|s| (k.clone(), s))),
            other => {
                let s = text(&other);
                if !s.is_empty() {
                    pairs.push((k.clone(), s));
                }
            }
        }
    }
    if !pairs.is_empty() {
        parsed.query_pairs_mut().extend_pairs(&pairs);
    }
    let mut out = HttpRequest { method: req.method, url: parsed.to_string(), headers: Vec::new(), body: None, tls: None };
    for (k, v) in extra {
        out.set_header(k, v.clone());
    }
    for (k, v) in &req.headers {
        let value = Template::parse(v)?.text(scope);
        if !value.is_empty() {
            out.set_header(k, value);
        }
    }
    if let Some(body) = &req.body {
        let rendered = Tree::parse(body)?.render(scope, false);
        out.body = Some(serde_json::to_vec(&rendered).map_err(|e| e.to_string())?);
        if out.header("content-type").is_none() {
            out.set_header("Content-Type", "application/json".into());
        }
    } else if let Some(form) = &req.form {
        let mut ser = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in form {
            let value = Template::parse(v)?.text(scope);
            if !value.is_empty() {
                ser.append_pair(k, &value);
            }
        }
        out.body = Some(ser.finish().into_bytes());
        out.set_header("Content-Type", "application/x-www-form-urlencoded".into());
    }
    if out.header("accept").is_none() {
        out.set_header("Accept", "application/json".into());
    }
    if out.method == Method::Get && out.body.is_some() {
        return Err("a GET request can't have a body".into());
    }
    Ok(out)
}

/// The host (and port) of a URL, to keep pagination on the first request's host.
pub fn origin(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    Some(format!("{}://{}:{}", u.scheme(), u.host_str()?, u.port_or_known_default()?))
}

/// A next page's URL, relative to the request's.
pub fn resolve(base: &str, next: &str) -> Option<String> {
    url::Url::parse(base).ok()?.join(next).ok().map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_render_their_url_query_headers_and_body() {
        let req: Request = serde_yaml_ng::from_str(
            "method: POST\nurl: '{{ config.base_url }}/api/search'\nquery: { since: '{{ window.start | rfc3339 }}', empty: '{{ page.cursor }}', tag: ['a', 'b'] }\nheaders: { X-Tenant: '{{ config.tenant }}' }\nbody: { query: 'q', variables: { first: 100, after: '{{ page.cursor }}' } }",
        )
        .unwrap();
        let scope = json!({ "config": { "base_url": "https://api.example.com", "tenant": "t1" }, "window": { "start": 1_793_959_200_000i64 }, "page": {} });
        let r = build(&req, scope.as_object().unwrap(), &[("User-Agent".into(), "x".into())]).unwrap();
        assert_eq!(r.url, "https://api.example.com/api/search?since=2026-11-06T10%3A00%3A00.000Z&tag=a&tag=b");
        assert_eq!(r.header("x-tenant"), Some("t1"));
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(serde_json::from_slice::<Value>(r.body.as_ref().unwrap()).unwrap(), json!({ "query": "q", "variables": { "first": 100, "after": null } }));
        let bad: Request = serde_yaml_ng::from_str("url: 'ftp://x.example.com'").unwrap();
        assert!(build(&bad, &Map::new(), &[]).unwrap_err().contains("http"));
        assert_eq!(resolve("https://api.example.com/v1/x?page=1", "/v1/x?page=2").as_deref(), Some("https://api.example.com/v1/x?page=2"));
        assert_eq!(origin("https://api.example.com/x"), origin("https://api.example.com:443/y"));
    }
}
