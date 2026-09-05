# Rust 1.88 / Edition 2024 Uplift — Execution Log

Progress record for
[`2026-09-04-rust-188-edition-2024-uplift.md`](2026-09-04-rust-188-edition-2024-uplift.md).
Kept separate from the plan so the plan stays a stable brief while this grows.

**Branch:** `feat/upgrade-to-188-RHEL97` (base `28c8813`)


Updated as work lands. Every claim here is backed by a captured exit code, per
AGENTS.md reminder 10 — no green is recorded without real output.

## Status at a glance

| Phase | Task | Status |
|---|---|---|
| 0 | napi/`ctor` edition-2024 spike | **Done — GREEN** |
| 1 | 1.1 `rust-toolchain.toml` components | **Done** |
| 1 | 1.2 MSRV floor to 1.88 (4 manifests) | **Done** |
| 1 | 1.2 `sysinfo` downgrade (Blocker B1) | **Done** |
| 1 | 1.2 clippy burn-down (Blocker B2) | **Done** |
| 1 | 1.3 `cargo fix --edition` + edition flip + resolver v3 | Not started |
| 1 | 1.4 drop-order triage, napi verify | Not started |
| 1 | 1.5 doc/config sweep | Not started |
| 2 | Modernization (2.1–2.6) | Not started |
| 3 | RHEL CI (3.0–3.4) | Not started |
| 4 | Benchmark gate, RC, API audit, 1.0.0 | Not started |

## 2026-09-04 — Phase 0: GREEN

Covered in the Phase 0 section below. `hyperdb-api-node` built at
`edition = "2024"` against a 2021 workspace: exit **0**, zero warnings, 148
`#[napi]` attributes re-expanded on a forced recompile, 51 MB cdylib produced.
Reverted with zero diff.

## 2026-09-04 — Phase 1 Tasks 1.1 and 1.2: done

- **1.1** — added `rust-analyzer` to `rust-toolchain.toml` components;
  `channel = "stable"` unchanged per D1.
- **1.2 MSRV floor** — `1.81` → `1.88` in `Cargo.toml`, `clippy.toml`, and
  `hyperdb-compile-check/Cargo.toml`; added the missing
  `rust-version.workspace = true` to `hyperdb-api-node/Cargo.toml`, which was
  the only member without one.
- **1.2 Blocker B1** — `sysinfo` `0.39` → `0.38`, resolved to **0.38.4**.
  `cargo check -p hyperdb-api --all-targets` exit **0**. The
  `physical_core_count` API break the plan warned about **did not
  materialize** — it was a clean swap, no bench-harness edit needed.
- **1.2 Blocker B2** — the clippy MSRV bump surfaced **15 sites / 30 warnings**,
  matching the plan review's measurement exactly: 11 `unnecessary_map_or`,
  3 `manual_is_multiple_of`, 1 `manual_repeat_n`. `manual_midpoint` and
  `manual_let_else` did not fire, as predicted. All machine-applicable;
  `cargo clippy --fix` resolved them across 8 files (`map_or(true, ..)` →
  `is_none_or(..)`, `len() % 2 != 0` → `!len().is_multiple_of(2)`).

  This pulls part of Task 2.5's `is_none_or` cluster forward into Phase 1 by
  necessity — B2 must land with the MSRV bump or `main` goes red.

## Gate status

Measured after Tasks 1.1 and 1.2, still on edition 2021:

- `cargo fmt --all -- --check` — exit **0**
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  exit **0** (was 101 before the B2 burn-down)
- `make test` — exit **0**, **1515 passed / 0 failed**, 0 failing suites,
  138s wall clock
- `cargo deny check` — exit **0** (advisories, bans, licenses, sources all ok)
- `cargo audit --deny warnings` — exit **0**, 536 crates scanned against 1239
  advisories
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` — **exit 101, pre-existing
  failure, not caused by this migration.** See below.

Five of six green. The MSRV raise, the `sysinfo` downgrade, and the lint
burn-down are therefore safe to land as one commit.

## Discovered, out of scope

**`hyperdb-mcp` has 7 broken intra-doc links**, all "public documentation links
to private item": `PersistentAttachOutcome` → `attach_default_persistent`;
`ensure_exists_in`, `list_in`, `upsert_stub_in`, `set_metadata_in` →
`qualified_catalog_in`; `reconcile_in` → `reconcile_live_tables` (×2). They
live in `engine.rs`, `table_catalog.rs`, and `attach.rs` — none of which this
migration touches.

This explains a discrepancy found while auditing the docs: `CONTRIBUTING.md`
listed `RUSTDOCFLAGS="-D warnings" cargo doc` as a CI gate, but no such job
exists in `ci.yml`. It was never added because it would fail. The doc has been
corrected to describe it as a local gate; **fixing the links themselves is
follow-up work**, either by making the referenced items public or by
downgrading the links to plain code spans.
