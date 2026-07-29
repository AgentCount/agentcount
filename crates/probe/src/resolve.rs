//! Classify a `tokenURI()` string into a fetch plan — pure, synchronous,
//! no I/O.
//!
//! This is deliberately NOT where SSRF safety lives: `resolve()` only parses
//! and rewrites, it never touches the network (no DNS, no connection), so it
//! cannot answer "is this host safe to talk to." That question belongs to
//! `netguard`, applied by `fetch.rs` to the initial URL AND to every redirect
//! hop — see that module's doc comment for why re-checking every hop matters.
//!
//! Five scheme buckets: `""` (empty, no request), `data:` (decoded inline,
//! or its raw-JSON-with-no-scheme cousin — see below), `https://`/`http://`
//! (through the netguard), and everything else (`Unsupported`). `ipfs://` is
//! deliberately NOT classified here as of P0 FIX 8: which gateway serves it
//! is no longer a single synchronous rewrite — `fetch.rs` tries up to three,
//! live, in sequence — so this module's only remaining ipfs-related job is
//! [`ipfs_cid_and_path`], extracting the CID/path an `ipfs://` URI names.
//!
//! **P0 FIX 7 (data URI coverage).** A `data:` payload is decoded through
//! one of five paths, tried in this order, and the path that succeeded is
//! recorded (`DataUriDecode::variant`) because — per the fix — which path
//! succeeded is itself data:
//! 1. `enc=<algorithm>[;level=<n>]` — decompress with the named algorithm
//!    (only `gzip` is implemented; see [`decode_data_uri`]'s doc for why the
//!    others are deliberately not).
//! 2. any `;base64,` meta, regardless of declared MIME type or charset —
//!    plain base64 decode.
//! 3. no `;base64,` token at all — the payload is literal/percent-encoded
//!    text.
//! 4. a payload that CLAIMS `;base64,` but plainly starts with `{` or `[` —
//!    real-world tooling sometimes forgets to actually encode; the decode is
//!    skipped and the payload used as-is.
//! 5. no `data:` scheme at all — the on-chain URI string itself is raw JSON
//!    (`{...}`/`[...]`). Handled in [`resolve`] itself, not
//!    [`decode_data_uri`], since there is no `data:` prefix to strip.
//!
//! An `enc=` algorithm we don't implement is a DIFFERENT outcome from a
//! malformed `data:` URI: we understood exactly what was declared, we
//! simply cannot decode it — [`Target::UnsupportedCompression`], which
//! `fetch.rs` turns into `FetchOutcome.error`, never a bare `Unsupported`
//! (which downstream reads as `checks::CheckStatus::Fail`).

use base64::Engine;
use serde::Serialize;

/// What an agent's declared URI resolves to, before any network I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `""` (or all-whitespace) — no request to make. 19,729 agents.
    Empty,
    /// A scheme we don't fetch — anything other than empty/`data:`/`http(s):`,
    /// or a `data:` URI too malformed to decode at all (see
    /// [`decode_data_uri`]'s `Malformed` case). `scheme` is a best-effort
    /// label for provenance, never used for control flow beyond this point.
    Unsupported { scheme: String },
    /// A `data:` URI declaring `enc=<algorithm>` for a compression codec we
    /// don't implement (P0 FIX 7). Distinct from `Unsupported`: we
    /// understood the declaration, we simply can't decode it — OUR
    /// limitation, not a defect in the agent's document. `fetch.rs` turns
    /// this into `FetchOutcome.error`, which downstream reads as
    /// `checks::CheckStatus::Error`, never `Fail`.
    UnsupportedCompression { scheme: String, algorithm: String },
    /// A `data:` URI (or a scheme-less raw-JSON string — see the module
    /// doc's item 5), already decoded to raw bytes. No network involved.
    /// `decode` records which of the five fallback paths produced `bytes`.
    Inline {
        bytes: Vec<u8>,
        decode: DataUriDecode,
    },
    /// A `http(s)://` URL to fetch.
    Http { url: url::Url },
}

