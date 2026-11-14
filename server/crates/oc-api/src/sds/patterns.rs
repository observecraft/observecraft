//! The standard pattern library of the Sensitive Data Scanner: ObserveCraft's own patterns (MIT), served as
//! Datadog's standard patterns (`/api/v2/sensitive-data-scanner/config/standard-patterns`,
//! `datadog_sensitive_data_scanner_standard_pattern`) and used by rules that name one (`standard_pattern_id`).
//!
//! A pattern is a regular expression (the `regex` crate: linear time, no backtracking), optionally the capture
//! group that holds the sensitive part, and a check of each candidate (a checksum, a number range, the
//! characters around it) that the expression can't make by itself.

/// A candidate match: its text and the text before and after it.
pub struct Candidate<'a> {
    pub text: &'a str,
    pub before: &'a str,
    pub after: &'a str,
}

pub type Check = fn(&Candidate<'_>) -> bool;

pub struct StandardPattern {
    /// Stable id (`standard_pattern_id`).
    pub id: &'static str,
    pub name: &'static str,
    /// `sensitive_data:<tag>` on matching logs.
    pub tag: &'static str,
    /// `sensitive_data_category:<category>`.
    pub category: &'static str,
    pub description: &'static str,
    pub pattern: &'static str,
    /// The capture group holding the sensitive part (0: the whole match).
    pub group: usize,
    pub check: Option<Check>,
    /// Recommended keywords (`use_recommended_keywords`).
    pub keywords: &'static [&'static str],
    /// 1 (high) to 5 (low).
    pub priority: i64,
}

impl StandardPattern {
    pub fn tags(&self) -> Vec<String> {
        vec![format!("sensitive_data:{}", self.tag), format!("sensitive_data_category:{}", self.category)]
    }
}

/// Whether `c` may stand next to a token without being part of it.
fn apart(c: Option<char>, token: &dyn Fn(char) -> bool) -> bool {
    c.is_none_or(|c| !token(c))
}

fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The candidate stands alone: no letter, digit, `_` or `-` right before or after it.
fn standalone(c: &Candidate<'_>) -> bool {
    apart(c.before.chars().next_back(), &token_char) && apart(c.after.chars().next(), &token_char)
}

fn digits(s: &str) -> Vec<u32> {
    s.chars().filter_map(|c| c.to_digit(10)).collect()
}

