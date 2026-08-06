//! Which address is "the agent's", and on whose authority.
//!
//! The rule itself is stated in the crate doc comment. This module is the
//! executable form of it: given what the chain and the document each said about
//! one agent, produce the set of addresses a transfer may be attributed
//! through — and, for every address that does *not* qualify, the named reason.
//!
//! **An ineligible target is recorded, not dropped.** `payment_targets` keeps a
//! row for every agent on every basis, eligible or not, because "this agent's
//! wallet equals its owner" and "this agent received nothing" are different
//! facts and a table that stores only the eligible ones cannot tell them apart.

use serde::{Deserialize, Serialize};

/// The all-zero address. A transfer *to* it is a burn — the opposite of
/// revenue — and no key can sign for it.
pub const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

/// The conventional "dead" burn address. Not zero, not a contract, and by
/// long-standing convention not spendable.
pub const DEAD_ADDRESS: &str = "0x000000000000000000000000000000000000dead";

/// Which authority an address is claimed under.
///
/// Stored as a column on every `payments` and `payment_targets` row so the two
/// can never be unioned by accident. See the crate doc for why only the first
/// is publishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// `getAgentWallet(agentId)` at the pinned block, non-zero and distinct
    /// from `ownerOf(agentId)`. Spec-defined, signature-verified, cleared on
    /// transfer. **The publishable basis.**
    VerifiedWallet,
    /// A `services[]` entry whose `name` is `agentWallet`. Not in the spec, not
    /// verified, writable by anyone who controls the document. Recorded so the
    /// gap against the verified basis is measurable; never the headline.
    DeclaredWallet,
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Basis::VerifiedWallet => "verified_wallet",
            Basis::DeclaredWallet => "declared_wallet",
        }
    }

    /// Whether a count on this basis may be published as "agents paid".
    ///
    /// A method, not a comment, so the binary and any future consumer read the
    /// same answer rather than each remembering the rule.
    pub fn is_publishable(self) -> bool {
        matches!(self, Basis::VerifiedWallet)
    }
}

/// Why an address is not a target this run will attribute a transfer through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ineligible {
    /// `getAgentWallet` returned the zero address. The spec says the value
    /// defaults to the owner, yet 19,624 of Base's 60,097 agents return zero —
    /// consistent with clearing on transfer, with registration paths that never
    /// set it, or with the deployment differing from the spec text. The census
    /// does not attribute the cause and does not attribute a payment either.
    WalletUnset,
    /// `getAgentWallet` equals `ownerOf` — the contract's default, which needs
    /// no signature and is not per-agent. **Part of the rule, not a filter**;
    /// see the crate doc.
    WalletEqualsOwner,
    /// The document declared no `agentWallet` service entry, or declared one
    /// carrying no parseable `0x…` address. (One of Base's 920 declaring
    /// documents is exactly that.)
    NoDeclaredAddress,
    /// The address is a burn address. **PAY-4.** Reachable only on the declared
    /// basis: nothing can produce an EIP-712 signature for the zero address, so
    /// `setAgentWallet` cannot name one.
    BurnAddress,
}

impl Ineligible {
    pub fn as_str(self) -> &'static str {
        match self {
            Ineligible::WalletUnset => "wallet_unset",
            Ineligible::WalletEqualsOwner => "wallet_equals_owner",
            Ineligible::NoDeclaredAddress => "no_declared_address",
            Ineligible::BurnAddress => "burn_address",
        }
    }
}

