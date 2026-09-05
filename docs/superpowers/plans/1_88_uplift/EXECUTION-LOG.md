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
| 1 | 1.5 doc/config sweep | **Done** |
| 2 | 2.2 async-closure spike | **Done — NO-GO, documented** |
| 2 | 2.5 API modernization | **Done — 1 bug fixed, 3 rejected** |
| 2 | 2.6 node cast conversions | **Done — 5 fixed, ~20 exempt** |
| 3 | 3.0 probe, 3.1 workflow, 3.3 Makefile | **Done — verified end-to-end** |
| 3 | 3.2 README section, 3.4 doc gap | **Done** |
| 4 | 4.0 benchmark gate | Next |

| 4 | 4.2 API audit | **Done — 1 of 141 sites was real** |
| 4 | 4.1, 4.3 RC and 1.0.0 release | Blocked on 4.0 |

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

## 2026-09-04 — Task 2.2 async closures: NO-GO, confirmed by spike

The plan review predicted this would fail and named the reason. Both hold. Four
approaches were compiled against a real edition-2024 crate; all four fail:

1. **`AsyncFn` + boxed `Send` future** — `E0277`:
   `<F as AsyncFnMut<(&Conn,)>>::CallRefFuture<'_> cannot be sent between
   threads safely`. Naming that associated type requires unstable
   `async_fn_traits`.
2. **Return-type notation** (`F(&Conn): Send`) — `E0214`: "parenthesized type
   parameters may only be used with a `Fn` trait". Not available for `AsyncFn`
   on stable.
3. **Current `Box::pin` form** — compiles. (Control.)
4. **Generic `F: for<'a> Fn(&'a Conn) -> Fut` with internal boxing** —
   `E0308`, with rustc pointing at `-> Fut`: "the lifetime requirement is
   introduced here". `Fut` would have to depend on the higher-ranked lifetime,
   which is exactly the case needing return-type notation or GATs.

Root cause, stated once: the hook future must be **both** `Send` (the pool runs
on multi-threaded runtimes) **and** able to borrow the `&AsyncConnection` it is
given. Stable Rust cannot express both together.

**The hard constraint held** — no `+ Send` was removed from `HookFuture`,
`AfterConnectHook`, `BeforeAcquireHook`, or `RecycleCheck`. That was the
failure mode the review flagged as most dangerous, since dropping it is the
obvious way to make the code compile and would silently regress thread safety
in a connection pool.

### This reverses one of the plan's assumptions

The plan treated the rustdoc example at `pool.rs:101` as outdated guidance to
be rewritten: "the rustdoc example currently *teaches*
`.after_connect(|conn| Box::pin(async move {` — update it; docs are how this
pattern spreads." That is wrong. `Box::pin(async move { .. })` is **required,
not conventional**, so the example is correct and was left alone.

**Deliverable:** a doc comment on `HookFuture` recording all three unstable
blockers and the `M-ASYNC-FN` trait carve-out that licenses the explicit
`Future` return, so the next reader does not repeat this spike. No API change,
therefore no `CHANGELOG` entry and no `feat!:` commit — the plan's assumption
that Task 2.2 would be a breaking change does not apply either.

`fmt` and `clippy -D warnings` both exit 0 on `hyperdb-api` after the change.

## 2026-09-04 — Task 2.5 API modernization: one real bug fixed, three items rejected

The `is_none_or` cluster had already landed via the B2 burn-down. Of the three
remaining items, **two were not candidates and the third needed a different
tool than the plan named.**

### Rejected: `is_some_and` at `provider.rs:278`

The let-chain pass already flattened `:215` correctly. It deliberately left
`:278` alone, and clippy was right to: the outer `if let` block has a statement
*after* the inner `if`.

```rust
if let Some(ref token) = self.cached_oauth_token {
    if token.is_likely_valid() { /* .. */ return Ok(token.clone()); }
    debug!("Cached OAuth Access Token expired, refreshing");  // <-- would be lost
}
```

Flattening to a let chain or `is_some_and` would drop that `debug!`, since it
runs precisely when the inner condition is false. Not a candidate.

### Rejected: tuple `FromIterator` at `engine.rs:963`

The plan called this "the one genuine hit" for the 1.85 tuple `Extend` impls.
It is not. The two `Vec::push` calls sit inside
`while let Some(chunk) = result.next_chunk()?` — a *fallible streaming* loop
over chunks, nested inside `if let Some(ref schema)` and `for row in &chunk`.
Collecting into `(Vec<_>, Vec<_>)` would mean turning fallible chunk streaming
into an iterator chain, which fights AGENTS.md's rule that results stream to
hold memory constant. Two pushes in a nested loop is the correct form here.

