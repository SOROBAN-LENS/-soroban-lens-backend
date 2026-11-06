# SorobanLens Backend

> Making Soroban smart contract failures understandable.

The SorobanLens backend fetches a Stellar transaction by hash, decodes its
result and diagnostic data, and returns a structured **diagnostic report**: what
failed, how sure we are, likely causes, and concrete debugging steps.

**Status: early development (milestone 1 of ~3). Not production-ready.**

## Role in SorobanLens

SorobanLens is three repositories:

| Repository | Role |
|------------|------|
| `soroban-lens-frontend` | Developer dashboard (React) that calls this API |
| **`soroban-lens-backend`** | REST API, Stellar RPC client, diagnostic engine |
| `soroban-lens-contracts` | Example Soroban contracts that fail on purpose, used as test fixtures |

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how they fit together.

## Example

```sh
curl -s -X POST localhost:8080/api/v1/analyze \
  -H 'content-type: application/json' \
  -d '{"transaction_hash":"ae853c97eb8a90190a1754061bc20a1d680e04b9c6b607f8a7369d7268c0244d"}'
```

For this real testnet transaction, the report (abridged) says:

```json
{
  "analysis_status": "completed",
  "transaction_status": "failed",
  "summary": "Transaction failed: Archived ledger entry in footprint.",
  "issues": [{
    "rule_id": "invoke.entry_archived",
    "category": "archived_state",
    "evidence": { "level": "confirmed", "sources": ["operation_result[0]", "diagnostic_event[0]"] },
    "details": {
      "function_name": "update_batch_ed25519_persistent",
      "host_error": "Error(Value, InvalidInput)",
      "event_message": "trying to access an archived contract data entry"
    }
  }]
}
```

(Testnet RPC keeps about 7 days of history, so this hash will eventually stop
resolving live; it is preserved as a test fixture.)

## Technologies

