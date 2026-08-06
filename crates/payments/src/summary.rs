//! Counting the rows — the two places the previous study's arithmetic broke.
//!
//! [`summarise`] is not a headline generator. It produces the counts a report
//! is *allowed* to quote, shaped so the two mistakes below cannot be made from
//! it:
//!
//! * **PAY-1** — the address→agent map is many-to-many, so "addresses paid" and
//!   "agents whose address was paid" are different numbers. [`Summary`] carries
//!   both and no field that blends them.
//! * **PAY-3** — whether the sender has code decides whether a value figure is
//!   plausibly revenue at all. Senders whose code was never read get their own
//!   bucket; they are never folded into either side.

use crate::units::{Token, format_units};

/// One stored `payments` row, as the summariser needs it.
///
/// Rows handed to [`summarise`] must be scoped to **one basis and one token** —
/// values in different decimals cannot be added, and counts across bases must
/// never be unioned. The token is a parameter of [`summarise`] rather than a
/// field here so that scoping is structural.
#[derive(Debug, Clone)]
pub struct PaidRow<'a> {
    pub agent_id: u64,
    pub credited_address: &'a str,
    /// [`crate::Verdict::is_counted`] for this row.
    pub counted: bool,
    /// `None` = `eth_getCode` was never made. Never defaulted.
    pub counterparty_is_contract: Option<bool>,
    pub eip3009_authorization: bool,
    /// The raw log value, as a decimal string.
    pub value_raw: &'a str,
}

/// Counts a report may quote, for one basis and one token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub token: Token,
    /// Distinct credited addresses with ≥1 counted incoming transfer.
    ///
    /// **PAY-1.** This and [`Summary::agents_paid`] are different numbers and
    /// the report must say which it is quoting. On Base, 298 addresses against
    /// 313 declaring agents: 15 agents' worth of difference from 5 shared
    /// addresses, one of which is declared by 62 agents.
    pub addresses_paid: usize,
    /// Distinct agents with ≥1 counted incoming transfer.
    pub agents_paid: usize,
    /// Agents whose only paid addresses are shared with another agent in this
    /// run. For these the census can say the address was paid; it **cannot**
    /// say which agent the payment was for.
    pub agents_on_shared_addresses: usize,
    /// Agents with ≥1 counted transfer carrying an EIP-3009 authorization.
    /// The only protocol-level evidence of payment in the whole pipeline.
    pub agents_x402: usize,
    pub transfers_counted: usize,
    /// Counted value whose sender **has code**. Vaults, routers, platforms.
    /// PAY-3 found 94% of Base's corrected value here, provably Morpho yield
    /// for the largest holder — an operator's own capital returning, not
    /// revenue.
    pub value_from_contract: String,
    /// Counted value whose sender is an EOA — a person or a bot wallet.
    pub value_from_eoa: String,
    /// Counted value whose sender's code was **never read**.
    ///
    /// Its own bucket, always. Folding it into either side above is exactly the
    /// PAY-3 mistake: "one operator earned 97.9%" was produced by never
    /// calling `owner()` or reading the code at the receiving address and
    /// assuming the favourable interpretation of what was not looked at.
    pub value_sender_unread: String,
    pub transfers_from_contract: usize,
    pub transfers_from_eoa: usize,
    pub transfers_sender_unread: usize,
}

impl Summary {
    /// The share of counted value that came from a contract, as a percentage —
    /// or `None` if any sender's code went unread.
    ///
    /// `None` rather than a share computed over what happens to be known. A
    /// denominator that quietly excludes the unknown is how a partial read
    /// becomes a confident percentage.
    pub fn contract_share_percent(&self) -> Option<f64> {
        if self.transfers_sender_unread > 0 {
            return None;
        }
        let c: f64 = self.value_from_contract.parse().ok()?;
        let e: f64 = self.value_from_eoa.parse().ok()?;
        if c + e == 0.0 {
            return None;
        }
        Some(100.0 * c / (c + e))
    }
}

