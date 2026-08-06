//! The exclusions, each a named rule with its own regression test.
//!
//! Every one of these exists because a published-looking number was wrong
//! without it. They are not a filter chain somebody assembled by taste: the
//! order is fixed, each rule has a name that appears verbatim in the
//! `payments.exclusion` column, and a row that was excluded says which rule
//! excluded it. A query can therefore reproduce the uncorrected figure as well
//! as the corrected one, which is the only way a reader can check that a
//! correction did what it claims.

use serde::{Deserialize, Serialize};

/// Which way the value moved, relative to the credited address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// The credited address is the `to` of the `Transfer`.
    In,
    /// The credited address is the `from`. Recorded, never counted as payment.
    ///
    /// Scanned because a balance says nothing — "funds received and swept out
    /// leave nothing" is why this measurement is over transfers at all — and
    /// because an address whose entire history is outgoing is visibly not a
    /// payee. Storing it costs one filter per token and buys the ability to
    /// answer "did this address ever do anything" without a second scan.
    Out,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Out => "out",
        }
    }
}

/// Why a transfer is not counted as a payment to this agent.
///
/// `None` from [`classify`] means counted. Every variant here is a *named rule*
/// stored on the row, so `SELECT exclusion, count(*)` is the audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exclusion {
    /// **PAY-4.** The credited address is a burn address, or the counterparty
    /// is. A transfer to the zero address is a burn — the opposite of revenue.
    ///
    /// Target-level burns are caught earlier, at
    /// [`crate::targets_for`]; this catches the transfer-level case where the
    /// *sender* is the zero address, which is a mint, not a payment.
    BurnAddress,
    /// **PAY-2.** The transfer arrived before the agent was minted. A wallet's
    /// history before the agent existed is not the agent's history.
    ///
    /// On Base this removed **6,748 of 18,328** transfers by count and
    /// **82.5% of all value** — $5,508,441 of $6,675,025 — and took the
    /// paid-agent count from 313 to 190. It is the single largest correction in
    /// the ledger, and the retracted "$8.8M received" is exactly what counting
    /// these produces.
    PreMint,
    /// **PAY-2, the honest half.** The agent's registration block is not
    /// recorded, so whether the transfer predates the mint is unknowable.
    ///
    /// Excluded rather than assumed post-mint. Assuming the favourable side of
    /// an unknown is how 88% of a headline turned out to predate the thing it
    /// was attributed to. A run whose sweep captured no `registration_block`
    /// (anything predating migration 0013) produces only this exclusion, which
    /// is a loud and correct way to fail.
    MintBlockUnknown,
    /// The sender is the agent's own NFT owner at the pinned block. The owner
    /// funding its own agent's wallet is not income.
    ///
    /// **One hop, and stated as one.** A previous owner is not caught (42
    /// transfers across 13 agents on Base), and an owner routing through a
    /// fresh intermediary is not caught at all. "External" therefore remains an
    /// upper bound on earnings.
    OwnerFunding,
    /// The credited address paid itself. Not a correction anybody was forced
    /// into — it is arithmetic — but a self-transfer satisfies every other rule
    /// here, and a rule set that lets an address manufacture its own payment
    /// history has no business publishing a count.
    SelfTransfer,
    /// The value moved away from the credited address. Never a payment to it.
    Outgoing,
}

impl Exclusion {
    pub fn as_str(self) -> &'static str {
        match self {
            Exclusion::BurnAddress => "burn_address",
            Exclusion::PreMint => "pre_mint",
            Exclusion::MintBlockUnknown => "mint_block_unknown",
            Exclusion::OwnerFunding => "owner_funding",
            Exclusion::SelfTransfer => "self_transfer",
            Exclusion::Outgoing => "outgoing",
        }
    }
}

/// One token transfer, as the log and the run's own rows describe it.
///
/// Every field is a fact somebody read, not a judgement. The judgement is
/// [`classify`]'s and nothing else's.
#[derive(Debug, Clone)]
pub struct TransferFacts<'a> {
    /// The agent's payment address — the one the attribution rule chose.
    pub credited_address: &'a str,
    pub direction: Direction,
    /// The other end of the transfer: the `from` for [`Direction::In`].
    pub counterparty: &'a str,
    /// `ownerOf(agentId)` at the pinned block.
    pub agent_owner: &'a str,
    /// The block the agent's `Registered` event was emitted in.
    pub agent_registration_block: Option<u64>,
    /// The block this transfer was mined in.
    pub block_number: u64,
    /// Whether the counterparty has code at the pinned block. `None` means the
    /// `eth_getCode` was not made — see [`crate::Summary`] for why that may
    /// never be defaulted either way.
    pub counterparty_is_contract: Option<bool>,
    /// Whether an EIP-3009 `AuthorizationUsed` from the same token appears in
    /// the same transaction — the x402 settlement signature.
    pub eip3009_authorization: bool,
}

