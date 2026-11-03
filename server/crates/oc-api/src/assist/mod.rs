//! Assist: the AI assistant. A chat over the org's telemetry where the model calls tools (the same
//! handlers as the UI, as the signed-in user) and asks before changing anything.
//! The model is Anthropic's or any OpenAI-compatible endpoint, set per install (`OC_ASSIST_*`).
pub mod agent;
pub mod http;
pub mod llm;
pub mod prompt;
pub mod tools;

use llm::Provider;

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

    #[test]
    fn the_api_key_never_shows_in_debug_output() {
        let p = from(&[("OC_ASSIST_PROVIDER", "anthropic"), ("OC_ASSIST_API_KEY", "sk-ant-secret")]).unwrap().unwrap();
        let shown = format!("{p:?}");
        assert!(!shown.contains("sk-ant-secret") && shown.contains("claude-opus-5"), "{shown}");
    }
}