### Fixed, and bigger than described: a latent `usize` overflow (`d7157c3`)

The plan framed `protocol/types.rs` as a `split_at_checked` readability tidy-up.
Inspection found a real bug in both variable-length readers:

```rust
let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
if buf.len() < 4 + len { /* .. */ }        // <-- 4 + len can overflow usize
```

On a 32-bit target `usize` is 32 bits, so a declared length near `u32::MAX`
wraps `4 + len` to a small value, the bounds check passes, and the following
slice index panics. **32-bit `i686` targets are Tier 1**, which M-OOBE requires
supporting, so this was reachable rather than theoretical.

`slice::split_first_chunk::<4>()` is the right tool, not `split_at_checked`: it
yields a typed `&[u8; 4]` plus remainder, so no length arithmetic is performed
at all and the overflow class is removed by construction. An intermediate
version using `split_at_checked(4)` + `try_into().expect(..)` worked but
introduced a panic path that `clippy::missing_panics_doc` correctly caught —
`split_first_chunk` needs no `expect` because the array length lives in the
type.

Also converted the six fixed-width readers to slice-to-array `try_into`,
removing six manual index chains. `TryFrom` enforces exact length by
definition, so semantics are unchanged.

Added four regression tests for behaviour the three existing round-trip tests
missed: oversized declared length, truncated length prefix, exact-length
enforcement, and zero-length payload as distinct from truncation.

**Honest limit on the evidence:** these tests guard the contract but cannot
show red-before-green for the overflow, because on a 64-bit host the old
arithmetic did not overflow and produced the same error. The fix rests on
construction, not on a failing test.

`make test` now passes **1519/1519** (1515 plus the four new).

## 2026-09-04 — Task 2.6 node casts: 5 fixed, ~20 correctly left alone (`82db12d`)

### The plan's premise was wrong in two ways

**First, these were not undocumented debt.** Every cast site already carried
`#[expect(clippy::cast_possible_truncation, reason = "...")]` with a considered
rationale, and `get_int32`'s rustdoc stated the contract explicitly — "I64/F32/F64
cells are truncated to i32... use `getBigInt()` or `getFloat64()`". So this was
a deliberate API decision to revisit, not sloppiness to clean up.

**Second, the plan conflated two different cast classes.** AGENTS.md rule 7
targets *integer-to-integer* narrowing, which truncates and wraps. But
**float-to-integer `as` saturates** in Rust: an out-of-range `f64` clamps to
`i32::MIN`/`MAX` and `NaN` becomes 0. One of the pre-existing `#[expect]`
reasons already said so correctly. Those are lossy but bounded, not a
corruption vector.

Of roughly 25 casts in the crate, only **five** are integer-to-integer:

| Site | Cast | Path |
|---|---|---|
| `result.rs` `get_int32` | `I64` cell → `i32` | read |
| `columnar.rs` `get_int32_column` | `Int64` column → `i32` | read |
| `inserter.rs` `add_value_typed` | `I32` → `i16` (SMALLINT) | **write** |
| `inserter.rs` `add_value_typed` | `I64` → `i32` (INT) | **write** |
| `inserter.rs` `add_value_typed` | `I64` → `i16` (SMALLINT) | **write** |

### A false safety claim in the write path

`add_value_typed` carried this justification:

> If a caller supplies a value that does not fit the declared column type,
> Hyper will reject the insert at `execute()` time — that is the documented
> contract of this path.

**That is false for exactly the narrowing paths.** `chunk.add_i16(x)` writes two
bytes; a wrapped value is a *perfectly valid* encoding of a different number, so
Hyper accepts it and the `.hyper` file silently holds corrupt data. Passing
`2^33` into a `SMALLINT` column stored `0` with no error. Comment corrected and
the three paths now go through checked `narrow_i16` / `narrow_i32` helpers.

### Decision: throw rather than return null

Chosen over `Option`-style `None` because **`null` is indistinguishable from a
SQL NULL** — a caller could not tell a real null apart from an overflow. The
read getters gained a `napi::Result` return; the write path already returned
`Result`, and `columnar.rs` already returned `Result`, so only `get_int32`
changed signature (still `number | null` in TypeScript, but now throwing).

### Follow-through, done unconditionally

