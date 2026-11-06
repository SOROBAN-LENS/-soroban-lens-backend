# Test fixtures

## `testnet/` — real network data

Each file is a JSON-RPC response for a **real** Stellar testnet transaction,
captured from `https://soroban-testnet.stellar.org` (protocol 30) on
2026-11-06. They were collected with `getTransactions` (one entry per file,
wrapped in the `getTransaction` response shape, with the page's
`latestLedger`/`oldestLedger` added). Note that `getTransactions` encodes
`createdAt` as a number while `getTransaction` uses a string; the client
accepts both.

| File | Transaction hash | On-chain outcome |
|------|------------------|------------------|
| `invoke_trapped_a.json` | `05235449…05dd8` | `InvokeHostFunction` trapped: `Error(Storage, ExceededLimit)`, key outside footprint |
| `invoke_trapped_b.json` | `c5ee2dc2…87812` | Same contract and failure as above |
| `invoke_entry_archived.json` | `ae853c97…0244d` | `InvokeHostFunction` entry archived (`Error(Value, InvalidInput)` event) |
| `invoke_insufficient_refundable_fee.json` | `a096e259…f7996` | Insufficient refundable fee (`Error(Budget, ExceededLimit)` event) |
| `classic_payment_no_destination.json` | `3f1c8cc2…49fa0` | Classic payment to a non-existent account |

The contracts involved belong to third parties on testnet; SorobanLens has no
relationship with them. Testnet RPC retains only ~7 days of history and
testnet is periodically reset, so these hashes will stop resolving live — the
fixtures are the durable record.

## Synthetic data

`tests/synthetic.rs` builds XDR programmatically for outcomes not covered by
real fixtures (bad sequence number, fee-bump inner failures, contract error
codes, corrupt XDR). Those tests verify decoding and rule wiring, **not**
real-network behaviour.

## Adding fixtures

TODO(next milestone): once `soroban-lens-contracts` deploys its example
contracts to testnet, add fixtures for each intentional failure scenario
(missing auth, invalid input, contract error) with a note linking the contract
and invocation that produced them.
