//! Password hashing (Argon2id, OWASP-recommended parameters) and policy.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use std::sync::OnceLock;

pub const MIN_LEN: usize = 12;
pub const MAX_LEN: usize = 128;

pub fn check_policy(p: &str) -> Result<(), String> {
    let n = p.chars().count();
    if n < MIN_LEN {
        return Err(format!("password must be at least {MIN_LEN} characters"));
    }
    if n > MAX_LEN {
        return Err(format!("password must be at most {MAX_LEN} characters"));
    }
    if p.chars().all(|c| c == p.chars().next().unwrap_or(' ')) {
        return Err("password must not be a single repeated character".into());
    }
    Ok(())
}

pub fn hash(p: &str) -> anyhow::Result<String> {
    // A random salt from the operating system's generator, chosen by argon2.
    Ok(Argon2::default().hash_password(p.as_bytes()).map_err(|e| anyhow::anyhow!("hash password: {e}"))?.to_string())
}

pub fn verify(p: &str, stored: &str) -> bool {
    PasswordHash::new(stored).map(|h| Argon2::default().verify_password(p.as_bytes(), &h).is_ok()).unwrap_or(false)
}

/// Verifies against a fixed hash so unknown accounts take as long as known ones (no user enumeration by timing).
pub fn verify_dummy(p: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let h = DUMMY.get_or_init(|| hash("dummy-password-for-timing").unwrap_or_default());
    let _ = verify(p, h);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let h = hash("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify("correct horse battery", &h));
        assert!(!verify("wrong horse battery", &h));
        assert!(!verify("x", "not-a-hash"));
    }

    #[test]
    fn hashes_stored_by_earlier_versions_still_verify() {
        // The Argon2id reference known-answer vector (password "password", salt "somesalt"), as PHC string.
        let stored = "$argon2id$v=19$m=256,t=2,p=1$c29tZXNhbHQ$nf65EOgLrQMR/uIPnA4rEsF5h7TKyQwu9U1bMCHGi/4";
        assert!(verify("password", stored));
        assert!(!verify("passwore", stored));
    }

    #[test]
    fn policy() {
        assert!(check_policy("short").is_err());
        assert!(check_policy("aaaaaaaaaaaaaa").is_err());
        assert!(check_policy("a reasonable passphrase").is_ok());
    }
}
