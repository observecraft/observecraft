//! RFC 6238 TOTP (SHA-1, 30 s, 6 digits) — what authenticator apps implement.

use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;

const STEP: u64 = 30;

pub fn new_secret() -> String {
    let mut b = [0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut b);
    BASE32_NOPAD.encode(&b)
}

pub fn otpauth_url(secret: &str, account: &str) -> String {
    let label = url::form_urlencoded::byte_serialize(format!("ObserveCraft:{account}").as_bytes()).collect::<String>();
    format!("otpauth://totp/{label}?secret={secret}&issuer=ObserveCraft&algorithm=SHA1&digits=6&period={STEP}")
}

fn code_at(secret: &[u8], counter: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("hmac key");
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([h[off] & 0x7f, h[off + 1], h[off + 2], h[off + 3]]);
    bin % 1_000_000
}

/// Accepts the current code and one step either side (clock drift).
pub fn verify(secret_b32: &str, code: &str, now_unix: u64) -> bool {
    let code = code.trim().replace(' ', "");
    let Ok(want) = code.parse::<u32>() else { return false };
    if code.len() != 6 {
        return false;
    }
    let Ok(secret) = BASE32_NOPAD.decode(secret_b32.as_bytes()) else { return false };
    let t = now_unix / STEP;
    // Compare every candidate so timing doesn't reveal which window matched.
    [t.saturating_sub(1), t, t + 1].iter().fold(false, |ok, c| ok | (code_at(&secret, *c) == want))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 Appendix B test vector (SHA-1, secret "12345678901234567890").
    #[test]
    fn rfc6238_vector() {
        let secret = BASE32_NOPAD.encode(b"12345678901234567890");
        assert!(verify(&secret, "287082", 59));
        assert!(verify(&secret, "081804", 1_111_111_109));
        assert!(!verify(&secret, "000000", 59));
        assert!(!verify(&secret, "28708", 59));
    }
}
