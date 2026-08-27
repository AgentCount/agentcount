//! Rung 6, `settled`: has this payee ever actually been paid?
//!
//! METHODOLOGY §10.3. Every other rung asks what a seller SAYS — its
//! listing, its reachability, its quote. This one asks what the chain shows,
//! and it is the only rung whose evidence nobody has to trust us for: a
//! transaction hash is checkable by anyone, forever.
//!
//! Facilitator-agnostic on purpose. The question is whether value arrived at
//! the payee, not whether it arrived through any particular company's
//! infrastructure — an economy measured only through one facilitator's API
//! is that facilitator's economy.
//!
//! # The exclusion that keeps this census out of its own numbers
//!
//! **Transfers from the shopper wallet never count.** METHODOLOGY §10.4
//! publishes that address before the first purchase precisely so it can be
//! excluded, and this is where the promise becomes mechanical rather than
//! remembered. Without it, every seller rung 4 buys from would gain a
//! settlement that this census itself created, and the settled rate would
//! measure our own spending.
//!
//! The excluded volume is counted and reported, never silently dropped —
//! same rule as everywhere: an exclusion nobody can see is indistinguishable
//! from a number nobody checked.
//!
//! # What this counts, and what that is a superset of
//!
//! Incoming stablecoin transfers to the payee — every one, whoever sent it
//! and for whatever reason. That is deliberate (an economy measured only
//! through one facilitator's API is that facilitator's economy) and it is a
//! SUPERSET of x402 settlements: a payee that also does something else with
//! the same address has that activity counted here too.
//!
//! Which is why [`SettledVerdict::distinct_payers`] is published beside
//! [`SettledVerdict::settlements`] and matters more. The first live scan
//! made the point better than any argument: over eleven hours of Base, one
//! payee in a forty-two-seller sample took **162,531 transfers from 29
//! distinct payers** — roughly 5,600 payments per payer. Whatever that is,
//! it is not a shop serving customers, and a settlement count alone would
//! have published it as the busiest seller in the census. The ratio is a
//! finding; the raw count is not a headline.
//!
//! # What `fail` means here, precisely
//!
//! "No qualifying settlement **in the scanned window**", and the window is
//! recorded on every row. A chain is long and a scan is bounded; a row that
//! said "never paid" while meaning "not paid since block N" would be the
//! kind of overstatement this project exists to avoid.

use serde::{Deserialize, Serialize};

use crate::SellerStatus;
use crate::identity::{Network, normalize_pay_to};
use crate::reachable::Answer;

/// One incoming transfer to a payee, as facts. No verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settlement {
    /// The payer, as the chain reported it.
    pub from: String,
    pub block: u64,
    pub tx_hash: String,
    /// The raw uint256, as a decimal string. Never narrowed — the same
    /// reason `payments.value_raw` is NUMERIC.
    pub value_raw: String,
}

/// Rung 6's answer and everything the row publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettledVerdict {
    pub answer: Answer,
    /// Qualifying settlements — after the exclusions below.
    pub settlements: usize,
    /// How many DISTINCT payers. One customer paying forty times and forty
    /// customers paying once are different facts about a business, and a
    /// count of transfers alone cannot tell them apart.
    pub distinct_payers: usize,
    pub first_block: Option<u64>,
    pub last_block: Option<u64>,
    /// Transfers from this census's own shopper wallet (§10.4). Counted so
    /// the exclusion is visible in the same row it applies to.
    pub excluded_ours: usize,
    /// Transfers where the payee paid itself. Not a customer.
    pub excluded_self: usize,
}

/// The same rules, applied one transfer at a time.
///
/// A sweep's settlement scan reads tens of millions of transfers — 28.5
/// million on the first production run — to produce six numbers per payee.
/// Holding them all to do that cost 4.7 GB and would be killed outright on
/// Cloud Run, so the scan absorbs each window and drops it. This type is
/// where the absorbing happens, in the crate that owns the rules, because
/// an incremental copy of the exclusions living in the binary would be a
/// second implementation of what counts as a payment.
///
/// [`judge`] is written in terms of this, so there is exactly one.
#[derive(Debug, Clone)]
pub struct Tally {
    payee: String,
    ours: String,
    settlements: usize,
    payers: std::collections::BTreeSet<String>,
    first_block: Option<u64>,
    last_block: Option<u64>,
    excluded_ours: usize,
    excluded_self: usize,
}

