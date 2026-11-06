//! Small builder that keeps rule definitions readable.

use std::collections::BTreeMap;

use soroban_lens_models::{
    DiagnosticIssue, ErrorCategory, Evidence, EvidenceLevel, Severity, SuggestedFix,
};

/// Official documentation links. Every URL here was verified to resolve.
pub(crate) mod docs {
    pub const AUTHORIZATION: &str =
        "https://developers.stellar.org/docs/learn/fundamentals/contract-development/authorization";
    pub const DEBUGGING: &str = "https://developers.stellar.org/docs/learn/fundamentals/contract-development/errors-and-debugging/debugging-errors";
    pub const CONTRACT_ERRORS: &str =
        "https://developers.stellar.org/docs/build/smart-contracts/example-contracts/errors";
    pub const STATE_ARCHIVAL: &str = "https://developers.stellar.org/docs/learn/fundamentals/contract-development/storage/state-archival";
    pub const ARCHIVAL_GUIDE: &str = "https://developers.stellar.org/docs/build/guides/archival";
    pub const FEES_AND_LIMITS: &str =
        "https://developers.stellar.org/docs/learn/fundamentals/fees-resource-limits-metering";
    pub const OPERATIONS_AND_TRANSACTIONS: &str = "https://developers.stellar.org/docs/learn/fundamentals/transactions/operations-and-transactions";
    pub const SIMULATE_TRANSACTION: &str = "https://developers.stellar.org/docs/data/apis/rpc/api-reference/methods/simulateTransaction";
    pub const GET_TRANSACTION: &str =
        "https://developers.stellar.org/docs/data/apis/rpc/api-reference/methods/getTransaction";
}

pub(crate) struct IssueBuilder {
    issue: DiagnosticIssue,
}

impl IssueBuilder {
    pub fn new(
        rule_id: impl Into<String>,
        category: ErrorCategory,
        title: impl Into<String>,
    ) -> Self {
        Self {
            issue: DiagnosticIssue {
                rule_id: rule_id.into(),
                category,
                severity: Severity::Error,
                title: title.into(),
                explanation: String::new(),
                technical_explanation: String::new(),
                possible_causes: Vec::new(),
                suggested_fixes: Vec::new(),
                evidence: Evidence {
                    level: EvidenceLevel::Confirmed,
                    sources: Vec::new(),
                },
                details: BTreeMap::new(),
            },
        }
    }

    pub fn severity(mut self, severity: Severity) -> Self {
        self.issue.severity = severity;
        self
    }

    pub fn explanation(mut self, text: impl Into<String>) -> Self {
        self.issue.explanation = text.into();
        self
    }

    pub fn technical(mut self, text: impl Into<String>) -> Self {
        self.issue.technical_explanation = text.into();
        self
    }

    pub fn cause(mut self, text: impl Into<String>) -> Self {
        self.issue.possible_causes.push(text.into());
        self
    }

    pub fn causes<I, S>(mut self, items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.issue
            .possible_causes
            .extend(items.into_iter().map(Into::into));
        self
    }

    pub fn fix(mut self, title: &str, description: impl Into<String>, url: Option<&str>) -> Self {
        self.issue.suggested_fixes.push(SuggestedFix {
            title: title.to_owned(),
            description: description.into(),
            reference_url: url.map(str::to_owned),
        });
        self
    }

    pub fn evidence(mut self, level: EvidenceLevel, source: impl Into<String>) -> Self {
        self.issue.evidence.level = level;
        self.issue.evidence.sources.push(source.into());
        self
    }

    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.issue.evidence.sources.push(source.into());
        self
    }

    pub fn detail(mut self, key: &str, value: impl ToString) -> Self {
        self.issue.details.insert(key.to_owned(), value.to_string());
        self
    }

    pub fn build(self) -> DiagnosticIssue {
        self.issue
    }
}
