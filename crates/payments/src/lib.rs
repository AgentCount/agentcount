//! # payments — who an incoming stablecoin transfer may be attributed to.
//!
//! This crate is one rule and four exclusions. It contains no I/O, no rung, no
//! aggregate and no number. The binary that reads logs is
//! `crates/sweeper/src/bin/payments.rs`; everything it is allowed to *conclude*
//! is decided here, in functions a reader can run without a database.
//!
//! **Payments are not a rung and must never be rendered as one.** The ladder
//! asks seven questions about conformance to ERC-8004. "Has this agent ever
//! been paid" is a question about the chain's token transfers, judged against
//! no clause of the spec, and it lives in its own table (`payments`,
//! migration 0019) for that reason. See `METHODOLOGY.md` §8.
//!
//! ---
//!
//! # THE ATTRIBUTION RULE
//!
//! > **An incoming token transfer may be attributed to agent *A* only if it was
//! > credited to the address `getAgentWallet(A)` returned by the Identity
//! > Registry at the run's pinned block, that address is non-zero, and that
//! > address is not equal to `ownerOf(A)` at the same block.**
//!
//! That is [`Basis::VerifiedWallet`], and it is the basis every published
//! payment figure must be stated on. Everything else the pipeline records —
//! including the whole of [`Basis::DeclaredWallet`] — is recorded so the gap is
//! visible, and is never the headline.
//!
//! ## Why that address and not the other one
//!
//! There are two candidate addresses. They are not two conventions of equal
//! standing; they differ in whether anybody proved anything.
//!
//! | | `getAgentWallet(agentId)` | `services[].name == "agentWallet"` |
//! |---|---|---|
//! | where it lives | on-chain, reserved registry metadata | off-chain JSON document |
//! | in the pinned spec? | **yes** (`spec/ERC8004SPEC.md` line 141) | **no** — appears nowhere |
//! | who can set it | the owner, **with an EIP-712 / ERC-1271 signature proving control of the new address** | whoever can write the document |
//! | on NFT transfer | **cleared automatically**, must be re-verified | survives; nothing invalidates it |
//! | can it name an address nobody controls | no | **yes, and it does** — see [`Ineligible::BurnAddress`] |
//!
//! Four reasons, in the order they actually decide it:
//!
//! 1. **Only one of them is a payment address.** The spec reserves
//!    `agentWallet` and defines it as "the address where the agent receives
//!    payments". `services[]` is a list of *service descriptors*
//!    (`{name, endpoint, version, …}`); nothing reserves the name `agentWallet`
//!    inside it or gives it payment semantics. Measuring the second one
//!    measures adoption of a community convention, which is a real finding and
//!    a different one.
//! 2. **Only one of them carries a proof.** Changing `getAgentWallet` requires
//!    a signature from the address being named. A `services[]` entry is a
//!    string. A payer reading the document gets an address with no proof of
//!    control, no link to the on-chain identity, served over mutable HTTP — and
//!    on Base, in **409 of 919** cases, disagreeing with the address the
//!    registry has verified, with a further **50** the registry has never
//!    verified at all (`analysis/payments-design.md` §4).
//! 3. **Every retraction was the looser basis failing.** All four entries in
//!    `analysis/payments-corrections-ledger.md` are one mistake — *an address
//!    was treated as an identity* — and the looser basis is that mistake
//!    written into the method. PAY-4 is the clearest: mainnet agent 28283
//!    declares the zero address, and the scan dutifully collected **313,255 of
//!    mainnet's 314,735 transfers (99.5%)** — every USDC and USDT burn on
//!    Ethereum — as that agent's income. No agent can declare a burn address
//!    through `setAgentWallet`, because nothing can sign for it.
//! 4. **Only one of them self-invalidates.** `agentWallet` is cleared when the
//!    NFT is transferred, so a stale address cannot outlive a change of owner.
//!    A document can. `analysis/payments-per-chain.md` §1 records what happens
//!    when that is forgotten: an event-derived wallet set of 19,570 Base agents
//!    was **~99% stale**, because clearing emits no `MetadataSet`.
//!
//! ## Why "distinct from the owner" is part of the rule and not a filter
//!
//! `getAgentWallet` "is initially set to the owner's address" — the default.
//! On Base at block 49,262,617: **40,473** agents have it set, **40,126 of them
//! (99.1%) equal to the NFT owner**, and only **347** to a distinct address
//! (`analysis/identity-role-audit.md`, `analysis/payments-design.md` §6).
//!
//! The 40,126 defaults required no `setAgentWallet` call and therefore no
//! signature: they are not a claim by anybody. Worse, they are not per-agent.
//! One owner on Base holds **2,293 agents**; a transfer to that address is
//! evidence about an operator and cannot be assigned to any one of its agents
//! without inventing the assignment. Crediting it to all of them is PAY-1 with
//! a bigger denominator.
//!
//! So an agent whose wallet equals its owner is [`Ineligible::WalletEqualsOwner`]
//! — **not** paid-zero. It is a population the census declines to attribute to,
//! and the row says which, so no reader can mistake one for the other. That
//! cohort is reportable at operator level and nowhere else, and this pipeline
//! does not report it at all.
//!
//! ## What the looser basis is still for
//!
//! [`Basis::DeclaredWallet`] is computed and stored anyway, on every run, in
//! the same table with a `basis` column. Three reasons:
//!
//! * **The gap is the finding.** "N agents on the address the spec verifies, M
//!   on the address anyone can write" is a single query, and the distance
//!   between them is the most honest thing this pipeline produces.
//! * **The prior study is only comparable on it.** Every number in
//!   `analysis/payments-per-chain.md` — 358 paid, 34 via x402 — is
//!   declared-basis. Recording it is how a future run can say whether the old
//!   figure was wrong or merely superseded.
//! * **Refusing to compute it would hide it.** The convention exists, 920
//!   documents use it, and it is where nearly all previously observed activity
//!   landed. Not measuring it would be a choice about what readers get to see.
//!
//! It is never the headline, it is never blended with the verified basis into
//! one count, and a row on it carries `basis = 'declared_wallet'` so no query
//! can accidentally union the two.
//!
//! ## What the rule does not fix
//!
//! Stated here because the doc comment is the only place a reader is
//! guaranteed to look:
//!
//! * **Direction is not purpose.** An incoming stablecoin transfer is not proof
//!   a service was rendered. Airdrops, refunds, mistakes and self-transfers are
//!   indistinguishable from revenue on-chain.
//! * **"Not owner-funded" is one hop.** [`Exclusion::OwnerFunding`] compares
//!   against the owner at the pinned block. An owner routing through a fresh
//!   intermediary is not caught, and a *previous* owner is not caught either —
//!   42 transfers across 13 agents on Base, uncorrected
//!   (`analysis/identity-role-audit.md` §3).
//! * **Two tokens per chain.** Every count is a lower bound.
//! * **Contract-sourced flow is not revenue.** PAY-3 found the largest holder's
//!   "payments" were Morpho vault yield returning its own capital. The pipeline
//!   records [`TransferFacts::counterparty_is_contract`] and refuses to guess
//!   when it was not read — see [`Summary`].

mod basis;
mod exclusions;
mod summary;
mod units;

pub use basis::{
    AgentIdentity, Basis, DEAD_ADDRESS, Ineligible, Target, TargetDecision, ZERO_ADDRESS,
    declared_wallets, is_burn_address, normalise_address, targets_for,
};
pub use exclusions::{Direction, Exclusion, TransferFacts, Verdict, classify};
pub use summary::{PaidRow, Summary, summarise};
pub use units::{Token, format_units};

/// Bump when the shape of a `payments` row changes.
///
/// Stamped onto `payment_scans.rule_version` so a stored row names the rule
/// that produced it, exactly as `check_results` rows name their
/// `checker_version`. A figure recomputed under a different rule is a different
/// figure, and the row has to be able to say so.
///
/// 1 (2026-08-06): first version. The attribution rule above, the four
/// exclusions in [`Exclusion`], both bases stored side by side.
pub const RULE_VERSION: &str = "1";
