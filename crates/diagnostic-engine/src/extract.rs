//! Decodes Stellar XDR from RPC responses into analyzer signals and report
//! overview data.
//!
//! All decoding uses explicit [`Limits`] because the XDR comes from an
//! external service and must be treated as untrusted input. Decoding failures
//! never abort the analysis; they are recorded as [`Limitation`]s.

use soroban_lens_error_analyzer::{
    FootprintResultCode, HostError, HostErrorCode, HostErrorEvent, HostErrorType, Invocation,
    InvokeResultCode, OperationCode, Signal, TxResultCode,
};
use soroban_lens_models::{
    DeclaredResources, Limitation, OperationOverview, TransactionInformation, TransactionOverview,
};
use stellar_xdr::{
    ContractEventBody, DiagnosticEvent, ExtendFootprintTtlResult, HostFunction,
    InnerTransactionResultResult, InvokeHostFunctionResult, Limits, Operation, OperationBody,
    OperationResult, OperationResultTr, ReadXdr, RestoreFootprintResult, ScError, ScErrorCode,
    ScVal, Transaction, TransactionEnvelope, TransactionExt, TransactionMeta, TransactionResult,
    TransactionResultResult,
};

/// Maximum nesting depth accepted when decoding XDR.
const XDR_DEPTH_LIMIT: u32 = 500;
/// Maximum decoded size of a single XDR value.
const XDR_LEN_LIMIT: usize = 16 * 1024 * 1024;
/// Maximum length of diagnostic event messages copied into reports.
const MAX_EVENT_MESSAGE_CHARS: usize = 512;

fn limits() -> Limits {
    Limits {
        depth: XDR_DEPTH_LIMIT,
        len: XDR_LEN_LIMIT,
    }
}

/// Everything extracted from a transaction's XDR.
#[derive(Debug)]
pub(crate) struct Extracted {
    pub signals: Vec<Signal>,
    pub overview: TransactionOverview,
    pub invocation: Option<Invocation>,
    pub diagnostic_event_count: usize,
    pub limitations: Vec<Limitation>,
}

