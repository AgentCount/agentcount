//! The unit: what counts as one seller.
//!
//! METHODOLOGY §10.1. A seller is a deduped **(payTo, host)** pair, and the
//! whole instrument's arithmetic rests on that being computed the same way
//! every time — a normalization that differs between the crawler and the
//! delta would silently split one seller into two and publish churn that
//! never happened.
//!
//! Two rules that look like details and are not:
//!
//! * **The full host, never the registrable domain.** `api.example.com` and
//!   `example.com` are different services with different operators' hands on
//!   them; collapsing them would manufacture a consistency nobody claimed.
//! * **The same payTo behind two hosts is two sellers**, and the same host
//!   quoting two payTos is two sellers. Groupings — the aggregator shape,
//!   forty sellers sharing one payTo — are published as findings over the
//!   population, never as merges of the unit.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Which chain a payment address belongs to. Normalization is per-network
/// because address encodings are: EVM addresses are hex and case-insensitive
/// (so they lowercase), and Solana's are base58 and case-SENSITIVE (so they
/// must not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    /// Base, and any other EVM network the census later enables. Sweep 1 is
    /// Base/USDC only (METHODOLOGY §10.5); the encoding rule is shared.
    Evm,
    /// Reserved for the stated Solana expansion. Present so that the
    /// case-sensitivity rule is written down before it is needed, rather
    /// than discovered by lowercasing somebody's address.
    Solana,
}

/// Why a candidate could not become a seller identity. Each variant is a
/// listing this census refuses to count rather than guess about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// Not a hex address of the right length (EVM).
    MalformedAddress,
    /// The zero address. A catalog entry paying nobody is not a seller.
    ZeroAddress,
    /// The resource URL did not parse, or carried no host.
    MalformedUrl,
    /// A scheme this instrument does not measure. Sellers are HTTP(S)
    /// endpoints; anything else is a different kind of thing.
    UnsupportedScheme,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::MalformedAddress => "malformed_address",
            Self::ZeroAddress => "zero_address",
            Self::MalformedUrl => "malformed_url",
            Self::UnsupportedScheme => "unsupported_scheme",
        };
        f.write_str(s)
    }
}

/// One seller: a normalized `(pay_to, host)` pair, and the only key the rest
/// of this instrument uses to say "the same seller".
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SellerId {
    pub pay_to: String,
    pub host: String,
}

impl SellerId {
    /// Build an identity from a raw catalog listing.
    ///
    /// `resource` is the priced URL, not a bare host — that is what catalogs
    /// carry, and taking the URL means the host normalization (default port
    /// stripped, IDN punycoded, lowercased) happens here rather than at
    /// however many call sites later remember to do it.
    pub fn new(pay_to: &str, resource: &str, network: Network) -> Result<Self, IdentityError> {
        Ok(Self {
            pay_to: normalize_pay_to(pay_to, network)?,
            host: normalize_host(resource)?,
        })
    }
}

impl fmt::Display for SellerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.pay_to, self.host)
    }
}

/// Normalize a payment address for its network, or refuse it.
pub fn normalize_pay_to(pay_to: &str, network: Network) -> Result<String, IdentityError> {
    let raw = pay_to.trim();
    match network {
        Network::Evm => {
            let hex = raw
                .strip_prefix("0x")
                .ok_or(IdentityError::MalformedAddress)?;
            if hex.len() != 40 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(IdentityError::MalformedAddress);
            }
            // Lowercased, never checksummed: EIP-55 casing is a checksum over
            // the same 20 bytes, so two catalogs writing one address in
            // different cases must not become two sellers.
            let lowered = hex.to_ascii_lowercase();
            if lowered.chars().all(|c| c == '0') {
                return Err(IdentityError::ZeroAddress);
            }
            Ok(format!("0x{lowered}"))
        }
        Network::Solana => {
            // Base58 is case-SENSITIVE: lowercasing would name a different
            // account, or none. Length-checked only, and left verbatim.
            if raw.len() < 32 || raw.len() > 44 || !raw.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(IdentityError::MalformedAddress);
            }
            if raw.chars().all(|c| c == '1') {
                return Err(IdentityError::ZeroAddress);
            }
            Ok(raw.to_string())
        }
    }
}

