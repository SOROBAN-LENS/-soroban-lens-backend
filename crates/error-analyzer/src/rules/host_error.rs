//! Host errors reported through Soroban diagnostic events.
//!
//! When a Soroban invocation fails, the host emits diagnostic events whose
//! topics are `["error", Error(type, code)]`. The first distinct error is
//! normally the root cause; later events typically re-report the same error as
//! it propagates up the call stack.
//!
//! When the operation result code is already specific (entry archived,
//! insufficient refundable fee, resource limit exceeded), the host error is
//! supporting evidence for that result rather than an independent problem, so
//! this rule stays silent and [`super::OperationResultRule`] attaches the
//! host error details to its own issue instead.
//!
//! Some well-known host debug messages are used to sharpen the evidence level.
//! They are produced by `soroban-env-host` and matched loosely (substring), so
//! a wording change degrades to the generic explanation rather than a wrong one.

use soroban_lens_models::{DiagnosticIssue, ErrorCategory, EvidenceLevel};

use super::priority;
use crate::issue::{IssueBuilder, docs};
use crate::{
    AnalysisInput, Finding, HostError, HostErrorCode, HostErrorEvent, HostErrorType,
    InvokeResultCode, Rule,
};

/// Explains the root host error found in diagnostic events.
pub struct HostErrorRule;

impl Rule for HostErrorRule {
    fn name(&self) -> &'static str {
        "host_error"
    }

    fn evaluate(&self, input: &AnalysisInput) -> Vec<Finding> {
        if !host_error_is_primary(input) {
            return Vec::new();
        }
        let mut events = input.host_errors();
        let Some(root) = events.next() else {
            return Vec::new();
        };

        let mut others: Vec<String> = Vec::new();
        for event in events {
            let display = event.error.display();
            if event.error != root.error && !others.contains(&display) {
                others.push(display);
            }
        }

        let mut issue = explain(root);
        if !others.is_empty() {
            issue
                .details
                .insert("additional_errors".into(), others.join(", "));
        }
        vec![Finding {
            priority: priority::HOST_ERROR,
            issue,
        }]
    }
}

/// True unless a more specific `InvokeHostFunction` result code explains the
/// failure on its own.
pub(crate) fn host_error_is_primary(input: &AnalysisInput) -> bool {
    matches!(
        input.invoke_result(),
        None | Some(InvokeResultCode::Trapped)
    )
}

fn message_contains(event: &HostErrorEvent, needle: &str) -> bool {
    event
        .message
        .as_deref()
        .is_some_and(|m| m.to_ascii_lowercase().contains(needle))
}

