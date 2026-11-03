//! Assist: the AI assistant. A chat over the org's telemetry where the model calls tools (the same
//! handlers as the UI, as the signed-in user) and asks before changing anything.
//! The model is Anthropic's or any OpenAI-compatible endpoint: the organization's own (Organization
//! Settings → Assist AI) or else the install's (`OC_ASSIST_*`).
pub mod agent;
pub mod code;
pub mod http;
pub mod llm;
pub mod prompt;
pub mod runs;
pub mod tools;
#[cfg(test)]
pub mod testing;

use llm::Provider;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The longest API key accepted.
const MAX_KEY: usize = 4096;

/// An organization's model, as its admins set it (the key travels apart: it is sealed, never shown).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = AssistProviderSettings)]
pub struct ProviderSettings {
    /// `anthropic` or `openai` (any OpenAI-compatible server).
    pub provider: String,
    /// Required for `openai`; Anthropic defaults to its current model.
    pub model: Option<String>,
    /// Required for `openai` (e.g. `https://api.openai.com/v1`); Anthropic defaults to its API.
    pub base_url: Option<String>,
    /// Anthropic only: let the API answer with a fallback model when the chosen one is overloaded.
    pub fallbacks: Option<bool>,
}

/// An organization's provider from its settings. Its base URL must be https and public, unless the
/// install allows private targets (`OC_ASSIST_ALLOW_PRIVATE`, a model on the same network); names are
/// resolved again at each request ([`http::Reach::Public`]).
pub fn org_provider(s: &ProviderSettings, api_key: Option<String>, allow_private: bool) -> Result<Provider, String> {
    let text = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let api_key = text(&api_key);
    if api_key.as_ref().is_some_and(|k| k.len() > MAX_KEY) {
        return Err(format!("The API key is longer than {MAX_KEY} characters"));
    }
    let base_url = text(&s.base_url).map(|u| check_base_url(&u, allow_private)).transpose()?;
    match s.provider.trim().to_ascii_lowercase().as_str() {
        "anthropic" => Ok(Provider::Anthropic {
            base_url: base_url.unwrap_or_else(|| "https://api.anthropic.com".into()),
            api_key: api_key.ok_or("Anthropic needs an API key")?,
            model: text(&s.model).unwrap_or_else(|| llm::ANTHROPIC_MODEL.into()),
            fallbacks: s.fallbacks.unwrap_or(true),
        }),
        "openai" => Ok(Provider::OpenAi {
            base_url: base_url.ok_or("An OpenAI-compatible provider needs its base URL (e.g. https://api.openai.com/v1)")?,
            api_key,
            model: text(&s.model).ok_or("An OpenAI-compatible provider needs a model name")?,
        }),
        other => Err(format!("The provider must be anthropic or openai, not {other}")),
    }
}

/// The SSRF guard's refusal, naming Assist's own switch.
pub fn private_target(e: String) -> String {
    e.replace("OC_SYNTHETICS_ALLOW_PRIVATE", "OC_ASSIST_ALLOW_PRIVATE")
}