/// Summarise one basis' worth of rows for one token.
pub fn summarise(rows: &[PaidRow<'_>], token: &Token) -> Summary {
    let counted: Vec<&PaidRow<'_>> = rows.iter().filter(|r| r.counted).collect();

    let mut addresses: Vec<String> = Vec::new();
    let mut agents: Vec<u64> = Vec::new();
    let mut x402_agents: Vec<u64> = Vec::new();
    let (mut v_contract, mut v_eoa, mut v_unread) = (0u128, 0u128, 0u128);
    let (mut n_contract, mut n_eoa, mut n_unread) = (0usize, 0usize, 0usize);

    for r in &counted {
        let addr = r.credited_address.to_ascii_lowercase();
        if !addresses.contains(&addr) {
            addresses.push(addr);
        }
        if !agents.contains(&r.agent_id) {
            agents.push(r.agent_id);
        }
        if r.eip3009_authorization && !x402_agents.contains(&r.agent_id) {
            x402_agents.push(r.agent_id);
        }
        // Saturating rather than wrapping: a wrapped total is a wrong number
        // that looks like a right one. u128 holds any realistic stablecoin
        // total at 18 decimals, and a value that does not parse is counted as
        // a transfer without contributing to the total, which is visible in the
        // count/value mismatch rather than silently absorbed.
        let v: u128 = r.value_raw.parse().unwrap_or(0);
        match r.counterparty_is_contract {
            Some(true) => {
                v_contract = v_contract.saturating_add(v);
                n_contract += 1;
            }
            Some(false) => {
                v_eoa = v_eoa.saturating_add(v);
                n_eoa += 1;
            }
            None => {
                v_unread = v_unread.saturating_add(v);
                n_unread += 1;
            }
        }
    }

    // PAY-1: which paid addresses more than one agent in this run reaches.
    let mut shared: Vec<String> = Vec::new();
    for addr in &addresses {
        let holders: Vec<u64> = rows
            .iter()
            .filter(|r| r.credited_address.eq_ignore_ascii_case(addr))
            .map(|r| r.agent_id)
            .fold(Vec::new(), |mut acc, id| {
                if !acc.contains(&id) {
                    acc.push(id);
                }
                acc
            });
        if holders.len() > 1 {
            shared.push(addr.clone());
        }
    }
    let agents_on_shared = agents
        .iter()
        .filter(|id| {
            let mut any = false;
            let mut all_shared = true;
            for r in &counted {
                if r.agent_id == **id {
                    any = true;
                    let a = r.credited_address.to_ascii_lowercase();
                    if !shared.contains(&a) {
                        all_shared = false;
                    }
                }
            }
            any && all_shared
        })
        .count();

    let fmt = |v: u128| format_units(&v.to_string(), token.decimals).unwrap_or_else(|| "0".into());

    Summary {
        token: token.clone(),
        addresses_paid: addresses.len(),
        agents_paid: agents.len(),
        agents_on_shared_addresses: agents_on_shared,
        agents_x402: x402_agents.len(),
        transfers_counted: counted.len(),
        value_from_contract: fmt(v_contract),
        value_from_eoa: fmt(v_eoa),
        value_sender_unread: fmt(v_unread),
        transfers_from_contract: n_contract,
        transfers_from_eoa: n_eoa,
        transfers_sender_unread: n_unread,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usdc() -> Token {
        Token {
            address: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913".into(),
            symbol: "USDC".into(),
            decimals: 6,
        }
    }

    fn row<'a>(agent_id: u64, address: &'a str, value: &'a str) -> PaidRow<'a> {
        PaidRow {
            agent_id,
            credited_address: address,
            counted: true,
            counterparty_is_contract: Some(false),
            eip3009_authorization: false,
            value_raw: value,
        }
    }

    const SHARED: &str = "0x93862e5b1c0a5a0e4f0d7a2b3c4d5e6f70819293";
    const SOLE: &str = "0x161dbdd7f1a0e4f0d7a2b3c4d5e6f7081929304a";

    // ── PAY-1 regression ─────────────────────────────────────────────────────

    #[test]
    fn pay1_addresses_paid_and_agents_paid_are_reported_separately() {
        // "Claimed: 313 agents have received an external payment."
        // Cause: a payment to a shared address was credited to EVERY agent
        // declaring it. 298 addresses received an external transfer against 313
        // declaring agents — 15 agents' worth of double-counting from 5 shared
        // addresses, one of them declared by 62 agents.
        //
        // Here: three agents, two addresses. A summary that offers one number
        // for "paid" cannot express that, so this one offers two.
        let rows = [
            row(1, SHARED, "1000000"),
            row(2, SHARED, "1000000"),
            row(3, SOLE, "5000000"),
        ];
        let s = summarise(&rows, &usdc());
        assert_eq!(s.addresses_paid, 2);
        assert_eq!(s.agents_paid, 3);
        assert_ne!(s.addresses_paid, s.agents_paid);
    }

    #[test]
    fn pay1_agents_whose_only_paid_address_is_shared_are_counted_as_unattributable() {
        let rows = [
            row(1, SHARED, "1000000"),
            row(2, SHARED, "1000000"),
            row(3, SOLE, "5000000"),
        ];
        let s = summarise(&rows, &usdc());
        // For agents 1 and 2 the census can say the ADDRESS was paid. It cannot
        // say which of them the payment was for, and the summary says so.
        assert_eq!(s.agents_on_shared_addresses, 2);
    }

    #[test]
    fn pay1_the_shared_address_is_counted_once_by_value_not_once_per_agent() {
        let rows = [row(1, SHARED, "1000000"), row(2, SHARED, "1000000")];
        let s = summarise(&rows, &usdc());
        // Two ROWS, because the pipeline stores one row per (agent, transfer)
        // and the caller asked for both. The address count is what stops the
        // agent fan-out becoming a payment count.
        assert_eq!(s.transfers_counted, 2);
        assert_eq!(s.addresses_paid, 1);
    }

    // ── PAY-3 regression ─────────────────────────────────────────────────────

    #[test]
    fn pay3_value_is_split_by_whether_the_sender_has_code() {
        // "Claimed: one operator earned 97.9% of all agent revenue."
        // Cause: `owner()` of the receiving contract was never read and the
        // contracts were never inspected, so Morpho vault flow was classified
        // as payment. Corrected split on Base: 6,127 transfers / $1,027,924
        // from contracts, 5,453 / $59,447 from EOAs — 94% of the value is
        // contract-sourced.
        let rows = [
            PaidRow {
                counterparty_is_contract: Some(true),
                ..row(1, SOLE, "1027924000000")
            },
            PaidRow {
                counterparty_is_contract: Some(false),
                ..row(2, SHARED, "59447000000")
            },
        ];
        let s = summarise(&rows, &usdc());
        assert_eq!(s.value_from_contract, "1027924");
        assert_eq!(s.value_from_eoa, "59447");
        assert_eq!(s.transfers_from_contract, 1);
        assert_eq!(s.transfers_from_eoa, 1);
        let share = s.contract_share_percent().unwrap();
        assert!((share - 94.5).abs() < 0.5, "{share}");
    }

    #[test]
    fn pay3_a_sender_whose_code_was_never_read_is_its_own_bucket() {
        // The mistake was not "we classified it wrong", it was "we never
        // looked and assumed". An unread sender must not land in either side.
        let rows = [
            PaidRow {
                counterparty_is_contract: None,
                ..row(1, SOLE, "100000000")
            },
            PaidRow {
                counterparty_is_contract: Some(false),
                ..row(2, SHARED, "1000000")
            },
        ];
        let s = summarise(&rows, &usdc());
        assert_eq!(s.value_sender_unread, "100");
        assert_eq!(s.value_from_eoa, "1");
        assert_eq!(s.value_from_contract, "0");
        assert_eq!(s.transfers_sender_unread, 1);
    }

    #[test]
    fn pay3_no_contract_share_is_published_while_any_sender_is_unread() {
        let rows = [
            PaidRow {
                counterparty_is_contract: None,
                ..row(1, SOLE, "100000000")
            },
            PaidRow {
                counterparty_is_contract: Some(true),
                ..row(2, SHARED, "1000000")
            },
        ];
        assert!(summarise(&rows, &usdc()).contract_share_percent().is_none());
    }

    // ── excluded rows never reach a count ────────────────────────────────────

    #[test]
    fn excluded_rows_are_stored_but_never_summed() {
        let rows = [
            PaidRow {
                counted: false,
                ..row(1, SOLE, "8845244000000")
            },
            row(2, SHARED, "1000000"),
        ];
        let s = summarise(&rows, &usdc());
        assert_eq!(s.agents_paid, 1);
        assert_eq!(s.value_from_eoa, "1");
    }

    #[test]
    fn x402_is_counted_per_agent_and_is_a_subset_of_paid() {
        let rows = [
            PaidRow {
                eip3009_authorization: true,
                ..row(1, SOLE, "1070000")
            },
            PaidRow {
                eip3009_authorization: true,
                ..row(1, SOLE, "1070000")
            },
            row(2, SHARED, "1000000"),
        ];
        let s = summarise(&rows, &usdc());
        assert_eq!(s.agents_x402, 1);
        assert_eq!(s.agents_paid, 2);
        assert!(s.agents_x402 <= s.agents_paid);
    }

    #[test]
    fn an_empty_run_summarises_to_zero_and_not_to_a_panic() {
        let s = summarise(&[], &usdc());
        assert_eq!(s.agents_paid, 0);
        assert_eq!(s.value_from_eoa, "0");
        assert!(s.contract_share_percent().is_none());
    }
}