/// Normalize a resource URL down to the host this census identifies a seller
/// by: lowercase, IDN in punycode, default port stripped, non-default port
/// kept (it is a different service).
pub fn normalize_host(resource: &str) -> Result<String, IdentityError> {
    let url = url::Url::parse(resource.trim()).map_err(|_| IdentityError::MalformedUrl)?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(IdentityError::UnsupportedScheme),
    }
    // `Url::host_str` is already lowercased and punycoded by the parser, and
    // `Url::port` is None when the port is the scheme's default — which is
    // exactly the rule, obtained from the spec's own implementation rather
    // than reimplemented here.
    let host = url.host_str().ok_or(IdentityError::MalformedUrl)?;
    if host.is_empty() {
        return Err(IdentityError::MalformedUrl);
    }
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_address_written_two_ways_is_one_seller() {
        // EIP-55 checksum casing is a checksum over the same 20 bytes. Two
        // catalogs disagreeing about case must not become two sellers.
        let checksummed = SellerId::new(
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            "https://api.example.com/weather",
            Network::Evm,
        )
        .unwrap();
        let lowered = SellerId::new(
            "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "https://api.example.com/weather",
            Network::Evm,
        )
        .unwrap();
        assert_eq!(checksummed, lowered);
    }

    #[test]
    fn the_same_pay_to_behind_two_hosts_is_two_sellers() {
        let a = SellerId::new(
            "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "https://api.example.com/x",
            Network::Evm,
        )
        .unwrap();
        let b = SellerId::new(
            "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "https://other.example.org/x",
            Network::Evm,
        )
        .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn the_same_host_quoting_two_pay_tos_is_two_sellers() {
        let a = SellerId::new(
            "0x1111111111111111111111111111111111111111",
            "https://api.example.com/x",
            Network::Evm,
        )
        .unwrap();
        let b = SellerId::new(
            "0x2222222222222222222222222222222222222222",
            "https://api.example.com/x",
            Network::Evm,
        )
        .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn a_subdomain_is_never_folded_into_its_registrable_domain() {
        // The rule with an argument behind it: different services, different
        // operators' hands. Folding these would manufacture consistency.
        assert_ne!(
            normalize_host("https://api.example.com/x").unwrap(),
            normalize_host("https://example.com/x").unwrap()
        );
    }

    #[test]
    fn a_default_port_is_stripped_and_a_real_one_is_kept() {
        assert_eq!(
            normalize_host("https://a.example:443/x").unwrap(),
            "a.example"
        );
        assert_eq!(
            normalize_host("http://a.example:80/x").unwrap(),
            "a.example"
        );
        // A service on another port is another service.
        assert_eq!(
            normalize_host("https://a.example:8443/x").unwrap(),
            "a.example:8443"
        );
    }

    #[test]
    fn hosts_are_lowercased_and_idn_is_punycoded() {
        assert_eq!(
            normalize_host("https://API.Example.COM/x").unwrap(),
            "api.example.com"
        );
        // One host written two ways — Unicode and punycode — is one host, or
        // a seller could list itself twice and be counted twice.
        assert_eq!(
            normalize_host("https://例え.jp/x").unwrap(),
            normalize_host("https://xn--r8jz45g.jp/x").unwrap()
        );
    }

    #[test]
    fn a_listing_paying_nobody_is_refused_rather_than_counted() {
        assert_eq!(
            normalize_pay_to("0x0000000000000000000000000000000000000000", Network::Evm),
            Err(IdentityError::ZeroAddress)
        );
    }

    #[test]
    fn malformed_addresses_and_urls_are_refused_not_guessed_at() {
        for bad in [
            "",
            "0x123",
            "833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "0xzz",
        ] {
            assert!(
                normalize_pay_to(bad, Network::Evm).is_err(),
                "accepted {bad:?}"
            );
        }
        assert_eq!(
            normalize_host("not a url"),
            Err(IdentityError::MalformedUrl)
        );
        // A seller is an HTTP endpoint; other schemes are a different kind of
        // thing and are refused rather than half-measured.
        assert_eq!(
            normalize_host("ipfs://bafy/x"),
            Err(IdentityError::UnsupportedScheme)
        );
    }

    #[test]
    fn solana_addresses_keep_their_case_because_base58_is_case_sensitive() {
        let addr = "7dHbWXmci3dT8UFYWYZweBLXgycu7Y3iL6trKn1Y7ARj";
        assert_eq!(normalize_pay_to(addr, Network::Solana).unwrap(), addr);
    }
}