impl Tally {
    /// `shopper_wallet` is the address published in METHODOLOGY §10.4.
    pub fn new(pay_to: &str, shopper_wallet: &str) -> Self {
        Self {
            payee: normalize(pay_to),
            ours: normalize(shopper_wallet),
            settlements: 0,
            payers: std::collections::BTreeSet::new(),
            first_block: None,
            last_block: None,
            excluded_ours: 0,
            excluded_self: 0,
        }
    }

    /// Take one incoming transfer into account, then forget it.
    pub fn absorb(&mut self, t: &Settlement) {
        let from = normalize(&t.from);
        if from == self.ours {
            // Ours. The wallet was published before the first purchase so
            // that this exclusion could be made by anyone, including us.
            self.excluded_ours += 1;
            return;
        }
        if from == self.payee {
            // A payee paying itself is not a customer.
            self.excluded_self += 1;
            return;
        }
        self.settlements += 1;
        self.payers.insert(from);
        self.first_block = Some(self.first_block.map_or(t.block, |b| b.min(t.block)));
        self.last_block = Some(self.last_block.map_or(t.block, |b| b.max(t.block)));
    }

    /// The verdict, from everything absorbed so far.
    pub fn finish(self) -> SettledVerdict {
        let answer = if self.settlements == 0 {
            Answer {
                status: SellerStatus::Fail,
                // Precisely what was and was not established: no settlement
                // in the window this row records, which is not "never paid".
                reason: Some("no_settlement_in_window".into()),
            }
        } else {
            Answer {
                status: SellerStatus::Pass,
                reason: None,
            }
        };
        SettledVerdict {
            answer,
            settlements: self.settlements,
            distinct_payers: self.payers.len(),
            first_block: self.first_block,
            last_block: self.last_block,
            excluded_ours: self.excluded_ours,
            excluded_self: self.excluded_self,
        }
    }
}

fn normalize(a: &str) -> String {
    normalize_pay_to(a, Network::Evm).unwrap_or_else(|_| a.trim().to_ascii_lowercase())
}

