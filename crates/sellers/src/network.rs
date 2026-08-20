//! One name per network, whatever the catalog called it.
//!
//! The x402 ecosystem names networks two ways and both are in live use:
//!
//! * **Friendly names** — `"base"`, `"base-sepolia"` — which x402 v1 quotes
//!   carry, and which the spec's own examples use.
//! * **CAIP-2 chain ids** — `"eip155:8453"` — which the Bazaar's v2 listings
//!   carry today (observed 2026-08-20: every one of its 15,155 resources
//!   names `eip155:8453`, not `base`).
//!
//! A census that matched on the raw string would have measured the same
//! network twice under two names, or — worse, and this was the near miss —
//! declared every seller in the Bazaar out of scope because the scope said
//! `base` and the catalog said `eip155:8453`. That is a wrong published
//! number produced entirely by a naming convention.
//!
//! So every network name is canonicalized to its CAIP-2 form on the way in,
//! once, in this module, and every comparison downstream is between
//! canonical forms. Unknown names are lowercased and kept verbatim rather
//! than rejected: a seller on a network this census does not sweep is
//! coverage this census does not have (`unprobed`, reason
//! `out_of_scope_network`), never an error and never a silent drop.

/// Base mainnet, CAIP-2.
pub const BASE: &str = "eip155:8453";
/// Base Sepolia, CAIP-2. Not swept — testnet volume is not economy — but
/// named so it can be recognised and excluded rather than counted.
pub const BASE_SEPOLIA: &str = "eip155:84532";
/// Solana mainnet, CAIP-2 (the genesis-hash form the standard defines).
/// Reserved for the stated expansion (METHODOLOGY §10.5).
pub const SOLANA: &str = "solana:5eykt4NQHMXjUTzZ7wm2Ge8Rm2Nfxvfg8P8NJdTREpY1";

/// The one name this census uses for a network.
///
/// Case-insensitive, and tolerant of the `-mainnet` suffix some catalogs
/// append. Anything unrecognised comes back lowercased and unchanged, which
/// is how an out-of-scope network stays visible instead of becoming an
/// error.
pub fn canonical(raw: &str) -> String {
    let trimmed = raw.trim();
    let lowered = trimmed.to_ascii_lowercase();
    match lowered.as_str() {
        "base" | "base-mainnet" => BASE.to_string(),
        "base-sepolia" | "basesepolia" => BASE_SEPOLIA.to_string(),
        "solana" | "solana-mainnet" => SOLANA.to_string(),
        _ => match trimmed.split_once(':') {
            // A CAIP-2 id is `namespace:reference`. The namespace is
            // case-insensitive; the REFERENCE may not be — Solana's is a
            // base58 genesis hash, and lowercasing it names no chain. The
            // same case-sensitivity rule `identity` keeps for Solana
            // addresses, one layer up, and it was a failing test here that
            // caught it rather than a wrong number later.
            Some((namespace, reference)) => {
                format!("{}:{}", namespace.to_ascii_lowercase(), reference)
            }
            // A friendly name with no namespace is case-insensitive.
            None => lowered,
        },
    }
}

/// Whether two network names refer to the same network, whichever
/// convention each was written in.
pub fn same(a: &str, b: &str) -> bool {
    canonical(a) == canonical(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_names_the_ecosystem_actually_uses_are_one_network() {
        // THE near miss: the scope said `base`, and every one of the
        // Bazaar's 15,155 resources says `eip155:8453`. Matching raw strings
        // would have declared the entire catalog out of scope and published
        // a delivery rate over nobody.
        assert!(same("base", "eip155:8453"));
        assert!(same("Base", "eip155:8453"));
        assert!(same("base-mainnet", "base"));
        assert_eq!(canonical("base"), BASE);
        assert_eq!(canonical("eip155:8453"), BASE);
    }

    #[test]
    fn mainnet_and_testnet_are_never_the_same_network() {
        // Testnet volume is not economy, and a census that blended them
        // would publish a number made largely of free money.
        assert!(!same("base", "base-sepolia"));
        assert!(!same(BASE, BASE_SEPOLIA));
    }

    #[test]
    fn an_unknown_network_is_kept_verbatim_not_rejected() {
        // Out-of-scope is coverage this census does not have, which it must
        // be able to say — `unprobed`, reason `out_of_scope_network`.
        assert_eq!(canonical("eip155:42161"), "eip155:42161");
        assert_eq!(canonical("Avalanche"), "avalanche");
        assert!(!same("eip155:42161", BASE));
    }

    #[test]
    fn solana_is_named_before_it_is_swept() {
        assert!(same("solana", SOLANA));
        assert!(!same("solana", "base"));
    }

    #[test]
    fn a_caip2_reference_keeps_its_case_because_base58_is_case_sensitive() {
        // Solana's CAIP-2 reference is a base58 genesis hash. Lowercasing it
        // names no chain — the same rule `identity` keeps for Solana
        // addresses. The namespace beside it is still case-insensitive.
        assert_eq!(canonical(SOLANA), SOLANA);
        assert!(same(
            "SOLANA:5eykt4NQHMXjUTzZ7wm2Ge8Rm2Nfxvfg8P8NJdTREpY1",
            SOLANA
        ));
        assert!(!same(
            "solana:5eykt4nqhmxjutzz7wm2ge8rm2nfxvfg8p8njdtrepy1",
            SOLANA
        ));
    }
}
