//! Per-chain connection setup, driven by the `chains` table.
//!
//! Chains are DATA, not code: which chains exist, their registry addresses,
//! deploy blocks, and reorg buffers all live in Postgres. Adding a chain is an
//! INSERT (see scripts/seed_chains.sql), not a refactor. This module turns one
//! `ChainConfig` row into a connected `Chain` the ingest loop can drive.
//!
//! Rust concept spotlight: **type erasure with `DynProvider`** — alloy's
//! builder returns a deeply-generic type; `.erased()` boxes it into one
//! nameable type, the same trick as `Box<dyn Trait>`.

use alloy::primitives::Address;
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use anyhow::{Context, Result};

/// One row of the `chains` table. `FromRow` maps columns to fields by name.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChainConfig {
    pub chain: String,
    /// EIP-155 id. Loaded and kept whole so a future startup check can assert
    /// the RPC actually serves this chain (guarding a mis-set RPC_URL).
    #[allow(dead_code)]
    pub chain_id: i64,
    pub identity_registry: String,
    pub reputation_registry: Option<String>,
    pub validation_registry: Option<String>,
    pub deploy_block: i64,
    pub confirmations: i32,
}

impl ChainConfig {
    /// The registry addresses to watch, parsed and validated. Refuses the
    /// zero address: a forgotten seed edit must fail loudly, not index nothing.
    pub fn registries(&self) -> Result<Vec<Address>> {
        let mut out = Vec::new();
        for (name, addr) in [
            ("identity", Some(self.identity_registry.as_str())),
            ("reputation", self.reputation_registry.as_deref()),
            ("validation", self.validation_registry.as_deref()),
        ] {
            let Some(addr) = addr else { continue }; // NULL = registry absent on this chain
            let parsed: Address = addr
                .parse()
                .with_context(|| format!("chain {}: bad {name} registry address {addr}", self.chain))?;
            if parsed == Address::ZERO {
                anyhow::bail!(
                    "chain {}: {name} registry is the zero address — run scripts/seed_chains.sql \
                     with the real ERC-8004 addresses before indexing",
                    self.chain
                );
            }
            out.push(parsed);
        }
        Ok(out)
    }

    /// The env var holding this chain's RPC URL, e.g. `RPC_URL_BASE`.
    pub fn rpc_env_var(&self) -> String {
        format!("RPC_URL_{}", self.chain.to_uppercase())
    }
}

/// A connected chain: a provider plus its config row.
pub struct Chain {
    pub config: ChainConfig,
    pub provider: DynProvider,
    pub registries: Vec<Address>,
}

impl Chain {
    pub async fn connect(config: ChainConfig, rpc_url: &str) -> Result<Self> {
        let registries = config.registries()?; // validate BEFORE dialing out
        let provider = ProviderBuilder::new()
            .connect(rpc_url)
            .await
            .with_context(|| format!("connecting to {} RPC", config.chain))?
            .erased();
        Ok(Self { config, provider, registries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(identity: &str) -> ChainConfig {
        ChainConfig {
            chain: "base".into(),
            chain_id: 8453,
            identity_registry: identity.into(),
            reputation_registry: None,
            validation_registry: None,
            deploy_block: 100,
            confirmations: 30,
        }
    }

    /// The guard that makes a forgotten seed edit fail loudly: a zero-address
    /// registry must refuse to configure, not silently index nothing.
    #[test]
    fn zero_address_registry_is_refused() {
        let err = config("0x0000000000000000000000000000000000000000")
            .registries()
            .unwrap_err();
        assert!(err.to_string().contains("zero address"));
    }

    #[test]
    fn null_registries_express_per_chain_feature_variance() {
        let regs = config("0xd8da6bf26964af9d7eed9e03e53415d37aa96045")
            .registries()
            .unwrap();
        assert_eq!(regs.len(), 1, "absent registries are skipped, not errors");
    }

    #[test]
    fn rpc_env_var_is_derived_from_the_chain_name() {
        assert_eq!(config("0x0").rpc_env_var(), "RPC_URL_BASE");
    }
}
