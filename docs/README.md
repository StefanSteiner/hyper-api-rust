# `docs/` — Workspace Documentation Index

Cross-cutting design documents, process guides, and reference material for the
hyper-api-rust workspace. One file per topic. For per-crate documentation, see
each crate's `README.md` and `DEVELOPMENT.md`. For governance, contribution
process, and commit conventions, see [`../CONTRIBUTING.md`](../CONTRIBUTING.md).
For workspace architecture and build instructions, see
[`../DEVELOPMENT.md`](../DEVELOPMENT.md).

## Documents

| Document | Scope |
|----------|-------|
| [BENCHMARK_GUIDE.md](BENCHMARK_GUIDE.md) | How to run the canonical Rust and Node.js benchmark suites, with per-platform setup and result conventions. Read this before reporting performance numbers. |
| [GITHUB_OPERATIONS.md](GITHUB_OPERATIONS.md) | What runs on every push and PR, what runs on tag pushes, and how releases become crates.io publishes and binary downloads. Read this when wiring up new automation or debugging CI. |
| [NODEJS_API_SUMMARY.md](NODEJS_API_SUMMARY.md) | One-page overview of the Node.js / TypeScript bindings (`hyperdb-api-node`): tagged template literals, async APIs, Arrow IPC throughput. Read this for a quick orientation before diving into the binding's per-crate README. |
| [RUST_DOCUMENTATION_STYLE.md](RUST_DOCUMENTATION_STYLE.md) | **How to write Rust documentation for this workspace.** Rustdoc conventions (`# Examples`, `# Errors`, `# Panics`, `# Safety`), README structure, mermaid/crates.io constraints, doc tests, the `docs/` folder organization itself. Read this when authoring or reviewing Rust documentation. The JS/TS counterpart lives at [`../hyperdb-api-node/DOCUMENTATION_STYLE.md`](../hyperdb-api-node/DOCUMENTATION_STYLE.md). |
| [RUST_GUIDELINES.md](RUST_GUIDELINES.md) | **How to write code for this workspace.** Microsoft Pragmatic Rust Guidelines adoption, lint configuration, API design rules, error handling, the integer-cast rubric, logging conventions, and the table of documented exceptions. Read this when writing or reviewing Rust code. |
| [TRANSACTIONS.md](TRANSACTIONS.md) | Transaction API design in `hyperdb-api`: ACID semantics (A, C, I guaranteed; D not provided by this API), raw `Connection` methods, the RAII `Transaction` / `AsyncTransaction` guards, behavioral notes. |

## Two Rust guides — one for code, one for docs

`RUST_GUIDELINES.md` and `RUST_DOCUMENTATION_STYLE.md` are sibling guides with
distinct, non-overlapping scopes:

- **Writing Rust code?** → `RUST_GUIDELINES.md` (lints, API design, error
  handling, exceptions).
- **Writing documentation about Rust code?** → `RUST_DOCUMENTATION_STYLE.md`
  (rustdoc, README structure, doc tests).

If a question is about *what* the code should do or how it should be
structured, it's a coding question — start with `RUST_GUIDELINES.md`. If it's
about how to *describe* the code in writing — comments, READMEs, design
documents — start with `RUST_DOCUMENTATION_STYLE.md`.

The JavaScript/TypeScript counterpart of `RUST_DOCUMENTATION_STYLE.md` is
[`../hyperdb-api-node/DOCUMENTATION_STYLE.md`](../hyperdb-api-node/DOCUMENTATION_STYLE.md),
co-located with the npm package whose code it governs.