/// Judge one payee's incoming transfers.
///
/// `shopper_wallet` is the address published in METHODOLOGY §10.4. It is a
/// required argument rather than a constant here so that the value the
/// census actually spends from and the value it excludes are the same one,
/// passed from one place.
pub fn judge(pay_to: &str, transfers: &[Settlement], shopper_wallet: &str) -> SettledVerdict {
    let mut tally = Tally::new(pay_to, shopper_wallet);
    for t in transfers {
        tally.absorb(t);
    }
    tally.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYEE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHOPPER: &str = "0x8945b93E68C8927250DDFC41cd10EAc6CbEEd25f";
    const CUSTOMER: &str = "0xcccccccccccccccccccccccccccccccccccccccc";
    const CUSTOMER2: &str = "0xdddddddddddddddddddddddddddddddddddddddd";

    fn settlement(from: &str, block: u64) -> Settlement {
        Settlement {
            from: from.into(),
            block,
            tx_hash: format!("0x{block:064x}"),
            value_raw: "1000".into(),
        }
    }

    #[test]
    fn a_real_payment_settles_the_seller() {
        let v = judge(PAYEE, &[settlement(CUSTOMER, 100)], SHOPPER);
        assert_eq!(v.answer.status, SellerStatus::Pass);
        assert_eq!(v.settlements, 1);
        assert_eq!(v.distinct_payers, 1);
        assert_eq!(v.first_block, Some(100));
        assert_eq!(v.last_block, Some(100));
    }

    #[test]
    fn our_own_shopper_payment_never_counts_as_settlement() {
        // THE exclusion. Without it, every seller rung 4 buys from would
        // gain a settlement this census itself created, and the settled rate
        // would measure our own spending back to us.
        let v = judge(PAYEE, &[settlement(SHOPPER, 100)], SHOPPER);
        assert_eq!(v.answer.status, SellerStatus::Fail);
        assert_eq!(v.settlements, 0);
        assert_eq!(v.excluded_ours, 1, "and the exclusion is visible");
    }

    #[test]
    fn the_shopper_wallet_matches_however_its_case_is_written() {
        // The published address is checksummed; the chain reports hex in
        // whatever case. Missing the match would silently readmit our own
        // money into the numbers we audit.
        let v = judge(PAYEE, &[settlement(&SHOPPER.to_lowercase(), 100)], SHOPPER);
        assert_eq!(v.excluded_ours, 1);
        assert_eq!(v.settlements, 0);
    }

    #[test]
    fn a_payee_paying_itself_is_not_a_customer() {
        let v = judge(PAYEE, &[settlement(PAYEE, 100)], SHOPPER);
        assert_eq!(v.settlements, 0);
        assert_eq!(v.excluded_self, 1);
        assert_eq!(v.answer.status, SellerStatus::Fail);
    }

    #[test]
    fn distinct_payers_tells_forty_customers_from_one() {
        // One customer paying forty times and forty customers paying once
        // are different facts about a business.
        let repeat = judge(
            PAYEE,
            &[
                settlement(CUSTOMER, 100),
                settlement(CUSTOMER, 101),
                settlement(CUSTOMER, 102),
            ],
            SHOPPER,
        );
        assert_eq!(repeat.settlements, 3);
        assert_eq!(repeat.distinct_payers, 1);

        let spread = judge(
            PAYEE,
            &[settlement(CUSTOMER, 100), settlement(CUSTOMER2, 101)],
            SHOPPER,
        );
        assert_eq!(spread.settlements, 2);
        assert_eq!(spread.distinct_payers, 2);
    }

    #[test]
    fn first_and_last_are_over_qualifying_transfers_only() {
        // An excluded transfer must not stretch the window a row reports:
        // "first paid at block 1" would otherwise be our own purchase.
        let v = judge(
            PAYEE,
            &[
                settlement(SHOPPER, 1),
                settlement(CUSTOMER, 500),
                settlement(CUSTOMER2, 900),
            ],
            SHOPPER,
        );
        assert_eq!(v.first_block, Some(500));
        assert_eq!(v.last_block, Some(900));
        assert_eq!(v.excluded_ours, 1);
    }

    #[test]
    fn absorbing_one_at_a_time_gives_exactly_what_judging_all_at_once_does() {
        // The property the scan's memory fix rests on. If these two ever
        // disagreed, the instrument would have two definitions of what
        // counts as a payment and would publish whichever ran that day.
        let all = [
            settlement(CUSTOMER, 500),
            settlement(SHOPPER, 1),
            settlement(CUSTOMER, 700),
            settlement(PAYEE, 900),
            settlement(CUSTOMER2, 300),
        ];
        let at_once = judge(PAYEE, &all, SHOPPER);

        let mut tally = Tally::new(PAYEE, SHOPPER);
        for t in &all {
            tally.absorb(t);
        }
        let streamed = tally.finish();

        assert_eq!(at_once, streamed);
        // ...and it is the right answer, not merely a consistent one.
        assert_eq!(streamed.settlements, 3);
        assert_eq!(streamed.distinct_payers, 2);
        assert_eq!(streamed.first_block, Some(300));
        assert_eq!(streamed.last_block, Some(700));
        assert_eq!(streamed.excluded_ours, 1);
        assert_eq!(streamed.excluded_self, 1);
    }

    #[test]
    fn a_tally_that_absorbed_nothing_is_not_a_settled_seller() {
        assert_eq!(
            Tally::new(PAYEE, SHOPPER).finish().answer.status,
            SellerStatus::Fail
        );
    }

    #[test]
    fn nothing_found_says_what_was_actually_established() {
        // Not "never paid" — "no settlement in the window this row records".
        let v = judge(PAYEE, &[], SHOPPER);
        assert_eq!(v.answer.status, SellerStatus::Fail);
        assert_eq!(v.answer.reason.as_deref(), Some("no_settlement_in_window"));
        assert_eq!(v.distinct_payers, 0);
        assert_eq!(v.first_block, None);
    }
}