The plan made this conditional on "if any change is JS-visible"; it is
JS-visible by construction, so all of it was done: rustdoc on both getters,
`index.d.ts` `@throws` declarations, four new smoke-test assertions covering
both throw paths *and* the recommended alternatives, and a
`hyperdb-api-node/CHANGELOG.md` entry.

One accuracy fix caught while writing the error messages: the first draft told
callers to use `getInt64Column()` "for lossless access", but that returns
`number` and its own docs warn it loses precision above 2^53. Reworded to say it
*widens* rather than claiming lossless. `getBigInt()` on the row path genuinely
is lossless and is described as such.

Gates: workspace `fmt` and `clippy -D warnings` exit 0; `npm test` exits 0 with
the new narrowing block passing.

## 2026-09-04 — Phase 3 RHEL CI: done (`d97f4af`), plus a dependency win (`909127c`)

### The probe earned its place — three findings the plan had wrong

Task 3.0 probed `ubi9/ubi:latest` rather than assuming, and all three of the
plan's assumptions about the container were wrong:

- **`protobuf-compiler` is not in any UBI repository.** `dnf search protobuf`
  returns only `protobuf-c` and `python3-protobuf`. Worse, the plan's first
  fallback — "enable `ubi-9-codeready-builder`" — was already exhausted:
  `dnf repolist` shows appstream, baseos **and** codeready-builder all enabled
  by default. protoc is fetched from the pinned upstream release instead.
- **`rust-toolset` is 1.92.0, not 1.88.0.** RHEL 9.7's release notes document
  1.88.0, but it is a *rolling* Application Stream and UBI tracks the latest.
  So this job proves "builds with RHEL's current system toolchain", which is
  the real enterprise contract, but it does **not** pin-test the 1.88 MSRV
  floor. Verifying that floor specifically would need a pinned toolchain, which
  contradicts the no-rustup premise.
- **`mold` is not available**, confirming the linker override must be
  neutralized rather than satisfied.

### Two corrections to the planned approach

**The `--config` override does not work; the env var does.** The plan review
proved `--config target.<triple>.rustflags=[]` is a no-op because cargo *joins*
rustflags across config sources. I tested the env form directly in a scratch
crate: both `RUSTFLAGS=""` and `CARGO_ENCODED_RUSTFLAGS=""` **replace** them.
So the job neutralizes both overrides by environment and never edits the
checkout — no copy, no deletion.