* Rust 2024 edition (MSRV 1.89), Tokio, Axum 0.8, tower-http
* [`stellar-xdr`](https://crates.io/crates/stellar-xdr) 30.0.0 (official XDR definitions, protocol 30)
* Stellar RPC JSON-RPC (`getTransaction`) via reqwest (rustls)
* serde, thiserror, tracing
* Tests: wiremock (mock RPC), tower `oneshot` (HTTP)

## Architecture

```
api/                     Axum server: routes, config, errors, rate limiting, storage
crates/models/           Shared, serialization-stable types (the API contract)
crates/stellar-client/   Stellar RPC client + TransactionSource trait
crates/error-analyzer/   Rule-based mapping: signals → explained issues (no I/O)
crates/diagnostic-engine/ XDR decoding + orchestration → DiagnosticReport
docs/                    Architecture, API reference, diagnostic rules
```

Details: [ARCHITECTURE](docs/ARCHITECTURE.md) · [API](docs/API.md) · [Diagnostic rules](docs/DIAGNOSTICS.md)

## Getting started

Prerequisites: Rust ≥ 1.89 ([rustup](https://rustup.rs)). No database is
needed for this milestone.

```sh
git clone <repo-url> soroban-lens-backend
cd soroban-lens-backend
cp .env.example .env          # optional; defaults work for testnet
cargo run -p soroban-lens-api
```

The server listens on `http://127.0.0.1:8080` and uses the public SDF testnet
RPC by default.

```sh
curl localhost:8080/health
curl localhost:8080/api/v1
```

### Persistent storage (PostgreSQL)

Without `DATABASE_URL`, reports are kept in memory and lost on restart. To keep
them, point the server at PostgreSQL; migrations run automatically at startup:

```sh
docker run -d --name soroban-lens-pg -p 5432:5432 \
  -e POSTGRES_USER=soroban_lens -e POSTGRES_PASSWORD=soroban_lens postgres:17-alpine
DATABASE_URL=postgres://soroban_lens:soroban_lens@localhost:5432/soroban_lens cargo run -p soroban-lens-api
```

Analyze a transaction without running the server:

```sh
cargo run -p soroban-lens-diagnostic-engine --example analyze -- <tx-hash> [rpc-url]
```

## Testing

```sh
cargo test --workspace          # all tests (no network access needed)
# also run the PostgreSQL store tests (they are skipped without this variable):
TEST_DATABASE_URL=postgres://soroban_lens:soroban_lens@localhost:5432/soroban_lens cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

The suite includes tests against **real testnet transactions** replayed through
a mock RPC server (`crates/diagnostic-engine/tests/real_fixtures.rs`), synthetic
XDR tests for other failure codes, RPC client failure modes (timeouts, HTTP
errors, malformed and oversized responses), HTTP-level API tests, and a shared
store test suite run against both the in-memory and PostgreSQL stores
(`api/tests/store.rs`). CI runs it against a PostgreSQL service container.

## Configuration

All settings are environment variables; see [`.env.example`](.env.example).

| Variable | Default | Purpose |
|----------|---------|---------|
| `SOROBAN_LENS_HOST` / `SOROBAN_LENS_PORT` | `127.0.0.1` / `8080` | Bind address |
| `SOROBAN_LENS_DEFAULT_NETWORK` | `testnet` | Network used when a request omits it |
| `STELLAR_RPC_TESTNET_URL` | SDF public testnet RPC | Empty value disables testnet |
| `STELLAR_RPC_MAINNET_URL` | *(unset)* | Mainnet requires a provider URL |
| `STELLAR_RPC_FUTURENET_URL` | *(unset)* | |
| `STELLAR_RPC_TIMEOUT_SECS` | `10` | Per-request RPC timeout |
| `SOROBAN_LENS_CORS_ORIGINS` | `http://localhost:5173` | Comma-separated; `*` for any (dev only) |
| `SOROBAN_LENS_MAX_BODY_BYTES` | `16384` | Request body limit |
| `SOROBAN_LENS_REQUEST_TIMEOUT_SECS` | `30` | Whole-request timeout |
| `SOROBAN_LENS_RATE_LIMIT_PER_MINUTE` | `30` | Analyze requests per IP; `0` disables |
| `DATABASE_URL` | *(unset)* | PostgreSQL URL; unset uses the in-memory store |
| `DATABASE_MAX_CONNECTIONS` | `10` | PostgreSQL pool size |
| `SOROBAN_LENS_STORE_CAPACITY` | `1000` | In-memory reports kept (in-memory store only) |
| `SOROBAN_LENS_LOG_FORMAT` | `pretty` | `pretty` or `json` |
| `RUST_LOG` | `info,…` | Log filter |

Invalid configuration stops the server at startup with a clear message.

## Connecting the frontend

1. Start the backend (`cargo run -p soroban-lens-api`).
2. In `soroban-lens-frontend`, point the API base URL at
   `http://localhost:8080` (Vite dev server on port 5173 is allowed by default).
3. For other origins, set `SOROBAN_LENS_CORS_ORIGINS`.

## Current status

### Implemented

* `GET /health`, `GET /api/v1`, `POST /api/v1/analyze`, `GET /api/v1/analyze/{id}`, `GET /api/v1/stats`
* Stellar RPC client (`getTransaction`) with timeouts, response size cap,
  structured errors, multi-network routing, no secret leakage
* XDR decoding of envelopes, results (including fee-bump), and diagnostic events
  (RPC field or TransactionMeta V3/V4)
* Rule-based analyzer: transaction codes, operation codes, Soroban
  invoke/extend/restore results, host error events (contract errors, auth,
  budget, storage, archived entries, value errors, Wasm traps), not-found
* Evidence levels and explicit limitations on every report
* Validation, body limits, CORS, timeouts, per-IP rate limiting, request IDs,
  structured logging (pretty/JSON), graceful shutdown

### Not yet implemented / known limitations

* **Persistence**: PostgreSQL storage works but has no retention policy yet
  (rows grow without bound) and no lookup by transaction hash or
  report-history endpoint. Without `DATABASE_URL`, storage is in-memory.
* **No simulation**: failures without diagnostic events cannot be re-simulated
  to find the host error.
* **Contract error names**: `Error(Contract, #n)` is reported by number; names
  from the contract spec are not resolved yet.
* **Classic operations** are reported but not diagnosed in depth.
* **Rate limiting** is per process and keyed by peer IP (proxies collapse
  clients into one IP; no `X-Forwarded-For` support yet).
* No authentication, no OpenAPI spec, no Docker image yet.
* Only failures seen in the fixtures have been validated against real data;
  other rules are tested with synthetic XDR.

### Planned next steps

`simulateTransaction` re-simulation · contract spec lookup
for error names · fixtures from `soroban-lens-contracts` · OpenAPI document ·
Dockerfile · trusted-proxy rate limiting · `getHealth`-based readiness endpoint.

## Contributing

Contributions are welcome. Good first areas: new diagnostic rules (with a real
fixture), documentation, and tests. Read [CONTRIBUTING.md](CONTRIBUTING.md) and
the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE)
