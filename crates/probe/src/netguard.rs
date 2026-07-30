//! Resolve an agent's on-chain `agentURI` into something we can safely fetch —
//! and guard against SSRF, because that URI is ATTACKER-CONTROLLED chain data.
//!
//! ERC-8004's Identity Registry stores an `agentURI` per agent. In the real
//! Base data it's one of: an `https://`/`http://` URL, a `data:` URI with the
//! card inlined (often base64), an `ipfs://` reference, or — for ~65% of agents
//! — an empty or malformed string (`undefined/agents/…`). [`resolve`] turns each
//! into a [`Resolution`]:
//!
//! * [`Resolution::Fetch`] — a network URL that passed the SSRF check.
//! * [`Resolution::Inline`] — a `data:` payload already decoded; no network.
//! * [`Resolution::Reject`] — unresolvable/unsafe; the reason is itself data.
//!
//! SSRF matters: an agent can register `http://169.254.169.254/…` (cloud
//! metadata) or an internal host, turning our prober into a scanner of our own
//! network. We resolve the host ourselves and refuse any non-public address.
//! Known limitation (documented, accepted for v1): resolve-then-connect leaves
//! a TOCTOU / DNS-rebinding gap; redirects are disabled in the client, which
//! closes the cheap variant.
//!
//! Moved here unchanged from the retired `crates/enricher/src/netguard.rs`
//! (imports only) — see `crates/probe/src/fetch.rs` for how its callers
//! changed: `resolve()` here is now the per-hop SSRF check `fetch.rs` runs
//! before the initial connection AND before following each redirect, not the
//! sole classifier of a `tokenURI()` string (that split off into
//! `crates/probe/src/resolve.rs`'s synchronous, non-DNS `Target` classifier).

use std::net::IpAddr;

use base64::Engine;

/// The outcome of turning an `agentURI` into a fetch plan.
///
/// `Fetch` and `Inline` carry payloads that the current sole caller does not
/// read: `fetch.rs` uses `resolve()` purely as the per-hop SSRF gate and
/// matches both with `_`. They are kept rather than reduced to unit variants
/// because they are not redundant — `Fetch` holds the URL *after* the
/// `ipfs://` → gateway rewrite, which is not the string that went in, and
/// `Inline` holds an already-decoded `data:` payload that a non-gate caller
/// would otherwise decode a second time.
#[allow(dead_code)]
pub enum Resolution {
    /// An http(s) URL whose host resolved to only public addresses.
    Fetch(url::Url),
    /// A `data:` URI decoded to raw bytes — serve it without any network I/O.
    Inline(Vec<u8>),
    /// Could not produce a safe, fetchable target. The string is a
    /// human-readable reason, stored as the observation outcome.
    Reject(String),
}

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

/// Turn an `agentURI` into a fetch plan. `ipfs_gateway` is the HTTPS gateway
/// prefix (e.g. `https://ipfs.io/ipfs/`) used to rewrite `ipfs://` references.
pub async fn resolve(agent_uri: &str, ipfs_gateway: &str) -> Resolution {
    let trimmed = agent_uri.trim();
    if trimmed.is_empty() {
        return Resolution::Reject("empty agentURI".into());
    }

    // `data:` — decode inline, no network. Common for small on-chain cards.
    if let Some(rest) = trimmed.strip_prefix("data:") {
        return match decode_data_uri(rest) {
            Ok(bytes) => Resolution::Inline(bytes),
            Err(e) => Resolution::Reject(format!("bad data uri: {e}")),
        };
    }

    // `ipfs://<cid>[/path]` — rewrite onto the configured HTTPS gateway.
    if let Some(rest) = trimmed.strip_prefix("ipfs://") {
        let rewritten = format!("{}{}", ipfs_gateway, rest.trim_start_matches('/'));
        return resolve_http(&rewritten).await;
    }

    // Otherwise it must parse as an http(s) URL. Bare domains, relative paths
    // ("undefined/agents/…"), and unsupported schemes all fall through to
    // Reject — which, for the ~65% of malformed URIs, is the correct answer.
    resolve_http(trimmed).await
}

