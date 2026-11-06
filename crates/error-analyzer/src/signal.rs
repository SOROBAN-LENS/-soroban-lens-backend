//! Failure signals: protocol facts extracted from a transaction.
//!
//! Signals are deliberately independent of the XDR library so that rules can
//! be unit-tested with plain Rust values, and so that alternative extractors
//! (e.g. from `simulateTransaction` responses) can feed the same rules.

use soroban_lens_models::{DeclaredResources, Network, TransactionStatus};

/// Transaction-level result codes (`TransactionResultCode` in Stellar XDR).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxResultCode {
    Success,
    Failed,
    FeeBumpInnerSuccess,
    FeeBumpInnerFailed,
    TooEarly,
    TooLate,
    MissingOperation,
    BadSeq,
    BadAuth,
    InsufficientBalance,
    NoAccount,
    InsufficientFee,
    BadAuthExtra,
    InternalError,
    NotSupported,
    BadSponsorship,
    BadMinSeqAgeOrGap,
    Malformed,
    SorobanInvalid,
    FrozenKeyAccessed,
}

impl TxResultCode {
    /// Snake-case name derived from the XDR constant (e.g. `tx_bad_seq`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Success => "tx_success",
            Self::Failed => "tx_failed",
            Self::FeeBumpInnerSuccess => "tx_fee_bump_inner_success",
            Self::FeeBumpInnerFailed => "tx_fee_bump_inner_failed",
            Self::TooEarly => "tx_too_early",
            Self::TooLate => "tx_too_late",
            Self::MissingOperation => "tx_missing_operation",
            Self::BadSeq => "tx_bad_seq",
            Self::BadAuth => "tx_bad_auth",
            Self::InsufficientBalance => "tx_insufficient_balance",
            Self::NoAccount => "tx_no_account",
            Self::InsufficientFee => "tx_insufficient_fee",
            Self::BadAuthExtra => "tx_bad_auth_extra",
            Self::InternalError => "tx_internal_error",
            Self::NotSupported => "tx_not_supported",
            Self::BadSponsorship => "tx_bad_sponsorship",
            Self::BadMinSeqAgeOrGap => "tx_bad_min_seq_age_or_gap",
            Self::Malformed => "tx_malformed",
            Self::SorobanInvalid => "tx_soroban_invalid",
            Self::FrozenKeyAccessed => "tx_frozen_key_accessed",
        }
    }
}

/// Result codes of `InvokeHostFunction` operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvokeResultCode {
    Success,
    Malformed,
    Trapped,
    ResourceLimitExceeded,
    EntryArchived,
    InsufficientRefundableFee,
}

/// Result codes shared by `ExtendFootprintTtl` and `RestoreFootprint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FootprintResultCode {
    Success,
    Malformed,
    ResourceLimitExceeded,
    InsufficientRefundableFee,
}

/// Operation-level result.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OperationCode {
    BadAuth,
    NoAccount,
    NotSupported,
    TooManySubentries,
    ExceededWorkLimit,
    TooManySponsoring,
    InvokeHostFunction(InvokeResultCode),
    ExtendFootprintTtl(FootprintResultCode),
    RestoreFootprint(FootprintResultCode),
    /// A classic (non-Soroban) operation result. SorobanLens does not diagnose
    /// these in depth yet.
    Classic {
        operation: String,
        result: String,
        success: bool,
    },
}

impl OperationCode {
    pub fn name(&self) -> String {
        match self {
            Self::BadAuth => "op_bad_auth".into(),
            Self::NoAccount => "op_no_account".into(),
            Self::NotSupported => "op_not_supported".into(),
            Self::TooManySubentries => "op_too_many_subentries".into(),
            Self::ExceededWorkLimit => "op_exceeded_work_limit".into(),
            Self::TooManySponsoring => "op_too_many_sponsoring".into(),
            Self::InvokeHostFunction(c) => format!("invoke_host_function_{}", invoke_suffix(*c)),
            Self::ExtendFootprintTtl(c) => format!("extend_footprint_ttl_{}", footprint_suffix(*c)),
            Self::RestoreFootprint(c) => format!("restore_footprint_{}", footprint_suffix(*c)),
            Self::Classic { result, .. } => result.clone(),
        }
    }

    pub fn is_success(&self) -> bool {
        match self {
            Self::InvokeHostFunction(c) => *c == InvokeResultCode::Success,
            Self::ExtendFootprintTtl(c) | Self::RestoreFootprint(c) => {
                *c == FootprintResultCode::Success
            }
            Self::Classic { success, .. } => *success,
            _ => false,
        }
    }
}

fn invoke_suffix(c: InvokeResultCode) -> &'static str {
    match c {
        InvokeResultCode::Success => "success",
        InvokeResultCode::Malformed => "malformed",
        InvokeResultCode::Trapped => "trapped",
        InvokeResultCode::ResourceLimitExceeded => "resource_limit_exceeded",
        InvokeResultCode::EntryArchived => "entry_archived",
        InvokeResultCode::InsufficientRefundableFee => "insufficient_refundable_fee",
    }
}