/// What one agent's chain reads and archived document say about payment
/// addresses. All addresses are lowercase hex; use [`normalise_address`] at the
/// boundary so nothing downstream has to remember to.
#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub agent_id: u64,
    /// `ownerOf(agentId)` at the pinned block.
    pub owner: String,
    /// `getAgentWallet(agentId)` at the pinned block. `None` when the call
    /// could not be made at all — which is **not** the same as the zero address
    /// and must not be recorded as [`Ineligible::WalletUnset`]; the binary
    /// writes no target row for an unread wallet.
    pub verified_wallet: Option<String>,
    /// Every `0x…` address found in a `services[]` entry named `agentWallet`,
    /// in declared order.
    ///
    /// All of them, not the first. `analysis/payments-design.md` §4 records
    /// that the spec defines no precedence rule here — "any implementation that
    /// picks the first entry and any implementation that picks the last will
    /// disagree, and both will be correct" — so this pipeline picks neither and
    /// records every address the document named, each as its own target row.
    /// An agent-level "has ever been paid" is an OR over them, which no choice
    /// of precedence would change.
    pub declared_wallets: Vec<String>,
    /// The block this agent's `Registered` event was emitted in. `None` when
    /// the run did not capture it (`agent_snapshots.registration_block` is
    /// NULL for runs predating migration 0013). Absence is fatal to attribution
    /// — see [`crate::Exclusion::MintBlockUnknown`].
    pub registration_block: Option<u64>,
}

/// One address this run will scan for, and the agent it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub agent_id: u64,
    pub basis: Basis,
    pub address: String,
    /// Position in `services[]` for a declared target, `None` for a verified
    /// one. Recorded because the spec has no precedence rule and a reader who
    /// wants "first entry wins" must be able to reconstruct it.
    pub declared_index: Option<usize>,
}

/// A target and its verdict. Eligible targets get scanned; ineligible ones are
/// stored with their reason and never scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDecision {
    pub target: Target,
    /// `None` when this address is eligible.
    pub ineligible: Option<Ineligible>,
}

impl TargetDecision {
    pub fn is_eligible(&self) -> bool {
        self.ineligible.is_none()
    }
}

/// Lowercase, whitespace-trimmed hex — or `None` if the string is not a
/// 20-byte hex address.
///
/// Deliberately strict. The declared basis is an attacker-controlled string:
/// `analysis/payments-per-chain.md` §2 is the record of what happens when a
/// value from that field is fed to a log filter without being checked. A
/// checksummed address, a padded 32-byte word, and `AGENT_WALLET` are all
/// rejected here rather than coerced into something that would match a topic.
pub fn normalise_address(raw: &str) -> Option<String> {
    let s = raw.trim();
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", hex.to_ascii_lowercase()))
}

/// **PAY-4.** Is this a burn address — an address that cannot be a payee?
///
/// The zero address and `0x…dead`, matched case-insensitively on the
/// normalised form. Both are excluded from the target set and from every
/// funnel. Checked across all four chains when the rule was written: Base and
/// BSC have no such declaration, so no previously reported figure moved on
/// account of it — mainnet's did, by 99.5% of its transfers.
pub fn is_burn_address(address: &str) -> bool {
    let a = address.trim().to_ascii_lowercase();
    a == ZERO_ADDRESS || a == DEAD_ADDRESS
}

/// Every target this agent contributes on one basis, eligible or not.
///
/// Returns an empty vector only when there is nothing to record at all — a
/// verified wallet that was never read. Every other outcome produces at least
/// one row, because "we looked and this agent has no attributable address" is a
/// fact worth storing.
pub fn targets_for(identity: &AgentIdentity, basis: Basis) -> Vec<TargetDecision> {
    match basis {
        Basis::VerifiedWallet => verified_targets(identity),
        Basis::DeclaredWallet => declared_targets(identity),
    }
}

fn verified_targets(identity: &AgentIdentity) -> Vec<TargetDecision> {
    // Not read at all → no row. `None` here means the `getAgentWallet` call
    // failed or was never made, and writing `wallet_unset` for it would turn
    // "we did not ask" into "the chain said zero" — the distinction every
    // status in this project exists to keep.
    let Some(raw) = identity.verified_wallet.as_deref() else {
        return Vec::new();
    };
    let address = normalise_address(raw).unwrap_or_else(|| raw.trim().to_ascii_lowercase());
    let mk = |ineligible: Option<Ineligible>| TargetDecision {
        target: Target {
            agent_id: identity.agent_id,
            basis: Basis::VerifiedWallet,
            address: address.clone(),
            declared_index: None,
        },
        ineligible,
    };

    // Order matters and is tested. Zero is `WalletUnset` rather than
    // `BurnAddress` because on this basis it means the registry has no
    // verified wallet — the burn reading belongs to the declared basis, where
    // an agent actually named it.
    if is_burn_address(&address) {
        return vec![mk(Some(Ineligible::WalletUnset))];
    }
    if address
        == normalise_address(&identity.owner).unwrap_or_else(|| identity.owner.to_lowercase())
    {
        return vec![mk(Some(Ineligible::WalletEqualsOwner))];
    }
    vec![mk(None)]
}

