//! # sellers — the Seller Census's rules, without the world attached.
//!
//! Instrument 02 asks whether the x402 economy everyone cites is real: who
//! is actually selling, and does it hold up. This crate is the part of that
//! question a reader can check without a catalog, a network, or a funded
//! wallet — the method of `METHODOLOGY.md` §10 expressed as functions:
//!
//! * [`identity`] — what counts as one seller (§10.1).
//! * [`network`] — one name per network, whichever convention a catalog
//!   wrote (`base` and `eip155:8453` are the same chain, and treating them
//!   as two would put the whole Bazaar out of scope).
//! * [`catalog`] — how many catalogs' listings become one population, losing
//!   nothing silently (§10.2), with one adapter per catalog in [`sources`].
//! * [`quote`] — whether a 402 is a quote a buyer can act on (§10.3, rung 3).
//! * [`reachable`] — rungs 2 and 3 judged from one observation, including
//!   the rule that a 402 is a seller working rather than a seller declining.
//! * [`consistent`] — rung 7: whether the catalog's claim and the endpoint's
//!   quote agree, field by field.
//! * [`shop`] — what this census is allowed to pay for (§10.4).
//!
//! **PURE**, like `crates/checks` and `crates/payments`, and for the same
//! reason: the crux of this instrument is its rules, and a rule that can
//! only be exercised against a live catalog and a funded wallet is a rule
//! nobody re-checks. The crawler, the prober and the shopper are binaries
//! that call into here; none of their I/O lives here.
//!
//! ## What this crate deliberately cannot do
//!
//! It cannot fetch, pay, score, rank, or aggregate. It has no notion of a
//! "good" seller and no arithmetic that combines rungs — the ladder produces
//! per-seller answers and population rates, exactly as the registration
//! census does, and the reason is `METHODOLOGY.md` §1: a boolean is
//! falsifiable, a score is not.

pub mod catalog;
pub mod consistent;
pub mod identity;
pub mod network;
pub mod quote;
pub mod reachable;
pub mod shop;
pub mod sources;

/// The seller-census semantics' own version, stamped onto every seller sweep
/// so a stored answer names the rules that produced it — the same contract
/// `checks::CHECKER_VERSION` has with the registration census.
///
/// 0.1.0 is METHODOLOGY §10 as locked on 2026-08-20: six questions with rung
/// 5 (`receipted`) reserved, robots.txt binding every request, the $0.10
/// stablecoin cap, and Base/USDC scope.
pub const SELLER_CHECKER_VERSION: &str = "0.1.0";

/// The answer to one rung, for one seller.
///
/// The registration census's vocabulary verbatim (`METHODOLOGY.md` §4) plus
/// one word this instrument needs. Nothing here is a score, and three of
/// these are never publishable as the seller's failure — see
/// [`SellerStatus::is_about_the_seller`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SellerStatus {
    /// The question was asked and the answer was yes.
    Pass,
    /// The question was asked and the answer was no. The seller's word.
    Fail,
    /// A prerequisite rung did not pass, so this question could not be asked.
    Skipped,
    /// OUR failure — a timeout, a TLS error, a prober that fell over. Never
    /// the seller's, and never churn (§9's rule, which this instrument
    /// inherits from day one rather than after an incident).
    Error,
    /// The origin is demonstrably there and declined us: a rate limit, an
    /// auth challenge, a robots.txt that said no. Not the seller's failure
    /// and not ours.
    Refused,
    /// **We chose not to ask, and the row says why** — the word this
    /// instrument adds. A resource priced above the cap, quoted in an asset
    /// we cannot read, or on a network this sweep does not cover is
    /// `unprobed`, and the count is published beside every rate it
    /// qualifies. See [`shop::Unprobed`].
    Unprobed,
}

impl SellerStatus {
    /// Whether this status says something about the SELLER, as opposed to
    /// about the conversation or about us.
    ///
    /// The one predicate that keeps a delivery rate honest: only `pass` and
    /// `fail` are facts about the seller, so only they may appear in a
    /// numerator or a denominator that is described as being about sellers.
    /// Everything else is published as its own count.
    pub fn is_about_the_seller(self) -> bool {
        matches!(self, Self::Pass | Self::Fail)
    }

    /// The word as it is stored and served — one spelling, everywhere.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skipped => "skipped",
            Self::Error => "error",
            Self::Refused => "refused",
            Self::Unprobed => "unprobed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pass_and_fail_are_facts_about_the_seller() {
        // The predicate a delivery rate's denominator depends on. If
        // `unprobed` or `refused` ever counted as a seller-fact, "X% of
        // sellers delivered" would silently include sellers nobody asked.
        assert!(SellerStatus::Pass.is_about_the_seller());
        assert!(SellerStatus::Fail.is_about_the_seller());
        for s in [
            SellerStatus::Skipped,
            SellerStatus::Error,
            SellerStatus::Refused,
            SellerStatus::Unprobed,
        ] {
            assert!(!s.is_about_the_seller(), "{s:?} is not the seller's answer");
        }
    }

    #[test]
    fn the_status_words_match_the_registration_censuss_spelling() {
        // Two instruments using different words for the same idea would make
        // every cross-instrument sentence a translation.
        assert_eq!(SellerStatus::Pass.as_str(), "pass");
        assert_eq!(SellerStatus::Fail.as_str(), "fail");
        assert_eq!(SellerStatus::Skipped.as_str(), "skipped");
        assert_eq!(SellerStatus::Error.as_str(), "error");
        assert_eq!(SellerStatus::Refused.as_str(), "refused");
        assert_eq!(SellerStatus::Unprobed.as_str(), "unprobed");
    }

    #[test]
    fn the_version_is_the_method_this_crate_implements() {
        assert_eq!(SELLER_CHECKER_VERSION, "0.1.0");
    }
}
