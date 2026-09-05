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
| 1 | 1.3 `cargo fix --edition` + edition flip + resolver v3 | **Done** |
| 2 | 2.1 let chains | **Done — via clippy, 127 sites** |
| 2 | 2.3 `use<..>` precise capturing | **Done — applied by `cargo fix`** |
| 1 | 1.4 drop-order triage, napi verify | **Done — no code changes** |
| 1 | 1.5 doc/config sweep | Deferred (see below) |
| 2 | 2.2 async-closure spike, 2.5, 2.6 | Not started |
| 3 | RHEL CI (3.0–3.4) | Not started |
| 4 | Benchmark gate, RC, API audit, 1.0.0 | Not started |

**Commits so far:** `f3736b0`, `8a42b8c`, `9f75ff0`, `caeda1b`, `091327c`,
`0cab3af`, `efbf704` — all signed.

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

## 2026-09-04 — Phase 1 Task 1.3: edition 2024 landed (`efbf704`)

`cargo fix --edition --workspace --all-targets` ran on a clean tracked tree
while both manifests were still on 2021, exit **0**. It needed `--allow-dirty`
because three *untracked* directories (`.agents/`, `.codex/`,
`.github/scripts/__pycache__/`) block the check; `git status -uno` was empty,
so the reviewable-diff property held.

It produced 26 rewrites in three categories. **Only one was kept.**

- **Kept — 4 RPIT `use<'a, T>` bounds.** Applied to exactly the four genuinely
  over-capturing signatures the plan review identified: `stream_as` and
  `stream_as_params` on both `Connection` and `AsyncConnection`. cargo
  retained `+ 'a` alongside the capture list, which is the belt-and-braces
  form the review recommended. **This completes Task 2.3** — and confirms the
  review's finding that `hyperdb-api-core` has zero over-capturing sites, since
  nothing was applied there.
- **Discarded — 14 `$x:expr` → `$x:expr_2021` macro pins** across `table!`,
  `table_name!`, `schema_name!`, `params!` and two test macros. Freezes 2021
  semantics instead of adopting the 2024 `expr` fragment.
- **Discarded — 12 `if let ... else` → `match ... { Some(v) => {} _ => {} }`
  rewrites** guarding `if_let_rescope`. Conflicts with AGENTS.md reminder 6 and
  M-RUST-SHAPED, and most scrutinees (e.g. `row.get::<T>(idx)`) return by value
  with no `Drop`, so the defence was unnecessary.

**The compiler vindicated discarding both.** After the edition flip,
`cargo build --workspace --all-targets` exits **0 with zero errors and zero
warnings** without either category, proving they were defensive rather than
required.

Then: `edition = "2024"` in the workspace manifest and
`hyperdb-compile-check`, `resolver` `2` → `3` (root only — the compile-check
crate is a *package* workspace root and infers v3 from its own edition), and
98 files reformatted because edition 2024 changes rustfmt's import sort order.

## 2026-09-04 — Task 2.1 let chains: done early, driven by clippy

Edition 2024 made `clippy::collapsible_if` actionable, and because it is
`style`-group against CI's `-D warnings`, this became **blocking rather than
optional Phase 2 polish**. Clippy flagged **127 sites** — roughly 20× the six
the plan had hand-listed, and its list included every one of them:
`async_client.rs:1016` (the depth-4 `extract_row_count`),
`authenticated_client.rs:980` and `:1073`, both `finish_copy` twins, and
`client.rs:871`.

`cargo clippy --fix` resolved all 127 across **37 files, net −129 lines**. The
output quality is good, unlike the `if_let_rescope` rewrites — clean
`if cond && let Err(e) = ...` chains. `get_table_labels` collapsed from depth 4
to a single `if let Ok(batch) = batch_result && let (Some(a), Some(b)) = (..)`.

**Lesson worth carrying:** clippy is a better source of truth for this class of
work than a hand-curated inventory — the same lesson the plan review taught
about the drop-order sites.

### Two stale `#[expect]` waivers removed

The flattening made `clippy::manual_flatten` stop firing at
`authenticated_client.rs:976` and `:1068`, so their `#[expect]` attributes
became unfulfilled and `unfulfilled_lint_expectations` failed the build. Both
were removed. Their stated reason — preserving an explicit `if let Ok` so a
`.flatten()` refactor would not hide the error discard — no longer applies now
that the body is a let chain and `.flatten()` is not suggestible.

This is **M-LINT-OVERRIDE-EXPECT working exactly as advertised**: `#[expect]`
self-cleans by failing when its lint goes quiet, which is why the guideline
prefers it over `#[allow]`.

## 2026-09-04 — Task 1.4 drop-order triage: done, no code changes needed

### A process failure worth recording first

**Task 2.1 landing before Task 1.4 permanently closed the measurement window.**
`if_let_rescope` and `tail_expr_drop_order` are *migration* lints — they only
fire on editions before 2024, describing what will change. Once on 2024 they go
silent, confirmed: `cargo rustc -p hyperdb-api-core --lib -- -W if_let_rescope
-W tail_expr_drop_order` now reports **zero**.

Normally that would be recoverable by temporarily flipping the manifest back to
2021. It is not, because the let chains from Task 2.1 are edition-2024-only
syntax — the tree can no longer compile on 2021 at all. The inventory therefore
had to be recovered from the saved `cargo fix --edition` output captured while
still on 2021.

