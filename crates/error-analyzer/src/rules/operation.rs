//! Operation-level result codes, with emphasis on Soroban operations.

use soroban_lens_models::{ErrorCategory, EvidenceLevel, Severity};

use super::{host_error, priority};
use crate::issue::{IssueBuilder, docs};
use crate::{
    AnalysisInput, Finding, FootprintResultCode, InvokeResultCode, OperationCode, Rule, Signal,
};

/// Explains failing operation results.
pub struct OperationResultRule;

impl Rule for OperationResultRule {
    fn name(&self) -> &'static str {
        "operation_result"
    }

    fn evaluate(&self, input: &AnalysisInput) -> Vec<Finding> {
        input
            .signals
            .iter()
            .filter_map(|signal| match signal {
                Signal::OperationResult { index, code } if !code.is_success() => {
                    explain(input, *index, code)
                }
                _ => None,
            })
            .collect()
    }
}

fn explain(input: &AnalysisInput, index: usize, code: &OperationCode) -> Option<Finding> {
    let source = format!("operation_result[{index}]");
    let (priority, builder) = match code {
        OperationCode::InvokeHostFunction(c) => (priority::INVOKE, invoke(input, *c)?),
        OperationCode::ExtendFootprintTtl(c) => (priority::INVOKE, footprint("extend_ttl", "ExtendFootprintTTL", *c)?),
        OperationCode::RestoreFootprint(c) => (priority::INVOKE, footprint("restore", "RestoreFootprint", *c)?),
        OperationCode::BadAuth => (
            priority::OPERATION,
            IssueBuilder::new("op.bad_auth", ErrorCategory::Authorization, "Operation source account not authorized")
                .explanation("The operation's source account did not provide enough valid signatures.")
                .technical("opBAD_AUTH: too few valid signatures for the operation's source account, or wrong network.")
                .causes([
                    "The operation has a different source account whose key did not sign.",
                    "The signing key's weight is below the operation's threshold.",
                ])
                .fix("Sign with the operation source account", "Add a signature from the operation's source account.", Some(docs::OPERATIONS_AND_TRANSACTIONS)),
        ),
        OperationCode::NoAccount => (
            priority::OPERATION,
            IssueBuilder::new("op.no_account", ErrorCategory::TransactionValidity, "Operation source account does not exist")
                .explanation("The operation's source account was not found on this network.")
                .technical("opNO_ACCOUNT: the operation's source account does not exist.")
                .cause("The account has not been created on this network.")
                .fix("Create the account", "Create and fund the account, or correct the operation source.", None),
        ),
        OperationCode::NotSupported
        | OperationCode::TooManySubentries
        | OperationCode::ExceededWorkLimit
        | OperationCode::TooManySponsoring => {
            let name = code.name();
            (
                priority::OPERATION,
                IssueBuilder::new(format!("op.{}", name.trim_start_matches("op_")), ErrorCategory::TransactionValidity, format!("Operation failed with {name}"))
                    .explanation("The operation was rejected by a protocol-level check before or during execution.")
                    .technical(format!("Operation result code: {name}."))
                    .cause("See the Stellar documentation for this operation result code.")
                    .fix("Inspect the raw result", "Decode the resultXdr to inspect the operation result.", Some(docs::GET_TRANSACTION)),
            )
        }
        OperationCode::Classic { operation, result, .. } => (
            priority::OPERATION,
            IssueBuilder::new("op.classic_failure", ErrorCategory::Unknown, format!("Classic operation `{operation}` failed: {result}"))
                .explanation(
                    "A classic (non-Soroban) operation failed. SorobanLens currently focuses on Soroban \
                     failures and reports classic operation failures without detailed diagnosis.",
                )
                .technical(format!("Operation {operation} returned {result}."))
                .cause(format!("See the Stellar documentation for `{result}`."))
                .fix("Look up the result code", "Consult the operation's result code documentation.", Some(docs::OPERATIONS_AND_TRANSACTIONS))
                .detail("operation", operation),
        ),
    };

    let builder = builder
        .evidence(EvidenceLevel::Confirmed, source)
        .detail("operation_index", index)
        .detail("result_code", code.name());

    // For specific Soroban result codes the host error is corroborating
    // evidence; attach it here instead of reporting it as a separate issue.
    let corroborating = matches!(code, OperationCode::InvokeHostFunction(_))
        && !host_error::host_error_is_primary(input);
    let issue = match input.host_errors().next() {
        Some(event) if corroborating => host_error::with_event_details(
            builder.source(format!("diagnostic_event[{}]", event.event_index)),
            event,
        ),
        _ => builder.build(),
    };

    Some(Finding { priority, issue })
}

