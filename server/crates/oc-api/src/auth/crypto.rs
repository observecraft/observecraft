//! Token generation, hashing and at-rest encryption.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// 256-bit random token, URL-safe, with a readable prefix (`ocs_` session, `ocak_` app key, …).
pub fn random_token(prefix: &str) -> String {
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(b))
}

/// Tokens are stored only as SHA-256: a database leak doesn't yield usable sessions or keys.
pub fn sha256(s: &str) -> Vec<u8> {
    Sha256::digest(s.as_bytes()).to_vec()
}

/// AES-256-GCM for secrets the server must read back (TOTP seeds, OIDC client secrets).
#[derive(Clone)]
pub struct SecretBox(Aes256Gcm);

impl SecretBox {
    /// The key is SHA-256 of `OC_SECRET_KEY`, so any sufficiently long random string works.
    pub fn new(secret: &str) -> Self {
        let key = Sha256::digest(secret.as_bytes());
        SecretBox(Aes256Gcm::new_from_slice(&key).expect("32-byte key"))
    }

    pub fn encrypt(&self, plaintext: &str) -> String {
        let mut nonce = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self.0.encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes()).expect("aes-gcm encrypt");
        let mut out = nonce.to_vec();
        out.extend(ct);
        format!("v1:{}", STANDARD.encode(out))
    }

    pub fn decrypt(&self, sealed: &str) -> anyhow::Result<String> {
        let raw = STANDARD.decode(sealed.strip_prefix("v1:").ok_or_else(|| anyhow::anyhow!("unknown secret format"))?)?;
        anyhow::ensure!(raw.len() > 12, "sealed secret too short");
        let (nonce, ct) = raw.split_at(12);
        let pt = self.0.decrypt(Nonce::from_slice(nonce), ct).map_err(|_| anyhow::anyhow!("secret cannot be decrypted (was OC_SECRET_KEY changed?)"))?;
        Ok(String::from_utf8(pt)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_box_round_trip_and_tamper() {
        let b = SecretBox::new("test-key");
        let s = b.encrypt("JBSWY3DPEHPK3PXP");
        assert_eq!(b.decrypt(&s).unwrap(), "JBSWY3DPEHPK3PXP");
        assert!(SecretBox::new("other").decrypt(&s).is_err());
        let mut t = s.clone();
        t.replace_range(t.len() - 4.., "AAAA");
        assert!(b.decrypt(&t).is_err());
    }

    #[test]
    fn tokens_are_unique_and_prefixed() {
        let a = random_token("ocs_");
        assert!(a.starts_with("ocs_") && a.len() > 40);
        assert_ne!(a, random_token("ocs_"));
    }
}
