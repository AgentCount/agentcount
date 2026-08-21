//! Reading an ERC-20 token: what it calls itself, who it moved value between,
//! and whether an EIP-3009 authorization was involved.
//!
//! Used by `crates/sweeper/src/bin/payments.rs` and nothing else. Like the rest
//! of this crate it makes RPC calls and forms no opinion: whether a transfer
//! counts as a payment is `crates/payments`' question, and every judgement in
//! it is a pure function.
//!
//! ## Why `symbol()` and `decimals()` are read rather than configured
//!
//! Because assuming was tried. `analysis/payments-per-chain.md` §2: BSC's USDC
//! and USDT are **18** decimals, not 6, and Celo's `0x765DE816…` — documented
//! for years as cUSD — now answers **`USDm`** at 18. Carrying Base's 6 across
//! all four chains would have overstated BSC by a factor of 10^12. The address
//! is configuration; everything else about the token is a read, stored on
//! `payment_scans` at the run's pinned block.
//!
//! ## Why the receipt is read as raw JSON
//!
//! Same reason [`crate::registry::Registry::tx_sender`] does: alloy's typed
//! transaction/receipt decoding knows transaction types `0x0`–`0x4`, and Celo's
//! CIP-64 (`0x7b`) and the OP-stack deposit (`0x7e`) are neither. On 2026-08-04
//! that cost three chains an entire census. This module wants two fields out of
//! a receipt — a log's address and its topics — and asks for exactly those.

use alloy::eips::BlockId;
use alloy::primitives::{Address, B256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::Filter;
use alloy::sol;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};

sol! {
    #[sol(rpc)]
    interface IErc20 {
        function symbol() external view returns (string);
        function decimals() external view returns (uint8);
    }

    /// The ERC-20 transfer log. `from` and `to` are indexed, so they are
    /// `topics[1]` and `topics[2]` and can be filtered on server-side — which
    /// is what makes scanning a few hundred addresses across a chain's whole
    /// history affordable at all.
    event Transfer(address indexed from, address indexed to, uint256 value);

    /// EIP-3009. `transferWithAuthorization` (and `receiveWithAuthorization`)
    /// emit this from the token contract in the same transaction as the
    /// `Transfer` it settles. It is the x402 signature: a `Transfer` whose
    /// transaction also carries an `AuthorizationUsed` from that same token is
    /// an authorised settlement, one without it is a plain transfer.
    event AuthorizationUsed(address indexed authorizer, bytes32 indexed nonce);
}

/// Which side of the transfer the target address is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// `topics[2]` — the target received.
    Incoming,
    /// `topics[1]` — the target sent.
    Outgoing,
}

/// One `Transfer` log, decoded.
#[derive(Debug, Clone)]
pub struct TransferLog {
    pub from: String,
    pub to: String,
    /// The raw uint256, as a decimal string. Never narrowed: `payments.value_raw`
    /// is `NUMERIC` for the same reason.
    pub value_raw: String,
    pub block_number: u64,
    pub tx_hash: String,
    pub log_index: u64,
}

/// What a token contract says about itself, at one block.
#[derive(Debug, Clone)]
pub struct TokenMetadata {
    pub address: String,
    pub symbol: String,
    pub decimals: u8,
}

pub struct Erc20 {
    provider: DynProvider,
    address: Address,
}

/// How many target addresses go into one `eth_getLogs` topic filter.
///
/// Providers cap the number of values in a topic position and disagree about
/// where. 100 is comfortably under every observed limit and keeps the number of
/// requests proportional to the target set rather than to the block range.
pub const TARGETS_PER_QUERY: usize = 100;

impl Erc20 {
    pub async fn connect(rpc_url: &str, token_address: &str) -> Result<Self> {
        let provider = ProviderBuilder::new()
            .connect(rpc_url)
            .await
            .with_context(|| format!("connecting to {rpc_url}"))?
            .erased();
        let address: Address = token_address.parse().context("parsing token address")?;
        Ok(Self { provider, address })
    }

