//! Shared data models for SorobanLens.
//!
//! Every type in this crate is part of the public JSON contract between the
//! backend and its clients (the SorobanLens frontend, CLI tools, and anyone
//! integrating with the REST API). Changes to field names, enum values, or
//! serialization shape are **breaking changes** and must be documented in
//! `docs/API.md`.
//!
//! All enums serialize as `snake_case` strings and all structs serialize with
//! `snake_case` field names.

mod hash;
mod network;
mod report;
mod transaction;

pub use hash::{InvalidTransactionHash, TransactionHash};
pub use network::{Network, UnknownNetwork};
pub use report::{
    AnalysisStatus, DeclaredResources, DiagnosticEventAvailability, DiagnosticIssue,
    DiagnosticReport, ErrorCategory, Evidence, EvidenceLevel, Limitation, OperationOverview,
    ReportMetadata, Severity, SuggestedFix, TransactionOverview,
};
pub use transaction::{TransactionInformation, TransactionRequest, TransactionStatus};

/// Version of the diagnostic report schema produced by this crate.
///
/// Bump the minor version for additive changes and the major version for
/// breaking changes to the serialized shape of [`DiagnosticReport`].
pub const REPORT_SCHEMA_VERSION: &str = "1.0";