/// `https://host[:port]/path`, without credentials or a trailing slash; an IP literal must be public.
fn check_base_url(raw: &str, allow_private: bool) -> Result<String, String> {
    let url = url::Url::parse(raw).map_err(|e| format!("Invalid base URL: {e}"))?;
    match url.scheme() {
        "https" => {}
        "http" if allow_private => {}
        _ => return Err("The base URL must be https".into()),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Put the key in the API key field, not in the base URL".into());
    }
    let ip: Option<std::net::IpAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => Some(ip.into()),
        Some(url::Host::Ipv6(ip)) => Some(ip.into()),
        Some(url::Host::Domain(_)) => None,
        None => return Err("The base URL has no host".into()),
    };
    if let Some(ip) = ip {
        crate::routes::synthetics_net::check_addr(ip, allow_private).map_err(private_target)?;
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// The install's model, from `OC_ASSIST_PROVIDER` (`anthropic` | `openai`), `OC_ASSIST_API_KEY`,
/// `OC_ASSIST_MODEL`, `OC_ASSIST_BASE_URL` and `OC_ASSIST_FALLBACKS`. `None` when no provider is set
/// (Assist then says it isn't configured).
pub fn provider_from(var: impl Fn(&str) -> Option<String>) -> Result<Option<Provider>, String> {
    let get = |k: &str| var(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let Some(kind) = get("OC_ASSIST_PROVIDER") else { return Ok(None) };
    match kind.to_ascii_lowercase().as_str() {
        "anthropic" => Ok(Some(Provider::Anthropic {
            base_url: get("OC_ASSIST_BASE_URL").unwrap_or_else(|| "https://api.anthropic.com".into()),
            api_key: get("OC_ASSIST_API_KEY").ok_or("OC_ASSIST_API_KEY is required for the anthropic provider")?,
            model: get("OC_ASSIST_MODEL").unwrap_or_else(|| llm::ANTHROPIC_MODEL.into()),
            fallbacks: get("OC_ASSIST_FALLBACKS").is_none_or(|v| v != "false"),
        })),
        "openai" => Ok(Some(Provider::OpenAi {
            base_url: get("OC_ASSIST_BASE_URL").ok_or("OC_ASSIST_BASE_URL is required for the openai provider (e.g. https://api.openai.com/v1)")?,
            api_key: get("OC_ASSIST_API_KEY"),
            model: get("OC_ASSIST_MODEL").ok_or("OC_ASSIST_MODEL is required for the openai provider")?,
        })),
        other => Err(format!("OC_ASSIST_PROVIDER must be anthropic or openai, not {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn from(vars: &[(&str, &str)]) -> Result<Option<Provider>, String> {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        provider_from(|k| vars.get(k).cloned())
    }

    #[test]
    fn the_provider_comes_from_the_environment() {
        assert_eq!(from(&[]), Ok(None));
        assert_eq!(
            from(&[("OC_ASSIST_PROVIDER", "anthropic"), ("OC_ASSIST_API_KEY", "sk-ant")]),
            Ok(Some(Provider::Anthropic { base_url: "https://api.anthropic.com".into(), api_key: "sk-ant".into(), model: "claude-opus-5".into(), fallbacks: true }))
        );
        assert!(matches!(from(&[("OC_ASSIST_PROVIDER", "anthropic"), ("OC_ASSIST_API_KEY", "k"), ("OC_ASSIST_FALLBACKS", "false")]), Ok(Some(Provider::Anthropic { fallbacks: false, .. }))));
        assert!(from(&[("OC_ASSIST_PROVIDER", "anthropic")]).unwrap_err().contains("OC_ASSIST_API_KEY"));
        assert_eq!(
            from(&[("OC_ASSIST_PROVIDER", "OpenAI"), ("OC_ASSIST_BASE_URL", "http://vllm:8000/v1"), ("OC_ASSIST_MODEL", "qwen")]),
            Ok(Some(Provider::OpenAi { base_url: "http://vllm:8000/v1".into(), api_key: None, model: "qwen".into() }))
        );
        assert!(from(&[("OC_ASSIST_PROVIDER", "openai"), ("OC_ASSIST_MODEL", "m")]).unwrap_err().contains("OC_ASSIST_BASE_URL"));
        assert!(from(&[("OC_ASSIST_PROVIDER", "gemini")]).is_err());
    }

    fn settings(provider: &str, model: Option<&str>, base_url: Option<&str>) -> ProviderSettings {
        ProviderSettings { provider: provider.into(), model: model.map(Into::into), base_url: base_url.map(Into::into), fallbacks: None }
    }

    #[test]
    fn an_org_sets_its_own_provider() {
        assert_eq!(
            org_provider(&settings("anthropic", None, None), Some("sk-org".into()), false),
            Ok(Provider::Anthropic { base_url: "https://api.anthropic.com".into(), api_key: "sk-org".into(), model: "claude-opus-5".into(), fallbacks: true })
        );
        assert_eq!(
            org_provider(&settings("openai", Some("gpt-5"), Some("https://api.openai.com/v1/")), None, false),
            Ok(Provider::OpenAi { base_url: "https://api.openai.com/v1".into(), api_key: None, model: "gpt-5".into() })
        );
        assert!(org_provider(&settings("anthropic", None, None), None, false).unwrap_err().contains("API key"));
        assert!(org_provider(&settings("openai", Some("m"), None), None, false).unwrap_err().contains("base URL"));
        assert!(org_provider(&settings("openai", None, Some("https://llm.example.com/v1")), None, false).unwrap_err().contains("model"));
        assert!(org_provider(&settings("gemini", None, None), Some("k".into()), false).is_err());
        assert!(org_provider(&settings("anthropic", None, None), Some("k".repeat(5000)), false).is_err());
    }

    #[test]
    fn an_org_provider_cant_point_inside_the_install() {
        for url in ["http://llm.example.com/v1", "https://127.0.0.1/v1", "https://169.254.169.254/latest", "https://[::1]:8000/v1", "https://10.0.0.5/v1", "ftp://llm.example.com", "not a url", "https://user:pw@llm.example.com/v1"] {
            assert!(org_provider(&settings("openai", Some("m"), Some(url)), None, false).is_err(), "{url}");
        }
        // Allowed when the install permits private targets (self-hosted models on the same network).
        assert!(org_provider(&settings("openai", Some("m"), Some("http://10.0.0.5:8000/v1")), None, true).is_ok());
    }

    #[test]
    fn the_api_key_never_shows_in_debug_output() {
        let p = from(&[("OC_ASSIST_PROVIDER", "anthropic"), ("OC_ASSIST_API_KEY", "sk-ant-secret")]).unwrap().unwrap();
        let shown = format!("{p:?}");
        assert!(!shown.contains("sk-ant-secret") && shown.contains("claude-opus-5"), "{shown}");
    }
}
