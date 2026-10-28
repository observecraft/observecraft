//! Where synthetic tests may connect. The server fetches URLs and opens connections that users
//! choose, so every target is resolved first and refused when any of its addresses is loopback,
//! private, link-local (cloud metadata at 169.254.169.254 included), unique-local, unspecified,
//! multicast or otherwise not publicly routable — unless the operator allows private targets
//! (`OC_SYNTHETICS_ALLOW_PRIVATE=true`). The checked address is the one connected to (no second
//! lookup a DNS rebinding could change), and HTTP redirects are checked again hop by hop.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Why `ip` is not a public address, or `None` when it is.
pub fn forbidden_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => forbidden_v6(v6),
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> Option<&'static str> {
    let [a, b, c, _] = ip.octets();
    if ip.is_unspecified() || a == 0 {
        Some("an unspecified address")
    } else if ip.is_loopback() {
        Some("a loopback address")
    } else if ip.is_private() {
        Some("a private address")
    } else if ip.is_link_local() {
        Some("a link-local address")
    } else if a == 100 && (b & 0xc0) == 64 {
        Some("a shared (carrier-grade NAT) address")
    } else if ip.is_multicast() {
        Some("a multicast address")
    } else if ip.is_broadcast() || a >= 240 || (a == 192 && b == 0 && c == 0) {
        Some("a reserved address")
    } else if a == 198 && (b & 0xfe) == 18 {
        Some("a reserved (benchmarking) address")
    } else {
        None
    }
}

