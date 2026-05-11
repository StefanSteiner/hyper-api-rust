# hyperdb-api-core

⚠️ **This crate is an implementation detail of
[`hyperdb-api`](https://crates.io/crates/hyperdb-api).**
Use `hyperdb-api` directly; don't add `hyperdb-api-core` to your dependencies.

This crate has no stable API. Breaking changes land here without a major version
bump of `hyperdb-api-core`; your build may break on any `hyperdb-api` patch
release if you depend on `hyperdb-api-core` directly.

For the public API and user-facing documentation, see
[`hyperdb-api` on crates.io](https://crates.io/crates/hyperdb-api) and
[docs.rs/hyperdb-api](https://docs.rs/hyperdb-api).

## Why does this crate exist?

`hyperdb-api-core` holds implementation details (SQL types, PostgreSQL wire
protocol, sync/async clients) that `hyperdb-api` depends on. It's published to
crates.io because Cargo requires any dependency of a published crate to be
resolvable on crates.io — **not** because it's a user-facing API.

## Will it ever be stabilized?

**No — not under this name.** `hyperdb-api-core` is committed to being forever
internal. The `-core` suffix is part of the "don't depend on me" signal.

If enough users surface concrete needs for low-level wire-protocol access
(custom connection pools, custom async runtimes, language bindings), the
relevant subset will be extracted into a new crate under a new name (e.g.
`hyperdb-client`) with its own semver 1.0 baseline. That promotion would ship
the public low-level API under a fresh, distinctly named crate — not by
redefining `hyperdb-api-core` as stable. The existing `-core` keeps its
"internal" meaning regardless.

If you have a use case like this, open an issue describing what you want to
build and which APIs you need.

## Acknowledgments

Several modules in this crate adapt code from
[sfackler/rust-postgres](https://github.com/sfackler/rust-postgres) (the
`postgres-protocol`, `tokio-postgres`, and `postgres-types` crates by Steven
Fackler, MIT or Apache-2.0). The adapted material is listed in detail in the
[`NOTICE`](https://github.com/tableau/hyper-api-rust/blob/main/NOTICE) file at
the workspace root. Per-file source-level credits are in the relevant module
docs. (Relative links to `NOTICE` work on GitHub but not on crates.io; the
absolute URL above is crates.io-friendly.)