That matters because the plan's copy-based approach failed hard: `cp -a /src
/build` filled the container VM's disk and corrupted containerd's content
store, because **`target/` is 32 GB** locally. The read-only mount has no such
problem.

**`make check-rhel` defaults to native arch, not `--platform linux/amd64`.**
The plan made amd64 mandatory to avoid a false pass on the x86_64-scoped mold
override (blocker B5). But GitHub's runners are natively x86_64, so **CI is
already the authoritative gate** for that, and forcing qemu/Rosetta emulation
locally would make the target too slow to ever be run. `RHEL_PLATFORM=linux/amd64`
opts in.

### Verified end-to-end

`make check-rhel` exits **0** against `rustc 1.92.0 (Red Hat 1.92.0-1.el9)`
with `cargo check --workspace --locked --all-targets`. The `--all-targets`
scope is only possible because the Blocker B1 `sysinfo` downgrade removed the
last dependency above the MSRV floor — so the plan's "promote to
`--all-targets` later" step is already done.

### Bonus: the crypto provider fix removed 17 crates

Chasing the C++ requirement uncovered that the workspace's deliberate
`rustls = { features = ["ring"] }` choice **was not taking effect**. reqwest's
`rustls` feature expands to `__rustls-aws-lc-rs`, forcing `rustls?/aws-lc-rs`,
and Cargo unifies features across the graph — so one requester overrode the
workspace's selection for everyone.

Two crates requested it: `hyperdb-bootstrap`, which declares its own reqwest
rather than inheriting the workspace entry, and `hyperdb-api-salesforce`. Both
now use `rustls-no-provider`. Dropped from the lockfile: `aws-lc-sys`,
`aws-lc-rs`, `cmake`, `jobserver`, `fs_extra`, `dunce`, `cfg_aliases`,
duplicate `rand`/`rand_chacha`/`rand_core`, `tinyvec`, `web-time`, `lru-slab`,
and the entire `quinn` HTTP/3 stack. 186 lines out of `Cargo.lock`.

Runtime TLS was the risk, since `rustls-no-provider` installs no default
`CryptoProvider`. **Verified rather than assumed:** `make verify-hyperd-pin`
makes four real HTTPS requests to downloads.tableau.com through the changed
reqwest, all returning 200. rustls auto-resolves ring now that exactly one
provider is compiled in.

This also dropped `cmake` from the RHEL prerequisites.

## Follow-up: the last C/C++ requirement is the plotters chart stack

Investigated but **deliberately not changed** — recorded so the next person
does not start from scratch.

`gcc`/`gcc-c++`/`fontconfig-devel` remain required by exactly one chain:

    hyperdb-mcp -> plotters 0.3.7 -> font-kit 0.14.3 -> pathfinder_simd 0.5.6

`pathfinder_simd` (Mozilla's Pathfinder rasterizer) compiles a C++ SIMD shim.
`font-kit` is a *system* font loader, which is where `freetype-sys` and
`yeslogic-fontconfig-sys` come from — those two are Linux-only, since font-kit
uses Core Text on macOS, which is why they never appear in a local
`cargo tree` on a Mac.

It arrives from `plotters = "0.3"` with default features, and plotters'
defaults include `ttf = ["font-kit", "ttf-parser", "lazy_static",
"pathfinder_geometry"]`.

**The swap looks viable.** plotters offers `ab_glyph`, a pure-Rust rasterizer
with no native deps. Font usage in `chart.rs` is trivially small: only
`("sans-serif", 11)` at 6 sites and `("sans-serif", 22)` at 7 — no bold or
italic variants, no user-supplied family names. Removing `ttf` would drop
`font-kit`, `pathfinder_simd`, `pathfinder_geometry`, `freetype-sys` and
`yeslogic-fontconfig-sys`, taking the RHEL prerequisites down to `rust-toolset`
plus `unzip` and fetched protoc — no C or C++ compiler at all.

**But it is not free.** plotters' `ab_glyph` backend bundles **no** default
font: its `FONTS` map starts empty and `"sans-serif"` resolves to
`FontError::FontUnavailable` until the application calls
`register_font(family, style, bytes)`. So it needs a committed font file
(DejaVu Sans or Liberation Sans for licensing), registration at MCP startup,
and a licensing review since the font would ship in both the crate and the npm
package.

Held back as its own change because it puts a binary asset in the repo, and
because "the chart still renders correctly" is a visual property the test suite
does not cover — unlike the aws-lc removal, where four real HTTPS requests
proved equivalence.

## 2026-09-05 — Batched docs pass: done (`018b6f8`)

All deferred existing-doc updates landed in one pass, after the code was
settled so every claim reflects what was verified rather than planned.

**The rustdoc gate now passes for the first time.** It was red for two
independent reasons. Six public doc comments in `hyperdb-mcp` linked to private
items; downgraded to code spans, since making internal helpers public to
satisfy a doc link would be backwards. And four links in `hyperdb-api-derive`
referenced `hyperdb_api` types that crate deliberately does not depend on (the
cycle break) — given explicit link-reference definitions to docs.rs, matching
the pattern already present in its crate-level docs.

`make doc` also now sets `RUSTDOCFLAGS="-D warnings"` and covers all seven
publishable crates. `hyperdb-api-derive` and `hyperdb-bootstrap` were omitted
before, which is exactly why their rustdoc had never been checked.

**Eight CHANGELOG entries, written per crate rather than boilerplate**, each
recording what changed for that crate's consumers. Created
`hyperdb-api-derive/CHANGELOG.md`, which did not exist despite the crate being
published and version-managed by release-please — an inconsistency with
AGENTS.md reminder 8, which lists it.

**README Enterprise Compatibility section** documents the empirically verified
prerequisite list, including the two surprises: `protoc` is not packaged for
UBI at all, and the C/C++ requirement comes from the plotters chart font stack
rather than anything in this workspace' own code.

**Two stale claims corrected that had misled this plan.** `CONTRIBUTING.md` and
`docs/GITHUB_OPERATIONS.md` both said release-please creates the tag and GitHub
Release; it does not, and that error is what caused Task 4.1 to omit the manual
tag step in an earlier draft. Also registered `rhel-compatibility` in the
operations table, which still claimed four workflows when there were five.

**Two broken config entries fixed**, both of which could only ever fail:
`cargo download-hyperd` referenced a package name that has not existed since
the crate rename, and `make test-redirect` invoked a `redirect` feature no
crate defines.

Gates after the pass: `fmt`, `clippy -D warnings`, `cargo deny`, and
`make doc` all exit 0.

## 2026-09-05 — Task 4.2 API audit: done (`7144165`)

The 1.0.0 gate. Every number below came from running the lints, not from the
plan's estimates.

### `must_use_candidate`: 141 sites, but only one that matters

The count matched the plan review exactly. The **distribution** is what
decided the outcome:

| Location | Sites | Is it a public API? |
|---|---:|---|
| `hyperdb-api-core` | 122 | No — its `lib.rs` says "This crate is not a public API" |
| `hyperdb-mcp` | 10 | No — internal daemon/server helpers in a binary crate |
| prost-generated protobuf | 8 | No — generated, and not editable |
| **`hyperdb-api`** | **1** | **Yes — the flagship public API** |

The lint measures *public API* ergonomics, so annotating 140 internal methods
to satisfy it would be cargo-culting. Instead: the one real site
(`Error::column_index_out_of_bounds`) is now `#[must_use]`, the workspace
default stays `allow`, and `hyperdb-api/src/lib.rs` opts back in with a
crate-level `#![warn(clippy::must_use_candidate)]`. Verified the crate-level
attribute does override the workspace `allow`.

