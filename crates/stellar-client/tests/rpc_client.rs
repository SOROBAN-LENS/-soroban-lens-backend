//! Integration tests for the Stellar RPC client against a mock JSON-RPC server.

use std::time::Duration;

use serde_json::json;
use soroban_lens_models::{Network, TransactionHash, TransactionStatus};
use soroban_lens_stellar_client::{
    NetworkRouter, RpcClientConfig, RpcError, StellarRpcClient, TransactionSource,
};
use url::Url;
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HASH: &str = "362e1f1671b3516bfb177a5260ec9c8e24ee6c8fc8e1a1271c8c91db02c9ead7";

fn hash() -> TransactionHash {
    TransactionHash::parse(HASH).expect("valid hash")
}

fn client_for(server: &MockServer) -> StellarRpcClient {
    let mut config = RpcClientConfig::new(
        Network::Testnet,
        Url::parse(&server.uri()).expect("mock url"),
    );
    config.request_timeout = Duration::from_millis(500);
    config.max_response_bytes = 64 * 1024;
    StellarRpcClient::new(config).expect("client")
}

#[tokio::test]
async fn parses_failed_transaction() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({
            "jsonrpc": "2.0",
            "method": "getTransaction",
            "params": { "hash": HASH }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "status": "FAILED",
                "txHash": HASH,
                "latestLedger": 5589931,
                "latestLedgerCloseTime": "1793973296",
                "oldestLedger": 5468972,
                "oldestLedgerCloseTime": "1793368500",
                "ledger": 5586927,
                "createdAt": "1793958276",
                "applicationOrder": 3,
                "feeBump": false,
                "envelopeXdr": "AAAA",
                "resultXdr": "BBBB",
                "resultMetaXdr": "CCCC",
                "events": { "transactionEventsXdr": [] }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let info = client_for(&server)
        .get_transaction(&hash())
        .await
        .expect("success");

    assert_eq!(info.status, TransactionStatus::Failed);
    assert_eq!(info.network, Network::Testnet);
    assert_eq!(info.ledger, Some(5586927));
    assert_eq!(info.created_at, Some(1793958276));
    assert_eq!(info.application_order, Some(3));
    assert_eq!(info.fee_bump, Some(false));
    assert_eq!(info.result_xdr.as_deref(), Some("BBBB"));
    assert_eq!(info.diagnostic_events_xdr, None, "field absent => None");
    assert_eq!(info.latest_ledger, 5589931);
    assert_eq!(info.oldest_ledger, 5468972);
}

#[tokio::test]
async fn parses_not_found_and_preserves_diagnostic_event_presence() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "status": "NOT_FOUND",
                "latestLedger": 100,
                "latestLedgerCloseTime": "1",
                "oldestLedger": 10,
                "oldestLedgerCloseTime": "0",
                "diagnosticEventsXdr": []
            }
        })))
        .mount(&server)
        .await;

    let info = client_for(&server)
        .get_transaction(&hash())
        .await
        .expect("success");
    assert_eq!(info.status, TransactionStatus::NotFound);
    assert_eq!(info.ledger, None);
    assert_eq!(info.diagnostic_events_xdr, Some(vec![]));
}

#[tokio::test]
async fn maps_json_rpc_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32602, "message": "invalid hash" }
        })))
        .mount(&server)
        .await;

    let err = client_for(&server)
        .get_transaction(&hash())
        .await
        .expect_err("error");
    assert!(
        matches!(err, RpcError::JsonRpc { code: -32602, ref message, .. } if message == "invalid hash"),
        "unexpected error: {err:?}"
    );
    assert!(!err.is_transient());
}

#[tokio::test]
async fn maps_http_error_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let err = client_for(&server)
        .get_transaction(&hash())
        .await
        .expect_err("error");
    assert!(matches!(err, RpcError::HttpStatus { status: 503, .. }));
    assert!(err.is_transient());
}

#[tokio::test]
async fn maps_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(3))
                .set_body_json(json!({})),
        )
        .mount(&server)
        .await;

    let err = client_for(&server)
        .get_transaction(&hash())
        .await
        .expect_err("error");
    assert!(matches!(err, RpcError::Timeout { .. }), "got {err:?}");
    assert!(err.is_transient());
}

#[tokio::test]
async fn maps_unreachable_endpoint() {
    // Port 9 (discard) on localhost is essentially never listening.
    let config = RpcClientConfig::new(
        Network::Testnet,
        Url::parse("http://127.0.0.1:9/").expect("url"),
    );
    let client = StellarRpcClient::new(config).expect("client");
    let err = client.get_transaction(&hash()).await.expect_err("error");
    assert!(
        matches!(err, RpcError::Unavailable { .. } | RpcError::Timeout { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn rejects_malformed_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&server)
        .await;

    let err = client_for(&server)
        .get_transaction(&hash())
        .await
        .expect_err("error");
    assert!(matches!(err, RpcError::InvalidResponse { .. }));
}

#[tokio::test]
async fn rejects_schema_violations() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "status": "PENDING", "latestLedger": 1, "oldestLedger": 1 }
        })))
        .mount(&server)
        .await;

    let err = client_for(&server)
        .get_transaction(&hash())
        .await
        .expect_err("error");
    assert!(matches!(err, RpcError::InvalidResponse { .. }));
}

#[tokio::test]
async fn rejects_oversized_response() {
    let server = MockServer::start().await;
    let huge = "A".repeat(128 * 1024);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "status": "FAILED", "latestLedger": 1, "oldestLedger": 1, "envelopeXdr": huge }
        })))
        .mount(&server)
        .await;

    let err = client_for(&server)
        .get_transaction(&hash())
        .await
        .expect_err("error");
    assert!(
        matches!(err, RpcError::InvalidResponse { ref reason, .. } if reason.contains("exceeded")),
        "got {err:?}"
    );
}

#[tokio::test]
async fn errors_never_contain_url_secrets() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let url = Url::parse(&format!("{}/secret-api-key-xyz", server.uri())).expect("url");
    let client =
        StellarRpcClient::new(RpcClientConfig::new(Network::Testnet, url)).expect("client");
    let err = client.get_transaction(&hash()).await.expect_err("error");
    assert!(!err.to_string().contains("secret-api-key-xyz"));
}

#[tokio::test]
async fn router_rejects_unconfigured_network() {
    let server = MockServer::start().await;
    let router = NetworkRouter::new().with_client(client_for(&server));
    assert_eq!(router.networks(), vec![Network::Testnet]);
    let err = router
        .get_transaction(Network::Mainnet, &hash())
        .await
        .expect_err("error");
    assert_eq!(err, RpcError::NetworkNotConfigured(Network::Mainnet));
}