pub(crate) fn extract(info: &TransactionInformation) -> Extracted {
    let mut out = Extracted {
        overview: TransactionOverview {
            ledger: info.ledger,
            created_at: info.created_at,
            application_order: info.application_order,
            fee_bump: info.fee_bump,
            fee_charged: None,
            result_code: None,
            operations: Vec::new(),
            declared_resources: None,
        },
        signals: Vec::new(),
        invocation: None,
        diagnostic_event_count: 0,
        limitations: Vec::new(),
    };

    if let Some(envelope) = &info.envelope_xdr {
        match TransactionEnvelope::from_xdr_base64(envelope, limits()) {
            Ok(env) => apply_envelope(&env, &mut out),
            Err(e) => out.limitations.push(decode_failed("envelope_xdr", &e)),
        }
    } else {
        out.limitations.push(missing("envelope_xdr"));
    }

    if let Some(result) = &info.result_xdr {
        match TransactionResult::from_xdr_base64(result, limits()) {
            Ok(res) => apply_result(&res, &mut out),
            Err(e) => out.limitations.push(decode_failed("result_xdr", &e)),
        }
    } else {
        out.limitations.push(missing("result_xdr"));
    }

    let events = collect_diagnostic_events(info, &mut out.limitations);
    out.diagnostic_event_count = events.len();
    for (index, event) in events.iter().enumerate() {
        if let Some(host_error) = host_error_from_event(index, event) {
            out.signals.push(Signal::HostError(host_error));
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

fn apply_envelope(env: &TransactionEnvelope, out: &mut Extracted) {
    let (operations, ext): (Vec<&Operation>, Option<&TransactionExt>) = match env {
        TransactionEnvelope::TxV0(v0) => (v0.tx.operations.iter().collect(), None),
        TransactionEnvelope::Tx(v1) => tx_parts(&v1.tx),
        TransactionEnvelope::TxFeeBump(fb) => match &fb.tx.inner_tx {
            stellar_xdr::FeeBumpTransactionInnerTx::Tx(inner) => tx_parts(&inner.tx),
        },
    };

    for (index, op) in operations.iter().enumerate() {
        let mut overview = OperationOverview {
            index,
            operation_type: snake_case(op.body.name()),
            result_code: None,
            contract_id: None,
            function_name: None,
        };
        if let OperationBody::InvokeHostFunction(invoke) = &op.body
            && let HostFunction::InvokeContract(args) = &invoke.host_function
        {
            let contract_id = args.contract_address.to_string();
            let function_name = args.function_name.0.to_utf8_string_lossy();
            overview.contract_id = Some(contract_id.clone());
            overview.function_name = Some(function_name.clone());
            if out.invocation.is_none() {
                out.invocation = Some(Invocation {
                    operation_index: index,
                    contract_id,
                    function_name,
                });
            }
        }
        out.overview.operations.push(overview);
    }

    if let Some(TransactionExt::V1(data)) = ext {
        let r = &data.resources;
        out.overview.declared_resources = Some(DeclaredResources {
            instructions: r.instructions,
            disk_read_bytes: r.disk_read_bytes,
            write_bytes: r.write_bytes,
            resource_fee: data.resource_fee,
            footprint_read_only: r.footprint.read_only.len(),
            footprint_read_write: r.footprint.read_write.len(),
        });
    }
}

fn tx_parts(tx: &Transaction) -> (Vec<&Operation>, Option<&TransactionExt>) {
    (tx.operations.iter().collect(), Some(&tx.ext))
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

fn apply_result(res: &TransactionResult, out: &mut Extracted) {
    out.overview.fee_charged = Some(res.fee_charged);

    let outer = tx_code(&res.result);
    out.overview.result_code = Some(outer.name().to_owned());
    out.signals.push(Signal::TransactionResult {
        code: outer,
        inner: false,
    });

    let op_results: Option<&[OperationResult]> = match &res.result {
        TransactionResultResult::TxSuccess(ops) | TransactionResultResult::TxFailed(ops) => {
            Some(ops)
        }
        TransactionResultResult::TxFeeBumpInnerSuccess(pair)
        | TransactionResultResult::TxFeeBumpInnerFailed(pair) => {
            let inner = inner_tx_code(&pair.result.result);
            out.signals.push(Signal::TransactionResult {
                code: inner,
                inner: true,
            });
            match &pair.result.result {
                InnerTransactionResultResult::TxSuccess(ops)
                | InnerTransactionResultResult::TxFailed(ops) => Some(ops),
                _ => None,
            }
        }
        _ => None,
    };

    for (index, op_result) in op_results.unwrap_or_default().iter().enumerate() {
        let code = operation_code(op_result);
        if let Some(overview) = out.overview.operations.get_mut(index) {
            overview.result_code = Some(code.name());
        }
        out.signals.push(Signal::OperationResult { index, code });
    }
}

fn tx_code(r: &TransactionResultResult) -> TxResultCode {
    use TransactionResultResult as R;
    use TxResultCode as T;
    match r {
        R::TxFeeBumpInnerSuccess(_) => T::FeeBumpInnerSuccess,
        R::TxFeeBumpInnerFailed(_) => T::FeeBumpInnerFailed,
        R::TxSuccess(_) => T::Success,
        R::TxFailed(_) => T::Failed,
        R::TxTooEarly => T::TooEarly,
        R::TxTooLate => T::TooLate,
        R::TxMissingOperation => T::MissingOperation,
        R::TxBadSeq => T::BadSeq,
        R::TxBadAuth => T::BadAuth,
        R::TxInsufficientBalance => T::InsufficientBalance,
        R::TxNoAccount => T::NoAccount,
        R::TxInsufficientFee => T::InsufficientFee,
        R::TxBadAuthExtra => T::BadAuthExtra,
        R::TxInternalError => T::InternalError,
        R::TxNotSupported => T::NotSupported,
        R::TxBadSponsorship => T::BadSponsorship,
        R::TxBadMinSeqAgeOrGap => T::BadMinSeqAgeOrGap,
        R::TxMalformed => T::Malformed,
        R::TxSorobanInvalid => T::SorobanInvalid,
        R::TxFrozenKeyAccessed => T::FrozenKeyAccessed,
    }
}

fn inner_tx_code(r: &InnerTransactionResultResult) -> TxResultCode {
    use InnerTransactionResultResult as R;
    use TxResultCode as T;
    match r {
        R::TxSuccess(_) => T::Success,
        R::TxFailed(_) => T::Failed,
        R::TxTooEarly => T::TooEarly,
        R::TxTooLate => T::TooLate,
        R::TxMissingOperation => T::MissingOperation,
        R::TxBadSeq => T::BadSeq,
        R::TxBadAuth => T::BadAuth,
        R::TxInsufficientBalance => T::InsufficientBalance,
        R::TxNoAccount => T::NoAccount,
        R::TxInsufficientFee => T::InsufficientFee,
        R::TxBadAuthExtra => T::BadAuthExtra,
        R::TxInternalError => T::InternalError,
        R::TxNotSupported => T::NotSupported,
        R::TxBadSponsorship => T::BadSponsorship,
        R::TxBadMinSeqAgeOrGap => T::BadMinSeqAgeOrGap,
        R::TxMalformed => T::Malformed,
        R::TxSorobanInvalid => T::SorobanInvalid,
        R::TxFrozenKeyAccessed => T::FrozenKeyAccessed,
    }
}

/// Expands to a match over classic operation results, yielding
/// `(operation_name, result_variant_name)`.
macro_rules! classic_result {
    ($tr:expr, $($variant:ident),+ $(,)?) => {
        match $tr {
            $(OperationResultTr::$variant(r) => (stringify!($variant), r.name()),)+
            other => (other.name(), "Unknown"),
        }
    };
}

fn operation_code(r: &OperationResult) -> OperationCode {
    let tr = match r {
        OperationResult::OpInner(tr) => tr,
        OperationResult::OpBadAuth => return OperationCode::BadAuth,
        OperationResult::OpNoAccount => return OperationCode::NoAccount,
        OperationResult::OpNotSupported => return OperationCode::NotSupported,
        OperationResult::OpTooManySubentries => return OperationCode::TooManySubentries,
        OperationResult::OpExceededWorkLimit => return OperationCode::ExceededWorkLimit,
        OperationResult::OpTooManySponsoring => return OperationCode::TooManySponsoring,
    };

    match tr {
        OperationResultTr::InvokeHostFunction(r) => OperationCode::InvokeHostFunction(match r {
            InvokeHostFunctionResult::Success(_) => InvokeResultCode::Success,
            InvokeHostFunctionResult::Malformed => InvokeResultCode::Malformed,
            InvokeHostFunctionResult::Trapped => InvokeResultCode::Trapped,
            InvokeHostFunctionResult::ResourceLimitExceeded => {
                InvokeResultCode::ResourceLimitExceeded
            }
            InvokeHostFunctionResult::EntryArchived => InvokeResultCode::EntryArchived,
            InvokeHostFunctionResult::InsufficientRefundableFee => {
                InvokeResultCode::InsufficientRefundableFee
            }
        }),
        OperationResultTr::ExtendFootprintTtl(r) => OperationCode::ExtendFootprintTtl(match r {
            ExtendFootprintTtlResult::Success => FootprintResultCode::Success,
            ExtendFootprintTtlResult::Malformed => FootprintResultCode::Malformed,
            ExtendFootprintTtlResult::ResourceLimitExceeded => {
                FootprintResultCode::ResourceLimitExceeded
            }
            ExtendFootprintTtlResult::InsufficientRefundableFee => {
                FootprintResultCode::InsufficientRefundableFee
            }
        }),
        OperationResultTr::RestoreFootprint(r) => OperationCode::RestoreFootprint(match r {
            RestoreFootprintResult::Success => FootprintResultCode::Success,
            RestoreFootprintResult::Malformed => FootprintResultCode::Malformed,
            RestoreFootprintResult::ResourceLimitExceeded => {
                FootprintResultCode::ResourceLimitExceeded
            }
            RestoreFootprintResult::InsufficientRefundableFee => {
                FootprintResultCode::InsufficientRefundableFee
            }
        }),
        classic => {
            let (operation, result) = classic_result!(
                classic,
                CreateAccount,
                Payment,
                PathPaymentStrictReceive,
                ManageSellOffer,
                CreatePassiveSellOffer,
                SetOptions,
                ChangeTrust,
                AllowTrust,
                AccountMerge,
                Inflation,
                ManageData,
                BumpSequence,
                ManageBuyOffer,
                PathPaymentStrictSend,
                CreateClaimableBalance,
                ClaimClaimableBalance,
                BeginSponsoringFutureReserves,
                EndSponsoringFutureReserves,
                RevokeSponsorship,
                Clawback,
                ClawbackClaimableBalance,
                SetTrustLineFlags,
                LiquidityPoolDeposit,
                LiquidityPoolWithdraw,
            );
            let operation = snake_case(operation);
            OperationCode::Classic {
                result: format!("{operation}_{}", snake_case(result)),
                success: result == "Success",
                operation,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Diagnostic events
// ---------------------------------------------------------------------------

/// Diagnostic events come from the RPC `diagnosticEventsXdr` field when the
/// node provides it; otherwise from `TransactionMeta` (V3 `sorobanMeta` or V4
/// `diagnosticEvents`), which is only populated if the node that produced the
/// meta had diagnostic events enabled.
fn collect_diagnostic_events(
    info: &TransactionInformation,
    limitations: &mut Vec<Limitation>,
) -> Vec<DiagnosticEvent> {
    if let Some(encoded) = info
        .diagnostic_events_xdr
        .as_ref()
        .filter(|v| !v.is_empty())
    {
        let mut events = Vec::with_capacity(encoded.len());
        let mut failures = 0usize;
        for e in encoded {
            match DiagnosticEvent::from_xdr_base64(e, limits()) {
                Ok(ev) => events.push(ev),
                Err(_) => failures += 1,
            }
        }
        if failures > 0 {
            limitations.push(Limitation {
                code: "diagnostic_event_decode_failed".into(),
                message: format!(
                    "{failures} diagnostic event(s) could not be decoded and were skipped."
                ),
            });
        }
        return events;
    }

    let Some(meta_b64) = &info.result_meta_xdr else {
        return Vec::new();
    };
    match TransactionMeta::from_xdr_base64(meta_b64, limits()) {
        Ok(TransactionMeta::V3(v3)) => v3
            .soroban_meta
            .map(|m| m.diagnostic_events.to_vec())
            .unwrap_or_default(),
        Ok(TransactionMeta::V4(v4)) => v4.diagnostic_events.to_vec(),
        Ok(_) => Vec::new(),
        Err(e) => {
            limitations.push(decode_failed("result_meta_xdr", &e));
            Vec::new()
        }
    }
}

/// Recognizes host error events: topics `[Symbol("error"), Error(..), ..]`.
fn host_error_from_event(index: usize, event: &DiagnosticEvent) -> Option<HostErrorEvent> {
    let ContractEventBody::V0(body) = &event.event.body;
    let mut topics = body.topics.iter();
    match topics.next() {
        Some(ScVal::Symbol(sym)) if sym.0.as_slice() == b"error" => {}
        _ => return None,
    }
    let error = match topics.next() {
        Some(ScVal::Error(e)) => convert_sc_error(e),
        _ => return None,
    };

    Some(HostErrorEvent {
        event_index: index,
        error,
        contract_id: event.event.contract_id.as_ref().map(|c| c.to_string()),
        message: event_message(&body.data),
    })
}

/// Host error events carry either a message string, or a vector of
/// `[message, arg0, arg1, …]` where the message contains `{}` placeholders.
fn event_message(data: &ScVal) -> Option<String> {
    let raw = match data {
        ScVal::String(s) => s.0.to_utf8_string_lossy(),
        ScVal::Symbol(s) => s.0.to_utf8_string_lossy(),
        ScVal::Vec(Some(items)) => match items.split_first() {
            Some((ScVal::String(s), args)) => format_message(&s.0.to_utf8_string_lossy(), args),
            _ => return None,
        },
        _ => return None,
    };
    Some(truncate(&raw, MAX_EVENT_MESSAGE_CHARS))
}

fn format_message(template: &str, args: &[ScVal]) -> String {
    let rendered: Vec<String> = args.iter().map(render_scval).collect();
    if template.matches("{}").count() == rendered.len() {
        let mut out = String::new();
        let mut parts = template.split("{}");
        if let Some(first) = parts.next() {
            out.push_str(first);
        }
        for (part, arg) in parts.zip(rendered.iter()) {
            out.push_str(arg);
            out.push_str(part);
        }
        out
    } else if rendered.is_empty() {
        template.to_owned()
    } else {
        format!("{template} [{}]", rendered.join(", "))
    }
}

/// Short display form for common scalar values; complex values are elided.
fn render_scval(v: &ScVal) -> String {
    match v {
        ScVal::Bool(b) => b.to_string(),
        ScVal::Void => "void".into(),
        ScVal::U32(n) => n.to_string(),
        ScVal::I32(n) => n.to_string(),
        ScVal::U64(n) => n.to_string(),
        ScVal::I64(n) => n.to_string(),
        ScVal::Symbol(s) => s.0.to_utf8_string_lossy(),
        ScVal::String(s) => format!("\"{}\"", s.0.to_utf8_string_lossy()),
        ScVal::Error(e) => convert_sc_error(e).display(),
        ScVal::Address(a) => a.to_string(),
        other => format!("<{}>", snake_case(other.name())),
    }
}

fn convert_sc_error(e: &ScError) -> HostError {
    let host = |error_type, code: &ScErrorCode| HostError::Host {
        error_type,
        code: convert_code(code),
    };
    match e {
        ScError::Contract(code) => HostError::Contract(*code),
        ScError::WasmVm(c) => host(HostErrorType::WasmVm, c),
        ScError::Context(c) => host(HostErrorType::Context, c),
        ScError::Storage(c) => host(HostErrorType::Storage, c),
        ScError::Object(c) => host(HostErrorType::Object, c),
        ScError::Crypto(c) => host(HostErrorType::Crypto, c),
        ScError::Events(c) => host(HostErrorType::Events, c),
        ScError::Budget(c) => host(HostErrorType::Budget, c),
        ScError::Value(c) => host(HostErrorType::Value, c),
        ScError::Auth(c) => host(HostErrorType::Auth, c),
    }
}

fn convert_code(c: &ScErrorCode) -> HostErrorCode {
    match c {
        ScErrorCode::ArithDomain => HostErrorCode::ArithDomain,
        ScErrorCode::IndexBounds => HostErrorCode::IndexBounds,
        ScErrorCode::InvalidInput => HostErrorCode::InvalidInput,
        ScErrorCode::MissingValue => HostErrorCode::MissingValue,
        ScErrorCode::ExistingValue => HostErrorCode::ExistingValue,
        ScErrorCode::ExceededLimit => HostErrorCode::ExceededLimit,
        ScErrorCode::InvalidAction => HostErrorCode::InvalidAction,
        ScErrorCode::InternalError => HostErrorCode::InternalError,
        ScErrorCode::UnexpectedType => HostErrorCode::UnexpectedType,
        ScErrorCode::UnexpectedSize => HostErrorCode::UnexpectedSize,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn missing(field: &str) -> Limitation {
    Limitation {
        code: format!("{field}_missing"),
        message: format!("The RPC response did not include `{field}`."),
    }
}

fn decode_failed(field: &str, err: &stellar_xdr::Error) -> Limitation {
    tracing::warn!(field, error = %err, "failed to decode XDR from RPC response");
    Limitation {
        code: format!("{field}_decode_failed"),
        message: format!(
            "`{field}` could not be decoded with the XDR definitions this backend was built with \
             (stellar-xdr {}). The network may be using a newer protocol version.",
            STELLAR_XDR_VERSION
        ),
    }
}

/// Version of the `stellar-xdr` crate, kept in sync with `Cargo.toml`.
pub(crate) const STELLAR_XDR_VERSION: &str = "30.0.0";

/// Converts `InvokeHostFunction` → `invoke_host_function`.
pub(crate) fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(max_chars).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_converts_variant_names() {
        assert_eq!(snake_case("InvokeHostFunction"), "invoke_host_function");
        assert_eq!(snake_case("Payment"), "payment");
        assert_eq!(snake_case("ExtendFootprintTtl"), "extend_footprint_ttl");
    }

    #[test]
    fn formats_host_message_placeholders() {
        let msg = format_message(
            "refundable resource fee was not sufficient: {} > {}",
            &[ScVal::I64(120), ScVal::I64(100)],
        );
        assert_eq!(msg, "refundable resource fee was not sufficient: 120 > 100");
        assert_eq!(format_message("no args", &[]), "no args");
        assert_eq!(format_message("mismatch", &[ScVal::U32(1)]), "mismatch [1]");
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("héllo", 10), "héllo");
        assert_eq!(truncate("héllo", 2), "hé…");
    }
}