fn luhn(d: &[u32]) -> bool {
    let sum: u32 = d
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &n)| {
            if i % 2 == 1 {
                if n * 2 > 9 {
                    n * 2 - 9
                } else {
                    n * 2
                }
            } else {
                n
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// A payment card number: a known issuer's prefix and length, and the Luhn checksum.
fn credit_card(c: &Candidate<'_>) -> bool {
    let d = digits(c.text);
    let s: String = d.iter().map(|n| char::from_digit(*n, 10).unwrap_or('0')).collect();
    let n = |k: usize| s.get(..k).and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
    let len = s.len();
    let issuer = (s.starts_with('4') && matches!(len, 13 | 16 | 19)) // Visa
        || (((51..=55).contains(&n(2)) || (2221..=2720).contains(&n(4))) && len == 16) // Mastercard
        || (matches!(n(2), 34 | 37) && len == 15) // American Express
        || ((n(4) == 6011 || n(2) == 65 || (644..=649).contains(&n(3))) && (16..=19).contains(&len)) // Discover
        || ((3528..=3589).contains(&n(4)) && (16..=19).contains(&len)) // JCB
        || (((300..=305).contains(&n(3)) || matches!(n(2), 36 | 38)) && (14..=19).contains(&len)) // Diners Club
        || (n(2) == 62 && (16..=19).contains(&len)); // UnionPay
    issuer && luhn(&d) && apart(c.before.chars().next_back(), &|ch| ch.is_ascii_digit() || ch == '-') && apart(c.after.chars().next(), &|ch| ch.is_ascii_digit())
}

/// An address whose top-level domain isn't a file extension (`image@2x.png`).
fn email(c: &Candidate<'_>) -> bool {
    const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "css", "js", "map", "ts", "json", "html", "pdf", "txt"];
    let tld = c.text.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    !EXTENSIONS.contains(&tld.as_str())
}

/// Random-looking: mixed-case letters (not a hex digest or a run of digits).
fn mixed_case(c: &Candidate<'_>) -> bool {
    c.text.chars().any(|ch| ch.is_ascii_lowercase()) && c.text.chars().any(|ch| ch.is_ascii_uppercase()) && apart(c.after.chars().next(), &|ch| ch.is_ascii_alphanumeric() || ch == '/' || ch == '+')
}

/// An IBAN: 15 to 34 characters and the ISO 7064 mod 97 checksum.
fn iban(c: &Candidate<'_>) -> bool {
    let s: String = c.text.chars().filter(|ch| !ch.is_whitespace()).collect();
    if !(15..=34).contains(&s.len()) || !standalone(c) {
        return false;
    }
    let mut rem: u64 = 0;
    for ch in s[4..].chars().chain(s[..4].chars()) {
        let v = match ch.to_digit(36) {
            Some(v) => v as u64,
            None => return false,
        };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

/// Four dotted octets, not part of a longer dotted number (`1.2.3.4.5`).
fn ipv4(c: &Candidate<'_>) -> bool {
    let mut b = c.before.chars().rev();
    let mut a = c.after.chars();
    let dotted_before = matches!((b.next(), b.next()), (Some('.'), Some(d)) if d.is_ascii_digit());
    let dotted_after = matches!((a.next(), a.next()), (Some('.'), Some(d)) if d.is_ascii_digit());
    !dotted_before && !dotted_after
}

/// A parsable IPv6 address standing alone (not `std::fmt`, a time or a MAC address).
fn ipv6(c: &Candidate<'_>) -> bool {
    let near = |ch: char| ch.is_ascii_alphanumeric() || ch == ':' || ch == '.' || ch == '_';
    c.text.parse::<std::net::Ipv6Addr>().is_ok()
        && c.text.chars().filter(|ch| ch.is_ascii_hexdigit()).count() >= 2
        && apart(c.before.chars().next_back(), &near)
        && apart(c.after.chars().next(), &near)
}

/// A US social security number the SSA could have issued.
fn us_ssn(c: &Candidate<'_>) -> bool {
    let mut parts = c.text.split('-');
    let (area, group, serial) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    area != "000"
        && area != "666"
        && !area.starts_with('9')
        && group != "00"
        && serial != "0000"
        && apart(c.before.chars().next_back(), &|ch| ch.is_ascii_digit() || ch == '-')
        && apart(c.after.chars().next(), &|ch| ch.is_ascii_digit() || ch == '-')
}

fn phone(c: &Candidate<'_>) -> bool {
    let n = digits(c.text).len();
    (7..=15).contains(&n) && apart(c.before.chars().next_back(), &|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '-' || ch == '.')
}

/// OpenAI's keys, not Anthropic's (`sk-ant-`).
fn openai(c: &Candidate<'_>) -> bool {
    !c.text.starts_with("sk-ant-") && standalone(c)
}

fn word_bounded(c: &Candidate<'_>) -> bool {
    apart(c.before.chars().next_back(), &word) && apart(c.after.chars().next(), &word)
}

macro_rules! pattern {
    ($id:literal, $name:literal, $category:literal, $priority:literal, $pattern:literal, $group:literal, $check:expr, [$($kw:literal),*], $description:literal) => {
        StandardPattern {
            id: $id,
            name: $name,
            tag: $id,
            category: $category,
            description: $description,
            pattern: $pattern,
            group: $group,
            check: $check,
            keywords: &[$($kw),*],
            priority: $priority,
        }
    };
}

pub const LIBRARY: &[StandardPattern] = &[
    pattern!(
        "credit_card",
        "Credit Card Number",
        "finance",
        1,
        r"\b\d(?:[ -]?\d){12,18}\b",
        0,
        Some(credit_card),
        ["card", "credit", "debit", "visa", "mastercard", "amex", "pan", "cc"],
        "Payment card numbers of the major networks (Visa, Mastercard, American Express, Discover, JCB, Diners Club, UnionPay), written with or without spaces or dashes, that pass the Luhn checksum."
    ),
    pattern!(
        "iban",
        "International Bank Account Number (IBAN)",
        "finance",
        2,
        r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){2,7}(?: ?[A-Z0-9]{1,3})?\b",
        0,
        Some(iban),
        ["iban", "account", "bank", "transfer", "sepa"],
        "International bank account numbers, grouped by four or not, that pass the mod 97 checksum."
    ),
    pattern!(
        "us_ssn",
        "US Social Security Number",
        "pii",
        1,
        r"\b\d{3}-\d{2}-\d{4}\b",
        0,
        Some(us_ssn),
        ["ssn", "social security", "social_security"],
        "US social security numbers written as AAA-GG-SSSS, without the ranges that are never issued (000, 666 and 9xx areas, 00 groups, 0000 serials)."
    ),
    pattern!(
        "email_address",
        "Email Address",
        "pii",
        3,
        r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,24}\b",
        0,
        Some(email),
        ["email", "e-mail", "mail", "address", "user"],
        "Email addresses with a domain name, leaving out image names such as image@2x.png."
    ),
    pattern!(
        "phone_number",
        "Phone Number",
        "pii",
        3,
        r"\+\d{1,3}[ .-]?(?:\(\d{1,4}\)|\d{1,4})(?:[ .-]?\d{2,4}){2,4}\b|\(\d{3}\) ?\d{3}[ .-]\d{4}\b|\b\d{3}[.-]\d{3}[.-]\d{4}\b",
        0,
        Some(phone),
        ["phone", "tel", "mobile", "cell", "fax", "call"],
        "Phone numbers in international form (+ and the country code) or written as North American numbers with separators."
    ),
    pattern!(
        "ipv4_address",
        "IPv4 Address",
        "network",
        4,
        r"\b(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\b",
        0,
        Some(ipv4),
        ["ip", "address", "client", "remote", "host"],
        "IPv4 addresses in dotted-decimal form, not part of a longer dotted version number."
    ),
    pattern!(
        "ipv6_address",
        "IPv6 Address",
        "network",
        4,
        r"[0-9A-Fa-f]{0,4}(?::[0-9A-Fa-f]{0,4}){2,7}",
        0,
        Some(ipv6),
        ["ip", "ipv6", "address", "client", "remote"],
        "IPv6 addresses in full or compressed form."
    ),
    pattern!(
        "aws_access_key_id",
        "AWS Access Key ID",
        "credentials",
        2,
        r"\b(?:AKIA|ASIA|ABIA|ACCA|AGPA|AIDA|AIPA|ANPA|ANVA|AROA|APKA)[A-Z2-7]{16}\b",
        0,
        None,
        ["aws", "access", "key", "id"],
        "AWS access key ids, long-term (AKIA) and temporary (ASIA)."
    ),
    pattern!(
        "aws_secret_access_key",
        "AWS Secret Access Key",
        "credentials",
        1,
        r#"(?i)(?:aws_?)?secret_?access_?key["']?\s*[:=]\s*["']?([A-Za-z0-9/+]{40})"#,
        1,
        Some(mixed_case),
        ["aws", "secret", "access", "key"],
        "AWS secret access keys assigned to a secret access key setting or field."
    ),
    pattern!(
        "private_key",
        "Private Key",
        "credentials",
        1,
        r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----(?:[A-Za-z0-9+/=\s:,-]*?-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----|[A-Za-z0-9+/=\s]*[A-Za-z0-9+/=])",
        0,
        None,
        ["private", "key", "pem", "rsa", "ssh"],
        "PEM private keys (RSA, EC, DSA, OpenSSH, PKCS #8, PGP), whole or cut off."
    ),
    pattern!(
        "jwt",
        "JSON Web Token",
        "credentials",
        2,
        r"\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*",
        0,
        None,
        ["jwt", "token", "bearer", "authorization"],
        "JSON Web Tokens: a JSON header and payload, Base64url encoded, and their signature."
    ),
    pattern!(
        "url_credentials",
        "Password in URL",
        "credentials",
        1,
        r"\b[A-Za-z][A-Za-z0-9+.-]*://[^\s:/@]+:([^\s@/]+)@[^\s/@]+",
        1,
        None,
        ["url", "uri", "dsn", "connection"],
        "The password of a URL's user information (scheme://user:password@host)."
    ),
    pattern!(
        "password_assignment",
        "Password Assignment",
        "credentials",
        2,
        r#"(?i)\b(?:password|passwd|pwd|pass|secret)["']?\s*[:=]\s*["']?([^\s"'&,;}]+)"#,
        1,
        None,
        ["password", "passwd", "secret"],
        "Values assigned to a password or secret field in query strings, JSON and key-value text."
    ),
    pattern!(
        "github_token",
        "GitHub Token",
        "credentials",
        1,
        r"\b(?:gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{82})\b",
        0,
        None,
        ["github", "token"],
        "GitHub personal access, OAuth, app and refresh tokens."
    ),
    pattern!(
        "gitlab_token",
        "GitLab Token",
        "credentials",
        1,
        r"\bgl(?:pat|dt|rt|ptt|cbt|oas)-[A-Za-z0-9_-]{20,}",
        0,
        Some(standalone),
        ["gitlab", "token"],
        "GitLab personal, deploy, runner, pipeline trigger and CI job tokens."
    ),
    pattern!("slack_token", "Slack Token", "credentials", 1, r"\bxox[abposre]-[A-Za-z0-9-]{10,}", 0, Some(standalone), ["slack", "token"], "Slack bot, user, app and configuration tokens."),
    pattern!(
        "slack_webhook",
        "Slack Webhook URL",
        "credentials",
        2,
        r"https://hooks\.slack\.com/(?:services|workflows|triggers)/[A-Za-z0-9/_+-]+",
        0,
        None,
        ["slack", "webhook"],
        "Slack incoming webhook and workflow trigger URLs."
    ),
    pattern!(
        "stripe_api_key",
        "Stripe Secret Key",
        "credentials",
        1,
        r"\b[rs]k_(?:live|test)_[A-Za-z0-9]{16,}\b",
        0,
        None,
        ["stripe", "key", "secret"],
        "Stripe secret and restricted API keys (publishable keys are public)."
    ),
    pattern!("google_api_key", "Google API Key", "credentials", 2, r"\bAIza[0-9A-Za-z_-]{35}", 0, Some(standalone), ["google", "gcp", "firebase", "key"], "Google Cloud and Firebase API keys."),
    pattern!("sendgrid_api_key", "SendGrid API Key", "credentials", 2, r"\bSG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}", 0, Some(standalone), ["sendgrid", "key"], "SendGrid API keys."),
    pattern!("npm_token", "npm Access Token", "credentials", 2, r"\bnpm_[A-Za-z0-9]{36}\b", 0, None, ["npm", "token", "registry"], "npm registry access tokens."),
    pattern!(
        "openai_api_key",
        "OpenAI API Key",
        "credentials",
        1,
        r"\bsk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}",
        0,
        Some(openai),
        ["openai", "key"],
        "OpenAI project, service account, admin and legacy API keys."
    ),
    pattern!(
        "anthropic_api_key",
        "Anthropic API Key",
        "credentials",
        1,
        r"\bsk-ant-(?:api|admin)\d{2}-[A-Za-z0-9_-]{20,}",
        0,
        Some(standalone),
        ["anthropic", "claude", "key"],
        "Anthropic API and admin keys."
    ),
    pattern!(
        "observecraft_key",
        "ObserveCraft Key",
        "credentials",
        1,
        r"\boc(?:ak|ik|s|l|sl|wt|ev|oa)_[A-Za-z0-9_-]{43}",
        0,
        Some(word_bounded),
        ["key", "token", "observecraft"],
        "ObserveCraft application keys, ingest keys and session tokens."
    ),
];

pub fn find(id: &str) -> Option<&'static StandardPattern> {
    LIBRARY.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    /// The sensitive parts `id` finds in `text`.
    fn hits(id: &str, text: &str) -> Vec<String> {
        let p = find(id).unwrap_or_else(|| panic!("no pattern {id}"));
        let re = Regex::new(p.pattern).unwrap();
        re.captures_iter(text)
            .filter_map(|c| c.get(p.group))
            .filter(|m| p.check.is_none_or(|check| check(&Candidate { text: m.as_str(), before: &text[..m.start()], after: &text[m.end()..] })))
            .map(|m| m.as_str().to_string())
            .collect()
    }

    fn finds(id: &str, text: &str, want: &str) {
        assert_eq!(hits(id, text), vec![want.to_string()], "{id} in {text:?}");
    }

    fn ignores(id: &str, text: &str) {
        assert!(hits(id, text).is_empty(), "{id} should ignore {text:?}: {:?}", hits(id, text));
    }

    #[test]
    fn credit_cards_pass_luhn_and_a_known_issuer() {
        finds("credit_card", "paid with 4111 1111 1111 1111 today", "4111 1111 1111 1111");
        finds("credit_card", "card=5555-5555-5555-4444;", "5555-5555-5555-4444");
        finds("credit_card", "amex 378282246310005", "378282246310005");
        finds("credit_card", "card 6011111111111117", "6011111111111117");
        ignores("credit_card", "order 4111 1111 1111 1112 failed");
        ignores("credit_card", "id 1234567812345670");
        ignores("credit_card", "trace 41111111111111110000000");
    }

    #[test]
    fn emails() {
        finds("email_address", "user alice.smith+promo@example.com logged in", "alice.smith+promo@example.com");
        finds("email_address", "<ops@mail.example.co.uk>", "ops@mail.example.co.uk");
        ignores("email_address", "image@2x.png");
        ignores("email_address", "user@localhost");
    }

    #[test]
    fn aws_keys() {
        finds("aws_access_key_id", "key=AKIAIOSFODNN7EXAMPLE", "AKIAIOSFODNN7EXAMPLE");
        finds("aws_access_key_id", "ASIAY34FZKBOKMUTVV7A expired", "ASIAY34FZKBOKMUTVV7A");
        ignores("aws_access_key_id", "AKIAIOSFODNN7EXAMPL");
        ignores("aws_access_key_id", "XAKIAIOSFODNN7EXAMPLE1");
        finds("aws_secret_access_key", "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        finds("aws_secret_access_key", "\"SecretAccessKey\": \"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        ignores("aws_secret_access_key", "checksum wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
        ignores("aws_secret_access_key", "aws_secret_access_key = 0123456789012345678901234567890123456789");
    }

    #[test]
    fn ibans_pass_mod_97() {
        finds("iban", "refund to DE89 3704 0044 0532 0130 00 please", "DE89 3704 0044 0532 0130 00");
        finds("iban", "iban=GB82WEST12345698765432", "GB82WEST12345698765432");
        ignores("iban", "iban=GB82WEST12345698765433");
        ignores("iban", "SKU AB12CDEF34567890");
    }

    #[test]
    fn json_web_tokens() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4ifQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        finds("jwt", &format!("Authorization: Bearer {jwt}"), jwt);
        ignores("jwt", "eyJhbGciOiJIUzI1NiJ9 alone");
    }

    #[test]
    fn private_keys() {
        let key = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA0Z3VS5JJcds3xfn\n-----END RSA PRIVATE KEY-----";
        finds("private_key", &format!("loaded {key} ok"), key);
        finds("private_key", "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA", "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA");
        ignores("private_key", "-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkq\n-----END PUBLIC KEY-----");
    }

    #[test]
    fn ip_addresses() {
        finds("ipv4_address", "client 203.0.113.42 connected", "203.0.113.42");
        ignores("ipv4_address", "version 1.2.3.4.5");
        ignores("ipv4_address", "addr 256.1.1.1");
        ignores("ipv4_address", "v1.2.3");
        finds("ipv6_address", "from 2001:db8:85a3::8a2e:370:7334 port 443", "2001:db8:85a3::8a2e:370:7334");
        finds("ipv6_address", "[fe80::1ff:fe23:4567:890a]", "fe80::1ff:fe23:4567:890a");
        ignores("ipv6_address", "at 12:30:45 UTC");
        ignores("ipv6_address", "mac 00:1A:2B:3C:4D:5E");
        ignores("ipv6_address", "std::fmt::Display");
    }

    #[test]
    fn us_social_security_numbers() {
        finds("us_ssn", "ssn 123-45-6789 on file", "123-45-6789");
        ignores("us_ssn", "ssn 000-45-6789");
        ignores("us_ssn", "ssn 666-45-6789");
        ignores("us_ssn", "ssn 900-45-6789");
        ignores("us_ssn", "ssn 123-00-6789");
        ignores("us_ssn", "ssn 123-45-0000");
        ignores("us_ssn", "date 2024-11-14");
    }

    #[test]
    fn phone_numbers() {
        finds("phone_number", "call +1 415-555-0132 now", "+1 415-555-0132");
        finds("phone_number", "tel (415) 555-0132", "(415) 555-0132");
        finds("phone_number", "whatsapp +442071838750", "+442071838750");
        ignores("phone_number", "ssn 123-45-6789");
        ignores("phone_number", "on 2024-11-14 12:30");
        ignores("phone_number", "order 4155550132");
    }

    #[test]
    fn saas_tokens() {
        finds("github_token", "token ghp_16C7e42F292c6912E7710c838347Ae178B4a used", "ghp_16C7e42F292c6912E7710c838347Ae178B4a");
        ignores("github_token", "ghp_short");
        finds("gitlab_token", &format!("PRIVATE-TOKEN: {}", ["gl", "pat-xYz12AbCdEfGhIjK3LmN"].concat()), &["gl", "pat-xYz12AbCdEfGhIjK3LmN"].concat());
        finds("slack_token", &["xo", "xb-1234567890-0987654321-AbCdEfGhIjKlMnOpQrStUvWx"].concat(), &["xo", "xb-1234567890-0987654321-AbCdEfGhIjKlMnOpQrStUvWx"].concat());
        finds(
            "slack_webhook",
            &format!("posting to {}", ["https://hooks.sla", "ck.com/services/T0000AAAA/B0000BBBB/XXXXXXXXXXXXXXXXXXXXXXXX"].concat()),
            &["https://hooks.sla", "ck.com/services/T0000AAAA/B0000BBBB/XXXXXXXXXXXXXXXXXXXXXXXX"].concat(),
        );
        finds("stripe_api_key", &format!("Stripe.api_key = {}", ["sk", "_live_4eC39HqLyjWDarjtT1zdp7dc"].concat()), &["sk", "_live_4eC39HqLyjWDarjtT1zdp7dc"].concat());
        ignores("stripe_api_key", &["pk", "_live_4eC39HqLyjWDarjtT1zdp7dc"].concat());
        finds("google_api_key", "key=AIzaSyA1bCdEfGhIjKlMnOpQrStUvWxYz012345", "AIzaSyA1bCdEfGhIjKlMnOpQrStUvWxYz012345");
        finds("sendgrid_api_key", &["SG", ".ngeVfQFYQlKU0ufo8x5d1A.TwL2iGABf9DHoTf-09kqeF8tAmbihYzrnopKc-1s5cr"].concat(), &["SG", ".ngeVfQFYQlKU0ufo8x5d1A.TwL2iGABf9DHoTf-09kqeF8tAmbihYzrnopKc-1s5cr"].concat());
        finds("npm_token", "//registry.npmjs.org/:_authToken=npm_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8", "npm_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
        finds("openai_api_key", "OPENAI_API_KEY=sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789", "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789");
        ignores("openai_api_key", &["sk", "_live_4eC39HqLyjWDarjtT1zdp7dc"].concat());
        let ant = format!("sk-ant-api03-{}", "Ab1_".repeat(20));
        finds("anthropic_api_key", &format!("x-api-key: {ant}"), &ant);
        ignores("openai_api_key", &ant);
        let key = format!("ocak_{}", "A".repeat(43));
        finds("observecraft_key", &format!("DD-APPLICATION-KEY: {key}"), &key);
        ignores("observecraft_key", "ocak_short");
    }

    #[test]
    fn credentials_in_urls() {
        finds("url_credentials", "connecting to postgres://app:s3cr3t-pw@db.example.com:5432/orders", "s3cr3t-pw");
        ignores("url_credentials", "see https://example.com/a:b@c");
        ignores("url_credentials", "https://example.com/path");
        finds("password_assignment", "login failed password=hunter22&user=bob", "hunter22");
        finds("password_assignment", "{\"secret\": \"t0ps3cret!\"}", "t0ps3cret!");
        ignores("password_assignment", "password reset requested for bob");
    }

    #[test]
    fn the_library_is_consistent() {
        let mut ids = std::collections::HashSet::new();
        for p in LIBRARY {
            assert!(ids.insert(p.id), "duplicate id {}", p.id);
            let re = Regex::new(p.pattern).unwrap_or_else(|e| panic!("{}: {e}", p.id));
            assert!(p.group < re.captures_len(), "{}: no group {}", p.id, p.group);
            assert!((1..=5).contains(&p.priority), "{}", p.id);
            assert!(["credentials", "finance", "pii", "network"].contains(&p.category), "{}", p.id);
            assert!(!p.description.is_empty() && !p.name.is_empty(), "{}", p.id);
        }
        assert!(LIBRARY.len() >= 20);
    }
}