fn explain(event: &HostErrorEvent) -> DiagnosticIssue {
    if message_contains(event, "archived") {
        return archived_entry(event);
    }

    use ErrorCategory as C;
    use HostErrorCode as Code;
    use HostErrorType as T;

    let builder = match event.error {
        HostError::Contract(code) => IssueBuilder::new(
            "host.contract_error",
            C::ContractExecution,
            format!("Contract returned error code {code}"),
        )
        .explanation(format!(
            "The contract deliberately returned its own error code {code}. Contract errors are defined \
             by the contract author (usually a #[contracterror] enum) and signal a business-rule failure, \
             such as an invalid amount or an unmet precondition."
        ))
        .technical(format!(
            "Diagnostic event reported Error(Contract, #{code}): a contract-defined error raised via a \
             Result::Err return or panic_with_error!."
        ))
        .cause(format!(
            "A condition checked by the contract failed; the meaning of #{code} is defined in the contract's source or spec."
        ))
        .fix(
            "Look up the error variant",
            format!("Find the variant with discriminant {code} in the contract's #[contracterror] enum or its interface spec."),
            Some(docs::CONTRACT_ERRORS),
        )
        .evidence(EvidenceLevel::Confirmed, source(event))
        .detail("contract_error_code", code),

        HostError::Host { error_type: T::Auth, code } => IssueBuilder::new(
            "host.auth",
            C::Authorization,
            "Soroban authorization failed",
        )
        .explanation(
            "A contract required authorization from an address (require_auth), and the transaction \
             did not carry a valid authorization for that call.",
        )
        .technical(format!("Diagnostic event reported Error(Auth, {code:?})."))
        .causes(match code {
            Code::ExistingValue => vec![
                "The authorization nonce was already used (a replayed or duplicated authorization entry).",
            ],
            _ => vec![
                "The authorization entry for the required address is missing.",
                "The authorization was signed for different arguments, a different contract, or has expired.",
                "The wrong account signed the authorization entry.",
            ],
        })
        .fix(
            "Re-simulate to obtain authorization entries",
            "Simulate the invocation (record auth mode), have each required address sign its entry, then submit.",
            Some(docs::AUTHORIZATION),
        )
        .evidence(EvidenceLevel::Confirmed, source(event)),

        HostError::Host { error_type: T::Budget, code } => IssueBuilder::new(
            "host.budget",
            C::ResourceLimit,
            "Execution budget exceeded",
        )
        .explanation("The contract ran out of its CPU or memory budget during execution.")
        .technical(format!("Diagnostic event reported Error(Budget, {code:?})."))
        .causes([
            "The declared instruction limit was too low for the work performed.",
            "The contract performs unbounded loops or large data processing.",
        ])
        .fix(
            "Re-simulate and reduce work",
            "Use fresh simulation resources and avoid unbounded iteration in a single call.",
            Some(docs::FEES_AND_LIMITS),
        )
        .evidence(EvidenceLevel::Confirmed, source(event)),

        HostError::Host { error_type: T::Storage, code: Code::MissingValue } => IssueBuilder::new(
            "host.storage_missing_value",
            C::MissingContractReference,
            "Required ledger entry not found",
        )
        .explanation(
            "The invocation tried to use a ledger entry that does not exist. This most often means the \
             contract ID is wrong or the contract (or its Wasm code) is not deployed on this network.",
        )
        .technical("Diagnostic event reported Error(Storage, MissingValue).")
        .causes([
            "The contract ID does not refer to a deployed contract on this network.",
            "The contract's Wasm code entry is missing.",
            "A storage key the contract expects to exist was never written.",
        ])
        .fix(
            "Verify the contract ID and network",
            "Confirm the contract is deployed on the network you are querying, and that the ID is correct.",
            None,
        )
        .evidence(EvidenceLevel::Likely, source(event)),

        HostError::Host { error_type: T::Storage, code: Code::ExceededLimit } => IssueBuilder::new(
            "host.storage_exceeded_limit",
            C::ResourceLimit,
            if message_contains(event, "footprint") {
                "Ledger entry accessed outside the declared footprint"
            } else {
                "Storage access outside the footprint or limits"
            },
        )
        .explanation(
            "The contract accessed a ledger entry that was not declared in the transaction's footprint, \
             or exceeded a storage limit.",
        )
        .technical("Diagnostic event reported Error(Storage, ExceededLimit).")
        .causes([
            "The footprint came from a simulation run against different state or arguments.",
            "The transaction footprint was built manually and is incomplete.",
        ])
        .fix(
            "Re-simulate the exact invocation",
            "Assemble the footprint from simulateTransaction with the same arguments, just before submission.",
            Some(docs::SIMULATE_TRANSACTION),
        )
        .evidence(
            if message_contains(event, "footprint") {
                EvidenceLevel::Confirmed
            } else {
                EvidenceLevel::Likely
            },
            source(event),
        ),

        HostError::Host {
            error_type: T::Value | T::Object,
            code: code @ (Code::UnexpectedType | Code::UnexpectedSize | Code::InvalidInput),
        } => IssueBuilder::new(
            "host.invalid_value",
            C::InvalidArguments,
            "Invalid value or argument",
        )
        .explanation(
            "A value passed to or produced by the contract had the wrong type or size. This often \
             happens when invocation arguments do not match the contract function's signature.",
        )
        .technical(format!("Diagnostic event reported {}.", event.error.display()))
        .causes([
            "An argument's type does not match the function signature (e.g. i128 vs u64, Address vs String).",
            "A vector or bytes value had an unexpected length.",
            "The contract converted a stored value to the wrong type.",
        ])
        .fix(
            "Compare arguments with the contract spec",
            format!(
                "Check the argument types against the contract interface (e.g. `stellar contract info interface`). Code: {code:?}."
            ),
            None,
        )
        .evidence(EvidenceLevel::Likely, source(event)),

        HostError::Host { error_type: T::WasmVm, code: Code::InvalidAction } => IssueBuilder::new(
            "host.wasm_trap",
            C::ContractExecution,
            "Contract panicked (Wasm trap)",
        )
        .explanation(
            "The contract's WebAssembly code hit a trap. In Rust contracts this usually means a panic: \
             an unwrap on None/Err, an arithmetic overflow, an out-of-bounds index, or an explicit panic!.",
        )
        .technical("Diagnostic event reported Error(WasmVm, InvalidAction).")
        .causes([
            "An unwrap()/expect() failed inside the contract.",
            "Arithmetic overflowed with overflow checks enabled.",
            "The contract executed an explicit panic! or unreachable instruction.",
        ])
        .fix(
            "Reproduce in a unit test",
            "Call the function with the same arguments in a soroban-sdk test to get the panic message.",
            Some(docs::DEBUGGING),
        )
        .evidence(EvidenceLevel::Likely, source(event)),

        HostError::Host { error_type, code } => IssueBuilder::new(
            format!("host.{}", error_type.name()),
            match code {
                Code::ExceededLimit => C::ResourceLimit,
                Code::UnexpectedType | Code::UnexpectedSize | Code::InvalidInput => C::InvalidArguments,
                _ => C::ContractExecution,
            },
            format!("Host error: {}", event.error.display()),
        )
        .explanation(
            "The Soroban host reported an error while executing the contract. SorobanLens does not yet \
             have a specific explanation for this error type and code.",
        )
        .technical(format!("Diagnostic event reported {}.", event.error.display()))
        .cause("See the event message and the Soroban host error documentation.")
        .fix(
            "Inspect diagnostic events",
            "Re-run the invocation via simulateTransaction and inspect all diagnostic events in order.",
            Some(docs::DEBUGGING),
        )
        .evidence(EvidenceLevel::Possible, source(event)),
    };

    with_event_details(builder, event)
}

