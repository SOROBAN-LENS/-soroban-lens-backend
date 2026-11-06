# SorobanLens API reference (v1)

Base URL (local): `http://localhost:8080`

All responses are JSON. All field names are `snake_case`; all enum values are
`snake_case` strings. The report schema version is reported in
`metadata.schema_version` (currently `1.0`). Breaking changes to any shape
documented here require a schema version bump and a note in this file.

Every response carries an `x-request-id` header (generated if the client did
not send one). Include it when reporting problems.

---

## `GET /health`

Liveness check. Does not contact Stellar RPC.

```json
{ "status": "ok", "service": "soroban-lens-api", "version": "0.1.0" }
```

## `GET /api/v1`

API metadata.

```json
{
  "name": "SorobanLens API",
  "version": "0.1.0",
  "api_version": "v1",
  "default_network": "testnet",
  "networks": ["testnet"],
  "engine": {
    "version": "0.1.0",
    "report_schema_version": "1.0",
    "stellar_xdr_version": "30.0.0",
    "rules": ["host_error", "operation_result", "transaction_result", "not_found"]
  },
  "persistence": "in_memory",
  "endpoints": [ { "method": "POST", "path": "/api/v1/analyze", "description": "…" } ]
}
```

`networks` lists only networks with a configured RPC endpoint. Clients should
use it to populate network selectors.

## `POST /api/v1/analyze`

Fetches a transaction from Stellar RPC and analyzes it. Rate-limited per
client IP (default 30/minute).

Request (`Content-Type: application/json`):

