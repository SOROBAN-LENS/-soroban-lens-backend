//! Transaction-level result codes (`TransactionResultCode`).

use soroban_lens_models::{ErrorCategory, EvidenceLevel};

use super::priority;
use crate::issue::{IssueBuilder, docs};
use crate::{AnalysisInput, Finding, Rule, Signal, TxResultCode};

/// Explains failing transaction-level result codes.
///
/// `tx_failed` / `tx_fee_bump_inner_failed` are not explained here: they only
/// mean "an operation failed", which [`super::OperationResultRule`] covers.
pub struct TransactionResultRule;

impl Rule for TransactionResultRule {
    fn name(&self) -> &'static str {
        "transaction_result"
    }

    fn evaluate(&self, input: &AnalysisInput) -> Vec<Finding> {
        input
            .signals
            .iter()
            .filter_map(|signal| match signal {
                Signal::TransactionResult { code, inner } => explain(*code, *inner),
                _ => None,
            })
            .map(|issue| Finding {
                priority: priority::TRANSACTION,
                issue,
            })
            .collect()
    }
}

fn explain(code: TxResultCode, inner: bool) -> Option<soroban_lens_models::DiagnosticIssue> {
    use ErrorCategory as C;
    use TxResultCode as T;

    let source = if inner {
        "inner_transaction_result"
    } else {
        "transaction_result"
    };
    let rule_id = format!("tx.{}", code.name().trim_start_matches("tx_"));

    let builder = match code {
        T::Success | T::Failed | T::FeeBumpInnerSuccess | T::FeeBumpInnerFailed => return None,

        T::BadSeq => IssueBuilder::new(rule_id, C::TransactionValidity, "Incorrect sequence number")
            .explanation(
                "Every Stellar transaction must use the source account's next sequence number. \
                 This transaction's sequence number did not match.",
            )
            .technical("txBAD_SEQ: the transaction sequence number does not equal the source account's current sequence number + 1.")
            .causes([
                "Another transaction from the same source account was submitted first.",
                "The transaction was built from stale account data.",
                "Several transactions were built concurrently from the same account.",
            ])
            .fix(
                "Rebuild with a fresh sequence number",
                "Reload the source account from the network immediately before building and signing, and serialize submissions per source account.",
                Some(docs::OPERATIONS_AND_TRANSACTIONS),
            ),

        T::BadAuth | T::BadAuthExtra => {
            let (title, technical) = if code == T::BadAuth {
                (
                    "Missing or invalid transaction signatures",
                    "txBAD_AUTH: too few valid signatures, or signatures made for a different network passphrase.",
                )
            } else {
                (
                    "Unused signatures attached to the transaction",
                    "txBAD_AUTH_EXTRA: the transaction carries signatures that were not needed by any signer.",
                )
            };
            IssueBuilder::new(rule_id, C::Authorization, title)
                .explanation(
                    "The transaction's envelope signatures did not satisfy the source account's signing requirements. \
                     This is classic account signature authorization, not Soroban contract authorization (require_auth).",
                )
                .technical(technical)
                .causes([
                    "The transaction was signed with the wrong key.",
                    "The transaction was signed for a different network (network passphrase mismatch).",
                    "The account's signer weights or thresholds require additional signatures.",
                ])
                .fix(
                    "Check signers and network passphrase",
                    "Verify the signing key, that the network passphrase matches the target network, and the account's thresholds.",
                    Some(docs::OPERATIONS_AND_TRANSACTIONS),
                )
        }

        T::InsufficientFee => IssueBuilder::new(rule_id, C::Fee, "Insufficient transaction fee")
            .explanation("The fee offered was lower than required for inclusion.")
            .technical("txINSUFFICIENT_FEE: the fee bid was below the minimum or surge-priced inclusion fee.")
            .causes([
                "Network surge pricing raised the inclusion fee.",
                "The Soroban resource fee was not included or was set too low.",
            ])
            .fix(
                "Raise the fee",
                "Use the resource fee returned by simulateTransaction and add a competitive inclusion fee.",
                Some(docs::FEES_AND_LIMITS),
            ),

        T::InsufficientBalance => IssueBuilder::new(rule_id, C::Fee, "Insufficient balance to pay the fee")
            .explanation("Paying this transaction's fee would take the source account below its minimum XLM reserve.")
            .technical("txINSUFFICIENT_BALANCE: fee would bring the account below the minimum balance.")
            .cause("The fee-paying account does not hold enough XLM above its reserve.")
            .fix("Fund the source account", "Add XLM to the fee-paying account.", Some(docs::FEES_AND_LIMITS)),

        T::NoAccount => IssueBuilder::new(rule_id, C::TransactionValidity, "Source account does not exist")
            .explanation("The transaction's source account was not found on this network.")
            .technical("txNO_ACCOUNT: the source account does not exist in the ledger.")
            .causes([
                "The account has not been created/funded on this network.",
                "The transaction targets a different network than intended.",
            ])
            .fix("Create or fund the account", "Fund the account (on testnet, via Friendbot) or check the network.", None),

        T::TooEarly | T::TooLate => {
            let early = code == T::TooEarly;
            IssueBuilder::new(
                rule_id,
                C::TransactionValidity,
                if early { "Transaction submitted before its time bounds" } else { "Transaction expired" },
            )
            .explanation(if early {
                "The ledger closed before the transaction's minimum time bound."
            } else {
                "The ledger closed after the transaction's maximum time bound, so it expired."
            })
            .technical(if early {
                "txTOO_EARLY: ledger close time is before the transaction's minTime."
            } else {
                "txTOO_LATE: ledger close time is after the transaction's maxTime."
            })
            .cause("The transaction's time bounds (preconditions) did not include the ledger close time.")
            .fix("Rebuild with valid time bounds", "Rebuild the transaction with a timeout that covers signing and submission.", Some(docs::OPERATIONS_AND_TRANSACTIONS))
        }

        T::BadMinSeqAgeOrGap => IssueBuilder::new(rule_id, C::TransactionValidity, "Sequence age or gap precondition not met")
            .explanation("The transaction's minimum sequence age or ledger gap precondition was not satisfied.")
            .technical("txBAD_MIN_SEQ_AGE_OR_GAP: minSeqAge or minSeqLedgerGap precondition failed.")
            .cause("The source account's sequence number changed too recently.")
            .fix("Review preconditions", "Wait for the precondition to be met or remove it.", Some(docs::OPERATIONS_AND_TRANSACTIONS)),

        T::MissingOperation | T::Malformed | T::NotSupported => IssueBuilder::new(rule_id, C::TransactionValidity, "Malformed or unsupported transaction")
            .explanation("The transaction envelope is structurally invalid or uses an unsupported feature.")
            .technical(format!("{}: the transaction failed structural validation.", code.name()))
            .causes([
                "The transaction has no operations or invalid preconditions.",
                "The client SDK produced XDR for a different protocol version.",
            ])
            .fix("Rebuild with a current SDK", "Rebuild the transaction with an up-to-date Stellar SDK and validate preconditions.", None),

        T::SorobanInvalid => IssueBuilder::new(rule_id, C::TransactionValidity, "Soroban transaction preconditions not met")
            .explanation(
                "The transaction's Soroban-specific data was invalid, so it could not be applied. \
                 This is checked before any contract code runs.",
            )
            .technical("txSOROBAN_INVALID: Soroban-specific preconditions were not satisfied (for example, declared resources or footprint outside network limits).")
            .causes([
                "Declared resources (instructions, read/write bytes, footprint size) exceed network limits.",
                "The SorobanTransactionData was missing or inconsistent with the operation.",
                "A Soroban transaction contained more than one operation.",
            ])
            .fix(
                "Re-simulate and assemble",
                "Run simulateTransaction and assemble the transaction from its result so footprint and resources are consistent.",
                Some(docs::SIMULATE_TRANSACTION),
            ),

        T::BadSponsorship => IssueBuilder::new(rule_id, C::TransactionValidity, "Unconfirmed sponsorship")
            .explanation("A sponsorship started in this transaction was not ended correctly.")
            .technical("txBAD_SPONSORSHIP: BeginSponsoringFutureReserves without a matching EndSponsoringFutureReserves.")
            .cause("The sponsorship sandwich is incomplete.")
            .fix("Complete the sponsorship", "Ensure every BeginSponsoringFutureReserves has a matching EndSponsoringFutureReserves.", None),

        T::InternalError | T::FrozenKeyAccessed => IssueBuilder::new(rule_id, C::Unknown, format!("Transaction failed with {}", code.name()))
            .explanation("The network rejected this transaction with a result code SorobanLens does not yet explain in detail.")
            .technical(format!("Transaction result code: {}.", code.name()))
            .cause("See the Stellar protocol documentation for this result code.")
            .fix("Inspect the raw result", "Decode the resultXdr with the Stellar CLI or Stellar Lab.", Some(docs::GET_TRANSACTION)),
    };

    Some(
        builder
            .evidence(EvidenceLevel::Confirmed, source)
            .detail("result_code", code.name())
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_lens_models::{Network, TransactionStatus};

    fn input(code: TxResultCode) -> AnalysisInput {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::Failed);
        input
            .signals
            .push(Signal::TransactionResult { code, inner: false });
        input
    }

    #[test]
    fn bad_seq_is_confirmed_transaction_validity() {
        let findings = TransactionResultRule.evaluate(&input(TxResultCode::BadSeq));
        assert_eq!(findings.len(), 1);
        let issue = &findings[0].issue;
        assert_eq!(issue.rule_id, "tx.bad_seq");
        assert_eq!(issue.category, ErrorCategory::TransactionValidity);
        assert_eq!(issue.evidence.level, EvidenceLevel::Confirmed);
        assert_eq!(issue.evidence.sources, vec!["transaction_result"]);
        assert_eq!(issue.details["result_code"], "tx_bad_seq");
        assert!(!issue.suggested_fixes.is_empty());
    }

    #[test]
    fn bad_auth_is_authorization() {
        let issue = &TransactionResultRule.evaluate(&input(TxResultCode::BadAuth))[0].issue;
        assert_eq!(issue.category, ErrorCategory::Authorization);
        assert_eq!(issue.rule_id, "tx.bad_auth");
    }

    #[test]
    fn insufficient_fee_is_fee() {
        let issue = &TransactionResultRule.evaluate(&input(TxResultCode::InsufficientFee))[0].issue;
        assert_eq!(issue.category, ErrorCategory::Fee);
    }

    #[test]
    fn generic_failure_codes_are_left_to_operation_rules() {
        assert!(
            TransactionResultRule
                .evaluate(&input(TxResultCode::Failed))
                .is_empty()
        );
        assert!(
            TransactionResultRule
                .evaluate(&input(TxResultCode::Success))
                .is_empty()
        );
        assert!(
            TransactionResultRule
                .evaluate(&input(TxResultCode::FeeBumpInnerFailed))
                .is_empty()
        );
    }

    #[test]
    fn inner_fee_bump_result_is_attributed_correctly() {
        let mut input = AnalysisInput::new(Network::Testnet, TransactionStatus::Failed);
        input.signals.push(Signal::TransactionResult {
            code: TxResultCode::BadSeq,
            inner: true,
        });
        let issue = &TransactionResultRule.evaluate(&input)[0].issue;
        assert_eq!(issue.evidence.sources, vec!["inner_transaction_result"]);
    }
}