fn archived_entry(event: &HostErrorEvent) -> DiagnosticIssue {
    with_event_details(
        IssueBuilder::new(
            "host.archived_entry",
            ErrorCategory::ArchivedState,
            "Archived ledger entry accessed",
        )
        .explanation(
            "The contract tried to use a ledger entry that has been archived because its \
             time-to-live (TTL) expired. It must be restored before it can be used.",
        )
        .technical(format!(
            "Diagnostic event reported {} with a message referring to an archived entry.",
            event.error.display()
        ))
        .cause("A persistent entry or contract instance/code was not extended before its TTL expired.")
        .fix(
            "Restore the archived entries",
            "Simulate to identify the archived keys, then submit a RestoreFootprint operation (or `stellar contract restore`).",
            Some(docs::ARCHIVAL_GUIDE),
        )
        .evidence(EvidenceLevel::Confirmed, source(event)),
        event,
    )
}

/// Adds the host error, emitting contract, and message to an issue.
pub(crate) fn with_event_details(builder: IssueBuilder, event: &HostErrorEvent) -> DiagnosticIssue {
    let mut builder = builder.detail("host_error", event.error.display());
    if let Some(contract) = &event.contract_id {
        builder = builder.detail("emitting_contract_id", contract);
    }
    if let Some(message) = &event.message {
        builder = builder.detail("event_message", message);
    }
    builder.build()
}

