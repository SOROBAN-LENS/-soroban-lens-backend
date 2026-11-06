# Diagnostic rules

This document lists every rule the engine ships with, the protocol data it
relies on, and how it behaves when that data is missing.

## Data sources

| Source | RPC field | Always present? | What it tells us |
|--------|-----------|-----------------|------------------|
| Transaction result | `resultXdr` | For `SUCCESS`/`FAILED` | Transaction and per-operation result codes, fee charged |
| Envelope | `envelopeXdr` | Usually | Operations, invoked contract and function, declared Soroban resources |
| Diagnostic events | `diagnosticEventsXdr`, or `TransactionMeta` V3/V4 | **No** — depends on the RPC operator | Host errors (`Error(type, code)`) with debug messages |

Observed on 2026-11-06 against `https://soroban-testnet.stellar.org`
(protocol 30): failed Soroban transactions **did** include
`diagnosticEventsXdr`; classic transactions did not (none are emitted for
them). Other providers may not include diagnostic events. When they are
missing for a failed Soroban transaction, the report carries the
`diagnostic_events_unavailable` limitation and the result-code rules still run.

## Rules

### `host_error` → `host.*`

Uses the **first** `error` diagnostic event (topics `["error", Error(..)]`) as
the root cause; later distinct errors are listed in `details.additional_errors`.
Runs only when the invocation result is `trapped` (or unknown). For
`entry_archived`, `insufficient_refundable_fee`, and `resource_limit_exceeded`,
the host error is attached to the operation issue as corroborating evidence
instead of becoming a separate issue (on real testnet data those events
describe the same failure in less specific terms).

| Rule ID | Trigger | Category | Evidence |
|---------|---------|----------|----------|
| `host.contract_error` | `Error(Contract, #n)` | contract_execution | confirmed |
| `host.auth` | `Error(Auth, *)` | authorization | confirmed |
| `host.budget` | `Error(Budget, *)` | resource_limit | confirmed |
| `host.archived_entry` | any error whose message mentions "archived" | archived_state | confirmed |
| `host.storage_missing_value` | `Error(Storage, MissingValue)` | missing_contract_reference | likely |
| `host.storage_exceeded_limit` | `Error(Storage, ExceededLimit)` | resource_limit | confirmed if message mentions "footprint", else likely |
| `host.invalid_value` | `Error(Value\|Object, UnexpectedType\|UnexpectedSize\|InvalidInput)` | invalid_arguments | likely |
| `host.wasm_trap` | `Error(WasmVm, InvalidAction)` | contract_execution | likely |
| `host.<type>` | any other host error | by code | possible |

Message matching is a case-insensitive substring check against messages
produced by `soroban-env-host`. If wording changes upstream, rules fall back to
the generic classification for the error type rather than a wrong one.

### `operation_result` → `invoke.*`, `extend_ttl.*`, `restore.*`, `op.*`

| Rule ID | Result code | Category |
|---------|-------------|----------|
| `invoke.trapped` | `INVOKE_HOST_FUNCTION_TRAPPED` | contract_execution |
| `invoke.resource_limit_exceeded` | `…_RESOURCE_LIMIT_EXCEEDED` | resource_limit |
| `invoke.entry_archived` | `…_ENTRY_ARCHIVED` | archived_state |
| `invoke.insufficient_refundable_fee` | `…_INSUFFICIENT_REFUNDABLE_FEE` | fee |
| `invoke.malformed` | `…_MALFORMED` | transaction_validity |
| `extend_ttl.*`, `restore.*` | ExtendFootprintTTL / RestoreFootprint failures | by code |
| `op.bad_auth`, `op.no_account`, … | generic operation codes | authorization / transaction_validity |
| `op.classic_failure` | any failing classic operation | unknown (not diagnosed in depth yet) |

All are `confirmed`: the result code is the network's own verdict.

### `transaction_result` → `tx.*`

Explains failing transaction-level codes: `tx.bad_seq`, `tx.bad_auth`,
`tx.bad_auth_extra`, `tx.insufficient_fee`, `tx.insufficient_balance`,
`tx.no_account`, `tx.too_early`, `tx.too_late`, `tx.bad_min_seq_age_or_gap`,
`tx.malformed`, `tx.missing_operation`, `tx.not_supported`,
`tx.soroban_invalid`, `tx.bad_sponsorship`, `tx.internal_error`,
`tx.frozen_key_accessed`. `tx_failed` itself produces no issue (it only means
an operation failed). Fee-bump inner results are unwrapped and attributed to
`inner_transaction_result`.

### `not_found` → `rpc.transaction_not_found`

Severity `info`, evidence `insufficient`. Explains the retention window and
other reasons a hash might not resolve.

### Fallback → `analysis.unexplained_failure`

Added when a transaction failed but no rule produced an error-severity issue
(e.g. the result XDR could not be decoded). Evidence `insufficient`.

## Adding a rule

1. Implement `soroban_lens_error_analyzer::Rule` in
   `crates/error-analyzer/src/rules/`.
2. Use the `IssueBuilder`; pick the narrowest category and an honest evidence
   level. Only link documentation URLs you have verified resolve.
3. Register it in `rules::default_rules()`.
4. Add unit tests, and a fixture-based test in
   `crates/diagnostic-engine/tests/` when real data is available.
5. Document it in this file.
