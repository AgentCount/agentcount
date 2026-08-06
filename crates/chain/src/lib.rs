//! # chain — the only crate that talks to a blockchain node.
//!
//! Isolating RPC here is what lets `crates/checks` be a pure library: checks
//! receive an [`registry::AgentSnapshot`] as data and never learn where it
//! came from.

pub mod registry;
pub mod reputation;
pub mod token;
pub use registry::{AgentSnapshot, Registry};
pub use reputation::{FeedbackReads, Reputation};
pub use token::{Erc20, Side, TokenMetadata, TransferLog};