fn source(event: &HostErrorEvent) -> String {
    format!("diagnostic_event[{}]", event.event_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OperationCode, Signal};
    use soroban_lens_models::{Network, TransactionStatus};

    fn input(errors: &[HostError]) -> AnalysisInput {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::Failed);
        input.diagnostic_events_available = true;
        for (i, e) in errors.iter().enumerate() {
            input.signals.push(Signal::HostError(HostErrorEvent {
                event_index: i,
                error: *e,
                contract_id: Some("CCONTRACT".into()),
                message: Some("failing with contract error".into()),
            }));
        }
        input
    }

    fn host(error_type: HostErrorType, code: HostErrorCode) -> HostError {
        HostError::Host { error_type, code }
    }

    #[test]
    fn contract_error_is_confirmed_contract_execution() {
        let findings = HostErrorRule.evaluate(&input(&[HostError::Contract(7)]));
        assert_eq!(findings.len(), 1);
        let issue = &findings[0].issue;
        assert_eq!(issue.rule_id, "host.contract_error");
        assert_eq!(issue.category, ErrorCategory::ContractExecution);
        assert_eq!(issue.evidence.level, EvidenceLevel::Confirmed);
        assert_eq!(issue.details["contract_error_code"], "7");
        assert_eq!(issue.details["host_error"], "Error(Contract, #7)");
        assert_eq!(issue.details["emitting_contract_id"], "CCONTRACT");
        assert_eq!(issue.evidence.sources, vec!["diagnostic_event[0]"]);
    }

    #[test]
    fn auth_error_is_authorization() {
        let issue = &HostErrorRule.evaluate(&input(&[host(
            HostErrorType::Auth,
            HostErrorCode::InvalidAction,
        )]))[0]
            .issue;
        assert_eq!(issue.category, ErrorCategory::Authorization);
        assert_eq!(issue.rule_id, "host.auth");
    }

    #[test]
    fn auth_existing_value_points_to_nonce_reuse() {
        let issue = &HostErrorRule.evaluate(&input(&[host(
            HostErrorType::Auth,
            HostErrorCode::ExistingValue,
        )]))[0]
            .issue;
        assert!(issue.possible_causes[0].contains("nonce"));
    }

    #[test]
    fn budget_error_is_resource_limit() {
        let issue = &HostErrorRule.evaluate(&input(&[host(
            HostErrorType::Budget,
            HostErrorCode::ExceededLimit,
        )]))[0]
            .issue;
        assert_eq!(issue.category, ErrorCategory::ResourceLimit);
    }

    #[test]
    fn storage_missing_value_is_likely_missing_reference() {
        let issue = &HostErrorRule.evaluate(&input(&[host(
            HostErrorType::Storage,
            HostErrorCode::MissingValue,
        )]))[0]
            .issue;
        assert_eq!(issue.category, ErrorCategory::MissingContractReference);
        assert_eq!(issue.evidence.level, EvidenceLevel::Likely);
    }

    #[test]
    fn value_type_error_is_invalid_arguments() {
        let issue = &HostErrorRule.evaluate(&input(&[host(
            HostErrorType::Value,
            HostErrorCode::UnexpectedType,
        )]))[0]
            .issue;
        assert_eq!(issue.category, ErrorCategory::InvalidArguments);
    }

    #[test]
    fn unknown_host_error_is_only_possible() {
        let issue = &HostErrorRule.evaluate(&input(&[host(
            HostErrorType::Events,
            HostErrorCode::InternalError,
        )]))[0]
            .issue;
        assert_eq!(issue.rule_id, "host.events");
        assert_eq!(issue.evidence.level, EvidenceLevel::Possible);
    }

    #[test]
    fn first_error_is_root_and_others_are_recorded_once() {
        let findings = HostErrorRule.evaluate(&input(&[
            HostError::Contract(3),
            HostError::Contract(3),
            host(HostErrorType::WasmVm, HostErrorCode::InvalidAction),
            host(HostErrorType::WasmVm, HostErrorCode::InvalidAction),
        ]));
        assert_eq!(findings.len(), 1);
        let issue = &findings[0].issue;
        assert_eq!(issue.rule_id, "host.contract_error");
        assert_eq!(
            issue.details["additional_errors"],
            "Error(WasmVm, InvalidAction)"
        );
    }

    #[test]
    fn footprint_message_confirms_storage_violation() {
        let mut input = input(&[]);
        input.signals.push(Signal::HostError(HostErrorEvent {
            event_index: 1,
            error: host(HostErrorType::Storage, HostErrorCode::ExceededLimit),
            contract_id: None,
            message: Some("trying to access contract data key outside of the footprint".into()),
        }));
        let issue = &HostErrorRule.evaluate(&input)[0].issue;
        assert_eq!(issue.evidence.level, EvidenceLevel::Confirmed);
        assert!(issue.title.contains("footprint"));
    }

    #[test]
    fn archived_message_overrides_value_error_classification() {
        // Real testnet behaviour: archived entries surface as Error(Value, InvalidInput).
        let mut input = input(&[]);
        input.signals.push(Signal::HostError(HostErrorEvent {
            event_index: 0,
            error: host(HostErrorType::Value, HostErrorCode::InvalidInput),
            contract_id: None,
            message: Some("trying to access an archived contract data entry".into()),
        }));
        let issue = &HostErrorRule.evaluate(&input)[0].issue;
        assert_eq!(issue.rule_id, "host.archived_entry");
        assert_eq!(issue.category, ErrorCategory::ArchivedState);
    }

    #[test]
    fn defers_to_specific_operation_result() {
        let mut input = input(&[host(HostErrorType::Budget, HostErrorCode::ExceededLimit)]);
        input.signals.push(Signal::OperationResult {
            index: 0,
            code: OperationCode::InvokeHostFunction(InvokeResultCode::InsufficientRefundableFee),
        });
        assert!(HostErrorRule.evaluate(&input).is_empty());

        input.signals.pop();
        input.signals.push(Signal::OperationResult {
            index: 0,
            code: OperationCode::InvokeHostFunction(InvokeResultCode::Trapped),
        });
        assert_eq!(HostErrorRule.evaluate(&input).len(), 1);
    }

    #[test]
    fn no_host_errors_no_findings() {
        assert!(HostErrorRule.evaluate(&input(&[])).is_empty());
    }
}
