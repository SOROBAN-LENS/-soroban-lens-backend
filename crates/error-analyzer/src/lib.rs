//! Rule-based error analyzer for SorobanLens.
//!
//! The analyzer turns [`Signal`]s — protocol facts such as result codes and
//! host error events — into [`DiagnosticIssue`]s with a category, plain and
//! technical explanations, possible causes, suggested fixes, and an explicit
//! evidence level.
//!
//! # Design
//!
//! * Each [`Rule`] inspects an [`AnalysisInput`] and emits zero or more
//!   [`Finding`]s. Rules are independent and run in registration order.
//! * The analyzer orders findings (severity → rule specificity → evidence),
//!   removes duplicates, and records [`Limitation`]s when the data is not
//!   sufficient to identify a root cause.
//! * Nothing here performs I/O or depends on XDR decoding, which keeps rules
//!   trivially unit-testable and allows future analyzers (e.g. simulation-based
//!   or ML-assisted) to be added behind the same [`Rule`] trait.

mod issue;
pub mod rules;
mod signal;

pub use signal::{
    AnalysisInput, FootprintResultCode, HostError, HostErrorCode, HostErrorEvent, HostErrorType,
    Invocation, InvokeResultCode, OperationCode, Signal, TxResultCode,
};

use soroban_lens_models::{
    DiagnosticIssue, ErrorCategory, EvidenceLevel, Limitation, Severity, TransactionStatus,
};

use crate::issue::IssueBuilder;

/// A diagnostic issue plus its ordering priority (lower = more specific).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub priority: u8,
    pub issue: DiagnosticIssue,
}

/// A diagnostic rule.
pub trait Rule: Send + Sync {
    /// Stable name for logging and debugging.
    fn name(&self) -> &'static str;
    fn evaluate(&self, input: &AnalysisInput) -> Vec<Finding>;
}

/// Output of an analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Analysis {
    pub issues: Vec<DiagnosticIssue>,
    pub limitations: Vec<Limitation>,
}

/// Runs a set of rules over an [`AnalysisInput`].
pub struct ErrorAnalyzer {
    rules: Vec<Box<dyn Rule>>,
}

impl Default for ErrorAnalyzer {
    fn default() -> Self {
        Self::with_rules(rules::default_rules())
    }
}

impl ErrorAnalyzer {
    pub fn with_rules(rules: Vec<Box<dyn Rule>>) -> Self {
        Self { rules }
    }

    pub fn rule_names(&self) -> Vec<&'static str> {
        self.rules.iter().map(|r| r.name()).collect()
    }

    pub fn analyze(&self, input: &AnalysisInput) -> Analysis {
        let mut findings: Vec<Finding> = self
            .rules
            .iter()
            .flat_map(|rule| rule.evaluate(input))
            .collect();

        // Most severe first; among equals, the most specific rule first, then
        // the best-supported evidence.
        findings.sort_by(|a, b| {
            (a.issue.severity, a.priority, a.issue.evidence.level).cmp(&(
                b.issue.severity,
                b.priority,
                b.issue.evidence.level,
            ))
        });
        findings.dedup_by(|a, b| {
            a.issue.rule_id == b.issue.rule_id && a.issue.details == b.issue.details
        });

        let mut issues: Vec<DiagnosticIssue> = findings.into_iter().map(|f| f.issue).collect();
        let mut limitations = Vec::new();

        if input.transaction_status == TransactionStatus::Failed {
            if input.soroban && !input.diagnostic_events_available {
                limitations.push(Limitation {
                    code: "diagnostic_events_unavailable".into(),
                    message: "The RPC node did not return diagnostic events for this transaction, \
                              so host-level error details (contract error codes, auth failures, \
                              budget exhaustion) cannot be inspected. Result codes are still \
                              analyzed. Diagnostic events are only available from RPC nodes that \
                              enable them, or by re-running the invocation with simulateTransaction."
                        .into(),
                });
            }

            let has_failure_issue = issues.iter().any(|i| i.severity == Severity::Error);
            if !has_failure_issue {
                issues.push(unexplained_failure());
                limitations.push(Limitation {
                    code: "no_matching_rule".into(),
                    message: "The transaction failed, but none of the available result codes or \
                              events matched a known diagnostic rule."
                        .into(),
                });
            }
        }

        Analysis {
            issues,
            limitations,
        }
    }
}