/// Counted, or excluded by a named rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Attributable to the agent as an incoming transfer in scope. **Not**
    /// "earned": direction is not purpose.
    Counted,
    Excluded(Exclusion),
}

impl Verdict {
    pub fn is_counted(self) -> bool {
        matches!(self, Verdict::Counted)
    }
    /// The value stored in `payments.exclusion`; `None` for a counted row.
    pub fn exclusion(self) -> Option<Exclusion> {
        match self {
            Verdict::Counted => None,
            Verdict::Excluded(e) => Some(e),
        }
    }
}

/// Apply every exclusion, in a fixed order, and say which one bit.
///
/// **The order is part of the rule.** Structural facts are tested before
/// attribution ones, so a transfer that is both outgoing and pre-mint is
/// reported as outgoing — the reason a reader would give if asked. Changing the
/// order changes what `SELECT exclusion, count(*)` means, which is why it is
/// pinned by a test rather than left to the reading order of an `if` chain.
///
/// 1. [`Exclusion::Outgoing`] — wrong direction; nothing else applies.
/// 2. [`Exclusion::BurnAddress`] — the transfer's own endpoints.
/// 3. [`Exclusion::SelfTransfer`] — the address paying itself.
/// 4. [`Exclusion::OwnerFunding`] — the owner funding its agent.
/// 5. [`Exclusion::MintBlockUnknown`] — we cannot place it in time.
/// 6. [`Exclusion::PreMint`] — it predates the agent.
pub fn classify(f: &TransferFacts<'_>) -> Verdict {
    use Exclusion::*;

    if f.direction == Direction::Out {
        return Verdict::Excluded(Outgoing);
    }
    if crate::is_burn_address(f.credited_address) || crate::is_burn_address(f.counterparty) {
        return Verdict::Excluded(BurnAddress);
    }
    if eq_address(f.counterparty, f.credited_address) {
        return Verdict::Excluded(SelfTransfer);
    }
    if eq_address(f.counterparty, f.agent_owner) {
        return Verdict::Excluded(OwnerFunding);
    }
    match f.agent_registration_block {
        None => Verdict::Excluded(MintBlockUnknown),
        Some(mint) if f.block_number < mint => Verdict::Excluded(PreMint),
        Some(_) => Verdict::Counted,
    }
}

