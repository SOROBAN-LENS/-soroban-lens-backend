use soroban_lens_models::Network;

/// Errors produced while retrieving data from Stellar RPC.
///
/// Messages deliberately avoid including the full endpoint URL, because RPC
/// provider URLs frequently embed API keys. Only the endpoint host is used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RpcError {
    /// No RPC endpoint is configured for the requested network.
    #[error("no RPC endpoint is configured for network `{0}`")]
    NetworkNotConfigured(Network),

    /// The request did not complete within the configured timeout.
    #[error("request to RPC endpoint `{endpoint}` timed out")]
    Timeout { endpoint: String },

    /// The endpoint could not be reached (DNS, connection refused, TLS…).
    #[error("RPC endpoint `{endpoint}` is unavailable: {reason}")]
    Unavailable { endpoint: String, reason: String },

    /// The endpoint responded with a non-success HTTP status.
    #[error("RPC endpoint `{endpoint}` returned HTTP status {status}")]
    HttpStatus { endpoint: String, status: u16 },

    /// The endpoint returned a JSON-RPC error object.
    #[error("RPC endpoint `{endpoint}` returned JSON-RPC error {code}: {message}")]
    JsonRpc {
        endpoint: String,
        code: i64,
        message: String,
    },

    /// The response could not be parsed or violated the documented schema.
    #[error("RPC endpoint `{endpoint}` returned an invalid response: {reason}")]
    InvalidResponse { endpoint: String, reason: String },
}

impl RpcError {
    /// True for errors that are likely transient and worth retrying later.
    pub fn is_transient(&self) -> bool {
        match self {
            RpcError::Timeout { .. } | RpcError::Unavailable { .. } => true,
            RpcError::HttpStatus { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }
}