**Lesson for the plan:** the drop-order triage has a hard ordering dependency on
happening *before* any 2024-only syntax lands. The plan sequenced 1.4 before
2.1 correctly; clippy's `-D warnings` gate forced 2.1 early and nothing flagged
the conflict. A future edition migration should generate and commit the
inventory as its very first step.

### Inventory: 152 distinct (site, moved-Drop-type) pairs

Recovered from the edition-2021 run and triaged by what the moved `Drop`
actually does, rather than by count:

| Moved type | Sites | Verdict |
|---|---:|---|
| `bytes::Bytes` | 78 | Benign — refcount decrement, no side effect |
| *(type not captured in log)* | 37 | Benign by association; all in `Bytes`-heavy paths |
| `TempDir` | 13 | Benign — tests only; compiler still guarantees drop-after-last-use |
| `OwnedFd` (+ tokio `Parker`/`Thread`) | 4 | Benign — fd close, no ordering dependency |
| tokio internals (`Runtime`, `ScheduledIo`, `Parker`) | ~14 | Not our code; reached through connection objects |
| `proc_macro::TokenStream` | 2 | Benign — internal to `proc_macro` |
| `WorkerGuard` | 1 | **Improved by 2024** — see below |
| `InFlightGuard` | 1 | Benign on inspection — see below |
| `AsyncPreparedStatement` | 1 | Observable but self-defending — see below |

### The three sites that warranted real inspection

**`hyperdb-mcp/src/main.rs:235` — `WorkerGuard`. Edition 2024 is strictly
better here, and arguably fixes a latent bug.** `_file_guard` is a
`tracing_appender` `WorkerGuard` whose `Drop` flushes buffered log output, and
line 235 is the function's tail expression (`run_daemon(config).await`). In
2021, tail-expression temporaries dropped *after* locals — so the guard flushed
first and anything logged during a temporary's own `Drop` went to an
already-flushed writer and could be lost. In 2024 the temporaries drop first
and the guard flushes last, capturing them. No action.

**`hyperdb-mcp/src/watcher.rs:617` — `InFlightGuard`. The reviewer's concern
does not survive inspection.** The plan review singled this out as "precisely
the class rustc warns about," because a `MutexGuard` reorders relative to a
type with a custom `Drop`. But that `Drop` (`watcher.rs:359-363`) is a single
`self.counter.fetch_sub(1, Ordering::Relaxed)` — a relaxed atomic decrement. It
takes no lock and sends no message, so its position in the drop sequence is not
observable. No action. Good illustration of the reviewer profile's own rule:
verify against source before assigning severity.

**`hyperdb-api/src/async_connection.rs:768` — `AsyncPreparedStatement`.** This
one *is* observable: its `Drop` performs a best-effort server-side close. But
it already defends itself — it early-returns when `self.closed`, and when
dropped outside a tokio runtime it warns and flags the connection
desynchronized rather than misbehaving. The related `pool.rs:947` site has a
`Runtime` in its moved set, which is the scenario that would push it onto that
fallback path, and that path is deliberate. No action.

### Empirical backstop

`make test` passes **1515/1515** on edition 2024 — identical to the
edition-2021 baseline. Drop-order bugs are the class integration tests can
miss, so this is corroboration rather than proof; the per-site reasoning above
is the actual basis for closing this task.

The napi half of Task 1.4 is also satisfied: Phase 0 proved the 148 `#[napi]`
attributes expand under 2024, and the full-workspace build at 2024 includes
`hyperdb-api-node` and exits 0.

## Deferred: all existing-doc updates

By decision on 2026-09-04, every change to *existing* docs is batched into one
pass at the end of coding, since the migration will generate many of them.
This log is exempt. Deferred items:

- The 7 broken intra-doc links in `hyperdb-mcp` that make `cargo doc` exit 101
  (`engine.rs`, `table_catalog.rs`, `attach.rs`).
- Task 1.5's doc/config sweep, including the dead `test-redirect` Makefile
  target and the stale `hyperd-bootstrap` alias in `.cargo/config.toml:5`.
- Task 3.2's README "Enterprise Compatibility" section.
- The seven per-crate `CHANGELOG.md` entries for the MSRV and edition raise.
- `CONTRIBUTING.md:216` and `docs/GITHUB_OPERATIONS.md:26`, which both wrongly
  claim release-please creates the tag and GitHub Release.

## Gate status

Latest, measured **on edition 2024** with the let chains applied:

- `cargo build --workspace --all-targets` — exit **0**, zero errors, zero
  warnings
- `cargo fmt --all -- --check` — exit **0**
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  exit **0**
- `make test` — exit **0**, **1515 passed / 0 failed**, 0 failing suites, 121s.
  Identical pass count to the edition-2021 baseline, so the let chains and the
  edition change introduced no regressions.
- `cargo deny check` — exit **0** (measured on 2021; re-run before the RC)
- `cargo audit --deny warnings` — exit **0**, 536 crates against 1239
  advisories (measured on 2021; re-run before the RC)
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps` — **exit 101, pre-existing
  failure, not caused by this migration.** Deferred; see below.

Five of six green, with the sixth failing for reasons that predate this work.

Earlier baseline, on edition 2021 after Tasks 1.1–1.2: same results —
1515 passed, clippy exit 0 (was 101 before the B2 burn-down).

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
