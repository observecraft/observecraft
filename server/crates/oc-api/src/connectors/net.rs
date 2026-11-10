//! The transport connector packages' requests go through, guarded as the Developer Portal connectors' are
//! (`routes::synthetics_net`): the host is resolved once and refused when it is internal (unless the operator sets
//! `OC_PORTAL_ALLOW_PRIVATE=true`), the connection goes to that checked address, redirects aren't followed, answers
//! are capped at [`MAX_ANSWER_BYTES`] and every request times out after [`TIMEOUT_SECS`]. Only https is called
//! (http too when private hosts are allowed, for internal servers).

use std::time::Duration;

use oc_connectors::http::{HttpRequest, HttpResponse, Transport};
use oc_connectors::manifest::Method;

use crate::routes::synthetics_net::resolve;

/// The largest answer read (a whole audit log in one list, an export chunk).
pub const MAX_ANSWER_BYTES: usize = 32 << 20;
pub const TIMEOUT_SECS: u64 = 30;

pub struct Net {
    pub allow_private: bool,
}

fn describe(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        let text = s.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = s.source();
    }
    out
}

#[async_trait::async_trait]
impl Transport for Net {
    async fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        let url = url::Url::parse(&req.url).map_err(|e| format!("Invalid URL: {e}"))?;
        if !(url.scheme() == "https" || (url.scheme() == "http" && self.allow_private)) {
            return Err("Only https URLs can be called".into());
        }
        let host = url.host_str().ok_or("The URL has no host")?.to_string();
        let port = url.port_or_known_default().ok_or("The URL has no port")?;
        let addr = resolve(&host, port, self.allow_private).await?;
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(TIMEOUT_SECS))
            .timeout(Duration::from_secs(TIMEOUT_SECS));
        if matches!(url.host(), Some(url::Host::Domain(_))) {
            builder = builder.resolve(&host, addr);
        }
        if let Some(tls) = &req.tls {
            let pem = format!("{}\n{}", tls.key.trim(), tls.certificate.trim());
            let identity = reqwest::Identity::from_pem(pem.as_bytes()).map_err(|_| "The client certificate or its key is not valid PEM".to_string())?;
            builder = builder.identity(identity);
        }
        let client = builder.build().map_err(|e| describe(&e))?;
        let method = match req.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
        };
        let mut rb = client.request(method, url);
        for (k, v) in &req.headers {
            rb = rb.header(k, v);
        }
        if let Some(body) = &req.body {
            rb = rb.body(body.clone());
        }
        let mut resp = rb.send().await.map_err(|e| describe(&e))?;
        let status = resp.status().as_u16();
        let headers = resp.headers().iter().map(|(k, v)| (k.as_str().to_ascii_lowercase(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| describe(&e))? {
            if body.len() + chunk.len() > MAX_ANSWER_BYTES {
                return Err(format!("The answer is larger than {} MB", MAX_ANSWER_BYTES >> 20));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse { status, headers, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn internal_hosts_are_refused_unless_allowed() {
        let net = Net { allow_private: false };
        for url in ["https://127.0.0.1/x", "https://169.254.169.254/latest/meta-data", "http://api.example.com/x"] {
            let req = HttpRequest { method: Method::Get, url: url.into(), headers: vec![], body: None, tls: None };
            assert!(net.send(&req).await.is_err(), "{url}");
        }
    }
}