fn declared_targets(identity: &AgentIdentity) -> Vec<TargetDecision> {
    if identity.declared_wallets.is_empty() {
        return vec![TargetDecision {
            target: Target {
                agent_id: identity.agent_id,
                basis: Basis::DeclaredWallet,
                address: String::new(),
                declared_index: None,
            },
            ineligible: Some(Ineligible::NoDeclaredAddress),
        }];
    }

    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (index, raw) in identity.declared_wallets.iter().enumerate() {
        let Some(address) = normalise_address(raw) else {
            out.push(TargetDecision {
                target: Target {
                    agent_id: identity.agent_id,
                    basis: Basis::DeclaredWallet,
                    address: raw.trim().to_string(),
                    declared_index: Some(index),
                },
                ineligible: Some(Ineligible::NoDeclaredAddress),
            });
            continue;
        };
        // 5 of the 7 Base agents declaring more than one entry repeat the same
        // address. Scanning it twice would cost calls; counting it twice would
        // cost correctness.
        if seen.contains(&address) {
            continue;
        }
        seen.push(address.clone());
        let ineligible = is_burn_address(&address).then_some(Ineligible::BurnAddress);
        out.push(TargetDecision {
            target: Target {
                agent_id: identity.agent_id,
                basis: Basis::DeclaredWallet,
                address,
                declared_index: Some(index),
            },
            ineligible,
        });
    }
    out
}

