use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Network, TransactionHash, TransactionStatus};

/// Outcome of the diagnostic process itself (not of the transaction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisStatus {
    /// The transaction was found and all available data was analyzed.
    Completed,
    /// The transaction was found, but some data needed for a full diagnosis was
    /// missing or could not be decoded. See [`DiagnosticReport::limitations`].
    Partial,
    /// The RPC node has no record of the transaction.
    TransactionNotFound,
}

/// Broad classification of a diagnosed problem. Shared terminology across the
/// SorobanLens frontend, backend, and contracts repositories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// Classic signature checks or Soroban `require_auth` failures.
    Authorization,
    /// A contract trapped, panicked, or returned a contract-defined error.
    ContractExecution,
    /// Arguments or values had the wrong type, size, or were otherwise invalid.
    InvalidArguments,
    /// CPU, memory, I/O, or other declared/network resource limits were exceeded.
    ResourceLimit,
    /// A contract, contract code, or required ledger entry could not be found.
    MissingContractReference,
    /// A ledger entry needed by the transaction has been archived and must be restored.
    ArchivedState,
    /// Transaction-level validity problems: sequence numbers, time bounds,
    /// malformed envelopes, missing accounts, Soroban preconditions.
    TransactionValidity,
    /// Inclusion or refundable resource fees were insufficient.
    Fee,
    /// Failure reported by `simulateTransaction`. Reserved: not emitted yet.
    Simulation,
    /// Problems retrieving data from Stellar RPC (including "not found").
    NetworkRpc,
    /// Failure information exists but does not match any known rule.
    Unknown,
}

/// How serious an issue is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// How strongly the available data supports an issue.
///
/// Ordered from strongest to weakest so that sorting puts the best-supported
/// issues first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    /// Directly stated by on-chain data (result codes, host error events).
    Confirmed,
    /// Strongly suggested by on-chain data, but the root cause is inferred.
    Likely,
    /// One of several plausible explanations; needs manual verification.
    Possible,
    /// Not enough data was available to determine the cause.
    Insufficient,
}

/// The evidence behind an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub level: EvidenceLevel,
    /// Where the evidence came from, e.g. `"transaction_result"`,
    /// `"operation_result[0]"`, `"diagnostic_event[3]"`.
    pub sources: Vec<String>,
}

/// A concrete debugging action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedFix {
    pub title: String,
    pub description: String,
    /// Link to official documentation, when one is relevant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_url: Option<String>,
}

/// One diagnosed problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticIssue {
    /// Stable identifier of the rule that produced this issue, e.g.
    /// `"invoke.trapped"`. Safe to use for programmatic matching.
    pub rule_id: String,
    pub category: ErrorCategory,
    pub severity: Severity,
    pub title: String,
    /// Plain-language explanation for developers new to Soroban.
    pub explanation: String,
    /// Precise explanation referencing protocol-level codes.
    pub technical_explanation: String,
    /// Likely or possible reasons. Not confirmed unless `evidence.level` is
    /// `confirmed` *and* only one cause is listed.
    pub possible_causes: Vec<String>,
    pub suggested_fixes: Vec<SuggestedFix>,
    pub evidence: Evidence,
    /// Structured facts extracted from the transaction (codes, contract IDs…).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
}

/// Something the engine could not determine, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limitation {
    /// Stable machine-readable code, e.g. `"diagnostic_events_unavailable"`.
    pub code: String,
    pub message: String,
}

/// Summary of one operation in the transaction envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationOverview {
    pub index: usize,
    /// Operation type, e.g. `"invoke_host_function"`, `"payment"`.
    pub operation_type: String,
    /// Result code name for this operation, if the result was decoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_code: Option<String>,
    /// Strkey (`C…`) of the invoked contract, for contract invocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
}

/// Soroban resources declared in the transaction's `SorobanTransactionData`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredResources {
    pub instructions: u32,
    pub disk_read_bytes: u32,
    pub write_bytes: u32,
    /// Resource fee in stroops.
    pub resource_fee: i64,
    pub footprint_read_only: usize,
    pub footprint_read_write: usize,
}

/// Decoded facts about the transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionOverview {
    pub ledger: Option<u32>,
    /// Unix timestamp (seconds).
    pub created_at: Option<i64>,
    pub application_order: Option<u32>,
    pub fee_bump: Option<bool>,
    /// Fee charged in stroops.
    pub fee_charged: Option<i64>,
    /// Transaction result code, e.g. `"tx_failed"`.
    pub result_code: Option<String>,
    pub operations: Vec<OperationOverview>,
    pub declared_resources: Option<DeclaredResources>,
}

/// Whether diagnostic events were available to the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticEventAvailability {
    /// True if the RPC response or transaction meta contained diagnostic events.
    pub available: bool,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportMetadata {
    pub schema_version: String,
    pub engine_version: String,
    pub analyzed_at: DateTime<Utc>,
    pub latest_ledger: Option<u32>,
    pub oldest_ledger: Option<u32>,
    pub diagnostic_events: DiagnosticEventAvailability,
}

/// The complete output of a SorobanLens analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub id: Uuid,
    pub transaction_hash: TransactionHash,
    pub network: Network,
    pub analysis_status: AnalysisStatus,
    pub transaction_status: TransactionStatus,
    /// One-sentence summary of the outcome.
    pub summary: String,
    pub transaction: Option<TransactionOverview>,
    /// Issues ordered from most to least significant.
    pub issues: Vec<DiagnosticIssue>,
    pub limitations: Vec<Limitation>,
    pub metadata: ReportMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_serialize_as_snake_case() {
        let json = serde_json::to_string(&ErrorCategory::MissingContractReference).expect("ser");
        assert_eq!(json, "\"missing_contract_reference\"");
        let json = serde_json::to_string(&AnalysisStatus::TransactionNotFound).expect("ser");
        assert_eq!(json, "\"transaction_not_found\"");
        let json = serde_json::to_string(&EvidenceLevel::Insufficient).expect("ser");
        assert_eq!(json, "\"insufficient\"");
    }

    #[test]
    fn evidence_levels_sort_strongest_first() {
        let mut levels = vec![
            EvidenceLevel::Insufficient,
            EvidenceLevel::Confirmed,
            EvidenceLevel::Possible,
            EvidenceLevel::Likely,
        ];
        levels.sort();
        assert_eq!(
            levels,
            vec![
                EvidenceLevel::Confirmed,
                EvidenceLevel::Likely,
                EvidenceLevel::Possible,
                EvidenceLevel::Insufficient
            ]
        );
    }
}
