//! Engine tests over **synthetic** XDR, built programmatically with the
//! official `stellar-xdr` types. These cover outcomes not present in the real
//! fixtures. They are labeled synthetic: they verify decoding and rule wiring,
//! not real-network behaviour.

use std::sync::Arc;

use async_trait::async_trait;
use soroban_lens_diagnostic_engine::{DiagnosticEngine, EngineError};
use soroban_lens_models::{
    AnalysisStatus, ErrorCategory, Network, TransactionHash, TransactionInformation,
    TransactionStatus,
};
use soroban_lens_stellar_client::{RpcError, TransactionSource};
use stellar_xdr::{
    ContractEvent, ContractEventBody, ContractEventType, ContractEventV0, DiagnosticEvent,
    ExtensionPoint, Hash, InnerTransactionResult, InnerTransactionResultExt,
    InnerTransactionResultPair, InnerTransactionResultResult, InvokeHostFunctionResult, Limits,
    OperationResult, OperationResultTr, ScError, ScString, ScSymbol, ScVal, TransactionResult,
    TransactionResultExt, TransactionResultResult, WriteXdr,
};

const HASH: &str = "05235449a69c717377572f9f56e4efcedd8a8ef20d212c8e68d479a964505dd8";

/// Source that returns a canned response.
struct Canned(Result<TransactionInformation, RpcError>);

#[async_trait]
impl TransactionSource for Canned {
    async fn get_transaction(
        &self,
        _network: Network,
        _hash: &TransactionHash,
    ) -> Result<TransactionInformation, RpcError> {
        self.0.clone()
    }
    fn networks(&self) -> Vec<Network> {
        vec![Network::Testnet]
    }
}

fn info(status: TransactionStatus) -> TransactionInformation {
    TransactionInformation {
        hash: TransactionHash::parse(HASH).expect("hash"),
        network: Network::Testnet,
        status,
        ledger: (status != TransactionStatus::NotFound).then_some(100),
        created_at: None,
        application_order: None,
        fee_bump: None,
        envelope_xdr: None,
        result_xdr: None,
        result_meta_xdr: None,
        diagnostic_events_xdr: None,
        latest_ledger: 200,
        oldest_ledger: 50,
    }
}

fn engine() -> DiagnosticEngine {
    DiagnosticEngine::new(Arc::new(Canned(Err(RpcError::NetworkNotConfigured(
        Network::Mainnet,
    )))))
}

fn result_xdr(result: TransactionResultResult) -> String {
    TransactionResult {
        fee_charged: 100,
        result,
        ext: TransactionResultExt::V0,
    }
    .to_xdr_base64(Limits::none())
    .expect("encode")
}

fn invoke(r: InvokeHostFunctionResult) -> OperationResult {
    OperationResult::OpInner(OperationResultTr::InvokeHostFunction(r))
}

fn error_event(error: ScError) -> String {
    DiagnosticEvent {
        in_successful_contract_call: false,
        event: ContractEvent {
            ext: ExtensionPoint::V0,
            contract_id: None,
            type_: ContractEventType::Diagnostic,
            body: ContractEventBody::V0(ContractEventV0 {
                topics: vec![
                    ScVal::Symbol(ScSymbol("error".try_into().expect("symbol"))),
                    ScVal::Error(error),
                ]
                .try_into()
                .expect("topics"),
                data: ScVal::String(ScString(
                    "escalating error to panic".try_into().expect("string"),
                )),
            }),
        },
    }
    .to_xdr_base64(Limits::none())
    .expect("encode")
}

#[test]
fn not_found_report() {
    let report = engine().analyze_information(&info(TransactionStatus::NotFound));
    assert_eq!(report.analysis_status, AnalysisStatus::TransactionNotFound);
    assert_eq!(report.transaction_status, TransactionStatus::NotFound);
    assert!(report.transaction.is_none());
    assert_eq!(report.issues[0].rule_id, "rpc.transaction_not_found");
    assert!(report.summary.contains("ledgers 50–200"));
    assert!(report.limitations.is_empty());
}

