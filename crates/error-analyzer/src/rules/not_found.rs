//! Transactions the RPC node has no record of.

use soroban_lens_models::{ErrorCategory, EvidenceLevel, Severity, TransactionStatus};

use super::priority;
use crate::issue::{IssueBuilder, docs};
use crate::{AnalysisInput, Finding, Rule};

/// Explains a `NOT_FOUND` response from `getTransaction`.
///
/// A "not found" answer is *not* evidence that the transaction failed: RPC
/// nodes only retain a limited window of recent ledgers.
pub struct NotFoundRule;

impl Rule for NotFoundRule {
    fn name(&self) -> &'static str {
        "not_found"
    }

    fn evaluate(&self, input: &AnalysisInput) -> Vec<Finding> {
        if input.transaction_status != TransactionStatus::NotFound {
            return Vec::new();
        }

        let window = match (input.oldest_ledger, input.latest_ledger) {
            (Some(oldest), Some(latest)) => format!(" (ledgers {oldest}–{latest})"),
            _ => String::new(),
        };

        let mut builder = IssueBuilder::new(
            "rpc.transaction_not_found",
            ErrorCategory::NetworkRpc,
            "Transaction not found",
        )
        .severity(Severity::Info)
        .explanation(format!(
            "The {} RPC node has no record of this transaction in its retention window{window}. \
             This does not mean the transaction failed.",
            input.network
        ))
        .technical("getTransaction returned status NOT_FOUND.")
        .causes([
            "The transaction was submitted to a different network.",
            "The transaction is older than the RPC node's retention window.",
            "The transaction was never accepted into a ledger (e.g. rejected at submission or still pending).",
            "The hash was copied incorrectly.",
        ])
        .fix(
            "Check the network and hash",
            "Confirm the network the transaction was sent to and re-check the hash.",
            None,
        )
        .fix(
            "Use a history source for older transactions",
            "For transactions outside the RPC retention window, use an archive or indexer (e.g. Horizon or a data lake).",
            Some(docs::GET_TRANSACTION),
        )
        .evidence(EvidenceLevel::Insufficient, "rpc_status");

        if let (Some(oldest), Some(latest)) = (input.oldest_ledger, input.latest_ledger) {
            builder = builder
                .detail("oldest_ledger", oldest)
                .detail("latest_ledger", latest);
        }

        vec![Finding {
            priority: priority::RPC,
            issue: builder.build(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_lens_models::Network;

    #[test]
    fn explains_not_found_with_retention_window() {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::NotFound);
        input.oldest_ledger = Some(10);
        input.latest_ledger = Some(20);
        let findings = NotFoundRule.evaluate(&input);
        assert_eq!(findings.len(), 1);
        let issue = &findings[0].issue;
        assert_eq!(issue.severity, Severity::Info);
        assert_eq!(issue.evidence.level, EvidenceLevel::Insufficient);
        assert!(issue.explanation.contains("ledgers 10–20"));
        assert!(
            issue
                .explanation
                .contains("does not mean the transaction failed")
        );
    }

    #[test]
    fn ignores_found_transactions() {
        let input = AnalysisInput::new(Network::Testnet, TransactionStatus::Failed);
        assert!(NotFoundRule.evaluate(&input).is_empty());
    }
}