fn unexplained_failure() -> DiagnosticIssue {
    IssueBuilder::new(
        "analysis.unexplained_failure",
        ErrorCategory::Unknown,
        "Failure could not be explained from the available data",
    )
    .explanation(
        "The network reports that this transaction failed, but SorobanLens could not find a \
         recognizable failure code or error event to explain why.",
    )
    .technical(
        "getTransaction returned status FAILED, but no failing transaction or operation result \
         code and no host error diagnostic event could be extracted.",
    )
    .cause("The result XDR was missing or could not be decoded.")
    .cause("The failure is of a kind SorobanLens does not have a rule for yet.")
    .fix(
        "Inspect the raw transaction",
        "Decode the transaction's resultXdr and resultMetaXdr with the Stellar CLI \
         (`stellar xdr decode`) or Stellar Lab to see the raw result codes.",
        Some(issue::docs::GET_TRANSACTION),
    )
    .evidence(EvidenceLevel::Insufficient, "transaction_status")
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_lens_models::Network;

    #[test]
    fn failed_transaction_without_signals_is_reported_as_unexplained() {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::Failed);
        input.soroban = true;
        let analysis = ErrorAnalyzer::default().analyze(&input);
        assert_eq!(analysis.issues.len(), 1);
        assert_eq!(analysis.issues[0].rule_id, "analysis.unexplained_failure");
        assert_eq!(
            analysis.issues[0].evidence.level,
            EvidenceLevel::Insufficient
        );
        let codes: Vec<_> = analysis
            .limitations
            .iter()
            .map(|l| l.code.as_str())
            .collect();
        assert_eq!(
            codes,
            vec!["diagnostic_events_unavailable", "no_matching_rule"]
        );
    }

    #[test]
    fn classic_failures_do_not_report_missing_diagnostic_events() {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::Failed);
        input.signals.push(Signal::OperationResult {
            index: 0,
            code: OperationCode::Classic {
                operation: "payment".into(),
                result: "payment_no_destination".into(),
                success: false,
            },
        });
        let analysis = ErrorAnalyzer::default().analyze(&input);
        assert_eq!(analysis.issues[0].rule_id, "op.classic_failure");
        assert!(analysis.issues[0].title.contains("payment_no_destination"));
        assert!(analysis.limitations.is_empty());
    }

    #[test]
    fn successful_transaction_produces_no_issues() {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::Success);
        input.signals.push(Signal::TransactionResult {
            code: TxResultCode::Success,
            inner: false,
        });
        input.signals.push(Signal::OperationResult {
            index: 0,
            code: OperationCode::InvokeHostFunction(InvokeResultCode::Success),
        });
        let analysis = ErrorAnalyzer::default().analyze(&input);
        assert!(analysis.issues.is_empty());
        assert!(analysis.limitations.is_empty());
    }

    #[test]
    fn issues_are_ordered_most_specific_first() {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::Failed);
        input.diagnostic_events_available = true;
        input.signals = vec![
            Signal::TransactionResult {
                code: TxResultCode::Failed,
                inner: false,
            },
            Signal::OperationResult {
                index: 0,
                code: OperationCode::InvokeHostFunction(InvokeResultCode::Trapped),
            },
            Signal::HostError(HostErrorEvent {
                event_index: 2,
                error: HostError::Contract(3),
                contract_id: None,
                message: None,
            }),
        ];
        let analysis = ErrorAnalyzer::default().analyze(&input);
        let ids: Vec<_> = analysis.issues.iter().map(|i| i.rule_id.as_str()).collect();
        assert_eq!(ids, vec!["host.contract_error", "invoke.trapped"]);
        assert!(analysis.limitations.is_empty());
    }
}
