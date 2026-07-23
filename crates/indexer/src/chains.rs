//! Per-chain connection setup.
//!
//! Ethereum and Base are both EVM chains, so the *code* to talk to them is
//! identical — only the RPC URL, the chain name, and the registry contract
//! addresses differ. This module wraps a connected RPC provider together with
//! that per-chain metadata so the ingest loop can stay chain-agnostic.
//!
//! Rust concept spotlight: **struct-holds-a-client + `impl` blocks.** We bundle
//! the alloy provider and some config into one `Chain` struct, then hang methods
//! off it in an `impl Chain { ... }` block. Methods that take `&self` borrow the
//! chain to use it; the constructor `connect` returns a brand-new owned `Chain`.

use anyhow::Result;

/// A connected chain: an RPC provider plus the bits of config that vary per
/// network. `run` (in `ingest.rs`) takes one of these and doesn't care whether
/// it's Ethereum or Base.
pub struct Chain {
    /// Human-readable name we store alongside every row ("ethereum" / "base").
    pub name: String,

    /// The alloy JSON-RPC provider — our handle for making chain calls.
    ///
    /// The real type is something like
    /// `alloy::providers::RootProvider<alloy::transports::http::Http<...>>`,
    /// which is a mouthful. When you wire alloy in, either write the concrete
    /// type or (more commonly) store a boxed `dyn Provider`. Left as a
    /// placeholder here to keep the skeleton dependency-light:
    ///
    ///     pub provider: alloy::providers::RootProvider<Http<Client>>,
    pub provider: ProviderPlaceholder,

    /// The three ERC-8004 registry contract addresses on THIS chain.
    pub registries: RegistryAddresses,
}

/// The ERC-8004 registries we watch. ERC-8004 ("Trustless Agents") splits agent
/// trust into three on-chain registries; we index events from all three.
///
/// NOTE: these addresses are chain-specific and, as of writing, you must fill in
/// the real deployed addresses per network. Using the wrong address just means
/// you'll index nothing — a quiet failure, so double-check them.
pub struct RegistryAddresses {
    /// Identity Registry — where agents register an id, a domain, and an address.
    pub identity: AddressPlaceholder,
    /// Reputation Registry — feedback/attestations between agents.
    pub reputation: AddressPlaceholder,
    /// Validation Registry — validators attesting to work being correct.
    pub validation: AddressPlaceholder,
}

impl Chain {
    /// Connect to a chain's RPC endpoint and bundle it with its registry config.
    ///
    /// `async` because opening the provider does a network handshake; the caller
    /// will `.await` it. Returns an owned `Chain` the caller then owns entirely.
    pub async fn connect(name: &str, rpc_url: &str) -> Result<Self> {
        // With alloy this is roughly:
        //     let provider = alloy::providers::ProviderBuilder::new()
        //         .on_http(rpc_url.parse()?);
        //
        // Then pick the right registry addresses for this `name`. A `match` on
        // the chain name is the idiomatic way; the `_` arm forces you to decide
        // what to do about an unknown chain instead of silently misbehaving:
        //
        //     let registries = match name {
        //         "ethereum" => RegistryAddresses { /* mainnet addrs */ },
        //         "base"     => RegistryAddresses { /* base addrs */ },
        //         other => anyhow::bail!("unknown chain: {other}"),
        //     };
        //
        //     Ok(Self { name: name.to_string(), provider, registries })

        let _ = (name, rpc_url);
        todo!("build an alloy provider from rpc_url and select this chain's registry addresses")
    }
}

// ── Placeholders so this file reads without pulling alloy in yet ─────────────
// Delete these three and use the real alloy types (`Provider`, `Address`) once
// you add alloy to this crate's dependencies and start filling in bodies.

/// Stand-in for an alloy provider handle.
pub struct ProviderPlaceholder;

/// Stand-in for an `alloy::primitives::Address` (a 20-byte Ethereum address).
pub struct AddressPlaceholder;

/// Convenience alias so the intent reads clearly above.
pub type Address = AddressPlaceholder;