### The two doc lints were already clean

`missing_errors_doc` and `missing_panics_doc` measured **zero** sites each over
`--workspace --all-targets`. The Cargo.toml comment claiming "a large number of
sites" pending a post-1.0 docs pass was simply stale — the work had already
been done. Both promoted from `warn` to `deny` so they stay closed.

### `multiple_crate_versions`: 17 splits, none code-fixable

Still `allow`, but the note is now a measured list rather than a guess:
`base64`, `hashbrown`, `syn`, `getrandom`, `rand`, `rand_core`, `thiserror`,
`thiserror-impl`, `r-efi`, `wit-bindgen`, and the RustCrypto 0.10/0.11 cluster.
The `windows_*` triplet the old note cited is **gone** — dropped with the
`quinn` stack when `aws-lc-rs` was replaced by `ring`, so the provider fix
measurably improved this too.

### M-SINGLE-ITEM-PATH: clean

Checked because this is the last cheap moment — after 1.0.0 a path change is a
major bump. `hyperdb-api` exposes three public modules (`copy`, `pool`,
`grpc`), and none of their items are also re-exported at the crate root, so
every public item has exactly one path.

Gates after the audit: `fmt`, `clippy -D warnings`, `make doc`, `cargo deny`
all exit 0; `make test` 1519/1519.

**Phase 4 now needs only the benchmark gate (4.0) and the release steps
(4.1, 4.3).**

### Task 4.0 — benchmark gate

Same-session A/B on one box, one `hyperd`. Base is the merge-base with
`origin/main` (`28c8813`, edition 2021) in a separate worktree; branch is
this HEAD. Both sides built `--release` and driven by
`bench_ab/ab_driver.sh`, which **interleaves** base and branch runs
(base→branch→base→…) so thermal drift and background load land on both sides
instead of penalizing whichever ran last. Artifacts are archived per run,
since the suite overwrites `test_results/benchmark_suite.{md,json}` in place.

- Host: Darwin 26.6.2, Apple M3 Max (14 cores), 96 GB, rustc 1.98.0
- `hyperd` `0.0.26359.r07abb490` — the pin in `hyperd-version.toml`, arm64
  native, the same binary on both sides
- Rust suite: 100M rows per workload, 4 workers, **5 runs per side**
- Node bench: 10M rows, **15 runs per side**

**Verdict: no regression on any single-connection path.** Medians, branch vs
base, with each workload's own run-to-run spread for scale:

| Path | Δ | noise |
|---|---:|---:|
| `insert.bulk` sync `Inserter (HyperBinary)` | −0.4% | ±2.1% |
| `insert.bulk` async `AsyncArrowInserter` | −0.2% | ±10.1% |
| `query.full_scan` sync | −0.2% | ±2.3% |
| `query.full_scan` async | +0.6% | ±2.4% |
| `query.filtered` sync | −0.3% | ±2.4% |
| `query.filtered` async | +1.3% | ±5.0% |

The `× 4` parallel workloads are **not usable as an A/B signal on this box**:
their run-to-run spread is ±20–61% and the deltas run in both directions
(`query.full_scan × 4` +12.0%, `spawn_blocking+ChunkSender × 4` −11.3%), which
is contention jitter from 4 workers on a 14-core laptop. This is why the
methodology records single-connection figures only.

