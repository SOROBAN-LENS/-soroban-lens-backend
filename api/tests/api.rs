//! HTTP-level tests for the SorobanLens API, using the full router and
//! middleware stack with a scripted transaction source (no network access).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::Utc;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use soroban_lens_api::config::Config;
use soroban_lens_api::store::InMemoryStore;
use soroban_lens_api::{AppState, build_router};
use soroban_lens_diagnostic_engine::DiagnosticEngine;
use soroban_lens_models::{Network, TransactionHash, TransactionInformation, TransactionStatus};
use soroban_lens_stellar_client::{RpcError, TransactionSource};
use tower::ServiceExt;

const FOUND: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const MISSING: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const TIMEOUT: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const DOWN: &str = "4444444444444444444444444444444444444444444444444444444444444444";

/// Scripted source: behaviour depends on the hash.
struct Scripted;

#[async_trait]
impl TransactionSource for Scripted {
    async fn get_transaction(
        &self,
        network: Network,
        hash: &TransactionHash,
    ) -> Result<TransactionInformation, RpcError> {
        let status = match hash.as_str() {
            FOUND => TransactionStatus::Failed,
            MISSING => TransactionStatus::NotFound,
            TIMEOUT => {
                return Err(RpcError::Timeout {
                    endpoint: "rpc.internal.example".into(),
                });
            }
            _ => {
                return Err(RpcError::Unavailable {
                    endpoint: "rpc.internal.example".into(),
                    reason: "connection refused (secret-detail)".into(),
                });
            }
        };
        Ok(TransactionInformation {
            hash: hash.clone(),
            network,
            status,
            ledger: None,
            created_at: None,
            application_order: None,
            fee_bump: None,
            envelope_xdr: None,
            result_xdr: None,
            result_meta_xdr: None,
            diagnostic_events_xdr: None,
            latest_ledger: 200,
            oldest_ledger: 100,
        })
    }

    fn networks(&self) -> Vec<Network> {
        vec![Network::Testnet]
    }
}

fn app_with(vars: &[(&str, &str)]) -> Router {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let config = Config::from_lookup(|k| map.get(k).cloned()).expect("config");
    let state = AppState {
        engine: Arc::new(DiagnosticEngine::new(Arc::new(Scripted))),
        store: Arc::new(InMemoryStore::new(100)),
        default_network: Network::Testnet,
        started_at: Utc::now(),
    };
    build_router(state, &config)
}

fn app() -> Router {
    app_with(&[])
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, headers, body)
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).expect("request")
}