#[test]
fn successful_invocation_has_no_issues() {
    let mut i = info(TransactionStatus::Success);
    i.result_xdr = Some(result_xdr(TransactionResultResult::TxSuccess(
        vec![invoke(InvokeHostFunctionResult::Success(Hash([0; 32])))]
            .try_into()
            .expect("ops"),
    )));
    let report = engine().analyze_information(&i);
    assert!(report.issues.is_empty());
    assert_eq!(
        report.summary,
        "Transaction succeeded in ledger 100; no failure detected."
    );
    // No envelope supplied, which is recorded honestly.
    assert_eq!(report.analysis_status, AnalysisStatus::Partial);
    assert_eq!(report.limitations[0].code, "envelope_xdr_missing");
}

#[test]
fn bad_sequence_number() {
    let mut i = info(TransactionStatus::Failed);
    i.result_xdr = Some(result_xdr(TransactionResultResult::TxBadSeq));
    let report = engine().analyze_information(&i);
    assert_eq!(report.issues[0].rule_id, "tx.bad_seq");
    assert_eq!(
        report.issues[0].category,
        ErrorCategory::TransactionValidity
    );
    assert_eq!(
        report.transaction.expect("overview").result_code.as_deref(),
        Some("tx_bad_seq")
    );
}

#[test]
fn fee_bump_inner_failure_is_unwrapped() {
    let mut i = info(TransactionStatus::Failed);
    i.result_xdr = Some(result_xdr(TransactionResultResult::TxFeeBumpInnerFailed(
        InnerTransactionResultPair {
            transaction_hash: Hash([1; 32]),
            result: InnerTransactionResult {
                fee_charged: 50,
                result: InnerTransactionResultResult::TxFailed(
                    vec![invoke(InvokeHostFunctionResult::ResourceLimitExceeded)]
                        .try_into()
                        .expect("ops"),
                ),
                ext: InnerTransactionResultExt::V0,
            },
        },
    )));
    let report = engine().analyze_information(&i);
    assert_eq!(report.issues[0].rule_id, "invoke.resource_limit_exceeded");
    assert_eq!(report.issues[0].category, ErrorCategory::ResourceLimit);
}

#[test]
fn contract_error_code_from_diagnostic_event() {
    let mut i = info(TransactionStatus::Failed);
    i.result_xdr = Some(result_xdr(TransactionResultResult::TxFailed(
        vec![invoke(InvokeHostFunctionResult::Trapped)]
            .try_into()
            .expect("ops"),
    )));
    i.diagnostic_events_xdr = Some(vec![
        error_event(ScError::Contract(4)),
        error_event(ScError::Contract(4)),
    ]);
    let report = engine().analyze_information(&i);
    assert_eq!(report.metadata.diagnostic_events.count, 2);
    let root = &report.issues[0];
    assert_eq!(root.rule_id, "host.contract_error");
    assert_eq!(root.details["contract_error_code"], "4");
    assert_eq!(
        report.summary,
        "Transaction failed: Contract returned error code 4."
    );
}

#[test]
fn undecodable_xdr_produces_partial_report_not_a_crash() {
    let mut i = info(TransactionStatus::Failed);
    i.envelope_xdr = Some("bm90IHhkcg==".into()); // base64 "not xdr"
    i.result_xdr = Some("%%%".into());
    i.diagnostic_events_xdr = Some(vec!["AAAA".into()]);
    let report = engine().analyze_information(&i);
    assert_eq!(report.analysis_status, AnalysisStatus::Partial);
    let codes: Vec<_> = report.limitations.iter().map(|l| l.code.as_str()).collect();
    assert!(codes.contains(&"envelope_xdr_decode_failed"));
    assert!(codes.contains(&"result_xdr_decode_failed"));
    assert!(codes.contains(&"diagnostic_event_decode_failed"));
    assert_eq!(report.issues[0].rule_id, "analysis.unexplained_failure");
}

#[tokio::test]
async fn rpc_errors_propagate() {
    let engine = DiagnosticEngine::new(Arc::new(Canned(Err(RpcError::Timeout {
        endpoint: "rpc.example".into(),
    }))));
    let err = engine
        .analyze(
            Network::Testnet,
            &TransactionHash::parse(HASH).expect("hash"),
        )
        .await
        .expect_err("error");
    assert!(matches!(err, EngineError::Rpc(RpcError::Timeout { .. })));
}
