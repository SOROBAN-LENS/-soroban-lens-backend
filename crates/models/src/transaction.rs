use serde::{Deserialize, Serialize};

use crate::{InvalidTransactionHash, Network, TransactionHash};

/// Body of `POST /api/v1/analyze`.
///
/// The hash is kept as a raw string here so the API layer can return a precise
/// validation error; call [`TransactionRequest::validated_hash`] to parse it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionRequest {
    /// Hex-encoded transaction hash (64 characters).
    pub transaction_hash: String,
    /// Network to query. Defaults to the backend's configured default network.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
}

impl TransactionRequest {
    pub fn validated_hash(&self) -> Result<TransactionHash, InvalidTransactionHash> {
        TransactionHash::parse(&self.transaction_hash)
    }
}

/// On-chain outcome of a transaction as reported by Stellar RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    /// Included in a ledger and applied successfully.
    Success,
    /// Included in a ledger but failed during application.
    Failed,
    /// The RPC node has no record of the transaction within its retention window.
    NotFound,
}

/// Raw transaction data retrieved from Stellar RPC `getTransaction`,
/// normalized into a transport-independent shape.
///
/// XDR payloads are kept base64-encoded; decoding and interpretation is the
/// job of the diagnostic engine, so this type stays a faithful record of what
/// the RPC node returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionInformation {
    pub hash: TransactionHash,
    pub network: Network,
    pub status: TransactionStatus,
    /// Ledger sequence that included the transaction.
    pub ledger: Option<u32>,
    /// Unix timestamp (seconds) of the ledger close that included the transaction.
    pub created_at: Option<i64>,
    /// Index of the transaction within its ledger.
    pub application_order: Option<u32>,
    pub fee_bump: Option<bool>,
    /// Base64 `TransactionEnvelope` XDR.
    pub envelope_xdr: Option<String>,
    /// Base64 `TransactionResult` XDR.
    pub result_xdr: Option<String>,
    /// Base64 `TransactionMeta` XDR.
    pub result_meta_xdr: Option<String>,
    /// Base64 `DiagnosticEvent` XDR values from the top-level
    /// `diagnosticEventsXdr` RPC field. `None` means the node did not include
    /// the field at all (it is only populated when the RPC operator enables
    /// diagnostic events); `Some(vec![])` means the field was present but empty.
    pub diagnostic_events_xdr: Option<Vec<String>>,
    /// Latest ledger known to the RPC node when the request was served.
    pub latest_ledger: u32,
    /// Oldest ledger retained by the RPC node when the request was served.
    pub oldest_ledger: u32,
}
