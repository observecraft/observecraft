//! Signed links for buttons that act without signing in (On-Call's Acknowledge and Resolve).
//!
//! A token is `base64url(claims JSON).base64url(HMAC-SHA256)` keyed with the server secret
//! (`OC_SECRET_KEY`) over a purpose prefix and the claims. The claims name the org, the page and the
//! responder, so the token alone says who acts on what (tenancy comes from it, never from the
//! request); it expires after 24 h and its nonce is recorded when used (single use, see
//! `routes/on_call_links.rs`).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub(crate) const TTL_S: i64 = 24 * 3600;
const PURPOSE: &[u8] = b"observecraft.email-action.v1.";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Acknowledge,
    Resolve,
}

impl Action {
    pub(crate) fn verb(self) -> &'static str {
        match self {
            Action::Acknowledge => "Acknowledge",
            Action::Resolve => "Resolve",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub(crate) struct Claims {
    pub org: String,
    pub page: i64,
    /// The responder the email was sent to: the actor.
    pub user: String,
    pub action: Action,
    /// Expiry, Unix seconds.
    pub exp: i64,
    pub nonce: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LinkError {
    Invalid,
    Expired,
}

fn mac(secret: &str, payload: &str) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(secret.as_bytes()).expect("hmac key");
    m.update(PURPOSE);
    m.update(payload.as_bytes());
    m
}

pub(crate) fn issue(secret: &str, org: &str, page: i64, user: &str, action: Action, now_s: i64) -> String {
    let claims = Claims { org: org.into(), page, user: user.into(), action, exp: now_s + TTL_S, nonce: crate::auth::crypto::random_token("") };
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap_or_default());
    let sig = URL_SAFE_NO_PAD.encode(mac(secret, &payload).finalize().into_bytes());
    format!("{payload}.{sig}")
}

pub(crate) fn verify(secret: &str, token: &str, now_s: i64) -> Result<Claims, LinkError> {
    let (payload, sig) = token.split_once('.').ok_or(LinkError::Invalid)?;
    let sig = URL_SAFE_NO_PAD.decode(sig).map_err(|_| LinkError::Invalid)?;
    mac(secret, payload).verify_slice(&sig).map_err(|_| LinkError::Invalid)?;
    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).map_err(|_| LinkError::Invalid)?).map_err(|_| LinkError::Invalid)?;
    if now_s >= claims.exp {
        return Err(LinkError::Expired);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "test-secret-key-0123456789";

    #[test]
    fn a_token_names_org_page_responder_and_action() {
        let t = issue(KEY, "org-a", 42, "user-1", Action::Acknowledge, 1_000);
        let c = verify(KEY, &t, 1_001).unwrap();
        assert_eq!((c.org.as_str(), c.page, c.user.as_str(), c.action, c.exp), ("org-a", 42, "user-1", Action::Acknowledge, 1_000 + TTL_S));
        // Two links never share a nonce.
        assert_ne!(verify(KEY, &issue(KEY, "org-a", 42, "user-1", Action::Acknowledge, 1_000), 1_001).unwrap().nonce, c.nonce);
    }

    #[test]
    fn expired_after_a_day() {
        let t = issue(KEY, "org-a", 42, "user-1", Action::Resolve, 1_000);
        assert!(verify(KEY, &t, 1_000 + TTL_S - 1).is_ok());
        assert_eq!(verify(KEY, &t, 1_000 + TTL_S), Err(LinkError::Expired));
    }

    #[test]
    fn tampering_with_org_page_or_action_breaks_the_signature() {
        let t = issue(KEY, "org-a", 42, "user-1", Action::Acknowledge, 1_000);
        let (payload, sig) = t.split_once('.').unwrap();
        let c: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        let changes: [fn(&mut Claims); 4] = [|c| c.org = "org-b".into(), |c| c.page = 43, |c| c.action = Action::Resolve, |c| c.exp += 3600];
        for change in changes {
            let mut forged = c.clone();
            change(&mut forged);
            let p = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());
            assert_eq!(verify(KEY, &format!("{p}.{sig}"), 1_001), Err(LinkError::Invalid));
        }
        // Another install's key, or garbage.
        assert_eq!(verify("another-key", &t, 1_001), Err(LinkError::Invalid));
        assert_eq!(verify(KEY, "nonsense", 1_001), Err(LinkError::Invalid));
        assert_eq!(verify(KEY, "a.b", 1_001), Err(LinkError::Invalid));
    }
}
