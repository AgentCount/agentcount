//! Per-chain connection setup.
//!
//! Ethereum and Base are both EVM chains, so the *code* to talk to them is
//! identical — only the RPC URL, the chain name, and the registry addresses
//! differ. This module wraps a connected provider with that per-chain metadata
//! so the ingest loop can stay chain-agnostic.
//!
//! Rust concept spotlight: **type erasure with `DynProvider`.** alloy's provider
//! builder returns a deeply-generic type (great for the compiler, painful to
//! store in a struct). `.erased()` boxes it into a single `DynProvider` type we
//! can name and stash easily — the same trick as `Box<dyn Trait>`, and a common
//! way to tame runaway generics.

use alloy::primitives::Address;
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use anyhow::{Context, Result};

/// A connected chain: a provider plus the config that varies per network. The
/// ingest loop takes one of these and doesn't care which chain it is.
pub struct Chain {
    /// Human-readable name stored alongside every row ("ethereum" / "base").
    pub name: String,
    /// The alloy JSON-RPC provider, type-erased so it's easy to hold here.
    pub provider: DynProvider,
    /// The ERC-8004 registry addresses to watch on THIS chain (identity,
    /// reputation, validation). We pass the whole list to one log filter.
    pub registries: Vec<Address>,
}

impl Chain {
    /// Connect to a chain's RPC endpoint and bundle it with its registry config.
    pub async fn connect(name: &str, rpc_url: &str) -> Result<Self> {
        // `connect` auto-detects the transport from the URL scheme and returns a
        // ready provider; `.erased()` collapses its generic type into DynProvider.
        let provider = ProviderBuilder::new()
            .connect(rpc_url)
            .await
            .with_context(|| format!("connecting to {name} RPC at {rpc_url}"))?
            .erased();

        Ok(Self {
            name: name.to_string(),
            provider,
            registries: registry_addresses(name)?,
        })
    }
}

/// The three registry addresses for a chain.
///
/// ⚠️ PLACEHOLDERS. ERC-8004's registry deployments must be filled in here with
/// the real addresses per network (they differ between Ethereum and Base). Until
/// then these are the zero address, so the indexer will connect and run happily
/// but find no events. The `match` on `name` forces you to decide what an unknown
/// chain does, rather than silently mis-indexing.
fn registry_addresses(name: &str) -> Result<Vec<Address>> {
    match name {
        "ethereum" | "base" => Ok(vec![
            Address::ZERO, // TODO: Identity Registry
            Address::ZERO, // TODO: Reputation Registry
            Address::ZERO, // TODO: Validation Registry
        ]),
        other => anyhow::bail!("unknown chain: {other}"),
    }
}
