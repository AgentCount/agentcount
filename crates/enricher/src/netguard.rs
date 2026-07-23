//! SSRF guard — the domain we probe is ATTACKER-CONTROLLED on-chain data.
//!
//! An agent can register `169.254.169.254`, `localhost`, or an internal
//! hostname, turning our prober into a scanner of our own network (or a cloud
//! metadata-credential thief). Before any request: parse the URL strictly,
//! resolve the host ourselves, and refuse anything that lands on a
//! non-public address.
//!
//! Known limitation (documented, accepted for v1): we resolve-then-connect,
//! so a DNS record that changes between our check and reqwest's own lookup
//! (TOCTOU / DNS rebinding) can slip through. Redirects are disabled in the
//! client, which closes the cheap variant of the same trick.

use std::net::IpAddr;

/// Is this an address we're willing to talk to? Explicit deny-list of every
/// non-public range (stable Rust has no `is_global()`), so anything is
/// "public" only after every check passes.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()          // 127.0.0.0/8
                || v4.is_private()      // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()   // 169.254/16 — cloud metadata lives here
                || v4.is_unspecified()  // 0.0.0.0
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                // 100.64.0.0/10 (CGNAT) — RFC 6598, used by cloud-internal nets
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 64))
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fc00::/7 unique-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 link-local
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // v4-mapped ::ffff:a.b.c.d — recurse on the embedded v4
                || v6.to_ipv4_mapped().map(|v4| !is_public_ip(IpAddr::V4(v4))).unwrap_or(false))
        }
    }
}

/// Validate a registered domain and build the agent-card URL. Rejects anything
/// that isn't a bare https host on the default port, then resolves it and
/// checks every returned address. Returns the URL to fetch, or a human-readable
/// rejection reason (which is DATA — it's stored as the probe outcome).
pub async fn check_target(domain: &str) -> Result<url::Url, String> {
    // Parsing through `url` (instead of format!-ing a string) is what stops
    // `evil.com/x?`, `user@host`, and `host:8080` from rewriting the request.
    let url = url::Url::parse(&format!("https://{domain}/.well-known/agent.json"))
        .map_err(|e| format!("unparseable domain: {e}"))?;

    let host = match url.host() {
        Some(url::Host::Domain(d)) => d.to_string(),
        // Literal IPs get checked directly — nobody legitimate registers one,
        // but the guard shouldn't care about legitimacy, only publicness.
        Some(url::Host::Ipv4(ip)) => {
            return if is_public_ip(IpAddr::V4(ip)) { Ok(url) } else { Err("non-public address".into()) };
        }
        Some(url::Host::Ipv6(ip)) => {
            return if is_public_ip(IpAddr::V6(ip)) { Ok(url) } else { Err("non-public address".into()) };
        }
        None => return Err("no host".into()),
    };
    if url.port().is_some() {
        return Err("explicit port not allowed".into());
    }
    if url.host_str() != Some(domain) {
        // The registered string smuggled a path/userinfo/query into the URL.
        return Err("domain is not a bare hostname".into());
    }

    let addrs: Vec<IpAddr> = tokio::net::lookup_host((host.as_str(), 443))
        .await
        .map_err(|e| format!("dns resolution failed: {e}"))?
        .map(|sa| sa.ip())
        .collect();
    if addrs.is_empty() {
        return Err("dns resolved to nothing".into());
    }
    if addrs.iter().any(|ip| !is_public_ip(*ip)) {
        return Err("resolves to a non-public address".into());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_metadata_ranges_are_rejected() {
        for bad in [
            "127.0.0.1", "10.0.0.1", "172.16.5.5", "192.168.1.1",
            "169.254.169.254",     // cloud metadata endpoint — the classic SSRF target
            "100.64.0.1", "0.0.0.0", "::1", "fc00::1", "fe80::1", "::ffff:10.0.0.1",
        ] {
            let ip: IpAddr = bad.parse().unwrap();
            assert!(!is_public_ip(ip), "{bad} must be rejected");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for good in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            let ip: IpAddr = good.parse().unwrap();
            assert!(is_public_ip(ip), "{good} must be allowed");
        }
    }

    #[tokio::test]
    async fn smuggled_urls_are_rejected() {
        // Each of these "domains" tries to rewrite the request.
        for evil in ["evil.com/steal?x=", "user@evil.com", "evil.com:8080", "evil.com?x=1"] {
            assert!(check_target(evil).await.is_err(), "{evil} must be rejected");
        }
    }
}
