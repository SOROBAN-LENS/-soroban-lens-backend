//! End-to-end tests over **real** Stellar testnet `getTransaction` responses.
//!
//! The fixtures in `tests/fixtures/testnet/` were captured from
//! `https://soroban-testnet.stellar.org` (protocol 30) on 2026-11-06. See
//! `tests/fixtures/README.md` for provenance. They are replayed through the
//! real `StellarRpcClient` via a mock HTTP server, so these tests cover JSON
//! parsing, XDR decoding, rule evaluation, and report assembly together.

use std::sync::Arc;

use soroban_lens_diagnostic_engine::DiagnosticEngine;
use soroban_lens_models::{
    AnalysisStatus, DiagnosticReport, ErrorCategory, EvidenceLevel, Network, TransactionHash,
    TransactionStatus,
};
use soroban_lens_stellar_client::{RpcClientConfig, StellarRpcClient};
use url::Url;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn analyze_fixture(name: &str) -> DiagnosticReport {
    let path = format!(
        "{}/tests/fixtures/testnet/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let body = std::fs::read_to_string(&path).expect("fixture exists");
    let json: serde_json::Value = serde_json::from_str(&body).expect("fixture is JSON");
    let hash = json["result"]["txHash"]
        .as_str()
        .expect("txHash")
        .to_owned();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let client = StellarRpcClient::new(RpcClientConfig::new(
        Network::Testnet,
        Url::parse(&server.uri()).expect("url"),
    ))
    .expect("client");
    let engine = DiagnosticEngine::new(Arc::new(client));
    engine
        .analyze(
            Network::Testnet,
            &TransactionHash::parse(&hash).expect("hash"),
        )
        .await
        .expect("analysis succeeds")
}

#[tokio::test]
async fn footprint_violation_is_identified_from_diagnostic_events() {
    let report = analyze_fixture("invoke_trapped_a.json").await;

    assert_eq!(report.transaction_status, TransactionStatus::Failed);
    assert_eq!(report.analysis_status, AnalysisStatus::Completed);
    assert!(report.metadata.diagnostic_events.available);
    assert_eq!(report.metadata.diagnostic_events.count, 24);

    let tx = report.transaction.as_ref().expect("overview");
    assert_eq!(tx.result_code.as_deref(), Some("tx_failed"));
    let op = &tx.operations[0];
    assert_eq!(op.operation_type, "invoke_host_function");
    assert_eq!(
        op.result_code.as_deref(),
        Some("invoke_host_function_trapped")
    );
    assert_eq!(
        op.contract_id.as_deref(),
        Some("CD4KFB23CQLJ47RIPESVBTQ444R75M6QVEZULWXHHFYYZT7SS2VGXOPW")
    );
    assert_eq!(op.function_name.as_deref(), Some("write_prices"));
    let resources = tx.declared_resources.as_ref().expect("soroban resources");
    assert_eq!(resources.instructions, 5_929_683);

    // Most specific issue first: the host error explaining the trap.
    let root = &report.issues[0];
    assert_eq!(root.rule_id, "host.storage_exceeded_limit");
    assert_eq!(root.category, ErrorCategory::ResourceLimit);
    assert_eq!(root.evidence.level, EvidenceLevel::Confirmed);
    assert_eq!(root.details["host_error"], "Error(Storage, ExceededLimit)");
    assert!(root.details["event_message"].contains("outside of the footprint"));

    assert_eq!(report.issues[1].rule_id, "invoke.trapped");
    assert!(report.summary.contains("outside the declared footprint"));
}

#[tokio::test]
async fn second_trapped_fixture_is_classified_consistently() {
    let report = analyze_fixture("invoke_trapped_b.json").await;
    let ids: Vec<_> = report.issues.iter().map(|i| i.rule_id.as_str()).collect();
    assert_eq!(ids, vec!["host.storage_exceeded_limit", "invoke.trapped"]);
}

#[tokio::test]
async fn archived_entry_is_archived_state_not_invalid_arguments() {
    let report = analyze_fixture("invoke_entry_archived.json").await;

    assert_eq!(
        report.issues.len(),
        1,
        "host error is folded into the op issue"
    );
    let issue = &report.issues[0];
    assert_eq!(issue.rule_id, "invoke.entry_archived");
    assert_eq!(issue.category, ErrorCategory::ArchivedState);
    assert_eq!(issue.evidence.level, EvidenceLevel::Confirmed);
    assert_eq!(issue.details["host_error"], "Error(Value, InvalidInput)");
    assert!(issue.details["event_message"].contains("archived"));
    assert_eq!(
        issue.details["function_name"],
        "update_batch_ed25519_persistent"
    );
    assert!(
        issue
            .evidence
            .sources
            .iter()
            .any(|s| s.starts_with("diagnostic_event["))
    );
}

#[tokio::test]
async fn insufficient_refundable_fee_uses_the_operation_result_as_primary() {
    let report = analyze_fixture("invoke_insufficient_refundable_fee.json").await;

    assert_eq!(report.issues.len(), 1);
    let issue = &report.issues[0];
    assert_eq!(issue.rule_id, "invoke.insufficient_refundable_fee");
    assert_eq!(issue.category, ErrorCategory::Fee);
    assert_eq!(issue.details["host_error"], "Error(Budget, ExceededLimit)");
    // The host message's `{}` placeholders are filled from the event data.
    let message = &issue.details["event_message"];
    assert!(message.starts_with("refundable resource fee was not sufficient"));
    assert!(
        !message.contains("{}"),
        "placeholders substituted: {message}"
    );
    assert_eq!(
        report.summary,
        "Transaction failed: Refundable resource fee too low."
    );
}

#[tokio::test]
async fn classic_payment_failure_is_reported_without_soroban_caveats() {
    let report = analyze_fixture("classic_payment_no_destination.json").await;

    assert_eq!(report.analysis_status, AnalysisStatus::Completed);
    assert!(report.limitations.is_empty());
    assert!(!report.metadata.diagnostic_events.available);
    let issue = &report.issues[0];
    assert_eq!(issue.rule_id, "op.classic_failure");
    assert_eq!(issue.details["result_code"], "payment_no_destination");
    assert!(
        report
            .transaction
            .expect("overview")
            .declared_resources
            .is_none()
    );
}

#[tokio::test]
async fn reports_serialize_with_stable_field_names() {
    let report = analyze_fixture("invoke_trapped_a.json").await;
    let json = serde_json::to_value(&report).expect("serialize");
    for key in [
        "id",
        "transaction_hash",
        "network",
        "analysis_status",
        "transaction_status",
        "summary",
        "transaction",
        "issues",
        "limitations",
        "metadata",
    ] {
        assert!(json.get(key).is_some(), "missing top-level key `{key}`");
    }
    assert_eq!(json["analysis_status"], "completed");
    assert_eq!(json["transaction_status"], "failed");
    assert_eq!(json["network"], "testnet");
    assert_eq!(json["issues"][0]["category"], "resource_limit");
    assert_eq!(json["issues"][0]["evidence"]["level"], "confirmed");
    assert_eq!(json["metadata"]["schema_version"], "1.0");
}