/// Which of P0 FIX 7's five fallback paths decoded a `data:` payload (or
/// stood in for one, in the scheme-less raw-JSON case) — recorded because,
/// per the fix, which path succeeded is itself data worth publishing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataUriDecode {
    /// One of `"compressed"`, `"base64"`, `"plain"`,
    /// `"base64_claimed_but_raw_json"`, `"plain_json_without_uri_scheme"`.
    pub variant: &'static str,
    /// The `enc=` value, set only when `variant == "compressed"`.
    pub algorithm: Option<String>,
}

/// Classify `uri` (an agent's raw `tokenURI()` string, exactly as read from
/// the chain).
pub fn resolve(uri: &str) -> Target {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Target::Empty;
    }

    if let Some(rest) = trimmed.strip_prefix("data:") {
        return match decode_data_uri(rest) {
            Ok((bytes, decode)) => Target::Inline { bytes, decode },
            Err(DataUriError::Malformed) => Target::Unsupported {
                scheme: "data".into(),
            },
            Err(DataUriError::UnsupportedCompression(algorithm)) => {
                Target::UnsupportedCompression {
                    scheme: "data".into(),
                    algorithm,
                }
            }
        };
    }

    match url::Url::parse(trimmed) {
        Ok(url) => match url.scheme() {
            "http" | "https" => Target::Http { url },
            other => Target::Unsupported {
                scheme: other.to_string(),
            },
        },
        // Doesn't even parse as a URL — either the classic on-chain garbage
        // bucket (a bare path like "undefined/agents/442/agent-card/v1"), or
        // — P0 FIX 7 item 5 — raw JSON stored with no URI scheme at all.
        // Real-world tooling sometimes stores the document literally instead
        // of wrapping it in a `data:` URI; treat that the same as any other
        // already-in-hand payload and let rung 3 decide whether it actually
        // parses.
        Err(_) => {
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                Target::Inline {
                    bytes: trimmed.as_bytes().to_vec(),
                    decode: DataUriDecode {
                        variant: "plain_json_without_uri_scheme",
                        algorithm: None,
                    },
                }
            } else {
                Target::Unsupported {
                    scheme: guess_scheme(trimmed),
                }
            }
        }
    }
}