fn footprint_suffix(c: FootprintResultCode) -> &'static str {
    match c {
        FootprintResultCode::Success => "success",
        FootprintResultCode::Malformed => "malformed",
        FootprintResultCode::ResourceLimitExceeded => "resource_limit_exceeded",
        FootprintResultCode::InsufficientRefundableFee => "insufficient_refundable_fee",
    }
}

/// Soroban host error types (`SCErrorType`), excluding contract errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostErrorType {
    WasmVm,
    Context,
    Storage,
    Object,
    Crypto,
    Events,
    Budget,
    Value,
    Auth,
}

impl HostErrorType {
    pub fn name(self) -> &'static str {
        match self {
            Self::WasmVm => "wasm_vm",
            Self::Context => "context",
            Self::Storage => "storage",
            Self::Object => "object",
            Self::Crypto => "crypto",
            Self::Events => "events",
            Self::Budget => "budget",
            Self::Value => "value",
            Self::Auth => "auth",
        }
    }
}

/// Soroban host error codes (`SCErrorCode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostErrorCode {
    ArithDomain,
    IndexBounds,
    InvalidInput,
    MissingValue,
    ExistingValue,
    ExceededLimit,
    InvalidAction,
    InternalError,
    UnexpectedType,
    UnexpectedSize,
}

impl HostErrorCode {
    pub fn name(self) -> &'static str {
        match self {
            Self::ArithDomain => "arith_domain",
            Self::IndexBounds => "index_bounds",
            Self::InvalidInput => "invalid_input",
            Self::MissingValue => "missing_value",
            Self::ExistingValue => "existing_value",
            Self::ExceededLimit => "exceeded_limit",
            Self::InvalidAction => "invalid_action",
            Self::InternalError => "internal_error",
            Self::UnexpectedType => "unexpected_type",
            Self::UnexpectedSize => "unexpected_size",
        }
    }
}

/// An `SCError` value reported by the Soroban host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostError {
    /// Contract-defined error code (`#[contracterror]` / `panic_with_error!`).
    Contract(u32),
    Host {
        error_type: HostErrorType,
        code: HostErrorCode,
    },
}

impl HostError {
    /// Display form matching the Soroban SDK's debug output, e.g.
    /// `Error(Contract, #3)` or `Error(Auth, InvalidAction)`.
    pub fn display(&self) -> String {
        match self {
            HostError::Contract(code) => format!("Error(Contract, #{code})"),
            HostError::Host { error_type, code } => {
                format!("Error({:?}, {:?})", error_type, code)
            }
        }
    }
}

/// An error reported through a Soroban diagnostic event
/// (topics `["error", Error(..)]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostErrorEvent {
    /// Index into the list of diagnostic events.
    pub event_index: usize,
    pub error: HostError,
    /// Strkey of the contract that emitted the event, if any.
    pub contract_id: Option<String>,
    /// Debug message attached by the host, if any (truncated).
    pub message: Option<String>,
}

/// A single extracted fact about a transaction's failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    TransactionResult {
        code: TxResultCode,
        /// True if this is the inner transaction of a fee-bump.
        inner: bool,
    },
    OperationResult {
        index: usize,
        code: OperationCode,
    },
    HostError(HostErrorEvent),
}

/// The contract call made by an `InvokeHostFunction` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub operation_index: usize,
    pub contract_id: String,
    pub function_name: String,
}

/// Everything the analyzer needs to know about a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisInput {
    pub network: Network,
    pub transaction_status: TransactionStatus,
    pub signals: Vec<Signal>,
    /// True if the transaction contains Soroban operations (InvokeHostFunction,
    /// ExtendFootprintTTL, RestoreFootprint).
    pub soroban: bool,
    /// Whether diagnostic events were available (from RPC or transaction meta).
    pub diagnostic_events_available: bool,
    pub invocation: Option<Invocation>,
    pub declared_resources: Option<DeclaredResources>,
    pub latest_ledger: Option<u32>,
    pub oldest_ledger: Option<u32>,
}

impl AnalysisInput {
    pub fn new(network: Network, transaction_status: TransactionStatus) -> Self {
        Self {
            network,
            transaction_status,
            signals: Vec::new(),
            soroban: false,
            diagnostic_events_available: false,
            invocation: None,
            declared_resources: None,
            latest_ledger: None,
            oldest_ledger: None,
        }
    }

    /// The first `InvokeHostFunction` result code, if any.
    pub fn invoke_result(&self) -> Option<InvokeResultCode> {
        self.signals.iter().find_map(|s| match s {
            Signal::OperationResult {
                code: OperationCode::InvokeHostFunction(c),
                ..
            } => Some(*c),
            _ => None,
        })
    }

    pub fn host_errors(&self) -> impl Iterator<Item = &HostErrorEvent> {
        self.signals.iter().filter_map(|s| match s {
            Signal::HostError(e) => Some(e),
            _ => None,
        })
    }
}
