//! The provider over HTTP: one streamed request per model turn.
use std::sync::OnceLock;
use std::time::Duration;

use super::agent::Model;
use super::llm::{Msg, Provider, SseParser, ToolSpec, Turn};

pub struct HttpModel {
    pub provider: Provider,
    pub max_tokens: u32,
    pub reach: Reach,
}

/// Where the provider's address comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reach {
    /// The install's own configuration.
    Trusted,
    /// An organization's: the host is resolved and checked at each request and the connection pinned to
    /// that address (no DNS rebinding), without redirects.
    Public { allow_private: bool },
}

/// The client for one request.
async fn client_for(provider: &Provider, reach: Reach) -> Result<reqwest::Client, String> {
    let Reach::Public { allow_private } = reach else { return Ok(client().clone()) };
    let url = url::Url::parse(provider.base_url()).map_err(|e| format!("Invalid base URL: {e}"))?;
    let host = url.host_str().ok_or("The base URL has no host")?.to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let addr = crate::routes::synthetics_net::resolve(&host, port, allow_private).await.map_err(super::private_target)?;
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(host.trim_start_matches('[').trim_end_matches(']'), addr)
        .build()
        .map_err(|e| e.to_string())
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    // No overall timeout: a turn streams for as long as the model writes.
    CLIENT.get_or_init(|| reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().expect("HTTP client"))
}

/// The response body of a failed request, short.
async fn error_text(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().or(v["message"].as_str()).map(str::to_string))
        .unwrap_or_else(|| body.chars().take(300).collect());
    format!("The model provider answered {status}: {message}")
}

impl Model for HttpModel {
    async fn turn(&self, system: &str, tools: &[ToolSpec], msgs: &[Msg], on_text: &mut (dyn FnMut(&str) + Send)) -> Result<Turn, String> {
        let r = self.provider.request(system, tools, msgs, self.max_tokens);
        let mut req = client_for(&self.provider, self.reach).await?.post(&r.url).json(&r.body);
        for (k, v) in &r.headers {
            req = req.header(*k, v);
        }
        let mut resp = req.send().await.map_err(|e| format!("Can't reach the model provider: {e}"))?;
        if !resp.status().is_success() {
            return Err(error_text(resp).await);
        }
        let mut parser = SseParser::default();
        let mut stream = self.provider.stream();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("The model stream broke: {e}"))? {
            for (event, data) in parser.feed(&chunk) {
                if let Some(text) = stream.event(&event, &data)? {
                    on_text(&text);
                }
            }
        }
        stream.finish()
    }
}