fn post_json(body: Value) -> Request<Body> {
    Request::post("/api/v1/analyze")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

#[tokio::test]
async fn health_is_ok_with_security_headers() {
    let (status, headers, body) = send(&app(), get("/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "soroban-lens-api");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(headers.contains_key("x-request-id"));
}

#[tokio::test]
async fn api_info_lists_networks_and_rules() {
    let (status, _, body) = send(&app(), get("/api/v1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["api_version"], "v1");
    assert_eq!(body["default_network"], "testnet");
    assert_eq!(body["networks"], json!(["testnet"]));
    assert_eq!(body["persistence"], "in_memory");
    assert!(body["engine"]["rules"].as_array().expect("rules").len() >= 4);
}

#[tokio::test]
async fn analyze_then_fetch_by_id() {
    let app = app();
    let (status, headers, report) =
        send(&app, post_json(json!({ "transaction_hash": FOUND }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(report["transaction_hash"], FOUND);
    assert_eq!(report["transaction_status"], "failed");
    assert_eq!(report["network"], "testnet");

    let id = report["id"].as_str().expect("id");
    let location = headers[header::LOCATION].to_str().expect("location");
    assert_eq!(location, format!("/api/v1/analyze/{id}"));

    let (status, _, fetched) = send(&app, get(location)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched, report);
}

#[tokio::test]
async fn hash_whitespace_is_trimmed() {
    // Case normalization is covered by the models crate's unit tests.
    let padded = format!("  {FOUND}\n");
    let (status, _, report) = send(&app(), post_json(json!({ "transaction_hash": padded }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(report["transaction_hash"], FOUND);
}

#[tokio::test]
async fn not_found_transaction_is_a_successful_analysis() {
    let (status, _, report) = send(&app(), post_json(json!({ "transaction_hash": MISSING }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(report["analysis_status"], "transaction_not_found");
    assert_eq!(report["issues"][0]["rule_id"], "rpc.transaction_not_found");
}

#[tokio::test]
async fn rejects_invalid_hashes() {
    for bad in [
        "",
        "abc",
        "zz11111111111111111111111111111111111111111111111111111111111111",
    ] {
        let (status, _, body) = send(&app(), post_json(json!({ "transaction_hash": bad }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "input {bad:?}");
        assert_eq!(body["error"]["code"], "invalid_transaction_hash");
    }
}

#[tokio::test]
async fn rejects_malformed_requests() {
    let app = app();

    let (status, _, body) = send(&app, post_json(json!({ "hash": FOUND }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");

    let (status, _, body) = send(
        &app,
        post_json(json!({ "transaction_hash": FOUND, "extra": true })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("msg")
            .contains("unknown field")
    );

    let request = Request::post("/api/v1/analyze")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not json"))
        .expect("request");
    let (status, _, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "malformed_json");

    let request = Request::post("/api/v1/analyze")
        .body(Body::from(json!({ "transaction_hash": FOUND }).to_string()))
        .expect("request");
    let (status, _, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(body["error"]["code"], "unsupported_media_type");
}

#[tokio::test]
async fn rejects_oversized_bodies() {
    let app = app_with(&[("SOROBAN_LENS_MAX_BODY_BYTES", "256")]);
    let padding = "x".repeat(1024);
    let (status, _, body) = send(
        &app,
        post_json(json!({ "transaction_hash": FOUND, "padding": padding })),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"]["code"], "payload_too_large");
}

#[tokio::test]
async fn rejects_unconfigured_network() {
    let (status, _, body) = send(
        &app(),
        post_json(json!({ "transaction_hash": FOUND, "network": "mainnet" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "network_not_configured");
}

#[tokio::test]
async fn rpc_failures_map_to_gateway_errors_without_leaking_details() {
    let app = app();

    let (status, _, body) = send(&app, post_json(json!({ "transaction_hash": TIMEOUT }))).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(body["error"]["code"], "rpc_timeout");

    let (status, _, body) = send(&app, post_json(json!({ "transaction_hash": DOWN }))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"]["code"], "rpc_unavailable");
    let text = body.to_string();
    assert!(!text.contains("rpc.internal.example"));
    assert!(!text.contains("secret-detail"));
}

#[tokio::test]
async fn get_analysis_validates_id() {
    let app = app();
    let (status, _, body) = send(&app, get("/api/v1/analyze/not-a-uuid")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_analysis_id");

    let (status, _, body) = send(
        &app,
        get("/api/v1/analyze/00000000-0000-4000-8000-000000000000"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "analysis_not_found");
}

#[tokio::test]
async fn analyze_is_rate_limited() {
    let app = app_with(&[("SOROBAN_LENS_RATE_LIMIT_PER_MINUTE", "2")]);
    for _ in 0..2 {
        let (status, _, _) = send(&app, post_json(json!({ "transaction_hash": FOUND }))).await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, headers, body) = send(&app, post_json(json!({ "transaction_hash": FOUND }))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
    assert!(headers.contains_key(header::RETRY_AFTER));

    // Read-only endpoints are not limited.
    let (status, _, _) = send(&app, get("/api/v1")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn stats_reflect_real_analyses() {
    let app = app();
    let (_, _, empty) = send(&app, get("/api/v1/stats")).await;
    assert_eq!(empty["total_analyses"], 0);

    send(&app, post_json(json!({ "transaction_hash": FOUND }))).await;
    send(&app, post_json(json!({ "transaction_hash": MISSING }))).await;
    send(&app, post_json(json!({ "transaction_hash": DOWN }))).await; // not stored

    let (status, _, stats) = send(&app, get("/api/v1/stats")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["total_analyses"], 2);
    assert_eq!(stats["failed_transactions"], 1);
    assert_eq!(stats["not_found_transactions"], 1);
    assert_eq!(stats["recent"].as_array().expect("recent").len(), 2);
    assert_eq!(
        stats["recent"][0]["transaction_hash"], MISSING,
        "newest first"
    );
}

#[tokio::test]
async fn unknown_routes_return_json_404() {
    let (status, _, body) = send(&app(), get("/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "route_not_found");
}

#[tokio::test]
async fn cors_allows_configured_origin_only() {
    let app = app();
    let preflight = |origin: &str| {
        Request::builder()
            .method("OPTIONS")
            .uri("/api/v1/analyze")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .body(Body::empty())
            .expect("request")
    };
    let (_, headers, _) = send(&app, preflight("http://localhost:5173")).await;
    assert_eq!(
        headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "http://localhost:5173"
    );
    let (_, headers, _) = send(&app, preflight("https://evil.example")).await;
    assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}
