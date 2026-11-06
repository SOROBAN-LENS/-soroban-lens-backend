//! API error responses.
//!
//! Every error is returned as:
//!
//! ```json
//! { "error": { "code": "invalid_transaction_hash", "message": "…" } }
//! ```
//!
//! `code` is stable and intended for programmatic handling; `message` is for
//! humans. Internal details (upstream URLs, stack traces) are logged, never
//! returned.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use soroban_lens_diagnostic_engine::EngineError;
use soroban_lens_stellar_client::RpcError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    /// Seconds after which the client may retry (sets `Retry-After`).
    pub retry_after: Option<u64>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An unexpected error occurred.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(ErrorBody {
            error: ErrorDetail {
                code: self.code,
                message: &self.message,
            },
        });
        let mut response = (self.status, body).into_response();
        if let Some(secs) = self.retry_after
            && let Ok(value) = secs.to_string().parse()
        {
            response.headers_mut().insert("retry-after", value);
        }
        response
    }
}

impl From<EngineError> for ApiError {
    fn from(err: EngineError) -> Self {
        match err {
            EngineError::Rpc(rpc) => rpc.into(),
        }
    }
}

impl From<RpcError> for ApiError {
    fn from(err: RpcError) -> Self {
        // Full detail (host only, never the URL) goes to the logs.
        tracing::warn!(error = %err, "Stellar RPC request failed");
        match err {
            RpcError::NetworkNotConfigured(network) => ApiError::bad_request(
                "network_not_configured",
                format!("Network `{network}` is not enabled on this server."),
            ),
            RpcError::Timeout { .. } => ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "rpc_timeout",
                "The Stellar RPC node did not respond in time. Please try again.",
            ),
            RpcError::Unavailable { .. } | RpcError::HttpStatus { .. } => ApiError::new(
                StatusCode::BAD_GATEWAY,
                "rpc_unavailable",
                "The Stellar RPC node is currently unavailable. Please try again later.",
            ),
            RpcError::JsonRpc { .. } => ApiError::new(
                StatusCode::BAD_GATEWAY,
                "rpc_error",
                "The Stellar RPC node rejected the request.",
            ),
            RpcError::InvalidResponse { .. } => ApiError::new(
                StatusCode::BAD_GATEWAY,
                "rpc_invalid_response",
                "The Stellar RPC node returned a response that could not be understood.",
            ),
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection {
            JsonRejection::MissingJsonContentType(_) => ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "Expected `Content-Type: application/json`.",
            ),
            JsonRejection::JsonSyntaxError(_) => {
                ApiError::bad_request("malformed_json", "The request body is not valid JSON.")
            }
            JsonRejection::JsonDataError(e) => {
                // serde's message (e.g. "unknown field `foo`") is safe and useful.
                ApiError::bad_request("invalid_request", e.body_text())
            }
            JsonRejection::BytesRejection(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "payload_too_large",
                    "The request body is too large.",
                )
            }
            other => ApiError::new(
                other.status(),
                "invalid_request",
                "The request body could not be read.",
            ),
        }
    }
}
