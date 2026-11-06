# Contributing to SorobanLens Backend

Thanks for helping make Soroban failures understandable.

## Ground rules

* Be accurate. SorobanLens exists to tell developers the truth about their
  transactions. Never present a guess as a confirmed cause; use the evidence
  levels (`confirmed`, `likely`, `possible`, `insufficient`) honestly.
* Do not invent Stellar RPC methods, XDR types, or documentation links. Verify
  against the official docs or the `stellar-xdr` source, and say so in your PR.
* Never commit secrets, `.env` files, or provider URLs containing API keys.

## Development setup

```sh
rustup toolchain install stable   # MSRV is 1.89
cargo build --workspace
cargo test --workspace
```

Before opening a PR:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Store changes must also pass the PostgreSQL tests. They are skipped unless
`TEST_DATABASE_URL` points at a database (see `api/tests/store.rs` for a
one-line Docker command). Schema changes go in a new file in `api/migrations/`;
never edit a migration that has been merged.

## Where things go

| Change | Location |
|--------|----------|
| New API field or enum value | `crates/models` + `docs/API.md` (bump `REPORT_SCHEMA_VERSION` if breaking) |
| New diagnostic rule | `crates/error-analyzer/src/rules/` + `docs/DIAGNOSTICS.md` |
| New data extracted from XDR | `crates/diagnostic-engine/src/extract.rs` |
| New RPC method | `crates/stellar-client` (with wiremock tests) |
| HTTP behaviour | `api/` (with tests in `api/tests/api.rs`) |

## Adding a diagnostic rule

1. Prefer starting from a **real transaction**. Capture its `getTransaction`
   response into `crates/diagnostic-engine/tests/fixtures/testnet/` and add a
   row to the fixtures README (hash, network, date, outcome).
2. Write the rule and unit tests in the analyzer crate.
3. Add an end-to-end assertion in `tests/real_fixtures.rs`.
4. Document the rule ID, trigger, category, and evidence level.

## Commits and pull requests

* Keep PRs focused; one logical change per PR.
* Describe *what* changed and *how you verified it* (tests, real hashes).
* Fill in the pull request template.

## Reporting issues

Use the issue templates. For incorrect diagnoses, include the transaction hash
and network — that makes the problem reproducible.

## Security

Please do not open public issues for security vulnerabilities. Report them
privately through GitHub's "Report a vulnerability" (Security Advisories)
feature on this repository.
