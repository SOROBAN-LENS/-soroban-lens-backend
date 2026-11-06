//! Stellar RPC client for SorobanLens.
//!
//! This crate talks to [Stellar RPC](https://developers.stellar.org/docs/data/apis/rpc)
//! over JSON-RPC 2.0 and converts responses into
//! [`soroban_lens_models::TransactionInformation`].
//!
//! Supported RPC methods (verified against the official API reference):
//!
//! * `getTransaction` — fetch a transaction by hash.
//!
//! TODO(next milestone): `getHealth`, `getLatestLedger`, `simulateTransaction`
//! (for re-simulating failed invocations) and `getLedgerEntries` (for checking
//! whether referenced contracts exist or are archived).
//!
//! The [`TransactionSource`] trait abstracts over the data source so the
//! diagnostic engine and API can be tested without network access.

mod client;
mod error;
mod router;
mod wire;

pub use client::{RpcClientConfig, StellarRpcClient};
pub use error::RpcError;
pub use router::NetworkRouter;

use async_trait::async_trait;
use soroban_lens_models::{Network, TransactionHash, TransactionInformation};

/// Anything that can look up a transaction by hash on a Stellar network.
#[async_trait]
pub trait TransactionSource: Send + Sync {
    async fn get_transaction(
        &self,
        network: Network,
        hash: &TransactionHash,
    ) -> Result<TransactionInformation, RpcError>;

    /// Networks this source can serve.
    fn networks(&self) -> Vec<Network>;
}