    /// `symbol()` and `decimals()` at a block, verbatim.
    ///
    /// A `symbol()` that cannot be decoded as a string falls back to the
    /// `bytes32` form some older tokens use, and then to the empty string with
    /// a warning — never to a name this project invented. `decimals()` has no
    /// fallback: a token whose decimals cannot be read cannot have its values
    /// interpreted, and guessing 6 is precisely the mistake this function
    /// exists to prevent.
    /// The chain's current head.
    ///
    /// The mirror of [`crate::registry::Registry::pinned_block`], for
    /// callers that scan token transfers without holding a registry — the
    /// Seller Census's rung 6 reads a chain it has no ERC-8004 registry on.
    /// Pinned ONCE at the start of a scan and recorded on every row it
    /// writes: a scan whose upper bound moved while it ran would produce
    /// rows that cannot be reproduced together.
    pub async fn head_block(&self) -> Result<u64> {
        Ok(self.provider.get_block_number().await?)
    }

    pub async fn metadata(&self, block: u64) -> Result<TokenMetadata> {
        let c = IErc20::new(self.address, &self.provider);
        let decimals = c
            .decimals()
            .block(BlockId::from(block))
            .call()
            .await
            .with_context(|| {
                format!(
                    "decimals() on {:?} — refusing to assume a value; see \
                     analysis/payments-per-chain.md §2",
                    self.address
                )
            })?;
        let symbol = match c.symbol().block(BlockId::from(block)).call().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "symbol() on {:?} did not decode as a string ({e}); trying bytes32",
                    self.address
                );
                self.symbol_as_bytes32(block).await.unwrap_or_default()
            }
        };
        Ok(TokenMetadata {
            address: format!("{:?}", self.address).to_lowercase(),
            symbol,
            decimals,
        })
    }

    /// The pre-string `symbol()` some tokens still use: a right-padded
    /// `bytes32`. Read as a raw call so no ABI decode has to be guessed at.
    async fn symbol_as_bytes32(&self, block: u64) -> Option<String> {
        use alloy::primitives::Bytes;
        use alloy::rpc::types::TransactionRequest;
        let call = TransactionRequest::default()
            .to(self.address)
            // keccak("symbol()")[0..4]
            .input(Bytes::from_static(&[0x95, 0xd8, 0x9b, 0x41]).into());
        let out = self
            .provider
            .call(call)
            .block(BlockId::from(block))
            .await
            .ok()?;
        let s: String = out
            .iter()
            .take(32)
            .take_while(|b| **b != 0)
            .map(|b| *b as char)
            .collect();
        (!s.is_empty()).then_some(s)
    }

    /// Every `Transfer` log in `[from_block, to_block]` where one of `targets`
    /// is on `side`.
    ///
    /// Ranges are halved on error rather than tuned per provider: providers cap
    /// both the block range and the response size, both surface as an error on
    /// a wide query, and telling them apart is not worth the code. Same
    /// technique, and same reasoning, as
    /// [`crate::registry::Registry::registrations`].
    pub async fn transfers(
        &self,
        targets: &[String],
        side: Side,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<TransferLog>> {
        self.transfers_batched(targets, side, from_block, to_block, TARGETS_PER_QUERY)
            .await
    }

    /// [`Self::transfers`] with the address batch size chosen by the caller.
    ///
    /// The default of [`TARGETS_PER_QUERY`] is conservative, and for the
    /// registration census's payment pass — a few hundred agent wallets over
    /// a registry's lifetime — it costs nothing. The Seller Census asks about
    /// thousands of payees at once, where the batch size decides whether a
    /// pass takes forty minutes or four hours.
    ///
    /// Measured against the production RPC (2026-08-21): **1,000 addresses in
    /// one filter over a 10,000-block window answers in 1.5 seconds.** The
    /// provider's documented rule is that a range of 10,000 blocks or fewer
    /// carries no response-size limit, which is what makes a large address
    /// list safe — the block range, not the address count, is what the cap
    /// binds on.
    pub async fn transfers_batched(
        &self,
        targets: &[String],
        side: Side,
        from_block: u64,
        to_block: u64,
        targets_per_query: usize,
    ) -> Result<Vec<TransferLog>> {
        let mut out = Vec::new();
        for chunk in targets.chunks(targets_per_query.max(1)) {
            let topic: Vec<B256> = chunk
                .iter()
                .filter_map(|a| a.parse::<Address>().ok())
                .map(|a| a.into_word())
                .collect();
            if topic.is_empty() {
                continue;
            }
            let mut spans: Vec<(u64, u64)> = vec![(from_block, to_block)];
            while let Some((lo, hi)) = spans.pop() {
                let base = Filter::new()
                    .address(self.address)
                    .event_signature(Transfer::SIGNATURE_HASH)
                    .from_block(lo)
                    .to_block(hi);
                let filter = match side {
                    Side::Incoming => base.topic2(topic.clone()),
                    Side::Outgoing => base.topic1(topic.clone()),
                };
                match self.provider.get_logs(&filter).await {
                    Ok(logs) => {
                        for log in logs {
                            let Ok(decoded) = log.log_decode::<Transfer>() else {
                                continue;
                            };
                            let (Some(block_number), Some(tx_hash), Some(log_index)) =
                                (log.block_number, log.transaction_hash, log.log_index)
                            else {
                                continue; // pending log; nothing to cite
                            };
                            out.push(TransferLog {
                                from: format!("{:?}", decoded.inner.from).to_lowercase(),
                                to: format!("{:?}", decoded.inner.to).to_lowercase(),
                                value_raw: decoded.inner.value.to_string(),
                                block_number,
                                tx_hash: format!("{tx_hash:?}").to_lowercase(),
                                log_index,
                            });
                        }
                    }
                    Err(e) => {
                        if lo >= hi {
                            return Err(e).with_context(|| {
                                format!(
                                    "eth_getLogs for Transfer on {:?} at block {lo} \
                                     (cannot split further)",
                                    self.address
                                )
                            });
                        }
                        let mid = lo + (hi - lo) / 2;
                        spans.push((mid + 1, hi));
                        spans.push((lo, mid));
                    }
                }
            }
        }
        Ok(out)
    }

    /// The EIP-3009 authorizer for a transaction, if this token emitted an
    /// `AuthorizationUsed` in it.
    ///
    /// `Ok(None)` means the transaction carries no such log from this token —
    /// a plain transfer. `Err` means the receipt could not be read, and the
    /// caller stores `NULL` rather than `false`: "we did not look" is not
    /// "there was none".
    ///
    /// **Why per transaction rather than a chain-wide scan of
    /// `AuthorizationUsed`.** Base's stablecoins carried **6,875,861**
    /// `AuthorizationUsed` transactions in the blocks scanned, of which 8,904
    /// reached an agent-declared address. Scanning all of them to intersect
    /// with a few thousand transfers is millions of logs to answer a question
    /// about thousands. One receipt per candidate transfer is bounded by the
    /// transfers actually found, and it returns the authorizer — which a
    /// chain-wide topic scan would also give, but only after being held in
    /// memory at that size.
    pub async fn authorization_in_tx(&self, tx_hash: &str) -> Result<Option<String>> {
        let hash: alloy::primitives::TxHash = tx_hash
            .parse()
            .with_context(|| format!("parsing tx hash {tx_hash}"))?;
        let raw: serde_json::Value = self
            .provider
            .raw_request::<_, serde_json::Value>("eth_getTransactionReceipt".into(), (hash,))
            .await
            .with_context(|| format!("eth_getTransactionReceipt({tx_hash})"))?;
        Ok(authorizer_from_receipt_json(
            &raw,
            &format!("{:?}", self.address).to_lowercase(),
        ))
    }

    /// Does this address have code at `block`?
    ///
    /// **PAY-3.** 94% of Base's corrected payment value arrived from senders
    /// with code — Morpho vaults returning an operator's own capital, read as
    /// revenue. The answer is stored per transfer, and a read that fails stores
    /// `NULL`, never `false`.
    pub async fn is_contract(&self, address: &str, block: u64) -> Result<bool> {
        let a: Address = address
            .parse()
            .with_context(|| format!("parsing address {address}"))?;
        let code = self
            .provider
            .get_code_at(a)
            .block_id(BlockId::from(block))
            .await
            .with_context(|| format!("eth_getCode({address})"))?;
        Ok(!code.is_empty())
    }
}