/// Extract the `<cid>[/path]` portion of an `ipfs://` URI, for the
/// multi-gateway fallback chain `fetch.rs` drives (P0 FIX 8). Picking which
/// gateway serves an agent — and trying more than one — takes live network
/// attempts, which this module deliberately never makes (see the module
/// doc), so unlike every other scheme this classification alone isn't
/// enough to produce a [`Target`]; `fetch.rs` calls this directly instead of
/// going through [`resolve`]. Returns `None` for anything that isn't
/// `ipfs://`, or is `ipfs://` with nothing after it.
pub fn ipfs_cid_and_path(uri: &str) -> Option<String> {
    let rest = uri.trim().strip_prefix("ipfs://")?;
    let cid_and_path = rest.trim_start_matches('/');
    if cid_and_path.is_empty() {
        None
    } else {
        Some(cid_and_path.to_string())
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

/// Why [`decode_data_uri`] could not hand back bytes. The two cases read
/// completely differently downstream (see `Target`'s doc): `Malformed` is a
/// fact about what the agent published (`checks::CheckStatus::Fail`, which,
/// same as before this fix, carries no further detail than the scheme
/// label); `UnsupportedCompression` is OUR limitation
/// (`checks::CheckStatus::Error`), and its algorithm name IS recorded.
enum DataUriError {
    Malformed,
    UnsupportedCompression(String),
}

/// Cap on bytes read out of a gzip stream — defense against a decompression
/// bomb hiding inside a `data:` URI. Generous relative to any real
/// registration file (every on-chain byte costs gas, so a legitimate
/// document is nowhere near this), and `fetch.rs`'s own `cap_bytes` applies
/// the real [`crate::MAX_BODY_BYTES`] cap to the result afterward exactly
/// like it does for every other inline payload — this limit only exists so
/// that step is reached with a bounded amount of memory, not an unbounded
/// one.
const MAX_GUNZIP_READ_BYTES: u64 = 8 * 1024 * 1024;

fn gunzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(bytes).take(MAX_GUNZIP_READ_BYTES);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

/// Decode the part of a `data:` URI after `data:` — i.e. `[<meta>][;base64],<payload>`
/// — trying, in order, the five fallback paths P0 FIX 7 specifies. Returns
/// the decoded bytes plus which path produced them.
///
/// **Only `gzip` is implemented.** The ecosystem also names `zstd`
/// (recommended), `brotli`, and `lz4`; measured against the reference
/// population (60,049 agents' `agent_snapshots.agent_uri`), every `enc=`
/// occurrence (399 of them) declares `gzip` — zero declare any other
/// algorithm. Adding a dependency to decode zero real documents was
/// deliberately not done; see `CHANGELOG-METHODOLOGY.md`'s FIX-7 entry. Any
/// `enc=` value other than `gzip` — including `zstd`/`brotli`/`lz4`, should
/// one appear in a future sweep — is `DataUriError::UnsupportedCompression`,
/// never treated as malformed.
///
/// A NUL byte anywhere in `rest` is not a decode error: Rust strings and
/// `Vec<u8>` carry interior NULs just fine, unlike C strings. Day 1 found 18
/// on-chain `tokenURI()`s with one.
fn decode_data_uri(rest: &str) -> Result<(Vec<u8>, DataUriDecode), DataUriError> {
    let comma = rest.find(',').ok_or(DataUriError::Malformed)?;
    let meta = &rest[..comma];
    let payload = &rest[comma + 1..];

    let is_base64 = meta
        .split(';')
        .any(|seg| seg.eq_ignore_ascii_case("base64"));

    // Item 3: no `;base64,` token at all — literal/percent-encoded text.
    // Handled first because it's independent of everything below: a plain
    // `data:` URI never has compression or a base64-vs-raw-JSON ambiguity.
    if !is_base64 {
        return Ok((
            percent_decode(payload),
            DataUriDecode {
                variant: "plain",
                algorithm: None,
            },
        ));
    }

    // Item 4: the meta claims base64, but the payload plainly starts with
    // `{` or `[` — some real-world producers forget to actually encode.
    // Skip the decode and use the payload as-is rather than failing a
    // base64 decode that was never going to succeed.
    let payload_trimmed = payload.trim_start();
    if payload_trimmed.starts_with('{') || payload_trimmed.starts_with('[') {
        return Ok((
            payload.as_bytes().to_vec(),
            DataUriDecode {
                variant: "base64_claimed_but_raw_json",
                algorithm: None,
            },
        ));
    }

    // Try padded standard base64, then no-pad — data URIs use both.
    let raw = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload))
        .map_err(|_| DataUriError::Malformed)?;

    let algorithm = meta.split(';').find_map(|seg| {
        let seg = seg.trim();
        (seg.len() > 4 && seg[..4].eq_ignore_ascii_case("enc=")).then(|| seg[4..].to_lowercase())
    });

    match algorithm {
        // Item 2: plain base64, no declared compression.
        None => Ok((
            raw,
            DataUriDecode {
                variant: "base64",
                algorithm: None,
            },
        )),
        // Item 1: `enc=` present — decompress with the named algorithm.
        Some(algo) if algo == "gzip" => {
            let decompressed = gunzip(&raw).map_err(|_| DataUriError::Malformed)?;
            Ok((
                decompressed,
                DataUriDecode {
                    variant: "compressed",
                    algorithm: Some(algo),
                },
            ))
        }
        Some(algo) => Err(DataUriError::UnsupportedCompression(algo)),
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

    fn inline_bytes(target: Target) -> (Vec<u8>, DataUriDecode) {
        match target {
            Target::Inline { bytes, decode } => (bytes, decode),
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn empty_string_is_empty() {
        assert_eq!(resolve(""), Target::Empty);
        assert_eq!(resolve("   "), Target::Empty);
    }

    #[test]
    fn a_data_uri_decodes_inline() {
        // base64 of {"a":1}
        let (bytes, decode) = inline_bytes(resolve("data:application/json;base64,eyJhIjoxfQ=="));
        assert_eq!(bytes, br#"{"a":1}"#);
        assert_eq!(decode.variant, "base64");
        assert_eq!(decode.algorithm, None);
    }

    #[test]
    fn a_malformed_data_uri_is_unsupported_not_a_fetch() {
        // No comma separator — can't be decoded.
        match resolve("data:application/json;base64") {
            Target::Unsupported { scheme } => assert_eq!(scheme, "data"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn data_uri_with_a_nul_byte_does_not_panic() {
        // Day 1 found 18 of these on-chain: a raw NUL landing inside the
        // payload. Rust bytes carry it just fine.
        let uri = "data:text/plain,hello\u{0}world";
        let (bytes, _) = inline_bytes(resolve(uri));
        assert!(
            bytes.contains(&0),
            "the NUL byte must survive, not panic or get dropped"
        );
    }

    #[test]
    fn https_url_is_fetchable() {
        match resolve("https://example.com/agent.json") {
            Target::Http { url } => assert_eq!(url.as_str(), "https://example.com/agent.json"),
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn http_url_is_fetchable() {
        match resolve("http://example.com/agent.json") {
            Target::Http { url } => assert_eq!(url.scheme(), "http"),
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn an_unrecognized_scheme_is_unsupported() {
        match resolve("ftp://example.com/x") {
            Target::Unsupported { scheme } => assert_eq!(scheme, "ftp"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_non_uri_string_is_unsupported() {
        // The classic on-chain garbage bucket: no scheme at all, and not
        // JSON either.
        match resolve("undefined/agents/442/agent-card/v1") {
            Target::Unsupported { .. } => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    // --- P0 FIX 7: data URI coverage ------------------------------------

    #[test]
    fn a_gzip_compressed_data_uri_decompresses_before_parsing() {
        // The exact shape found on-chain: `enc=gzip;level=6;base64,<payload>`.
        // Deliverable fixture: unsupported-compression is covered separately
        // below; this one is the supported, real-population case (399
        // agents in the reference run).
        use std::io::Write;
        let plaintext = br#"{"type":"agent","name":"gzip test"}"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
        encoder.write_all(plaintext).unwrap();
        let gz = encoder.finish().unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(gz);
        let uri = format!("data:application/json;enc=gzip;level=6;base64,{b64}");

        let (bytes, decode) = inline_bytes(resolve(&uri));
        assert_eq!(bytes, plaintext);
        assert_eq!(decode.variant, "compressed");
        assert_eq!(decode.algorithm.as_deref(), Some("gzip"));
    }

    #[test]
    fn an_unsupported_compression_algorithm_is_error_not_fail() {
        // zstd is the ecosystem's RECOMMENDED algorithm but has zero
        // occurrences in the reference population (measured against
        // agent_snapshots.agent_uri for the 60,049-agent reference run) —
        // implemented here anyway (cheap: just naming the codec) but
        // untested against real data beyond this synthetic fixture. brotli
        // and lz4 are the same story. None decodes; all three, and any
        // other declared `enc=` value, must be `UnsupportedCompression`,
        // never the generic `Unsupported` a malformed document gets.
        for algo in ["zstd", "brotli", "lz4", "made-up-codec"] {
            let uri = format!("data:application/json;enc={algo};base64,eyJhIjoxfQ==");
            match resolve(&uri) {
                Target::UnsupportedCompression { scheme, algorithm } => {
                    assert_eq!(scheme, "data");
                    assert_eq!(algorithm, algo);
                }
                other => panic!("expected UnsupportedCompression for {algo}, got {other:?}"),
            }
        }
    }

    #[test]
    fn any_base64_meta_variant_decodes_regardless_of_mime_or_charset() {
        // Non-standard variants observed in production: `text/plain`, no
        // MIME at all, and an explicit charset param alongside base64. None
        // of these are the `enc=` case, so all three decode as plain base64.
        for uri in [
            "data:text/plain;base64,eyJhIjoxfQ==",
            "data:;base64,eyJhIjoxfQ==",
            "data:application/json;charset=utf-8;base64,eyJhIjoxfQ==",
        ] {
            let (bytes, decode) = inline_bytes(resolve(uri));
            assert_eq!(bytes, br#"{"a":1}"#, "failed for {uri}");
            assert_eq!(decode.variant, "base64", "failed for {uri}");
        }
    }

    #[test]
    fn a_plain_non_base64_data_uri_is_url_decoded() {
        let (bytes, decode) = inline_bytes(resolve(r#"data:application/json,{"a":1}"#));
        assert_eq!(bytes, br#"{"a":1}"#);
        assert_eq!(decode.variant, "plain");

        // Percent-encoded, the shape actually seen on-chain.
        let (bytes, decode) = inline_bytes(resolve("data:application/json,%7B%22a%22%3A1%7D"));
        assert_eq!(bytes, br#"{"a":1}"#);
        assert_eq!(decode.variant, "plain");
    }

    #[test]
    fn a_base64_meta_whose_payload_is_actually_raw_json_skips_the_decode() {
        // Zero occurrences in the reference population (measured), but
        // cheap to implement and stated by the work order to occur in the
        // wild — implemented and fixtured, untested against real data.
        let (bytes, decode) = inline_bytes(resolve(r#"data:application/json;base64,{"a":1}"#));
        assert_eq!(bytes, br#"{"a":1}"#);
        assert_eq!(decode.variant, "base64_claimed_but_raw_json");
    }

    #[test]
    fn raw_json_with_no_uri_scheme_is_inline_with_a_named_variant() {
        // 127 agents in the reference population declare their tokenURI as
        // bare JSON, no scheme prefix at all.
        let (bytes, decode) = inline_bytes(resolve(r#"{"a":1}"#));
        assert_eq!(bytes, br#"{"a":1}"#);
        assert_eq!(decode.variant, "plain_json_without_uri_scheme");

        let (bytes, decode) = inline_bytes(resolve("[1,2,3]"));
        assert_eq!(bytes, b"[1,2,3]");
        assert_eq!(decode.variant, "plain_json_without_uri_scheme");
    }

    // --- P0 FIX 8: ipfs:// classification moves to fetch.rs -------------

    #[test]
    fn ipfs_cid_and_path_extracts_the_cid_and_any_subpath() {
        assert_eq!(
            ipfs_cid_and_path("ipfs://bafyfakecid").as_deref(),
            Some("bafyfakecid")
        );
        assert_eq!(
            ipfs_cid_and_path("ipfs://bafyfakecid/metadata/1.json").as_deref(),
            Some("bafyfakecid/metadata/1.json")
        );
        // Extra leading slashes (ipfs://<cid> vs ipfs:///<cid>) are trimmed
        // the same way the old resolve()-based rewriting did.
        assert_eq!(
            ipfs_cid_and_path("ipfs:///bafyfakecid").as_deref(),
            Some("bafyfakecid")
        );
    }

    #[test]
    fn ipfs_cid_and_path_is_none_for_an_empty_cid_or_a_non_ipfs_uri() {
        assert_eq!(ipfs_cid_and_path("ipfs://"), None);
        assert_eq!(ipfs_cid_and_path("ipfs:///"), None);
        assert_eq!(ipfs_cid_and_path("https://example.com"), None);
        assert_eq!(ipfs_cid_and_path(""), None);
    }
}