/// Address equality, case-insensitive on the hex.
///
/// One place, so a checksummed value from one RPC and a lowercase value from
/// another cannot silently disagree. `ownerOf` and a log topic reach this
/// function from different code paths.
fn eq_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DEAD_ADDRESS, ZERO_ADDRESS};

    const WALLET: &str = "0x93862e5b1c0a5a0e4f0d7a2b3c4d5e6f70819293";
    const OWNER: &str = "0x820c5091b047b652888f6aa7e1ee615d99f7c8cd";
    const STRANGER: &str = "0x48380bcf1c09773c9e96901f89a7a6b75e2bbecc";
    const MINT_BLOCK: u64 = 45_000_000;

    fn incoming(from: &'static str, block: u64) -> TransferFacts<'static> {
        TransferFacts {
            credited_address: WALLET,
            direction: Direction::In,
            counterparty: from,
            agent_owner: OWNER,
            agent_registration_block: Some(MINT_BLOCK),
            block_number: block,
            counterparty_is_contract: Some(false),
            eip3009_authorization: false,
        }
    }

    #[test]
    fn an_external_post_mint_incoming_transfer_is_counted() {
        assert_eq!(
            classify(&incoming(STRANGER, MINT_BLOCK + 1)),
            Verdict::Counted
        );
        // The mint block itself counts: the agent existed in that block.
        assert_eq!(classify(&incoming(STRANGER, MINT_BLOCK)), Verdict::Counted);
    }

    // ── PAY-2 regression ─────────────────────────────────────────────────────

    #[test]
    fn pay2_a_transfer_before_the_mint_is_not_the_agents() {
        // "Claimed: 313 agents paid, $8,845,244 received, median $37."
        // 6,748 of 18,328 external Base transfers predate the mint of every
        // agent declaring that address; 123 agents had NOTHING but pre-mint
        // money. Corrected: 190 agents, $1,090,098, median $16 — value
        // overstated 8x, 88% of it predating the agents it was credited to.
        assert_eq!(
            classify(&incoming(STRANGER, MINT_BLOCK - 1)),
            Verdict::Excluded(Exclusion::PreMint)
        );
        assert_eq!(
            classify(&incoming(STRANGER, 0)),
            Verdict::Excluded(Exclusion::PreMint)
        );
    }

    #[test]
    fn pay2_an_unknown_mint_block_excludes_rather_than_assuming_post_mint() {
        let mut f = incoming(STRANGER, MINT_BLOCK + 1);
        f.agent_registration_block = None;
        assert_eq!(classify(&f), Verdict::Excluded(Exclusion::MintBlockUnknown));
    }

    #[test]
    fn pay2_the_uncorrected_figure_is_still_recomputable_from_the_rows() {
        // The point of storing the exclusion rather than dropping the row: a
        // reader can reproduce the retracted number and see the correction
        // work, instead of taking "we fixed it" on trust.
        let rows = [
            incoming(STRANGER, MINT_BLOCK - 1),
            incoming(STRANGER, MINT_BLOCK + 1),
            incoming(STRANGER, MINT_BLOCK + 2),
        ];
        let verdicts: Vec<_> = rows.iter().map(classify).collect();
        let counted = verdicts.iter().filter(|v| v.is_counted()).count();
        let uncorrected = verdicts
            .iter()
            .filter(|v| v.is_counted() || v.exclusion() == Some(Exclusion::PreMint))
            .count();
        assert_eq!((counted, uncorrected), (2, 3));
    }

    // ── owner funding ────────────────────────────────────────────────────────

    #[test]
    fn a_transfer_from_the_agents_own_owner_is_not_income() {
        assert_eq!(
            classify(&incoming(OWNER, MINT_BLOCK + 1)),
            Verdict::Excluded(Exclusion::OwnerFunding)
        );
    }

    #[test]
    fn owner_funding_is_matched_case_insensitively() {
        let mut f = incoming(STRANGER, MINT_BLOCK + 1);
        let checksummed = "0x820C5091B047B652888F6AA7E1EE615D99F7C8CD";
        f.counterparty = checksummed;
        assert_eq!(classify(&f), Verdict::Excluded(Exclusion::OwnerFunding));
    }

    // ── PAY-4 regression, at transfer level ──────────────────────────────────

    #[test]
    fn pay4_a_burn_never_counts_in_either_position() {
        // Credited address is a burn: mainnet agent 28283's declaration, which
        // collected 313,255 of mainnet's 314,735 transfers (99.5%).
        let mut to_burn = incoming(STRANGER, MINT_BLOCK + 1);
        to_burn.credited_address = ZERO_ADDRESS;
        assert_eq!(
            classify(&to_burn),
            Verdict::Excluded(Exclusion::BurnAddress)
        );

        let mut dead = incoming(STRANGER, MINT_BLOCK + 1);
        dead.credited_address = DEAD_ADDRESS;
        assert_eq!(classify(&dead), Verdict::Excluded(Exclusion::BurnAddress));

        // Sender is the zero address: that is a MINT of the token, not a
        // payment by anybody.
        assert_eq!(
            classify(&incoming(ZERO_ADDRESS, MINT_BLOCK + 1)),
            Verdict::Excluded(Exclusion::BurnAddress)
        );
    }

    // ── direction and self-payment ───────────────────────────────────────────

    #[test]
    fn an_outgoing_transfer_is_recorded_and_never_counted() {
        let mut f = incoming(STRANGER, MINT_BLOCK + 1);
        f.direction = Direction::Out;
        assert_eq!(classify(&f), Verdict::Excluded(Exclusion::Outgoing));
    }

    #[test]
    fn an_address_cannot_pay_itself_into_the_population() {
        let mut f = incoming(STRANGER, MINT_BLOCK + 1);
        f.counterparty = WALLET;
        assert_eq!(classify(&f), Verdict::Excluded(Exclusion::SelfTransfer));
    }

    // ── the order is part of the rule ────────────────────────────────────────

    #[test]
    fn the_exclusion_order_is_fixed_and_reported_reason_is_the_first_that_bites() {
        // Outgoing AND pre-mint AND from the owner → reported as outgoing.
        let mut f = incoming(OWNER, MINT_BLOCK - 1);
        f.direction = Direction::Out;
        assert_eq!(classify(&f), Verdict::Excluded(Exclusion::Outgoing));

        // From the owner AND pre-mint → owner funding, because that is the
        // reason a reader would give.
        let f = incoming(OWNER, MINT_BLOCK - 1);
        assert_eq!(classify(&f), Verdict::Excluded(Exclusion::OwnerFunding));

        // Burn beats self-transfer: the zero address paying "itself".
        let mut f = incoming(ZERO_ADDRESS, MINT_BLOCK + 1);
        f.credited_address = ZERO_ADDRESS;
        assert_eq!(classify(&f), Verdict::Excluded(Exclusion::BurnAddress));
    }

    #[test]
    fn every_exclusion_has_a_stable_stored_name() {
        // These strings are in a CHECK constraint (migration 0019) and in
        // published queries. Renaming one silently would orphan both.
        for (e, s) in [
            (Exclusion::BurnAddress, "burn_address"),
            (Exclusion::PreMint, "pre_mint"),
            (Exclusion::MintBlockUnknown, "mint_block_unknown"),
            (Exclusion::OwnerFunding, "owner_funding"),
            (Exclusion::SelfTransfer, "self_transfer"),
            (Exclusion::Outgoing, "outgoing"),
        ] {
            assert_eq!(e.as_str(), s);
        }
    }
}