```json
{ "transaction_hash": "05235449a69c…5dd8", "network": "testnet" }
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `transaction_hash` | string | yes | 64 hex characters; trimmed and lowercased |
| `network` | `"mainnet"` \| `"testnet"` \| `"futurenet"` | no | Defaults to `default_network`. `"public"`/`"pubnet"` are accepted aliases for `mainnet` |

Unknown fields are rejected.

Response: **`201 Created`**, `Location: /api/v1/analyze/{id}`, body is a
[`DiagnosticReport`](#diagnosticreport).

A transaction that the RPC node cannot find is still a successful analysis
(`201`) with `analysis_status: "transaction_not_found"`. "Not found" is **not**
evidence that the transaction failed: RPC nodes retain only a recent window of
ledgers (about 7 days on the public testnet endpoint).

## `GET /api/v1/analyze/{id}`

Returns a previously created report (`200`). Reports are held in memory, are
capped (default 1000, oldest evicted) and are lost on restart.

## `GET /api/v1/stats`

Real counts of analyses served by **this server process** since it started,
plus the 10 most recent analyses. These are not network-wide statistics.

```json
{
  "since": "2026-11-06T14:00:00Z",
  "persistence": "in_memory",
  "total_analyses": 2,
  "successful_transactions": 0,
  "failed_transactions": 1,
  "not_found_transactions": 1,
  "recent": [
    {
      "id": "…", "transaction_hash": "…", "network": "testnet",
      "transaction_status": "not_found", "analysis_status": "transaction_not_found",
      "summary": "…", "issue_count": 1, "analyzed_at": "…"
    }
  ]
}
```

---

## Errors

```json
{ "error": { "code": "invalid_transaction_hash", "message": "transaction hash must be 64 hexadecimal characters, got 6 characters" } }
```

`code` is stable; `message` is human-readable and may change.

| HTTP | `code` | Meaning |
|------|--------|---------|
| 400 | `invalid_transaction_hash` | Hash is empty, wrong length, or not hex |
| 400 | `invalid_request` | Body has missing/unknown/wrongly-typed fields |
| 400 | `malformed_json` | Body is not valid JSON |
| 400 | `network_not_configured` | Requested network has no RPC endpoint on this server |
| 400 | `invalid_analysis_id` | ID is not a UUID |
| 404 | `analysis_not_found` | No stored report with that ID |
| 404 | `route_not_found` | Unknown endpoint |
| 408 | — | Request exceeded the server request timeout |
| 413 | `payload_too_large` | Body larger than `SOROBAN_LENS_MAX_BODY_BYTES` |
| 415 | `unsupported_media_type` | Missing `Content-Type: application/json` |
| 429 | `rate_limited` | Too many analyze requests; see `Retry-After` |
| 500 | `internal_error` | Unexpected server error (details are logged, not returned) |
| 502 | `rpc_unavailable` | Stellar RPC unreachable or returned an HTTP error |
| 502 | `rpc_error` | Stellar RPC returned a JSON-RPC error |
| 502 | `rpc_invalid_response` | Stellar RPC response could not be parsed |
| 504 | `rpc_timeout` | Stellar RPC did not respond in time |

RPC error responses never include the RPC URL or upstream error text.

---

## DiagnosticReport

```json
{
  "id": "e9bcc8ed-aa3a-4721-b473-4c63c773be69",
  "transaction_hash": "05235449a69c717377572f9f56e4efcedd8a8ef20d212c8e68d479a964505dd8",
  "network": "testnet",
  "analysis_status": "completed",
  "transaction_status": "failed",
  "summary": "Transaction failed: Ledger entry accessed outside the declared footprint.",
  "transaction": {
    "ledger": 5530122,
    "created_at": 1793674250,
    "application_order": 5,
    "fee_bump": false,
    "fee_charged": 5500,
    "result_code": "tx_failed",
    "operations": [
      {
        "index": 0,
        "operation_type": "invoke_host_function",
        "result_code": "invoke_host_function_trapped",
        "contract_id": "CD4KFB23CQLJ47RIPESVBTQ444R75M6QVEZULWXHHFYYZT7SS2VGXOPW",
        "function_name": "write_prices"
      }
    ],
    "declared_resources": {
      "instructions": 5929683, "disk_read_bytes": 0, "write_bytes": 0,
      "resource_fee": 10530, "footprint_read_only": 3, "footprint_read_write": 0
    }
  },
  "issues": [
    {
      "rule_id": "host.storage_exceeded_limit",
      "category": "resource_limit",
      "severity": "error",
      "title": "Ledger entry accessed outside the declared footprint",
      "explanation": "The contract accessed a ledger entry that was not declared…",
      "technical_explanation": "Diagnostic event reported Error(Storage, ExceededLimit).",
      "possible_causes": ["The footprint came from a simulation run against different state or arguments.", "…"],
      "suggested_fixes": [
        {
          "title": "Re-simulate the exact invocation",
          "description": "Assemble the footprint from simulateTransaction…",
          "reference_url": "https://developers.stellar.org/docs/data/apis/rpc/api-reference/methods/simulateTransaction"
        }
      ],
      "evidence": { "level": "confirmed", "sources": ["diagnostic_event[1]"] },
      "details": {
        "host_error": "Error(Storage, ExceededLimit)",
        "emitting_contract_id": "CD4KFB23…",
        "event_message": "trying to access contract data key outside of the footprint"
      }
    }
  ],
  "limitations": [],
  "metadata": {
    "schema_version": "1.0",
    "engine_version": "0.1.0",
    "analyzed_at": "2026-11-06T14:30:00Z",
    "latest_ledger": 5590113,
    "oldest_ledger": 5469154,
    "diagnostic_events": { "available": true, "count": 24 }
  }
}
```

(Excerpt of a real report for a real testnet transaction; long strings shortened.)

### Field reference

| Field | Notes |
|-------|-------|
| `analysis_status` | `completed` — all needed data was analyzed. `partial` — some data was missing or undecodable; see `limitations`. `transaction_not_found` |
| `transaction_status` | `success`, `failed`, `not_found` (as reported by RPC) |
| `transaction` | `null` when not found. Any field may be `null` if the corresponding XDR was unavailable |
| `issues` | Ordered: severity, then rule specificity, then evidence. `issues[0]` is the primary explanation |
| `issues[].rule_id` | Stable identifier, safe for programmatic matching (see [DIAGNOSTICS.md](DIAGNOSTICS.md)) |
| `issues[].details` | String map of extracted facts. Keys vary by rule; omitted when empty |
| `issues[].details.event_message` | Text from on-chain diagnostic events. **Untrusted** — render as plain text, never as HTML |
| `limitations` | What could not be determined and why (`code` + `message`) |
| `metadata.diagnostic_events` | Whether host diagnostic events were available for this transaction |

### Enumerations

**`category`** — `authorization`, `contract_execution`, `invalid_arguments`,
`resource_limit`, `missing_contract_reference`, `archived_state`,
`transaction_validity`, `fee`, `simulation` (reserved, not emitted yet),
`network_rpc`, `unknown`.

**`severity`** — `error`, `warning`, `info`.

**`evidence.level`** — how strongly the data supports the issue:

| Level | Meaning |
|-------|---------|
| `confirmed` | Stated directly by on-chain data (result codes, host error events) |
| `likely` | Strongly suggested by on-chain data; root cause inferred |
| `possible` | One of several plausible explanations; verify manually |
| `insufficient` | Not enough data to determine the cause |

`confirmed` applies to the *fact* (e.g. "the invocation trapped"). The
`possible_causes` list is always a set of candidates unless it has one entry.
