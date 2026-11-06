use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;
use soroban_lens_models::{Network, TransactionHash, TransactionInformation, TransactionStatus};
use url::Url;

use crate::wire::{
    GetTransactionParams, GetTransactionResult, JsonRpcRequest, JsonRpcResponse,
    WireTransactionStatus,
};
use crate::{RpcError, TransactionSource};

/// Configuration for a single RPC endpoint.
#[derive(Debug, Clone)]
pub struct RpcClientConfig {
    pub network: Network,
    /// Full endpoint URL. May contain a provider API key; it is never logged
    /// or included in error messages.
    pub url: Url,
    /// Total time allowed for a single request, including reading the body.
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
    /// Upper bound on response body size. Protects against unexpectedly large
    /// or malicious responses.
    pub max_response_bytes: usize,
}

impl RpcClientConfig {
    pub fn new(network: Network, url: Url) -> Self {
        Self {
            network,
            url,
            request_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(5),
            max_response_bytes: 8 * 1024 * 1024,
        }
    }
}

/// JSON-RPC client for one Stellar RPC endpoint.
#[derive(Debug)]
pub struct StellarRpcClient {
    http: reqwest::Client,
    config: RpcClientConfig,
    /// Host-only label used in logs and errors (never the full URL).
    endpoint_label: String,
    next_id: AtomicU64,
}

impl StellarRpcClient {
    pub fn new(config: RpcClientConfig) -> Result<Self, RpcError> {
        let endpoint_label = endpoint_label(&config.url);
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .connect_timeout(config.connect_timeout)
            .user_agent(concat!("soroban-lens/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| RpcError::Unavailable {
                endpoint: endpoint_label.clone(),
                reason: format!("failed to build HTTP client: {e}"),
            })?;
        Ok(Self {
            http,
            config,
            endpoint_label,
            next_id: AtomicU64::new(1),
        })
    }

    pub fn network(&self) -> Network {
        self.config.network
    }

    /// Host of the configured endpoint, safe to display.
    pub fn endpoint_label(&self) -> &str {
        &self.endpoint_label
    }

    /// Calls `getTransaction` for the given hash.
    pub async fn get_transaction(
        &self,
        hash: &TransactionHash,
    ) -> Result<TransactionInformation, RpcError> {
        let result: GetTransactionResult = self
            .call(
                "getTransaction",
                GetTransactionParams {
                    hash: hash.as_str(),
                },
            )
            .await?;
        Ok(self.to_information(hash, result))
    }

    async fn call<P: Serialize, T: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<T, RpcError> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            method,
            params,
        };

        tracing::debug!(endpoint = %self.endpoint_label, method, "sending RPC request");

        let mut response = self
            .http
            .post(self.config.url.clone())
            .json(&request)
            .send()
            .await
            .map_err(|e| self.map_transport_error(e))?;

        let status = response.status();
        if !status.is_success() {
            return Err(RpcError::HttpStatus {
                endpoint: self.endpoint_label.clone(),
                status: status.as_u16(),
            });
        }

        // Read the body incrementally so oversized responses are rejected
        // without buffering them entirely.
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| self.map_transport_error(e))?
        {
            if body.len() + chunk.len() > self.config.max_response_bytes {
                return Err(self.invalid(format!(
                    "response exceeded {} bytes",
                    self.config.max_response_bytes
                )));
            }
            body.extend_from_slice(&chunk);
        }

        let parsed: JsonRpcResponse<T> = serde_json::from_slice(&body)
            .map_err(|e| self.invalid(format!("malformed JSON-RPC response: {e}")))?;

        match (parsed.result, parsed.error) {
            (_, Some(err)) => Err(RpcError::JsonRpc {
                endpoint: self.endpoint_label.clone(),
                code: err.code,
                message: err.message,
            }),
            (Some(result), None) => Ok(result),
            (None, None) => Err(self.invalid("response has neither `result` nor `error`".into())),
        }
    }

    fn to_information(
        &self,
        hash: &TransactionHash,
        r: GetTransactionResult,
    ) -> TransactionInformation {
        let status = match r.status {
            WireTransactionStatus::Success => TransactionStatus::Success,
            WireTransactionStatus::Failed => TransactionStatus::Failed,
            WireTransactionStatus::NotFound => TransactionStatus::NotFound,
        };
        let created_at = r.created_at.as_deref().and_then(|s| match s.parse::<i64>() {
            Ok(ts) => Some(ts),
            Err(_) => {
                tracing::warn!(endpoint = %self.endpoint_label, value = s, "ignoring non-numeric createdAt");
                None
            }
        });
        TransactionInformation {
            hash: hash.clone(),
            network: self.config.network,
            status,
            ledger: r.ledger,
            created_at,
            application_order: r.application_order,
            fee_bump: r.fee_bump,
            envelope_xdr: r.envelope_xdr,
            result_xdr: r.result_xdr,
            result_meta_xdr: r.result_meta_xdr,
            diagnostic_events_xdr: r.diagnostic_events_xdr,
            latest_ledger: r.latest_ledger,
            oldest_ledger: r.oldest_ledger,
        }
    }

    fn map_transport_error(&self, err: reqwest::Error) -> RpcError {
        let endpoint = self.endpoint_label.clone();
        if err.is_timeout() {
            RpcError::Timeout { endpoint }
        } else if err.is_decode() {
            RpcError::InvalidResponse {
                endpoint,
                reason: "failed to read response body".into(),
            }
        } else {
            // `without_url` keeps API keys embedded in URLs out of messages.
            RpcError::Unavailable {
                endpoint,
                reason: err.without_url().to_string(),
            }
        }
    }

    fn invalid(&self, reason: String) -> RpcError {
        RpcError::InvalidResponse {
            endpoint: self.endpoint_label.clone(),
            reason,
        }
    }
}

#[async_trait]
impl TransactionSource for StellarRpcClient {
    async fn get_transaction(
        &self,
        network: Network,
        hash: &TransactionHash,
    ) -> Result<TransactionInformation, RpcError> {
        if network != self.config.network {
            return Err(RpcError::NetworkNotConfigured(network));
        }
        StellarRpcClient::get_transaction(self, hash).await
    }

    fn networks(&self) -> Vec<Network> {
        vec![self.config.network]
    }
}

fn endpoint_label(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        _ => "unknown-endpoint".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_label_never_includes_path_or_credentials() {
        let url = Url::parse("https://user:secret@rpc.example.com/api-key-123/rpc").expect("url");
        assert_eq!(endpoint_label(&url), "rpc.example.com");
        let url = Url::parse("http://127.0.0.1:8000/").expect("url");
        assert_eq!(endpoint_label(&url), "127.0.0.1:8000");
    }
}
