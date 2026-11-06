## Summary

<!-- What does this change and why? Link related issues. -->

## How was this verified?

<!-- Tests added/updated, real transaction hashes checked, manual steps. -->

## Checklist

- [ ] `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- [ ] `cargo test --workspace` passes
- [ ] API or report shape changes are reflected in `docs/API.md` (schema version bumped if breaking)
- [ ] New/changed rules are documented in `docs/DIAGNOSTICS.md`
- [ ] Stellar APIs, XDR types, and doc links used here were verified against official sources
- [ ] No secrets, `.env` files, or keyed RPC URLs are included