fn invoke(input: &AnalysisInput, code: InvokeResultCode) -> Option<IssueBuilder> {
    let host_error_known = input.host_errors().next().is_some();
    let mut b = match code {
        InvokeResultCode::Success => return None,

        InvokeResultCode::Trapped => {
            let b = IssueBuilder::new("invoke.trapped", ErrorCategory::ContractExecution, "Contract invocation trapped")
                .explanation(
                    "The contract call stopped with an error and all of its changes were rolled back. \
                     A trap is the generic outcome for any error during execution: a contract-defined \
                     error, a panic, a failed authorization check, a missing ledger entry, and so on.",
                )
                .technical("INVOKE_HOST_FUNCTION_TRAPPED: the host function invocation returned an error.");
            if host_error_known {
                b.cause("The specific host error is reported in a separate issue derived from diagnostic events.")
            } else {
                b.causes([
                    "The contract returned a contract-defined error (Error(Contract, #n)).",
                    "The contract panicked (e.g. unwrap on None, arithmetic overflow, explicit panic!).",
                    "A require_auth check failed because authorization entries were missing or invalid.",
                    "A ledger entry the contract read did not exist, or arguments had the wrong type.",
                ])
                .fix(
                    "Reproduce with simulateTransaction",
                    "Simulate the same invocation against current network state; simulation returns diagnostic events that identify the host error.",
                    Some(docs::SIMULATE_TRANSACTION),
                )
            }
            .fix(
                "Read the contract's error definitions",
                "If the error is Error(Contract, #n), look up variant n in the contract's #[contracterror] enum.",
                Some(docs::CONTRACT_ERRORS),
            )
            .fix(
                "Debug locally",
                "Reproduce the call in a soroban-sdk unit test with the same arguments to get a full error and backtrace.",
                Some(docs::DEBUGGING),
            )
        }

        InvokeResultCode::ResourceLimitExceeded => {
            IssueBuilder::new("invoke.resource_limit_exceeded", ErrorCategory::ResourceLimit, "Declared resource limits exceeded")
                .explanation(
                    "The contract call needed more resources than the transaction declared, so execution was stopped. \
                     Soroban transactions must declare CPU instructions, read/write bytes, and footprint up front.",
                )
                .technical("INVOKE_HOST_FUNCTION_RESOURCE_LIMIT_EXCEEDED: execution exceeded one of the declared SorobanResources limits or a network limit.")
                .causes([
                    "Ledger state changed between simulation and submission, increasing the work required.",
                    "The resources were set manually, or reused from an older simulation.",
                    "The invocation's real cost is close to or above network-wide per-transaction limits.",
                ])
                .fix(
                    "Re-simulate immediately before submitting",
                    "Use the resources from a fresh simulateTransaction result; consider adding a safety margin to the instruction count.",
                    Some(docs::SIMULATE_TRANSACTION),
                )
                .fix(
                    "Reduce the work done in one call",
                    "Paginate loops or split work across several transactions if the cost is near network limits.",
                    Some(docs::FEES_AND_LIMITS),
                )
        }

        InvokeResultCode::EntryArchived => IssueBuilder::new("invoke.entry_archived", ErrorCategory::ArchivedState, "Archived ledger entry in footprint")
            .explanation(
                "The contract call needed contract data, code, or an instance that has been archived because its \
                 time-to-live (TTL) expired. Archived entries must be restored before they can be used.",
            )
            .technical("INVOKE_HOST_FUNCTION_ENTRY_ARCHIVED: a persistent entry in the footprint is archived.")
            .causes([
                "A persistent storage entry or contract instance/code was not extended and its TTL expired.",
            ])
            .fix(
                "Restore the archived entries",
                "Simulate the invocation to learn which entries need restoring, then submit a RestoreFootprint operation (or `stellar contract restore`) before retrying.",
                Some(docs::ARCHIVAL_GUIDE),
            )
            .fix(
                "Extend TTLs proactively",
                "Have the contract or an off-chain job extend TTLs of long-lived entries.",
                Some(docs::STATE_ARCHIVAL),
            ),

        InvokeResultCode::InsufficientRefundableFee => IssueBuilder::new("invoke.insufficient_refundable_fee", ErrorCategory::Fee, "Refundable resource fee too low")
            .explanation(
                "The transaction did not reserve enough fee for refundable costs such as storage rent, events, and the return value.",
            )
            .technical("INVOKE_HOST_FUNCTION_INSUFFICIENT_REFUNDABLE_FEE: refundable resource fee was exhausted.")
            .causes([
                "The invocation wrote more data or emitted more events than when it was simulated.",
                "The resource fee was set manually below the simulated value.",
            ])
            .fix("Use the simulated resource fee", "Assemble the transaction from a fresh simulateTransaction result.", Some(docs::FEES_AND_LIMITS)),

        InvokeResultCode::Malformed => IssueBuilder::new("invoke.malformed", ErrorCategory::TransactionValidity, "Malformed contract invocation")
            .explanation("The InvokeHostFunction operation itself was invalid, so no contract code ran.")
            .technical("INVOKE_HOST_FUNCTION_MALFORMED: the operation failed validation.")
            .causes([
                "The host function or its arguments were encoded incorrectly.",
                "The footprint or authorization entries are inconsistent with the operation.",
            ])
            .fix("Rebuild via simulation", "Build the invocation with a current SDK and assemble it from simulateTransaction.", Some(docs::SIMULATE_TRANSACTION)),
    };

    if let Some(invocation) = &input.invocation {
        b = b
            .detail("contract_id", &invocation.contract_id)
            .detail("function_name", &invocation.function_name);
    }
    if code == InvokeResultCode::ResourceLimitExceeded
        && let Some(r) = &input.declared_resources
    {
        b = b
            .detail("declared_instructions", r.instructions)
            .detail("declared_disk_read_bytes", r.disk_read_bytes)
            .detail("declared_write_bytes", r.write_bytes);
    }
    Some(b)
}

