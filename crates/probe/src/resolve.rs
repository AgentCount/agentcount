//! Classify a `tokenURI()` string into a fetch plan — pure, synchronous,
//! no I/O.
//!
//! This is deliberately NOT where SSRF safety lives: `resolve()` only parses
//! and rewrites, it never touches the network (no DNS, no connection), so it
//! cannot answer "is this host safe to talk to." That question belongs to
//! `netguard`, applied by `fetch.rs` to the initial URL AND to every redirect
//! hop — see that module's doc comment for why re-checking every hop matters.
//!
//! Six scheme buckets, matching the population `d2-task-2-brief.md` measured:
//! `""` (empty, no request), `data:` (decoded inline), `https://`/`http://`
//! (through the netguard), `ipfs://` (rewritten onto a gateway, then through
//! the netguard like any other URL), and everything else (`Unsupported`).

use base64::Engine;

/// What an agent's declared URI resolves to, before any network I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `""` (or all-whitespace) — no request to make. 19,729 agents.
    Empty,
    /// A scheme we don't fetch — anything other than empty/`data:`/`http(s):`
    /// /`ipfs:`, or a `data:`/`ipfs:` URI too malformed to decode/rewrite.
    /// `scheme` is a best-effort label for provenance, never used for control
    /// flow beyond this point.
    Unsupported { scheme: String },
    /// A `data:` URI, already decoded to raw bytes. No network involved.
    Inline { bytes: Vec<u8> },
    /// A URL to fetch. `via_gateway` carries the gateway prefix used when the
    /// original scheme was `ipfs://`, so a reader can tell an agent's failure
    /// from our gateway's.
    Http {
        url: url::Url,
        via_gateway: Option<String>,
    },
}

/// Classify `uri` (an agent's raw `tokenURI()` string, exactly as read from
/// the chain). `ipfs_gateway` is the HTTPS gateway prefix (e.g.
/// `https://ipfs.io/ipfs/`) `ipfs://` references are rewritten onto.
pub fn resolve(uri: &str, ipfs_gateway: &str) -> Target {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Target::Empty;
    }

    if let Some(rest) = trimmed.strip_prefix("data:") {
        return match decode_data_uri(rest) {
            Ok(bytes) => Target::Inline { bytes },
            Err(_) => Target::Unsupported {
                scheme: "data".into(),
            },
        };
    }

    if let Some(rest) = trimmed.strip_prefix("ipfs://") {
        let cid_and_path = rest.trim_start_matches('/');
        if cid_and_path.is_empty() {
            return Target::Unsupported {
                scheme: "ipfs".into(),
            };
        }
        let rewritten = format!("{ipfs_gateway}{cid_and_path}");
        return match url::Url::parse(&rewritten) {
            Ok(url) => Target::Http {
                url,
                via_gateway: Some(ipfs_gateway.to_string()),
            },
            Err(_) => Target::Unsupported {
                scheme: "ipfs".into(),
            },
        };
    }

    match url::Url::parse(trimmed) {
        Ok(url) => match url.scheme() {
            "http" | "https" => Target::Http {
                url,
                via_gateway: None,
            },
            other => Target::Unsupported {
                scheme: other.to_string(),
            },
        },
        // Doesn't even parse as a URL — a bare path like
        // "undefined/agents/442/agent-card/v1", or garbage. `guess_scheme`
        // is best-effort labeling only.
        Err(_) => Target::Unsupported {
            scheme: guess_scheme(trimmed),
        },
    }
}

/// Best-effort label for a string that didn't parse as a URL at all — used
/// only for the `scheme` field callers record, never for control flow.
fn guess_scheme(s: &str) -> String {
    match s.split_once(':') {
        Some((scheme, _))
            if !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') =>
        {
            scheme.to_string()
        }
        _ => "unknown".to_string(),
    }
}

/// Decode the part of a `data:` URI after `data:` — i.e. `[<meta>][;base64],<payload>`.
/// A NUL byte anywhere in `rest` is not a decode error: Rust strings and
/// `Vec<u8>` carry interior NULs just fine, unlike C strings. Day 1 found 18
/// on-chain `tokenURI()`s with one.
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
        Ok(percent_decode(payload))
    }
}

/// Minimal `%XX` percent-decoder for the plain (non-base64) `data:` payload
/// case. Bytes that aren't part of a valid `%XX` escape pass through as-is.
fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 3 <= bytes.len()
            && let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GW: &str = "https://ipfs.io/ipfs/";

    #[test]
    fn empty_string_is_empty() {
        assert_eq!(resolve("", GW), Target::Empty);
        assert_eq!(resolve("   ", GW), Target::Empty);
    }

    #[test]
    fn a_data_uri_decodes_inline() {
        // base64 of {"a":1}
        match resolve("data:application/json;base64,eyJhIjoxfQ==", GW) {
            Target::Inline { bytes } => assert_eq!(bytes, br#"{"a":1}"#),
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn a_malformed_data_uri_is_unsupported_not_a_fetch() {
        // No comma separator — can't be decoded.
        match resolve("data:application/json;base64", GW) {
            Target::Unsupported { scheme } => assert_eq!(scheme, "data"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn data_uri_with_a_nul_byte_does_not_panic() {
        // Day 1 found 18 of these on-chain: a raw NUL landing inside the
        // payload. Rust bytes carry it just fine.
        let uri = "data:text/plain,hello\u{0}world";
        match resolve(uri, GW) {
            Target::Inline { bytes } => {
                assert!(
                    bytes.contains(&0),
                    "the NUL byte must survive, not panic or get dropped"
                )
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn https_url_is_fetchable() {
        match resolve("https://example.com/agent.json", GW) {
            Target::Http { url, via_gateway } => {
                assert_eq!(url.as_str(), "https://example.com/agent.json");
                assert_eq!(via_gateway, None);
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn http_url_is_fetchable() {
        match resolve("http://example.com/agent.json", GW) {
            Target::Http { url, .. } => assert_eq!(url.scheme(), "http"),
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn ipfs_uri_is_rewritten_onto_the_gateway() {
        match resolve("ipfs://bafyfakecid", GW) {
            Target::Http { url, via_gateway } => {
                assert_eq!(url.host_str(), Some("ipfs.io"));
                assert!(url.path().contains("bafyfakecid"));
                assert_eq!(via_gateway.as_deref(), Some(GW));
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn ipfs_uri_with_subpath_maps_onto_the_gateway() {
        match resolve("ipfs://bafyfakecid/metadata/1.json", GW) {
            Target::Http { url, via_gateway } => {
                assert_eq!(url.host_str(), Some("ipfs.io"));
                assert!(url.path().contains("bafyfakecid/metadata/1.json"));
                assert_eq!(via_gateway.as_deref(), Some(GW));
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn an_unrecognized_scheme_is_unsupported() {
        match resolve("ftp://example.com/x", GW) {
            Target::Unsupported { scheme } => assert_eq!(scheme, "ftp"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_non_uri_string_is_unsupported() {
        // The classic on-chain garbage bucket: no scheme at all.
        match resolve("undefined/agents/442/agent-card/v1", GW) {
            Target::Unsupported { .. } => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }
}