fn forbidden_v6(ip: Ipv6Addr) -> Option<&'static str> {
    let s = ip.segments();
    // Addresses that carry an IPv4 address are judged by it: mapped (::ffff:a.b.c.d), compatible
    // (::a.b.c.d), NAT64 (64:ff9b::a.b.c.d) and 6to4 (2002:aabb:ccdd::).
    let embedded = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    if let Some(v4) = ip.to_ipv4_mapped() {
        return forbidden_v4(v4);
    }
    if ip.is_unspecified() {
        return Some("an unspecified address");
    }
    if ip.is_loopback() {
        return Some("a loopback address");
    }
    if s[..6] == [0; 6] {
        return forbidden_v4(embedded(s[6], s[7])).or(Some("a reserved address"));
    }
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0; 4] {
        return forbidden_v4(embedded(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        return forbidden_v4(embedded(s[1], s[2]));
    }
    match s[0] {
        x if (x & 0xfe00) == 0xfc00 => Some("a unique-local address"),
        x if (x & 0xffc0) == 0xfe80 => Some("a link-local address"),
        x if (x & 0xffc0) == 0xfec0 => Some("a site-local address"),
        x if (x & 0xff00) == 0xff00 => Some("a multicast address"),
        0x0100 if s[1..4] == [0; 3] => Some("a discard-only address"),
        _ => None,
    }
}

/// Checks one address against the policy.
pub fn check_addr(ip: IpAddr, allow_private: bool) -> Result<(), String> {
    match forbidden_reason(ip) {
        Some(reason) if !allow_private => Err(format!("{ip} is {reason}; private targets are disabled on this server (OC_SYNTHETICS_ALLOW_PRIVATE)")),
        _ => Ok(()),
    }
}

/// Picks the address to connect to among a host's addresses: every one must pass the policy (a name
/// that also resolves to an internal address is refused as a whole), and IPv4 is preferred.
pub fn pick(addrs: &[SocketAddr], allow_private: bool) -> Result<SocketAddr, String> {
    if addrs.is_empty() {
        return Err("The host has no address".into());
    }
    for a in addrs {
        check_addr(a.ip(), allow_private)?;
    }
    Ok(addrs.iter().find(|a| a.is_ipv4()).copied().unwrap_or(addrs[0]))
}

/// Resolves `host` (a name or an IP literal, IPv6 with or without brackets) and returns the checked
/// address to connect to on `port`.
pub async fn resolve(host: &str, port: u16, allow_private: bool) -> Result<SocketAddr, String> {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        check_addr(ip, allow_private)?;
        return Ok(SocketAddr::new(ip, port));
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((bare, port)).await.map_err(|e| format!("DNS lookup of {bare} failed: {e}"))?.collect();
    pick(&addrs, allow_private).map_err(|e| format!("{bare} resolves to {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn public_addresses_pass() {
        for ok in ["93.184.215.14", "8.8.8.8", "1.1.1.1", "172.32.0.1", "100.128.0.1", "2606:4700:4700::1111", "2a00:1450:4001:80b::200e", "64:ff9b::808:808", "2002:808:808::1"] {
            assert_eq!(forbidden_reason(ip(ok)), None, "{ok}");
        }
    }

    #[test]
    fn internal_ipv4_addresses_are_refused() {
        let cases = [
            ("127.0.0.1", "a loopback address"),
            ("127.255.0.9", "a loopback address"),
            ("10.1.2.3", "a private address"),
            ("172.16.0.1", "a private address"),
            ("172.31.255.255", "a private address"),
            ("192.168.1.10", "a private address"),
            ("169.254.169.254", "a link-local address"),
            ("0.0.0.0", "an unspecified address"),
            ("0.1.2.3", "an unspecified address"),
            ("100.64.0.1", "a shared (carrier-grade NAT) address"),
            ("224.0.0.251", "a multicast address"),
            ("255.255.255.255", "a reserved address"),
            ("240.0.0.1", "a reserved address"),
            ("192.0.0.170", "a reserved address"),
            ("198.18.0.1", "a reserved (benchmarking) address"),
        ];
        for (addr, reason) in cases {
            assert_eq!(forbidden_reason(ip(addr)), Some(reason), "{addr}");
        }
    }

    #[test]
    fn internal_ipv6_addresses_are_refused() {
        let cases = [
            ("::1", "a loopback address"),
            ("::", "an unspecified address"),
            ("fd00::1", "a unique-local address"),
            ("fc12:3456::1", "a unique-local address"),
            ("fe80::1", "a link-local address"),
            ("fec0::1", "a site-local address"),
            ("ff02::1", "a multicast address"),
            ("100::1", "a discard-only address"),
            // IPv4 inside IPv6 is judged by the IPv4 address.
            ("::ffff:127.0.0.1", "a loopback address"),
            ("::ffff:169.254.169.254", "a link-local address"),
            ("::ffff:10.0.0.1", "a private address"),
            ("::10.0.0.1", "a private address"),
            ("::8.8.8.8", "a reserved address"),
            ("64:ff9b::a00:1", "a private address"),
            ("2002:a9fe:a9fe::1", "a link-local address"),
            ("2002:7f00:1::1", "a loopback address"),
        ];
        for (addr, reason) in cases {
            assert_eq!(forbidden_reason(ip(addr)), Some(reason), "{addr}");
        }
    }

    #[test]
    fn the_operator_setting_allows_private_targets() {
        assert!(check_addr(ip("10.0.0.5"), false).unwrap_err().contains("10.0.0.5 is a private address"));
        assert!(check_addr(ip("10.0.0.5"), false).unwrap_err().contains("OC_SYNTHETICS_ALLOW_PRIVATE"));
        assert_eq!(check_addr(ip("10.0.0.5"), true), Ok(()));
        assert_eq!(check_addr(ip("169.254.169.254"), true), Ok(()));
        assert_eq!(check_addr(ip("8.8.4.4"), false), Ok(()));
    }

    #[test]
    fn a_host_with_any_internal_address_is_refused() {
        let addrs: Vec<SocketAddr> = vec!["[2606:4700::1]:443".parse().unwrap(), "93.184.215.14:443".parse().unwrap()];
        assert_eq!(pick(&addrs, false), Ok("93.184.215.14:443".parse().unwrap()), "IPv4 first");
        let mixed: Vec<SocketAddr> = vec!["93.184.215.14:443".parse().unwrap(), "10.0.0.7:443".parse().unwrap()];
        assert!(pick(&mixed, false).unwrap_err().contains("10.0.0.7"));
        assert_eq!(pick(&mixed, true), Ok("93.184.215.14:443".parse().unwrap()));
        assert!(pick(&[], true).is_err());
    }

    #[tokio::test]
    async fn literals_are_checked_without_a_lookup() {
        assert!(resolve("127.0.0.1", 80, false).await.unwrap_err().contains("loopback"));
        assert!(resolve("[::1]", 80, false).await.unwrap_err().contains("loopback"));
        assert_eq!(resolve("[::1]", 8080, true).await, Ok("[::1]:8080".parse().unwrap()));
        assert_eq!(resolve("127.0.0.1", 8103, true).await, Ok("127.0.0.1:8103".parse().unwrap()));
        assert!(resolve("localhost", 80, false).await.unwrap_err().contains("loopback"));
    }
}