/// Parse an http(s) URL, then confirm its host resolves only to public IPs.
async fn resolve_http(candidate: &str) -> Resolution {
    let url = match url::Url::parse(candidate) {
        Ok(u) => u,
        Err(e) => return Resolution::Reject(format!("unparseable uri: {e}")),
    };
    match url.scheme() {
        "http" | "https" => {}
        other => return Resolution::Reject(format!("unsupported scheme: {other}")),
    }

    match url.host() {
        // A literal IP: check it directly, no DNS.
        Some(url::Host::Ipv4(ip)) if !is_public_ip(IpAddr::V4(ip)) => {
            return Resolution::Reject("non-public address".into());
        }
        Some(url::Host::Ipv6(ip)) if !is_public_ip(IpAddr::V6(ip)) => {
            return Resolution::Reject("non-public address".into());
        }
        Some(url::Host::Ipv4(_)) | Some(url::Host::Ipv6(_)) => return Resolution::Fetch(url),
        Some(url::Host::Domain(host)) => {
            // Resolve the hostname ourselves and reject if ANY address is
            // non-public (defeats a DNS record pointing at an internal host).
            let port = url.port_or_known_default().unwrap_or(443);
            let host = host.to_string();
            let addrs = match tokio::net::lookup_host((host.as_str(), port)).await {
                Ok(a) => a.map(|sa| sa.ip()).collect::<Vec<_>>(),
                Err(e) => return Resolution::Reject(format!("dns resolution failed: {e}")),
            };
            if addrs.is_empty() {
                return Resolution::Reject("dns resolved to nothing".into());
            }
            if addrs.iter().any(|ip| !is_public_ip(*ip)) {
                return Resolution::Reject("resolves to a non-public address".into());
            }
        }
        None => return Resolution::Reject("no host".into()),
    }
    Resolution::Fetch(url)
}

/// Decode the part of a `data:` URI after `data:` — i.e. `[<meta>][;base64],<payload>`.
fn decode_data_uri(rest: &str) -> Result<Vec<u8>, String> {
    let comma = rest.find(',').ok_or("no comma separator")?;
    let meta = &rest[..comma];
    let payload = &rest[comma + 1..];
    if meta
        .split(';')
        .any(|seg| seg.eq_ignore_ascii_case("base64"))
    {
        // Try padded standard base64, then no-pad — data URIs use both.
        base64::engine::general_purpose::STANDARD
            .decode(payload)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload))
            .map_err(|e| format!("base64 decode failed: {e}"))
    } else {
        // Plain (possibly percent-encoded) text; take the bytes as-is. Good
        // enough for the JSON cards we actually see.
        Ok(payload.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_metadata_ranges_are_rejected() {
        for bad in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.5.5",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata endpoint — the classic SSRF target
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
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
    async fn malformed_and_empty_uris_are_rejected() {
        // The ~65% real-world garbage bucket, plus unsupported schemes.
        for bad in [
            "",
            "   ",
            "undefined/agents/442/agent-card/v1",
            "ftp://x/y",
            "not a uri",
        ] {
            assert!(
                matches!(
                    resolve(bad, "https://ipfs.io/ipfs/").await,
                    Resolution::Reject(_)
                ),
                "{bad:?} must reject"
            );
        }
    }

    #[tokio::test]
    async fn literal_private_host_is_rejected_without_dns() {
        // A literal IP is checked directly (no DNS), so this is deterministic.
        assert!(matches!(
            resolve(
                "http://169.254.169.254/latest/meta-data/",
                "https://ipfs.io/ipfs/"
            )
            .await,
            Resolution::Reject(_)
        ));
    }

    #[tokio::test]
    async fn literal_public_ip_is_fetchable() {
        assert!(matches!(
            resolve(
                "https://1.1.1.1/.well-known/agent.json",
                "https://ipfs.io/ipfs/"
            )
            .await,
            Resolution::Fetch(_)
        ));
    }

    #[tokio::test]
    async fn data_uri_is_decoded_inline() {
        // base64 of {"a":1}
        let r = resolve(
            "data:application/json;base64,eyJhIjoxfQ==",
            "https://ipfs.io/ipfs/",
        )
        .await;
        match r {
            Resolution::Inline(bytes) => assert_eq!(bytes, br#"{"a":1}"#),
            _ => panic!("expected Inline"),
        }
    }

    #[tokio::test]
    async fn ipfs_is_rewritten_onto_the_gateway() {
        // ipfs:// rewrites onto the (public) gateway host, so it resolves to Fetch.
        let r = resolve("ipfs://bafyfakecid/card.json", "https://ipfs.io/ipfs/").await;
        match r {
            Resolution::Fetch(u) => {
                assert_eq!(u.host_str(), Some("ipfs.io"));
                assert!(u.path().contains("bafyfakecid/card.json"));
            }
            Resolution::Reject(e) => panic!("expected Fetch, got reject: {e}"),
            _ => panic!("expected Fetch"),
        }
    }
}