/// The `authorizer` from the first `AuthorizationUsed` emitted by `token` in a
/// raw `eth_getTransactionReceipt` result.
///
/// Split out and pure so the topic matching is testable without a node. Reads
/// `logs[].address` and `logs[].topics[0..2]` and nothing else — a receipt
/// whose transaction type alloy cannot decode is still perfectly readable this
/// way, which is the whole reason for the raw request.
fn authorizer_from_receipt_json(receipt: &serde_json::Value, token: &str) -> Option<String> {
    let sig = format!("{:?}", AuthorizationUsed::SIGNATURE_HASH).to_lowercase();
    receipt
        .get("logs")?
        .as_array()?
        .iter()
        .filter(|log| {
            log.get("address")
                .and_then(|v| v.as_str())
                .is_some_and(|a| a.eq_ignore_ascii_case(token))
        })
        .find_map(|log| {
            let topics = log.get("topics")?.as_array()?;
            if !topics.first()?.as_str()?.eq_ignore_ascii_case(&sig) {
                return None;
            }
            // topics[1] is the indexed authorizer, left-padded to 32 bytes.
            let word = topics.get(1)?.as_str()?;
            let hex = word.strip_prefix("0x").unwrap_or(word);
            (hex.len() == 64).then(|| format!("0x{}", hex[24..].to_ascii_lowercase()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(logs: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "logs": logs })
    }

    const TOKEN: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
    const OTHER: &str = "0xd9aaec86b65d86f6a7b5b1b0c42ffa531710b6ca";

    #[test]
    fn the_authorization_signature_hash_is_the_published_one() {
        // keccak256("AuthorizationUsed(address,bytes32)"). If this ever moves,
        // every x402 count silently becomes zero — which would look like a
        // finding rather than a bug.
        assert_eq!(
            format!("{:?}", AuthorizationUsed::SIGNATURE_HASH),
            "0x98de503528ee59b575ef0c0a2576a82497bfc029a5685b209e9ec333479b10a5"
        );
    }

    #[test]
    fn an_authorization_from_this_token_yields_its_authorizer() {
        let sig = format!("{:?}", AuthorizationUsed::SIGNATURE_HASH);
        let r = receipt(serde_json::json!([{
            "address": TOKEN,
            "topics": [
                sig,
                "0x00000000000000000000000048380bcf1c09773c9e96901f89a7a6b75e2bbecc",
                "0x1111111111111111111111111111111111111111111111111111111111111111"
            ]
        }]));
        assert_eq!(
            authorizer_from_receipt_json(&r, TOKEN).as_deref(),
            Some("0x48380bcf1c09773c9e96901f89a7a6b75e2bbecc")
        );
    }

    #[test]
    fn an_authorization_from_a_different_token_is_not_this_tokens_settlement() {
        // A transaction can move two stablecoins. Flagging our Transfer because
        // the OTHER token was settled under authorization would invent an x402
        // payment — the exact over-count `x402scan-crosscheck.md` §4a tested for
        // and refused.
        let sig = format!("{:?}", AuthorizationUsed::SIGNATURE_HASH);
        let r = receipt(serde_json::json!([{
            "address": OTHER,
            "topics": [sig, "0x00000000000000000000000048380bcf1c09773c9e96901f89a7a6b75e2bbecc",
                       "0x11"]
        }]));
        assert!(authorizer_from_receipt_json(&r, TOKEN).is_none());
    }

    #[test]
    fn a_plain_transfer_receipt_carries_no_authorizer() {
        let r = receipt(serde_json::json!([{
            "address": TOKEN,
            "topics": [format!("{:?}", Transfer::SIGNATURE_HASH), "0x00", "0x00"]
        }]));
        assert!(authorizer_from_receipt_json(&r, TOKEN).is_none());
    }

    #[test]
    fn a_malformed_receipt_never_panics() {
        for v in [
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!({"logs": "nope"}),
            receipt(serde_json::json!([{"address": TOKEN}])),
            receipt(serde_json::json!([{"address": TOKEN, "topics": []}])),
            // An indexed topic that is not a 32-byte word: refused, not sliced.
            receipt(serde_json::json!([{
                "address": TOKEN,
                "topics": [format!("{:?}", AuthorizationUsed::SIGNATURE_HASH), "0xdead"]
            }])),
        ] {
            assert!(authorizer_from_receipt_json(&v, TOKEN).is_none());
        }
    }
}
