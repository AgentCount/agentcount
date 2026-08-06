//! Token units — read from the contract, never assumed.
//!
//! This module exists because assuming was tried and would have been wrong by
//! twelve orders of magnitude. From `analysis/payments-per-chain.md` §2:
//!
//! | chain | tokens | decimals |
//! |---|---|---|
//! | base | USDC, USDbC | 6, 6 |
//! | bsc | USDC, USDT | **18, 18** |
//! | mainnet | USDC, USDT | 6, 6 |
//! | celo | USDC, `0x765DE816…` | 6, **18** |
//!
//! > "**BSC's USDC and USDT are 18 decimals, not 6**, and Celo's
//! > `0x765DE816…` — long known as cUSD — now reports `USDm` at 18. Carrying
//! > Base's 6 across all four chains would have overstated BSC by a factor of
//! > 10¹²."
//!
//! So the pipeline hardcodes **addresses** and reads **`symbol()` and
//! `decimals()`** off each contract at the run's pinned block, stores both on
//! `payment_scans`, and formats from the stored value. A token that renames
//! itself — which cUSD did — shows up in the row rather than in a footnote
//! nobody updated.

use serde::{Deserialize, Serialize};

/// A token as the contract described itself at the pinned block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    /// Lowercase hex. The one part of this struct that is configuration.
    pub address: String,
    /// `symbol()`, verbatim. Not a label this project chose: the Celo entry
    /// long documented as cUSD answers `USDm`, and the row must say so.
    pub symbol: String,
    /// `decimals()`, verbatim.
    pub decimals: u8,
}

/// Format a raw on-chain integer value using the token's own decimals.
///
/// Takes the raw value as a decimal string because `payments.value_raw` is
/// `NUMERIC`: a uint256 does not fit in an i64, or reliably in an i128, and
/// widening it at the database boundary to make Rust's arithmetic convenient
/// would be the same class of mistake as assuming the decimals.
///
/// Returns `None` for anything that is not a non-negative decimal integer.
/// Never rounds, never truncates, and never divides by a constant.
pub fn format_units(raw: &str, decimals: u8) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let raw = raw.trim_start_matches('0');
    let raw = if raw.is_empty() { "0" } else { raw };
    let d = decimals as usize;
    if d == 0 {
        return Some(raw.to_string());
    }
    let (whole, frac) = if raw.len() > d {
        (&raw[..raw.len() - d], raw[raw.len() - d..].to_string())
    } else {
        ("0", format!("{}{raw}", "0".repeat(d - raw.len())))
    };
    let frac = frac.trim_end_matches('0');
    Some(if frac.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{frac}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spot_checked_base_transfer_formats_to_the_documented_dollar_value() {
        // `analysis/payments-corrections-ledger.md`: transaction 0x459a808c…,
        // raw Transfer value 68289018474 = 68,289.018474 USDC = $68,289. The
        // largest single external transfer in the Base set.
        assert_eq!(format_units("68289018474", 6).unwrap(), "68289.018474");
    }

    #[test]
    fn bsc_is_not_divided_by_a_million() {
        // The whole reason this function takes `decimals` instead of assuming
        // 6. One BSC "dollar" is 10^18 raw units. Carrying Base's 6 across all
        // four chains would have overstated BSC by 10^12.
        assert_eq!(format_units("1000000000000000000", 18).unwrap(), "1");
        assert_eq!(
            format_units("1000000000000000000", 6).unwrap(),
            "1000000000000"
        );
    }

    #[test]
    fn celos_renamed_stablecoin_is_eighteen_decimals() {
        // `0x765DE816…` was documented for years as cUSD at 18 decimals and now
        // reports `USDm`. The symbol is read, the decimals are read, and the
        // row carries both — a rename shows up in the data, not in a footnote.
        let t = Token {
            address: "0x765de816845861e75a25fca122bb6898b8b1282a".into(),
            symbol: "USDm".into(),
            decimals: 18,
        };
        assert_eq!(
            format_units("2813000000000000000000", t.decimals).unwrap(),
            "2813"
        );
    }

    #[test]
    fn values_smaller_than_one_unit_keep_every_digit() {
        // Celo's 84 x402 settlements total $0.94, a mean of about a cent.
        // Rounding those to zero would erase the finding.
        assert_eq!(format_units("11000", 6).unwrap(), "0.011");
        assert_eq!(format_units("1", 18).unwrap(), "0.000000000000000001");
        assert_eq!(format_units("0", 6).unwrap(), "0");
        assert_eq!(format_units("000000", 6).unwrap(), "0");
    }

    #[test]
    fn a_uint256_that_does_not_fit_in_any_rust_integer_still_formats() {
        // 2^256 - 1. `value_raw` is NUMERIC precisely so this is storable; the
        // formatter must not be the thing that narrows it.
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(
            format_units(max, 6).unwrap(),
            "115792089237316195423570985008687907853269984665640564039457584007913129.639935"
        );
    }

    #[test]
    fn anything_that_is_not_a_raw_integer_is_refused_rather_than_guessed() {
        for bad in ["", "  ", "-1", "1.5", "0x10", "1e18", "abc"] {
            assert!(format_units(bad, 6).is_none(), "{bad}");
        }
    }
}
