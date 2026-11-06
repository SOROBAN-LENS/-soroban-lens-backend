//! SorobanLens diagnostic engine.
//!
//! The engine ties the pieces together:
//!
//! ```text
//! TransactionSource (Stellar RPC) ──► TransactionInformation
//!        │
//!        ▼
//! extract (XDR decoding) ──► Signals + TransactionOverview + Limitations
//!        │
//!        ▼
//! ErrorAnalyzer (rules) ──► DiagnosticIssues
//!        │
//!        ▼
//!   DiagnosticReport
//! ```
//!
//! [`DiagnosticEngine::analyze_information`] is pure and synchronous, which
//! makes it easy to test against recorded RPC fixtures.

mod extract;

use std::sync::Arc;

use chrono::Utc;
use soroban_lens_error_analyzer::{AnalysisInput, ErrorAnalyzer};
use soroban_lens_models::{
    AnalysisStatus, DiagnosticEventAvailability, DiagnosticIssue, DiagnosticReport, Limitation,
    Network, REPORT_SCHEMA_VERSION, ReportMetadata, Severity, TransactionHash,
    TransactionInformation, TransactionStatus,
};
use soroban_lens_stellar_client::{RpcError, TransactionSource};
use uuid::Uuid;

/// Version of the diagnostic engine, reported in every report.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version of the XDR definitions used for decoding.
pub const XDR_VERSION: &str = extract::STELLAR_XDR_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// The transaction could not be retrieved, so no analysis was performed.
    #[error(transparent)]
    Rpc(#[from] RpcError),
}

pub struct DiagnosticEngine {
    source: Arc<dyn TransactionSource>,
    analyzer: ErrorAnalyzer,
}

impl DiagnosticEngine {
    pub fn new(source: Arc<dyn TransactionSource>) -> Self {
        Self::with_analyzer(source, ErrorAnalyzer::default())
    }

    pub fn with_analyzer(source: Arc<dyn TransactionSource>, analyzer: ErrorAnalyzer) -> Self {
        Self { source, analyzer }
    }

    /// Networks the underlying transaction source can serve.
    pub fn networks(&self) -> Vec<Network> {
        self.source.networks()
    }

    pub fn rule_names(&self) -> Vec<&'static str> {
        self.analyzer.rule_names()
    }

    /// Fetches a transaction and analyzes it.
    pub async fn analyze(
        &self,
        network: Network,
        hash: &TransactionHash,
    ) -> Result<DiagnosticReport, EngineError> {
        let info = self.source.get_transaction(network, hash).await?;
        Ok(self.analyze_information(&info))
    }

    /// Analyzes already-retrieved transaction information.
    pub fn analyze_information(&self, info: &TransactionInformation) -> DiagnosticReport {
        let mut input = AnalysisInput::new(info.network, info.status);
        input.latest_ledger = Some(info.latest_ledger);
        input.oldest_ledger = Some(info.oldest_ledger);

        let mut limitations: Vec<Limitation> = Vec::new();
        let mut overview = None;
        let mut event_count = 0;

        if info.status != TransactionStatus::NotFound {
            let extracted = extract::extract(info);
            input.signals = extracted.signals;
            input.invocation = extracted.invocation;
            input.declared_resources = extracted.overview.declared_resources.clone();
            input.soroban = extracted.overview.declared_resources.is_some()
                || extracted.overview.operations.iter().any(|op| {
                    matches!(
                        op.operation_type.as_str(),
                        "invoke_host_function" | "extend_footprint_ttl" | "restore_footprint"
                    )
                });
            input.diagnostic_events_available = extracted.diagnostic_event_count > 0;
            event_count = extracted.diagnostic_event_count;
            limitations.extend(extracted.limitations);
            overview = Some(extracted.overview);
        }

        let analysis = self.analyzer.analyze(&input);
        limitations.extend(analysis.limitations);

        let analysis_status = match info.status {
            TransactionStatus::NotFound => AnalysisStatus::TransactionNotFound,
            _ if limitations.is_empty() => AnalysisStatus::Completed,
            _ => AnalysisStatus::Partial,
        };

        let summary = summarize(info, &analysis.issues, &limitations);

        DiagnosticReport {
            id: Uuid::new_v4(),
            transaction_hash: info.hash.clone(),
            network: info.network,
            analysis_status,
            transaction_status: info.status,
            summary,
            transaction: overview,
            issues: analysis.issues,
            limitations,
            metadata: ReportMetadata {
                schema_version: REPORT_SCHEMA_VERSION.to_owned(),
                engine_version: ENGINE_VERSION.to_owned(),
                analyzed_at: Utc::now(),
                latest_ledger: Some(info.latest_ledger),
                oldest_ledger: Some(info.oldest_ledger),
                diagnostic_events: DiagnosticEventAvailability {
                    available: event_count > 0,
                    count: event_count,
                },
            },
        }
    }
}

fn summarize(
    info: &TransactionInformation,
    issues: &[DiagnosticIssue],
    limitations: &[Limitation],
) -> String {
    match info.status {
        TransactionStatus::NotFound => format!(
            "Transaction not found on {} (RPC retention window: ledgers {}–{}). It may be on \
             another network, outside the retention window, or never included in a ledger.",
            info.network, info.oldest_ledger, info.latest_ledger
        ),
        TransactionStatus::Success => match info.ledger {
            Some(ledger) => {
                format!("Transaction succeeded in ledger {ledger}; no failure detected.")
            }
            None => "Transaction succeeded; no failure detected.".to_owned(),
        },
        TransactionStatus::Failed => {
            let primary = issues.iter().find(|i| i.severity == Severity::Error);
            let mut summary = match primary {
                Some(issue) => format!("Transaction failed: {}.", issue.title),
                None => "Transaction failed.".to_owned(),
            };
            let root_cause_unknown = primary.is_some_and(|i| i.rule_id == "invoke.trapped")
                && limitations
                    .iter()
                    .any(|l| l.code == "diagnostic_events_unavailable");
            if root_cause_unknown {
                summary.push_str(
                    " The specific host error could not be determined because diagnostic \
                     events were not available.",
                );
            }
            summary
        }
    }
}