**One real regression, in the Node bindings.** `executeQueryColumnar` on a
1M-row scan went 0.072 s → 0.076 s (median +5.6%, mean +2.5%, ≈4 ms). At 15
runs per side the distributions are visibly shifted, not overlapping noise:
base clusters at 0.072, branch at 0.076. The cause is the Task 2.6 conversion
in `get_int32_column` — threading a `Result` through every element defeats the
auto-vectorization the previous `map(|&x| x as i32)` got, costing ~4 ns per
value. That is the deliberate price of not silently truncating `i64` → `i32`
on the JS boundary (reminder 7), and it is bounded and documented rather than
accidental. `Columnar Filtered` (100K rows) shows no measurable change,
and the write path that uses `narrow_i16`/`narrow_i32`
(`RowInserter`, −0.3%) is flat because 4.6 s of COPY dominates it.

**Fixed, by operator decision, in `3b8a5b4`.** Splitting the fused check into
a bounds scan plus a narrowing pass cut the cost from +5.6% to **+1.4%**,
which is inside the session-to-session drift measured on this box (the
pre-migration baseline itself moved 0.0720 → 0.0730 → 0.0740 across three
sessions as the machine warmed over ~90 minutes of benchmarking).

Two forms of the second pass were measured at 15 runs per side and are
**indistinguishable at +1.4% each**:

1. `i32::try_from(x).expect(...)`, relying on the scan for the invariant.
2. A bounds-justified `as i32` under an `#[expect(cast_possible_truncation)]`.

So the residual is not the per-element branch — it is the second read of the
slice. Form 1 ships, because it performs the same while needing no
`cast_possible_truncation` exception, which keeps reminder 7's posture intact.
This also corrects the original diagnosis: the first attempt assumed
`expect()` would restore vectorization, and variant 2 proved it does not.

Rejection behavior and the error message are unchanged, and `smoke.mjs`'s
narrowing tests still pass. Clippy caught a `manual_range_contains` on the
first draft of the bounds predicate, so the scan uses `!(LO..=HI).contains(&x)`.

**IPC could not be measured.** `BENCH_TRANSPORT=ipc` fails identically on
*both* sides — `hyperd` never creates the Unix socket the client dials
(`failed to connect to unix socket …/hyper-<pid>/domain/hyper: No such file or
directory`). Because it reproduces at the pre-migration baseline, the
migration cannot be responsible. macOS IPC has never been captured in
`BENCHMARK_GUIDE.md` (only the Windows Named Pipe section exists), so there is
no recorded state to have regressed from. Logged as pre-existing, separate
work — **no IPC numbers are claimed.**

Not logged to `docs/hyperd-release-benchmarks.md`: that table is keyed by
`hyperd` release, and the pin is unchanged here.

**`BENCHMARK_GUIDE.md`'s macOS section was refreshed** with the shipping
1.0.0 numbers, by operator decision. The previous entry was from 2026-05-02 on
`hyperdb-api` 0.1.0-rc.1, rustc 1.94, and macOS 26.4, and disagreed with this
box on *both* sides of the A/B — it claimed 18.81 M rows/s for the sync
full-scan where base and branch both measure ~31 M/s. Because that gap spans a
different `hyperd`, compiler, and OS, the guide now says explicitly that the
improvement is not a controlled comparison and cannot be pinned on any one
change.

Three things the refresh had to get right, which the old entry did not say:

- The `× 4` rows carry a ±20–61% spread on this host, so the table now warns
  to read them as order-of-magnitude and to compare only single-connection
  figures across releases.
- Three Node paths are **bimodal**, so the Node table is a median of 15 runs
  rather than 5. A 5-run sample put `executeQueryToArrow`-filtered at 0.178 s;
  15 runs put it at 0.005 s, with 10 of 15 in the fast mode. Publishing the
  5-run median would have shown a fabricated ~35× regression.
- The Rust-vs-Node table needed Rust at **10M** to match the Node bench, not
  the 100M used above it, so a separate 5-run 10M collection was taken.

That comparison also flipped: Node's `ArrowInserter` (41.3 M/s) now measures
*ahead* of Rust's `AsyncArrowInserter × 4` (37.8 M/s) at 10M. Verified as a
scale artifact, not JS beating native — Rust's parallel variant pays a fixed
4-worker setup cost and reaches 48.5 M/s at 100M. The Node harness's timed
region was read to confirm it is end-to-end (typed-array fill, Arrow build,
IPC serialize, execute), so the number is fair. The guide frames it as "the
Arrow path costs you nothing at 10M" rather than a language ranking.

## Previously deferred (now complete)

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