/// Every `0x…` address a registration document declares under the
/// `agentWallet` convention, in declared order.
///
/// Reads `services` with rung 4's alias rule (`endpoints` is a legacy alias,
/// `services` wins when both are present) so this and rung 6 cannot disagree
/// about whether an agent declared anything. Entries are matched on
/// `name == "agentWallet"` exactly — case-sensitively, because that is how the
/// 920 documents in the reference population spell it and a looser match would
/// silently widen a population the census has already published a count for.
///
/// The address is taken from `address`, then `endpoint`, then `value`: the
/// convention has no schema, and all three spellings appear in the wild. Every
/// candidate is put through [`normalise_address`], so a non-address string in
/// any of them yields nothing rather than a target.
pub fn declared_wallets(document: &[u8]) -> Vec<String> {
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(document) else {
        return Vec::new();
    };
    let Some(services) = doc
        .get("services")
        .filter(|v| !v.is_null())
        .or_else(|| doc.get("endpoints").filter(|v| !v.is_null()))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    services
        .iter()
        .filter(|e| e.get("name").and_then(|v| v.as_str()) == Some("agentWallet"))
        .filter_map(|e| {
            ["address", "endpoint", "value"]
                .iter()
                .find_map(|k| e.get(*k).and_then(|v| v.as_str()))
                .and_then(normalise_address)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(owner: &str, verified: Option<&str>) -> AgentIdentity {
        AgentIdentity {
            agent_id: 1,
            owner: owner.to_string(),
            verified_wallet: verified.map(str::to_string),
            declared_wallets: Vec::new(),
            registration_block: Some(100),
        }
    }

    const OWNER: &str = "0x820c5091b047b652888f6aa7e1ee615d99f7c8cd";
    const DISTINCT: &str = "0x93862e5b1c0a5a0e4f0d7a2b3c4d5e6f70819293";

    // ── THE RULE ─────────────────────────────────────────────────────────────

    #[test]
    fn the_rule_a_verified_wallet_distinct_from_the_owner_is_the_only_attributable_address() {
        let d = targets_for(&agent(OWNER, Some(DISTINCT)), Basis::VerifiedWallet);
        assert_eq!(d.len(), 1);
        assert!(d[0].is_eligible());
        assert_eq!(d[0].target.address, DISTINCT);
        assert_eq!(d[0].target.basis, Basis::VerifiedWallet);
    }

    #[test]
    fn the_rule_a_wallet_equal_to_the_owner_is_the_contract_default_and_is_not_attributable() {
        // 40,126 of Base's 40,473 set wallets (99.1%) are exactly this. They
        // required no `setAgentWallet` and therefore no signature, and one
        // owner holds 2,293 agents — a transfer there is evidence about an
        // operator, not about an agent.
        let d = targets_for(&agent(OWNER, Some(OWNER)), Basis::VerifiedWallet);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].ineligible, Some(Ineligible::WalletEqualsOwner));
    }

    #[test]
    fn the_rule_is_case_insensitive_about_the_owner_comparison() {
        // A checksummed `getAgentWallet` against a lowercase `ownerOf` must not
        // read as "distinct". That would promote 40,126 defaults into the
        // publishable population on a formatting difference.
        let checksummed = "0x820C5091B047B652888F6AA7E1EE615D99F7C8CD";
        let d = targets_for(&agent(OWNER, Some(checksummed)), Basis::VerifiedWallet);
        assert_eq!(d[0].ineligible, Some(Ineligible::WalletEqualsOwner));
    }

    #[test]
    fn a_zero_verified_wallet_is_unset_not_paid_zero() {
        let d = targets_for(&agent(OWNER, Some(ZERO_ADDRESS)), Basis::VerifiedWallet);
        assert_eq!(d[0].ineligible, Some(Ineligible::WalletUnset));
    }

    #[test]
    fn a_wallet_that_was_never_read_produces_no_row_at_all() {
        // "We did not ask" must not become "the chain said zero". The binary
        // writes nothing, and a later pass can fill it in.
        assert!(targets_for(&agent(OWNER, None), Basis::VerifiedWallet).is_empty());
    }

    #[test]
    fn the_two_bases_never_share_a_row() {
        let mut a = agent(OWNER, Some(DISTINCT));
        a.declared_wallets = vec![DISTINCT.to_string()];
        let v = targets_for(&a, Basis::VerifiedWallet);
        let d = targets_for(&a, Basis::DeclaredWallet);
        assert_eq!(v[0].target.basis, Basis::VerifiedWallet);
        assert_eq!(d[0].target.basis, Basis::DeclaredWallet);
        // Same address, two rows, two bases. A union is possible only if
        // somebody writes `WHERE basis IN (…)` on purpose.
        assert_eq!(v[0].target.address, d[0].target.address);
        assert!(Basis::VerifiedWallet.is_publishable());
        assert!(!Basis::DeclaredWallet.is_publishable());
    }

    // ── PAY-4 regression ─────────────────────────────────────────────────────

    #[test]
    fn pay4_a_declared_burn_address_is_never_a_target() {
        // Mainnet agent 28283 declares the zero address. The uncorrected scan
        // collected every USDC and USDT burn on Ethereum as its income:
        // 313,255 of mainnet's 314,735 transfers, 99.5%.
        for burn in [
            ZERO_ADDRESS,
            DEAD_ADDRESS,
            "0x000000000000000000000000000000000000DEAD",
        ] {
            let mut a = agent(OWNER, None);
            a.declared_wallets = vec![burn.to_string()];
            let d = targets_for(&a, Basis::DeclaredWallet);
            assert_eq!(d.len(), 1, "{burn}");
            assert_eq!(d[0].ineligible, Some(Ineligible::BurnAddress), "{burn}");
        }
        assert!(is_burn_address(ZERO_ADDRESS));
        assert!(is_burn_address(DEAD_ADDRESS));
        assert!(!is_burn_address(DISTINCT));
    }

    #[test]
    fn pay4_cannot_happen_on_the_verified_basis() {
        // Nothing can produce an EIP-712 signature for the zero address, so
        // `setAgentWallet` cannot name one — and if the getter returns zero it
        // means "unset", never "this agent chose to be paid at a burn address".
        let d = targets_for(&agent(OWNER, Some(ZERO_ADDRESS)), Basis::VerifiedWallet);
        assert_ne!(d[0].ineligible, Some(Ineligible::BurnAddress));
    }

    // ── the declared basis, and what it lets through ─────────────────────────

    #[test]
    fn a_declared_entry_with_no_parseable_address_is_recorded_not_dropped() {
        // One of Base's 920 declaring documents carries no `0x…` address at all.
        let mut a = agent(OWNER, None);
        a.declared_wallets = vec![];
        let d = targets_for(&a, Basis::DeclaredWallet);
        assert_eq!(d[0].ineligible, Some(Ineligible::NoDeclaredAddress));
    }

    #[test]
    fn every_declared_address_becomes_its_own_target_and_duplicates_collapse() {
        // 7 Base agents declare more than one entry; 5 repeat the same address
        // and 2 name genuinely different ones. The spec defines no precedence
        // rule, so both are kept and neither is called "the" wallet.
        let mut a = agent(OWNER, None);
        a.declared_wallets = vec![
            DISTINCT.to_string(),
            DISTINCT.to_uppercase().replace("0X", "0x"),
            "0x1234567890abcdef1234567890abcdef12345678".to_string(),
        ];
        let d = targets_for(&a, Basis::DeclaredWallet);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].target.declared_index, Some(0));
        assert_eq!(d[1].target.declared_index, Some(2));
    }

    #[test]
    fn normalise_refuses_anything_that_is_not_a_twenty_byte_address() {
        assert_eq!(
            normalise_address("  0xABCdef0000000000000000000000000000000001 ").as_deref(),
            Some("0xabcdef0000000000000000000000000000000001")
        );
        for bad in [
            "0x",
            "not an address",
            "AGENT_WALLET",
            // A 32-byte topic word, which WOULD match a log filter if coerced.
            "0x000000000000000000000000abcdef0000000000000000000000000000000001",
            "0xzzzzef0000000000000000000000000000000001",
            "abcdef0000000000000000000000000000000001",
        ] {
            assert!(normalise_address(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn declared_wallets_are_read_with_rung_4s_alias_rule() {
        let doc = br#"{"services":[
            {"name":"a2a","endpoint":"https://x/"},
            {"name":"agentWallet","address":"0xABCdef0000000000000000000000000000000001"},
            {"name":"agentWallet","endpoint":"0x1234567890abcdef1234567890abcdef12345678"},
            {"name":"agentWallet","value":"0x000000000000000000000000000000000000dEaD"},
            {"name":"agentWallet","address":"eip155:8453:0xdeadbeef"},
            {"name":"AgentWallet","address":"0x9999999999999999999999999999999999999999"}
        ]}"#;
        let got = declared_wallets(doc);
        assert_eq!(
            got,
            vec![
                "0xabcdef0000000000000000000000000000000001",
                "0x1234567890abcdef1234567890abcdef12345678",
                DEAD_ADDRESS,
            ],
            "a CAIP-10 string yields nothing, and the name match is exact"
        );

        // The legacy alias is accepted; `services` wins when both are present.
        let legacy = br#"{"endpoints":[{"name":"agentWallet","address":"0x1234567890abcdef1234567890abcdef12345678"}]}"#;
        assert_eq!(declared_wallets(legacy).len(), 1);
        let both = br#"{"services":[],"endpoints":[{"name":"agentWallet","address":"0x1234567890abcdef1234567890abcdef12345678"}]}"#;
        assert!(declared_wallets(both).is_empty());
    }

    #[test]
    fn unparseable_documents_never_panic_and_declare_nothing() {
        for bytes in [
            b"not json".as_slice(),
            b"",
            &[0xff, 0xfe],
            b"{\"services\":{}}",
        ] {
            assert!(declared_wallets(bytes).is_empty());
        }
    }
}