fn footprint(prefix: &str, op_label: &str, code: FootprintResultCode) -> Option<IssueBuilder> {
    let (suffix, category, title, technical) = match code {
        FootprintResultCode::Success => return None,
        FootprintResultCode::Malformed => (
            "malformed",
            ErrorCategory::TransactionValidity,
            "Malformed footprint operation",
            "the operation failed validation (e.g. footprint contains non-Soroban or invalid keys).",
        ),
        FootprintResultCode::ResourceLimitExceeded => (
            "resource_limit_exceeded",
            ErrorCategory::ResourceLimit,
            "Footprint operation exceeded declared resources",
            "the declared read/write resources were insufficient.",
        ),
        FootprintResultCode::InsufficientRefundableFee => (
            "insufficient_refundable_fee",
            ErrorCategory::Fee,
            "Refundable fee too low for footprint operation",
            "the refundable resource fee did not cover rent.",
        ),
    };
    Some(
        IssueBuilder::new(format!("{prefix}.{suffix}"), category, title)
            .severity(Severity::Error)
            .explanation(format!("The {op_label} operation failed: {technical}"))
            .technical(format!("{op_label} result: {suffix}."))
            .cause("Resources or fees were not taken from a fresh simulation.")
            .fix(
                "Re-simulate",
                "Simulate the operation and assemble the transaction from the result.",
                Some(docs::ARCHIVAL_GUIDE),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HostError, HostErrorEvent, Invocation};
    use soroban_lens_models::{DeclaredResources, Network, TransactionStatus};

    fn input_with(code: OperationCode) -> AnalysisInput {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::Failed);
        input
            .signals
            .push(Signal::OperationResult { index: 0, code });
        input
    }

    #[test]
    fn trapped_without_events_suggests_simulation() {
        let findings = OperationResultRule.evaluate(&input_with(
            OperationCode::InvokeHostFunction(InvokeResultCode::Trapped),
        ));
        let issue = &findings[0].issue;
        assert_eq!(issue.rule_id, "invoke.trapped");
        assert_eq!(issue.category, ErrorCategory::ContractExecution);
        assert_eq!(issue.evidence.level, EvidenceLevel::Confirmed);
        assert_eq!(issue.evidence.sources, vec!["operation_result[0]"]);
        assert!(
            issue.possible_causes.len() >= 3,
            "lists several possible causes"
        );
        assert!(
            issue
                .suggested_fixes
                .iter()
                .any(|f| f.title.contains("simulateTransaction"))
        );
    }

    #[test]
    fn trapped_with_host_error_defers_to_host_issue() {
        let mut input = input_with(OperationCode::InvokeHostFunction(InvokeResultCode::Trapped));
        input.signals.push(Signal::HostError(HostErrorEvent {
            event_index: 0,
            error: HostError::Contract(1),
            contract_id: None,
            message: None,
        }));
        let issue = &OperationResultRule.evaluate(&input)[0].issue;
        assert_eq!(issue.possible_causes.len(), 1);
        assert!(issue.possible_causes[0].contains("separate issue"));
    }

    #[test]
    fn includes_invocation_details() {
        let mut input = input_with(OperationCode::InvokeHostFunction(InvokeResultCode::Trapped));
        input.invocation = Some(Invocation {
            operation_index: 0,
            contract_id: "CABC".into(),
            function_name: "transfer".into(),
        });
        let issue = &OperationResultRule.evaluate(&input)[0].issue;
        assert_eq!(issue.details["contract_id"], "CABC");
        assert_eq!(issue.details["function_name"], "transfer");
        assert_eq!(issue.details["result_code"], "invoke_host_function_trapped");
    }

    #[test]
    fn resource_limit_includes_declared_resources() {
        let mut input = input_with(OperationCode::InvokeHostFunction(
            InvokeResultCode::ResourceLimitExceeded,
        ));
        input.declared_resources = Some(DeclaredResources {
            instructions: 1000,
            disk_read_bytes: 200,
            write_bytes: 300,
            resource_fee: 5,
            footprint_read_only: 1,
            footprint_read_write: 1,
        });
        let issue = &OperationResultRule.evaluate(&input)[0].issue;
        assert_eq!(issue.category, ErrorCategory::ResourceLimit);
        assert_eq!(issue.details["declared_instructions"], "1000");
    }

    #[test]
    fn entry_archived_is_archived_state() {
        let issue = &OperationResultRule.evaluate(&input_with(OperationCode::InvokeHostFunction(
            InvokeResultCode::EntryArchived,
        )))[0]
            .issue;
        assert_eq!(issue.category, ErrorCategory::ArchivedState);
    }

    #[test]
    fn restore_footprint_failure_is_explained() {
        let issue = &OperationResultRule.evaluate(&input_with(OperationCode::RestoreFootprint(
            FootprintResultCode::InsufficientRefundableFee,
        )))[0]
            .issue;
        assert_eq!(issue.rule_id, "restore.insufficient_refundable_fee");
        assert_eq!(issue.category, ErrorCategory::Fee);
    }

    #[test]
    fn specific_result_absorbs_host_error_as_evidence() {
        let mut input = input_with(OperationCode::InvokeHostFunction(
            InvokeResultCode::InsufficientRefundableFee,
        ));
        input.signals.push(Signal::HostError(HostErrorEvent {
            event_index: 5,
            error: HostError::Host {
                error_type: crate::HostErrorType::Budget,
                code: crate::HostErrorCode::ExceededLimit,
            },
            contract_id: None,
            message: Some("refundable resource fee was not sufficient".into()),
        }));
        let issue = &OperationResultRule.evaluate(&input)[0].issue;
        assert_eq!(
            issue.evidence.sources,
            vec!["operation_result[0]", "diagnostic_event[5]"]
        );
        assert_eq!(issue.details["host_error"], "Error(Budget, ExceededLimit)");
    }

    #[test]
    fn successful_operations_are_ignored() {
        assert!(
            OperationResultRule
                .evaluate(&input_with(OperationCode::InvokeHostFunction(
                    InvokeResultCode::Success
                )))
                .is_empty()
        );
        assert!(
            OperationResultRule
                .evaluate(&input_with(OperationCode::Classic {
                    operation: "payment".into(),
                    result: "payment_success".into(),
                    success: true,
                }))
                .is_empty()
        );
    }
}
