# Rust 1.88 / Edition 2024 Uplift Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use
> `superpowers:subagent-driven-development` and execute this plan task by task.
> Steps use checkbox (`- [ ]`) syntax for tracking. The main thread owns plan
> revision, commits, final validation, and merge-readiness judgment.

**Goal:** Raise the workspace MSRV floor from a fictional 1.81 to a real 1.88,
migrate to edition 2024, adopt the language features that unlock (let chains,
`async` closures, precise capturing), and add a RHEL 9.7 `rust-toolset`
compatibility gate so enterprise consumers can build without `rustup`.

**Architecture:** No architectural change. This is a toolchain, edition, and
lint-posture migration plus a new CI job. The only public API change is the
addition of `use<..>` precise-capturing bounds on eight return-position
`impl Trait` signatures in `hyperdb-api` and `hyperdb-api-core`.

**Tech stack:** Rust workspace, edition 2021 to 2024; rustc 1.98.0 locally,
1.88.0 on RHEL 9.7; `napi` 3 / `ctor` 1.0.5 for the Node bindings; UBI9
container via Docker or Podman; real `hyperd` via
`HYPERD_PATH=~/dev/bin/hyperd`; Conventional Commits.

**Design specification:**
[`docs/superpowers/specs/2026-09-04-rust-188-edition-2024-design.md`](../../specs/2026-09-04-rust-188-edition-2024-design.md)

**Final integration base:** `origin/main` @ `28c8813`
**Branch:** `feat/upgrade-to-188-RHEL97`

---

## Execution log

Progress, captured exit codes, and discovered-but-out-of-scope findings live in
[`EXECUTION-LOG.md`](EXECUTION-LOG.md), kept separate so this plan stays a
stable brief rather than growing with every task.

---

**Prerequisite, already done:**
[`docs/RUST_GUIDELINES.md`](../../../RUST_GUIDELINES.md) was re-synced against
the Microsoft Pragmatic Rust Guidelines **version 2026.6** on 2026-09-04,
*before* this plan executes, so the code changes below conform to current
guidance rather than to a stale snapshot. That review
produced two corrections now folded into this plan — the Phase 1 commit type
(**M-MSRV**: an MSRV bump is a minor release, not breaking) and the resolver
bump in Task 1.3 (**M-LATEST-EDITION**). It also added an AI-assisted-development
section whose constraints bind every agent executing this plan.

---

## Global constraints

These apply to every task and every agent.

- Read and obey repository `AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`,
  [`docs/RUST_GUIDELINES.md`](../../../RUST_GUIDELINES.md), and
  [`docs/RUST_DOCUMENTATION_STYLE.md`](../../../RUST_DOCUMENTATION_STYLE.md)
  before editing. Search the whole repository before concluding an API, test,
  or documentation surface is absent.
- Hyper-backed commands use exactly `HYPERD_PATH=~/dev/bin/hyperd`. Never
  invent `hyperd` flags or engine parameters; confirm against `hyperd --help`,
  an existing script, or `AGENTS.md` first. Fabricated parameters make tests
  hang while appearing to run.
- A command is green only when its real output and zero exit status were seen.
  No output for roughly 30 seconds is a hang/failure requiring investigation,
  not a pass.
- **Do not hand-edit any crate version** or the root `CHANGELOG.md`.
  release-please owns both. User-visible changes get an `### Added` /
  `### Changed` bullet under `## [Unreleased]` in the *affected crate's* own
  `CHANGELOG.md`.
- Lint waivers are `#[expect(lint_name, reason = "...")]`, never bare
  `#[allow]`. A workspace-level relaxation additionally needs a row in the
  Exceptions table of [`docs/RUST_GUIDELINES.md`](../../../RUST_GUIDELINES.md).
  **M-LINT-OVERRIDE-EXPECT carves out generated code and macro output**, where
  `#[allow]` stays appropriate — which matters in Task 1.2's lint burn-down and
  in Task 2.6, since `hyperdb-api-node` is largely napi-generated surface and
  `hyperdb-api-derive` emits macro output. Do not mechanically rewrite
  `#[allow]` to `#[expect]` in generated or macro-emitted code: `#[expect]`
  warns when the lint does *not* fire, which is exactly the wrong behavior for
  code whose shape varies with the macro input.
- Narrowing integer `as` casts follow the four-way procedure in
  [`docs/RUST_GUIDELINES.md`](../../../RUST_GUIDELINES.md) — `try_from().ok()?`
  for tolerable failure, `try_from().expect("<reason>")` for a validated
  invariant, `#[expect]` for type-algebra-safe conversions, and `#[expect]`
  containing the word "reinterpret" for encode/decode bit-pattern pairs.
  Never introduce a new bare narrowing `as`.
- Commit types: `feat(toolchain)!:` for Phase 1, `refactor:` for genuinely
  internal Phase 2 work, `feat!:` for Tasks 2.2 and 2.3 (both touch public
  API), and **`ci:`** for all of Phase 3. `fix(ci):` would trigger an
  unintended patch release.
- **The `!` goes immediately before the colon.** `feat(toolchain)!:` is valid
  Conventional Commits; `feat!(toolchain):` is **not** — the standard
  `conventional-commits-parser` header regex release-please uses fails to
  match it at all, so the commit is treated as non-conventional and produces
  no changelog entry and no breaking-change note. That is the exact failure
  the `!` was added to avoid.
- **Phase 1 *is* marked breaking, and the commit type does not set the
  version.** Phase 4 cuts the release with a `Release-As: 1.0.0-rc.1` footer,
  which overrides release-please's computed bump entirely — so `!` is purely a
  changelog annotation here, and an MSRV plus edition raise deserves it.
  This is a deliberate departure from **M-MSRV** ("bumping MSRV does not
  require a major release"). M-MSRV's stated concern is that forcing a major
  bump "could possibly bifurcate downstream dependencies"; with few current
  API users and an intentional 1.0.0 stabilization, that motivation does not
  apply. Upstream's Golden Rule is spirit over letter.
- Commits on `main` must be signed.
- **M-NO-META-DESIGN-DOCUMENTATION.** Do not add design-journey prose to crate
  READMEs or rustdoc, and specifically **do not produce a self-report table**
  of which guidelines this migration satisfied — upstream names that exact
  artifact as an agent anti-pattern. Rationale belongs in the design spec.
- **M-TAUTOLOGICAL-TESTS.** Any test added while modernizing must assert a
  property, not restate a constant or mirror the branches of the code under
  test. This is a live risk in Phase 2, where mechanical rewrites invite
  mechanical tests.
- Run `make clean-test-files` before every commit so no `.hyper` or
  `hyperd*.log` artifact lands in a change.
- Developer/tester agents do not commit. The main thread stages explicit paths
  and makes each task's Conventional Commit after review.

## Model allocation

Per Harness doctrine (`~/.claude/agent-team-patterns.md`): judgment runs on
Opus, doing runs on Sonnet, and reasoning effort escalates only where being
wrong is expensive. Doer role profiles are `model: sonnet` precisely *because*
an Opus reviewer gates them — `doer != validator != merger`.

**Main thread, no subagent** — doctrine's "don't delegate mechanical edits;
main-thread Edit/Write is faster":

- Every small scoped manifest edit (Tasks 1.1, 1.2, the resolver bump).
- The `cargo fix --edition` sequencing in Task 1.3. Ordering is load-bearing
  and the resulting diff needs judgment, not delegation.
- Reconciling reviewer findings — doctrine assigns synthesis to the main
  thread explicitly.
- All commits (the publisher gate).

**Sonnet 5 (`claude-sonnet-5-thinking-high`)** — well-scoped production work
against explicit file:line targets, gated downstream by an Opus review:

- Task 2.1, let-chain flattening. Six named sites, mechanical rewrite.
- Task 2.5, the `is_none_or` cluster, the tuple collect, `split_at_checked`.
- Task 2.6's *non-cast* parts only: the `index.d.ts` / `smoke.mjs` / README
  updates if behavior changes, and deleting the dead nested
  `hyperdb-api-node/.github/workflows/CI.yml`.
- Task 1.4's match-ergonomics review of `columnar.rs:231-287`. Binding modes
  are compiler-checked, so a wrong answer fails loudly rather than silently.
- Tasks 3.2 through 3.4: README section, Makefile target, doc sweeps
  (`writer` / `doc-editor` roles, both `model: sonnet` in the role table).
- Any test-writing (`tester` role).

Note the only Sonnet slug available is the `-thinking-high` effort tier, so
these run at high effort rather than the default the doctrine would prefer for
mechanical stages. Still materially cheaper than Opus.

**Opus 5** — judgment, where being wrong is expensive:

- All reviewers, always (`reviewer.md` is `model: opus`).
- Task 1.4's RPIT hand-fixes. Lifetime-capture semantics on public APIs.
- Task 2.2's async-closure work, including the `dyn AsyncFn` object-safety
  question, which is a design call rather than a rewrite.
- Task 2.3, `use<..>` on public signatures.
- **Task 2.6's cast conversions.** Deliberately Opus despite the four-way
  decision procedure looking rule-driven. Picking the right branch requires
  deciding whether a JS-supplied `f64` can actually arrive out of range for
  the target type — that is domain judgment, not rule lookup. And this is the
  worst possible place to be wrong: these casts sit on the JS boundary, where
  a narrowing `as` silently wraps and writes corrupt data into a user's
  `.hyper` file rather than erroring. AGENTS.md reminder 7 calls this out as a
  documented source of data-corruption bugs in this codebase.
- Task 4.2's API audit — `must_use_candidate` is an explicit per-method
  judgment call.
- The final adversarial sweep, at max effort. Doctrine reserves the top tier
  for "the final adversarial sweep, reconciling contradictory reviewer
  findings, a security-sensitive review."

## Verification gate

[`CONTRIBUTING.md`](../../../../CONTRIBUTING.md) enumerates exactly what CI
enforces. All five run locally before any push, not just `cargo test`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps   # published crates
cargo deny check
cargo audit --deny warnings
```

The clippy line matches `.github/workflows/ci.yml:101` exactly, including
`--all-features` — a no-op today given `hyperdb-api` has no features, but it
will not be once the post-1.0.0 feature work lands. Note the `cargo doc` gate
is **not** a CI job; run it locally or via `make doc`.

Note that the clippy gate uses `--all-targets`, which is what makes the
`sysinfo` decision in Task 1.2 load-bearing.

---

## Phase 0 — De-risk the napi/edition-2024 interaction

> **RESULT: GREEN — executed 2026-09-04.** `hyperdb-api-node` built at
> `edition = "2024"` with the workspace still on 2021. Exit code **0**, zero
> errors, zero warnings, on a forced recompile (`touch src/lib.rs`) that
> re-expanded all 148 `#[napi]` attributes, producing a 51 MB
> `libhyperdb_api_node.dylib`. `ctor 1.0.5`'s macro-generated bare
> `#[no_mangle]` does survive expansion into an edition-2024 consumer, exactly
> as D3 argued. No `[patch.crates-io]` and no per-crate edition pin needed —
> the migration proceeds as planned. Spike reverted with zero diff.

**This runs first. A red result changes the shape of the entire migration.**

`hyperdb-api-node/src/` is clean: zero hand-written `#[no_mangle]`,
`#[export_name]`, `#[link_section]`, or `extern` blocks, and its only two
`unsafe {}` blocks (`inserter.rs:459` and `:475`, both `val.cast::<...>()`)
already carry `// SAFETY:` comments. `napi 3.12.1` does contain bare
`#[no_mangle]` in `bindgen_runtime/module_register.rs`, but `napi` is itself
`edition = "2021"`, so that compiles under its own edition and is irrelevant.

The single live risk is `ctor 1.0.5`, a transitive dependency of both `napi`
and `napi-derive`, which emits bare `#[no_mangle]` into the *consuming* crate
at `parse.rs:1024` and `:1050`.

### Task 0.1 — Prove `ctor` survives expansion into an edition-2024 cdylib

- [ ] In `hyperdb-api-node/Cargo.toml`, temporarily replace
      `edition.workspace = true` with `edition = "2024"`. Change nothing else,
      anywhere.
- [ ] Run `cargo build -p hyperdb-api-node 2>&1 | tail -40` and capture the
      real output plus exit status.
- [ ] **Green:** revert the override and proceed to Phase 1. Record the
      captured output in the task evidence log.
- [ ] **Red with `usage of unsafe attribute`:** stop and escalate. Options are
      `cargo update -p ctor` if a fixed release exists, a `[patch.crates-io]`
      stopgap, or keeping this crate on a literal `edition = "2021"` while the
      workspace moves to 2024. Editions are per-crate, so the last option is
      viable and costs only a documented comment.
- [ ] Confirm `git status --porcelain` is clean afterward. This spike must not
      leak a stray `edition = "2024"` into the branch.

---

## Phase 1 — Configuration and edition migration

### Task 1.1 — `rust-toolchain.toml` components

- [ ] Add `rust-analyzer` to `components`, keeping `channel = "stable"`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy", "rust-analyzer"]
profile = "minimal"
```

- [ ] Trim the now-obsolete "Editor Setup" workaround in `AGENTS.md`. A
      rustup-provisioned `rust-analyzer` component tracks the active toolchain,
      which is precisely what that section tells contributors to arrange by
      hand.

### Task 1.2 — MSRV floor (Blockers B1 and B2)

- [ ] `Cargo.toml:31` — `rust-version = "1.81"` becomes `"1.88"`.
- [ ] `clippy.toml:7` — `msrv = "1.81"` becomes `"1.88"`.
- [ ] `hyperdb-compile-check/Cargo.toml:13` — same bump.
- [ ] `hyperdb-api-node/Cargo.toml` — add the missing
      `rust-version.workspace = true`. It is the only member without one, and
      its locked `napi` / `napi-derive` already declare 1.88.
- [ ] **Resolve B1 by downgrading `sysinfo` to `"0.38"`** (decided
      2026-09-04). `Cargo.toml:50` currently pins `"0.39"`. Per crates.io,
      **0.38.4 is the newest release at `rust-version = 1.88`** — the whole
      0.38.x and 0.37.x lines are 1.88, and 0.39.0 is where the floor jumped
      to 1.95. This makes the 1.88 MSRV claim honest for *every* target, lets
      the Phase 3 RHEL job graduate to `--all-targets`, and lets Task 4.0's
      benchmark gate run at the floor rather than only on `stable`.

      Note 0.38.4 sits *exactly* at our floor with no slack, so a future
      `sysinfo` bump will need an MSRV decision alongside it.

      **Expect a small API break, not a clean version swap.**
      `hyperdb-api/benches/common.rs` uses `sysinfo::{ProcessesToUpdate,
      System}` (line 38), `sysinfo::get_current_pid()` (line 142), and
      `sysinfo::System::physical_core_count()` (line 258). That last one has
      moved between associated function and instance method across sysinfo
      releases, so verify it compiles rather than assuming. Fixing the bench
      harness is in scope for this task.
- [ ] Once the downgrade lands, promote the Phase 3 job from `cargo check
      --workspace --locked` to `--all-targets`, and delete the comment that
      explains why `--all-targets` was avoided.
- [ ] Burn down B2 in this same task — the `clippy` CI job runs
      `-- -D warnings`, so splitting the bump from the burn-down leaves `main`
      red. **Measured baseline (clippy with `msrv = "1.88"`, 2026-09-04): 15
      sites**, far smaller than feared, and `cargo clippy --fix` handles most:
      - **11 `unnecessary_map_or`** — the `map_or(true, ..)` to `is_none_or`
        cluster. Note this is more than the 5 sites listed in Task 2.5: it
        also includes `hyperdb-api-salesforce/src/provider.rs:366` and five
        more in `hyperdb-mcp/src/schema.rs` (`:560`, `:563`, `:564`, `:573`,
        `:574`).
      - **3 `manual_is_multiple_of`** — `hyperdb-api-core/src/client/row.rs:56`,
        `hyperdb-api/tests/stress_test/simulation.rs:420`,
        `hyperdb-api/examples/additional_examples/threaded_inserter.rs:218`.
      - **1 `manual_repeat_n`** — `hyperdb-mcp/tests/doctor_tests.rs:840`.

      `manual_midpoint` and `manual_let_else` were named in an earlier draft
      and do **not** fire; ignore them. Re-measure before starting rather than
      trusting this list, and expect a warning that `clippy.toml` and
      `Cargo.toml` MSRVs disagree until both are bumped together.
- [ ] **Add the changelog entries for the MSRV and edition raise.** This is
      the most user-visible change in the whole plan — any consumer on
      1.81 through 1.87 gets a hard build failure — and an earlier draft
      specified no changelog bullet for it at all. release-please only
      generates the *root* changelog, so without this the seven per-crate
      `CHANGELOG.md` files that crates.io and GitHub readers actually see
      would say nothing about an MSRV break. Add a `### Changed` bullet under
      `## [Unreleased]` in each of the seven crates AGENTS.md reminder 8
      enumerates, recording both the 1.88 floor and the 2024 edition.
- [ ] Do **not** touch `version = "0.7.3"`.

### Task 1.3 — Edition 2024 migration, in order

Order matters. `cargo fix --edition` reads the *current* edition from the
manifest and applies the migration lints for the next one; bumping the manifest
first makes it a no-op.

- [ ] Confirm a clean tree: `git status --porcelain` empty. `cargo fix` refuses
      otherwise.
- [ ] With both manifests **still on 2021**, run:

```bash
cargo fix --edition --workspace --all-targets
```

`--all-targets` is mandatory or tests, benches, and examples are silently
skipped.

- [ ] Run the excluded crate separately (Blocker B6). It builds a real Hyper
      database at compile time, so it may need `HYPERD_PATH`:

```bash
HYPERD_PATH=~/dev/bin/hyperd \
  cargo fix --edition --all-targets \
  --manifest-path hyperdb-compile-check/Cargo.toml
```

- [ ] Review the auto-fix diff before going further. If `cargo fix` bails on
      pre-existing `deny`-level diagnostics (expected, given
      `correctness = "deny"`), relax the offending level for the duration of
      the fix run rather than reaching for `--broken-code`.
- [ ] **Now** flip the edition: `Cargo.toml:30` and
      `hyperdb-compile-check/Cargo.toml:12` to `edition = "2024"`.
- [ ] **Bump the resolver — root manifest only.** `hyperdb-compile-check`
      needs no resolver edit: it is a *package* workspace root, so it infers
      v3 from its own edition flip. Only the virtual root manifest is
      affected. `Cargo.toml:2` sets `resolver = "2"`, which pins
      the workspace to the older resolver and blocks the v3 default that
      edition 2024 otherwise implies. Change it to `resolver = "3"`. It must
      stay explicit: the root manifest is *virtual* (a `[workspace]` with no
      `[package]`), and Cargo does not infer a resolver from the edition for
      virtual workspaces. Upstream **M-LATEST-EDITION** notes the field is
      "generally not needed" — that applies to normal crates, not this case.
      Resolver v3 is MSRV-aware, so it also makes future `cargo update` runs
      prefer dependency versions compatible with `rust-version = "1.88"`,
      which partially mitigates Blocker B1.
- [ ] `cargo build --workspace --all-targets` and capture output.

### Task 1.4 — Hand-fix what `cargo fix` cannot

- [ ] **RPIT capture — four sites, not eight, and `cargo fix` handles them
      here.** `cargo rustc -p hyperdb-api --lib -- -W impl_trait_overcaptures`
      reports exactly four, all in `hyperdb-api`:
      `async_connection.rs:438`, `async_connection.rs:566`,
      `connection.rs:840`, `connection.rs:1005`. The same run against
      `hyperdb-api-core` reports **zero**.

      The other four previously listed —
      `hyperdb-api-core/src/client/grpc/result.rs:82`, `:106`, `:160`, and
      `hyperdb-api/src/process.rs:1385` — do **not** over-capture. Each takes
      only `&self`, and its single elided lifetime already appears in the
      RPIT's `Item` type, so capture is identical in 2021 and 2024. An earlier
      draft of this plan claimed `result.rs:106` and `process.rs:1385` changed
      "silently"; that was backwards — they are the safest of the eight and
      need no edit.

      **Sequencing note:** `impl_trait_overcaptures` is part of the
      `rust-2024-compatibility` group that `cargo fix --edition` enables, and
      its suggestion is `MachineApplicable` (`suggested_replacement =
      " + use<'a, T>"`). So `cargo fix` in the previous step **will already
      have rewritten these four public signatures**. Do not plan to apply
      them again in Task 2.3 — verify instead, and see the changelog step
      below, since a public-API change now lands inside the Phase 1 commit.
- [ ] **`if_let_rescope` / `tail_expr_drop_order` — generate the inventory,
      do not grep for it.** This is the plan's only silent-behavior-change
      risk, and an earlier hand-built list was both misdirected and roughly
      three times too small. Ask the compiler, per crate:

```bash
for c in hyperdb-api-core hyperdb-api hyperdb-api-node hyperdb-mcp \
         hyperdb-api-salesforce sea-query-hyperdb hyperdb-bootstrap; do
  echo "=== $c ==="
  cargo rustc -q -p "$c" --lib -- -W if_let_rescope -W tail_expr_drop_order
done
```

      Then repeat with `--all-targets` coverage for tests, benches, examples.
      Measured lib-target baseline (2026-09-04): **84 sites** —
      `hyperdb-api-core` 31 + 4, `hyperdb-api` 21 + 9, `hyperdb-api-node`
      7 + 1, `hyperdb-mcp` 6 + 1, `hyperdb-api-salesforce` 4 + 0.
      Note `hyperdb-api-core` carries the largest share and holds the wire
      protocol.

      **Why grepping misleads here:** `if let Ok(x) = m.lock()` *moves* the
      guard into the binding, so no temporary survives and drop order is
      unchanged. Most textual `if let Ok(..) = ..lock()` sites therefore do
      not fire — of nine such `hyperdb-mcp` lines previously listed, only
      [watcher.rs:617](../../../../hyperdb-mcp/src/watcher.rs) actually does.
      Conversely, real firing sites look nothing like a lock: `OwnedFd` plus
      `thread::spawn` in an accept loop (`daemon/health.rs:139`), `Bytes`
      temporaries in `while let Some(chunk) = ...next_chunk()` streaming
      loops (`engine.rs:915`, `:948`), a tokio `JoinSet` (`server.rs:2490`),
      and a `semver::Identifier` (`diagnostics.rs:1628`).

- [ ] **Triage each generated site by whether the reordered type's `Drop` has
      observable side effects** — locks released, channel sends, process
      handles, file descriptors. Prioritize guard-interaction sites:
      `watcher.rs:617` reorders a `MutexGuard` relative to an
      `InFlightGuard` that has a custom `Drop` (declared at
      `watcher.rs:359`), which is exactly the class rustc warns about.
- [ ] Budget real time for this. Both lints are allow-by-default,
      `cargo fix --edition` has **no** machine-applicable fix for
      `tail_expr_drop_order`, and edition 2024 stops mentioning them once
      migrated — so `make test` is the only backstop, and drop-order bugs are
      precisely what integration tests miss.
- [ ] **Match-ergonomics inventory, 104 sites** (70 `ref`, 34 `ref mut`).
      Mostly benign. Review `hyperdb-api-node/src/columnar.rs:231-287` as a
      single block; see Task 2.6.
- [ ] **Never-type fallback, 24 sites.** `unwrap_or_else(|_| panic!(...))`,
      overwhelmingly in tests. Skim only.

### Task 1.5 — Documentation and config consistency

- [ ] `docs/RUST_GUIDELINES.md` — **both previously cited line numbers are
      stale and one instruction was outright wrong.** The file was re-synced
      after this plan was drafted, shifting everything. Current state:
      - Line **53** (cited here as `:26`) reads "`#[expect]` has been
        available since Rust 1.81, well below our MSRV." **No change needed**
        — 1.81 is `#[expect]`'s *stabilization* version, a fact independent of
        our MSRV. Changing it to 1.88 would make the doc wrong.
      - Line **59** (cited as `:31`) — the M-OOBE row **already** points at
        `.github/workflows/rhel-compatibility.yml`. Already done. It is inline
        code rather than a link precisely so it does not dangle before Phase 3
        lands; keep it that way.
      - The Exceptions table is now at line **180** (cited as `:113`).
      Re-derive any reference before editing; this file keeps moving.
- [ ] `AGENTS.md` reminder 8 anchor and the release-please staleness — both
      **already fixed** on this branch (the anchor now points at
      `#what-contributors-do`, and the "no workflow invokes Release Please"
      claim is corrected). Verify rather than redo.
- [ ] `CONTRIBUTING.md:201` — "Contributors do **not** edit `CHANGELOG.md`
      files by hand" still contradicts AGENTS.md reminder 8 and this plan.
      Decision D6 resolved it by reading; close it in the text by scoping the
      sentence to the root changelog.
- [ ] `CONTRIBUTING.md:216` and `docs/GITHUB_OPERATIONS.md:26` — both claim
      release-please "tags the merge commit and creates the GitHub Release."
      It does not: `skip-github-release: true` is set at both levels of
      `release-please-config.json` and the tag is manual. These stale claims
      misled an earlier draft of Task 4.1 into omitting the tag step entirely.
- [ ] `.cargo/config.toml:5` — the `download-hyperd` alias references
      `--package hyperd-bootstrap --bin hyperd-bootstrap`, but the crate is
      `hyperdb-bootstrap`. `cargo download-hyperd` is currently broken.
      Opportunistic fix while in the file.
- [ ] **Delete the dead `test-redirect` Makefile target.** The `redirect`
      feature no longer exists on any crate, so
      `cargo test -p hyperdb-api --features redirect` fails outright. Two
      edits, and the footprint is exactly this — `build.ps1` has no
      equivalent, no Rust code carries `cfg(feature = "redirect")`, the `help`
      block never listed it, and it was never in `.PHONY`:
      - Remove `Makefile:128-132` (the comment `# Run tests with redirect
        feature enabled`, the `test-redirect:` target, and its three recipe
        lines).
      - Remove the `test-redirect` word from the `NEED_AUTO_DOWNLOAD`
        dependency list at `Makefile:29`, leaving the other targets intact.

      Verify with `make help` (should still parse) and
      `make -n test` (auto-download dependency still wired).

---

## Phase 2 — Codebase modernization

Sequenced deliberately: Tasks 2.2 through 2.6 need only the MSRV bump, but
**Task 2.1 requires edition 2024** and must follow Phase 1.

### Task 2.1 — Let chains (1.88, edition 2024 only)

Roughly 109 nesting candidates exist; 18 are depth 3 or deeper. Highest value
first, ranked by nesting removed:

- [ ] `hyperdb-api-core/src/client/async_client.rs:1016` — `extract_row_count`,
      depth 4 to 1. Collapses `Message::CommandComplete`, tag, `last`, and
      parse into one chain. The clearest win in the codebase.
- [ ] `hyperdb-api-core/src/client/grpc/authenticated_client.rs:980` and
      `:1073` — `get_table_labels` / `get_column_labels`, depth 4 to 1. Arrow
      batch plus two `downcast_ref` plus per-row JSON parse.
- [ ] `hyperdb-api-core/src/client/connection.rs:980` and
      `hyperdb-api-core/src/client/async_connection.rs:750` — `finish_copy`,
      depth 3 to 1, sync and async twins of the same COPY-complete parse.
- [ ] `hyperdb-api-core/src/client/client.rs:871` — `exec` command-tag row
      count, depth 3 to 1.
- [ ] `hyperdb-mcp/src/watcher.rs:875`, `hyperdb-mcp/src/diagnostics.rs:643`,
      `hyperdb-mcp/src/ingest.rs:1489` — depth 3 to 1 and depth 2 to 1.

`hyperdb-api-salesforce/src/provider.rs:215` and `:278` look like let-chain
candidates but read better as `is_some_and`. Handle them in Task 2.5.

### Task 2.2 — Async closures (1.85)

Concentrated almost entirely in one file, which makes this high leverage.

**Run this as a spike with a go/no-go gate, like Phase 0. It is likely a
NO-GO, and the failure mode is dangerous.**

**Guideline basis: M-ASYNC-FN** — prefer `async fn foo()` over
`fn foo() -> impl Future` where both are viable, with an explicit `Future`
return justified only inside traits or under stack-size pressure. The pool's
`HookFuture` alias is the shape upstream argues against.

**But the trait carve-out almost certainly applies, because the `Send` bound
is not expressible on stable.** The hook types guarantee `Send` futures
(`HookFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>`).
Requiring an `AsyncFn`'s returned future to be `Send` fails to compile:

```text
error[E0277]: `<F as AsyncFnMut<(&Conn,)>>::CallRefFuture<'_>`
              cannot be sent between threads safely
 = help: the trait `Send` is not implemented for
         `<F as AsyncFnMut<(&Conn,)>>::CallRefFuture<'_>`
```

`AsyncFnMut::CallRefFuture` sits behind unstable `async_fn_traits`, and
return-type notation (`F(..): Send`) is also unstable. So an `AsyncFn`'s
future cannot be boxed as `dyn Future + Send`, which is what the pool needs.

> **HARD CONSTRAINT.** Do **not** remove `+ Send` from `HookFuture`,
> `AfterConnectHook`, `BeforeAcquireHook`, or `RecycleCheck` to make an
> `AsyncFn` signature typecheck. That is the obvious way to get the code
> compiling and it is a silent thread-safety regression in a connection pool
> used from multi-threaded tokio runtimes. If `+ Send` is in the way, the
> answer is NO-GO, not a weaker bound.

**On NO-GO:** keep `HookFuture` and every `Send` bound exactly as they are,
record M-ASYNC-FN's trait carve-out as the justification, and limit the task
to the call-site and documentation cleanup below. That is a real if smaller
win — it stops the rustdoc from teaching `Box::pin`.

- [ ] `hyperdb-api/src/pool.rs:125` — the `HookFuture` alias
      (`Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>`) that forces the
      ceremony.
- [ ] `hyperdb-api/src/pool.rs:133`, `:141`, `:147` — `AfterConnectHook`,
      `BeforeAcquireHook`, `RecycleCheck`.
- [ ] `hyperdb-api/src/pool.rs:382` and `:398` — `after_connect` /
      `before_acquire` bounds, candidates for
      `F: for<'a> AsyncFn(&'a AsyncConnection) -> Result<()>`.
- [ ] `hyperdb-api/src/pool.rs:101` — the rustdoc example currently *teaches*
      `.after_connect(|conn| Box::pin(async move {`. Update it; docs are how
      this pattern spreads.
- [ ] `hyperdb-api/tests/pool_tests.rs:143` — first concrete call-site cleanup.

**This task changes public API, so it is `feat!:`, not `refactor:`.**
`HookFuture`, `AfterConnectHook`, `BeforeAcquireHook`, and `RecycleCheck` are
all `pub type` aliases; `PoolConfig::after_connect` and `::before_acquire` are
`pub` fields typed with them (`pool.rs:225`, `:227`); and
`RecycleStrategy::Custom(RecycleCheck)` is a public enum variant
(`pool.rs:169`). Add a `### Changed` bullet under `## [Unreleased]` in
`hyperdb-api/CHANGELOG.md` for whatever ships — including a docs-only change,
since the rustdoc example is public API guidance.

### Task 2.3 — Precise capturing `use<..>` (1.82): verify, don't apply

`cargo fix --edition` in Task 1.3 already applied these (see the RPIT bullet in
Task 1.4). This task is now a review pass over four signatures, not eight.

- [ ] Confirm `cargo fix` produced `+ use<'a, T>` on the four real
      over-capture sites: `hyperdb-api/src/connection.rs:840` and `:1005`
      (`Result<impl Iterator<Item = Result<T>> + use<'a, T>>`), and
      `hyperdb-api/src/async_connection.rs:438` and `:566`
      (`impl Stream<Item = Result<T>> + use<'a, T>`).
- [ ] Confirm nothing was applied to `hyperdb-api-core` — it has zero
      over-capturing sites, and `grpc/result.rs` / `process.rs::iter` must be
      left alone.
- [ ] Consider keeping `+ 'a` alongside `+ use<'a, T>`. The compiler should
      derive `Opaque: 'a` from the capture list given `T: 'a`, but the
      explicit outlives bound is free insurance for a caller that stores the
      iterator behind a `: 'a` bound.
- [ ] Add one `### Changed` bullet under `## [Unreleased]` in
      **`hyperdb-api/CHANGELOG.md`** only. Not `hyperdb-api-core` — it has no
      public API change here, and announcing one in a crate positioned as
      "forever internal" would be actively misleading. Not the root
      `CHANGELOG.md`, ever.
- [ ] If the bullet lands in the Phase 1 commit instead (because that is
      where `cargo fix` applied the change), that is fine — Phase 1 is already
      `feat(toolchain)!:`. Just do not double-record it.

### Task 2.4 — Safety and strict provenance: nothing to do

Recorded so it is not re-investigated. The scan found **zero** integer-to-pointer
casts, **zero** `usize as *const/*mut`, and **zero** `mem::transmute` on
pointers or integers, so no `ptr::with_exposed_provenance` migration exists.
`hyperdb-compile-check/src/db.rs:123` already uses `std::ptr::from_ref`.
`unsafe_op_in_unsafe_fn` likewise needs no work: both `unsafe fn`
(`hyperdb-mcp/src/paths.rs:207` and `:214`) already wrap their operations,
because `Cargo.toml:83` has denied the lint all along.

- [ ] No action. Confirm the above still holds at execution time and move on.

### Task 2.5 — API and iteration modernization

- [ ] **`Option::is_none_or` (1.82), the real cluster.**
      `hyperdb-api-core/src/client/row.rs:155` and `:517`,
      `hyperdb-api/src/process.rs:357`, `hyperdb-mcp/src/daemon/run.rs:247`,
      `hyperdb-mcp/src/schema.rs:559`. The Task 1.2 clippy MSRV bump surfaces
      these automatically.
- [ ] **`is_some_and`** for `hyperdb-api-salesforce/src/provider.rs:215` and
      `:278`.
- [ ] **Tuple `FromIterator` (1.85), one genuine hit.**
      `hyperdb-mcp/src/engine.rs:945-968` in `execute_chart_query_to_json`
      pushes `rows_json` and `measures` in the same loop; collect into
      `(Vec<_>, Vec<_>)`. The superficially similar pairs at
      `hyperdb-api-core/src/client/prepare.rs:378` and
      `async_connection.rs:866` are filled from *different* message arms, not
      parallel iteration — leave them.
- [ ] **`split_at_checked`** for the length-check-then-index pattern at
      `hyperdb-api-core/src/protocol/types.rs:148` (`i16_from_hyper_binary`)
      and `:317` (`text_from_hyper_binary`). Leave the deliberate typmod
      packing at `types/sql_type.rs:926` alone.

### Task 2.6 — `hyperdb-api-node` (Node.js bindings)

Assuming Task 0.1 is green, this crate needs no edition-specific source
changes. What it needs is the bookkeeping its own
[`AGENTS.md`](../../../../hyperdb-api-node/AGENTS.md) mandates.

- [ ] **Cast cleanup**, the densest narrowing-cast debt in the repo and the
      highest corruption risk, because it sits on the JS boundary where an
      out-of-range value silently wraps instead of erroring:
      `inserter.rs:343`, `:353`, `:358` (`*n as i16`), `inserter.rs:425`,
      `:445`, `result.rs:171`, `:179`, `:187`, `:195` (`as i32` from
      i64/f32/f64/Numeric), `result.rs:331` (date millis to days),
      `columnar.rs:89`, `:94`, `:133`, and `prepared.rs:504`.
      JS numbers arrive as `f64`, so most take the *first* branch of the
      guidelines procedure — `try_from(...).map_err(...)?` surfacing a
      `napi::Error` to JS — not an `#[expect]`. The `query_stats.rs:108-120`
      `v as f64` casts are the opposite case: lossy but intentional, so
      `#[expect(clippy::cast_precision_loss, reason = "...")]`.
- [ ] **Review `columnar.rs:231-287`** as one block: ten `ref mut` sites, all
      `if let ColumnData::Int32(ref mut v) = columns[col_idx]`.
- [ ] **These changes are JS-visible by construction, so this follow-through
      is unconditional** — not "if behavior changes." Roughly 14 sites move
      from silently wrapping to surfacing a `napi::Error`, which any JS caller
      can observe. Update the hand-written `index.d.ts`, add cases to
      `__test__/smoke.mjs` covering the new throw paths, update the
      type-mapping table in `hyperdb-api-node/README.md`, and add an
      `### Changed` bullet to `hyperdb-api-node/CHANGELOG.md` under
      `## [Unreleased]`.
- [ ] **Do not regenerate `index.d.ts`.** Its header reads
      `/* auto-generated by NAPI-RS */`, but `hyperdb-api-node/AGENTS.md:102`
      and `DEVELOPMENT.md` both state it is hand-written and must be updated
      manually. It is 722 tracked lines. Reconcile the contradiction in the
      docs as part of this task.
- [ ] **Delete or mark `hyperdb-api-node/.github/workflows/CI.yml` as dead.**
      GitHub only loads workflows from the repository-root `.github/workflows/`,
      so it never runs. It pins `dtolnay/rust-toolchain@stable` and declares a
      six-target matrix against the three triples the live
      `npm-build-publish.yml` actually builds. A stale toolchain pin sitting in
      the tree during a toolchain migration is a trap for the next reader.

---

## Phase 3 — Enterprise CI and the RHEL 9.7 hybrid strategy

All Phase 3 commits use the **`ci:`** type.

### Task 3.0 — Probe the container before writing the workflow

AGENTS.md reminder 9 forbids relying on unverified tooling. `rust-toolset` and
`protobuf-compiler` availability in the UBI *repository subset* is the go/no-go.

- [ ] Run and record the real output:

```bash
docker run --rm --platform linux/amd64 \
  registry.access.redhat.com/ubi9/ubi:latest \
  bash -lc 'dnf -q list --available rust-toolset protobuf-compiler \
    fontconfig-devel clang mold git 2>&1 | tail -20'
```

- [ ] If `protobuf-compiler` is absent, work the fallback ladder in order:
      enable `ubi-9-codeready-builder`, then set `PROTOC` to a pinned `protoc`
      release tarball, then scope the job to crates that do not invoke
      `tonic_prost_build`.

### Task 3.1 — `.github/workflows/rhel-compatibility.yml`

Two design details are load-bearing and must survive review: the job strips
**both** developer-convenience override files, and it uses plain `cargo check`
rather than `--all-targets`.

- [ ] Create the workflow:

```yaml
name: rhel-compatibility

on:
  push:
    branches: [main]
    paths: ['**/*.rs', '**/Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
            '.github/workflows/rhel-compatibility.yml']
  pull_request:
    paths: ['**/*.rs', '**/Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
            '.github/workflows/rhel-compatibility.yml']
  workflow_dispatch:

jobs:
  rhel-native:
    name: RHEL 9.7 rust-toolset (no rustup)
    runs-on: ubuntu-latest
    timeout-minutes: 45
    container:
      image: registry.access.redhat.com/ubi9/ubi:latest
    steps:
      - name: Install system-native Rust toolchain
        run: |
          dnf install -y rust-toolset protobuf-compiler fontconfig-devel git
          dnf clean all

      - uses: actions/checkout@v7

      - name: Prove we are on the distro compiler, not rustup
        run: |
          command -v rustup && { echo "rustup unexpectedly present"; exit 1; } || true
          which cargo rustc
          rustc --version   # expect 1.88.0
          cargo --version

      - name: Remove rustup-only override files
        # rust-toolchain.toml is honored only by rustup's proxy shims, so
        # /usr/bin/cargo ignores it -- deleting it is defensive, and it makes
        # the "no rustup" contract explicit and self-documenting.
        # .cargo/config.toml is NOT optional to remove: it pins
        # linker = "clang" + -fuse-ld=mold for x86_64-unknown-linux-gnu, and
        # UBI9 has neither. cargo check still links build scripts and
        # proc-macro crates for the host, so this breaks the job otherwise.
        run: rm -f rust-toolchain.toml .cargo/config.toml

      - name: cargo check (system toolchain)
        # Deliberately NOT --all-targets: the sysinfo 0.39 dev-dependency
        # declares rust-version = 1.95 and would hard-error on 1.88.
        # Promote to --all-targets once sysinfo is downgraded.
        run: cargo check --workspace --locked
```

- [ ] Land it as a **non-required** check. Promote to required once it has been
      green for several runs.

### Task 3.2 — `README.md` Enterprise Compatibility section

- [ ] Insert as a peer section after `## Platform Support` (which ends at
      `README.md:339` with the MSRV line) and before `## Documentation` at
      `README.md:341`. Platform Support already owns the OS matrix and the MSRV
      pointer, so enterprise content belongs immediately after it.

```markdown
## Enterprise Compatibility

This crate builds on Red Hat Enterprise Linux 9.7 with **no rustup required**.
RHEL 9.7 ships Rust Toolset 1.88.0 in AppStream, which matches this
workspace's MSRV exactly and provides stable Rust 2024 Edition support:

    dnf install -y rust-toolset protobuf-compiler
    cargo build --release

Notes for system-toolchain builds:

- **MSRV is 1.88** (`rust-version` in `Cargo.toml`), chosen to match RHEL 9.7's
  Rust Toolset. Local development uses floating `stable` via
  `rust-toolchain.toml`; that file is only read by rustup's proxy shims and is
  ignored by a distro-packaged `cargo`.
- **`.cargo/config.toml` is a developer convenience**, not a build requirement.
  It selects clang + mold on `x86_64-unknown-linux-gnu` for faster linking.
  Remove or override it when building with the system toolchain.
- **`protoc` is required** because `hyperdb-api-core` generates gRPC bindings
  at build time via `tonic-prost-build`.
- Compatibility is enforced on every PR by
  [`.github/workflows/rhel-compatibility.yml`](.github/workflows/rhel-compatibility.yml),
  which builds in a `ubi9/ubi` container using only `dnf install rust-toolset`.
```

### Task 3.3 — `Makefile` `check-rhel` target

- [ ] Insert after `verify-hyperd-pin` (`Makefile:145-148`) and before
      `npm-pack` (`Makefile:150`), matching the existing kebab-case naming.
- [ ] Register `check-rhel` in the `.PHONY` list at `Makefile:1` and in the
      `help` block at `Makefile:33`.

```makefile
# Validate that the workspace compiles with RHEL 9.7's system-native
# rust-toolset (1.88.0) and no rustup. Mirrors
# .github/workflows/rhel-compatibility.yml.
#
# --platform linux/amd64 is REQUIRED, not cosmetic: the clang+mold linker
# override in .cargo/config.toml is scoped to x86_64-unknown-linux-gnu, so an
# arm64 container on Apple Silicon would never exercise it and would report a
# false pass while CI fails on GitHub's x86_64 runner.
CONTAINER_ENGINE ?= $(shell command -v podman 2>/dev/null || command -v docker 2>/dev/null)

check-rhel:
	@test -n "$(CONTAINER_ENGINE)" || \
		{ echo "ERROR: neither podman nor docker found on PATH"; exit 1; }
	@$(CONTAINER_ENGINE) info >/dev/null 2>&1 || \
		{ echo "ERROR: container daemon not reachable (on macOS: 'colima start')"; exit 1; }
	@echo "==> RHEL 9.7 rust-toolset check via $(CONTAINER_ENGINE) (linux/amd64)"
	$(CONTAINER_ENGINE) run --rm -t \
		--platform linux/amd64 \
		-v "$(CURDIR)":/src \
		-w /src \
		-e CARGO_TARGET_DIR=/tmp/rhel-target \
		registry.access.redhat.com/ubi9/ubi:latest \
		bash -euo pipefail -c '\
			dnf install -y -q rust-toolset protobuf-compiler fontconfig-devel git; \
			rustc --version; \
			cp -a /src /build && cd /build; \
			rm -f .cargo/config.toml rust-toolchain.toml; \
			cargo check --workspace --locked'
```

**Why it copies the tree instead of using `--config` overrides.** An earlier
draft passed `--config "target.x86_64-unknown-linux-gnu.rustflags=[]"` to
neutralize the mold flag without touching the developer's files. That
provably does not work: Cargo **joins** `rustflags` array values across
config sources rather than replacing them, so `rustflags=[]` is a no-op and
`-C link-arg=-fuse-ld=mold` still reaches a `cc` that has no mold. (The
`linker=cc` half does work — strings take precedence — which makes the
failure look half-fixed.) Copying to `/build` and deleting both override
files mirrors the CI job, which is the proven path, and it means the
read-write `/src` mount is never modified.

`CARGO_TARGET_DIR=/tmp/rhel-target` keeps x86_64 container artifacts from
clobbering the host `target/`. Podman users on SELinux hosts may need `:Z` on
the volume mount.

A `Check-Rhel` mirror in `build.ps1` is optional — the target validates
Linux-native compilation, so Windows parity adds little.

### Task 3.4 — Close the documentation gap

- [ ] The M-OOBE row in `docs/RUST_GUIDELINES.md` (now line **59**, not `:31`)
      **already** names `.github/workflows/rhel-compatibility.yml` as its
      enforcement — done on this branch, verify only. RHEL's 1.88 toolset *is*
      the MSRV, so this workflow is the missing M-OOBE check; a redundant
      `cargo +1.88 check` job is not needed. Leave the reference as inline
      code rather than a link, so it does not dangle before this phase lands.
- [ ] Add rows to the Exceptions table (now line **180**, not `:113`) for any
      workspace-level lint relaxation introduced by Task 1.2. Given the
      measured B2 baseline is only 15 auto-fixable sites, expect none.

---

## Phase 4 — Release as 1.0.0, via release candidates

Phases 0 through 3 land on `main` **unreleased**. Phase 4 then cuts a single
`1.0.0-rc.1` representing the whole migration, gates promotion on a public-API
audit, and releases `1.0.0`.

No new machinery is needed — this is all already wired:

- `docs/GITHUB_OPERATIONS.md` documents the `Release-As:` footer, and
  release-please produces a release PR at that exact version on its next run.
- `hyperdb-api/Cargo.toml:16-18` wraps the `hyperdb-api-core` exact-match pin
  in `# x-release-please-start-version` markers, so `version = "=0.7.3"`
  correctly becomes `="1.0.0-rc.1"`. Same markers exist in `hyperdb-api-core`,
  `hyperdb-api-derive`, `hyperdb-mcp`, and `hyperdb-compile-check`.
- `.github/workflows/npm-build-publish.yml:305-340` validates
  `^[0-9]+\.[0-9]+\.[0-9]+(-(rc|alpha|beta)\.[0-9]+)?$` and maps `*-rc.*` to
  npm dist-tag `rc`, so `npm install hyperdb-api-node` will not pull a
  prerelease. The version comes from the git tag, not `package.json`.
- crates.io behaves equivalently: `cargo add hyperdb-api` skips prereleases
  unless a user opts in with `@1.0.0-rc.1`.

### Task 4.0 — Benchmark regression gate (before cutting the RC)

Nothing ships until a fresh benchmark run shows no performance regression
against pre-migration `main`. This runs after all of Phases 1 through 3 are
merged and before Task 4.1.

**Use a same-session A/B baseline, not the numbers already in the docs.**
This is the methodology `docs/hyperd-release-benchmarks.md` already follows —
it records "Same-session 0.0.25080 A/B baseline: 27.17 / 26.12 / 31.33" and
warns that "the insert path carries cold-start variance," so cross-session
comparison against a published table is not trustworthy on a thermally
throttled laptop. Measure the old and new builds back to back, same machine,
same session.

- [ ] Benchmark pre-migration `main` first, from a clean worktree at
      `28c8813` (the plan's integration base), in **release mode** — debug
      builds are not representative (AGENTS.md reminder 5):

Use the Methodology the tracker mandates, **not** the suite's defaults:
median of **≥3 runs at 100M rows**, TCP, 4 workers, and single-connection
figures only. `docs/hyperd-release-benchmarks.md` is explicit that a 10M-row
run "is too short to distinguish signal from variance," and that multi-connection
(`× 4`) workloads throttle thermally on laptops and are excluded from headline
deltas.

```bash
export HYPERD_PATH=~/dev/bin/hyperd

# 100M rows per workload, 4 workers. Run at least 3 times; take the median.
cargo run -p hyperdb-api --release --example benchmark_suite -- 100000000 4
```

(The benchmarks live in `hyperdb-api/benches/` but are registered as
`[[example]]` targets — the manifest comment says "registered as examples for
easy `cargo run`" — so `--example` is correct and there are no `[[bench]]`
targets.)

- [ ] Save the baseline artifacts before they are overwritten — the suite
      writes `test_results/benchmark_suite.md` and
      `test_results/benchmark_suite.json` on every run:

```bash
cp test_results/benchmark_suite.md   /tmp/bench-baseline-28c8813.md
cp test_results/benchmark_suite.json /tmp/bench-baseline-28c8813.json
```

- [ ] Re-run the identical command on the migrated branch, same session.
- [ ] Repeat both legs with `BENCH_TRANSPORT=ipc` — the default is TCP, and
      the IPC path is what most local consumers actually use.
- [ ] **Run the Node.js bench too.** It is a separate harness
      (`hyperdb-api-node/__test__/benchmark.mjs`, via `npm run build && npm
      run benchmark`) and it is the *only* one that exercises Task 2.6's cast
      conversions. Skipping it would miss the most likely regression.

**Where regressions would plausibly come from**, in rough order of risk:

- **Task 2.6's `as` to `TryFrom` conversions** on the JS boundary. Each adds
  a branch per value in insert and read paths, at roughly 14 sites. This is
  the top suspect and only the Node bench sees it.
- **Task 2.5's `split_at_checked`** in `hyperdb-api-core/src/protocol/types.rs`
  — per-value wire decode, genuinely hot.
- **Edition 2024 drop-order changes.** Locks and guards releasing at
  different points can shift throughput in the concurrent insert and parallel
  query workloads, which is a second reason to take Task 1.4's triage
  seriously.
- **Dependency movement.** If Blocker B1 was resolved by downgrading
  `sysinfo`, or if resolver v3 changed any resolved version, codegen changed
  with it.
- Let chains, `is_none_or`, and `use<..>` should all be neutral — same
  branching, or type-level only with no codegen effect. A delta there points
  at measurement noise rather than the change.

**Judging the result:**

- [ ] Compare the insert table (`Inserter` sync, `ChunkSender` sync,
      `AsyncArrowInserter`) and the query table (`full_scan` and `filtered`,
      sync and async) leg by leg.
- [ ] Treat a marginal delta as unproven rather than real: re-run the baseline
      leg a second time and see whether the gap survives. The tracker's own
      notes call small insert deltas "soft" for exactly this reason.
- [ ] Any sustained regression on a dominant path blocks the RC until it is
      explained. "Explained" is a permissible outcome — a documented, accepted
      cost is fine; an unexplained one is not.
- [ ] Attach both A/B artifact pairs to the release PR as the evidence record.
- [ ] **Record the 1.0.0 row in
      [`docs/hyperd-release-benchmarks.md`](../../../hyperd-release-benchmarks.md)**
      — one insert row and one query row. This is the whole point of the run:
      that row becomes the baseline the *next* engine bump measures against,
      and a new `hyperd` pin is already planned for 1.0.1. Without an API-only
      row here, the 1.0.1 A/B would compare new-engine-plus-migrated-API
      against old-engine-plus-pre-migration-API and report the sum as an
      engine delta.

      Repeat the `Release` and `Build` values from the row above — hyperd does
      not change in 1.0.0 — and set the new `API` column to `1.0.0`. In Notes,
      say what the migration changed and which paths could plausibly move, so
      a later reader can distinguish an accepted cost from an unexplained one.
      The tracker's "How to add a row" section covers this trigger.
- [ ] If numbers shift materially, also refresh the relevant platform table
      under "Results by platform" in `docs/BENCHMARK_GUIDE.md`.

**Confirmed coupling to Blocker B1.** `benchmark_suite.rs` does `mod common;`,
`hyperdb-api/benches/common.rs` uses `sysinfo` (lines 38, 142, 258), and
`sysinfo` is a dev-dependency of `hyperdb-api` — which examples build against.
So **this gate cannot run on a 1.88 toolchain** unless B1 was resolved by
downgrading `sysinfo`. Running it on local `stable` is fine and is what the
tracker's existing rows did, but it means the benchmark gate never exercises
the MSRV floor. Worth stating in the tracker row's Notes.

### Task 4.1 — Cut `1.0.0-rc.1`

- [ ] Confirm Phases 0 through 3 are all merged, the five-command gate plus
      the RHEL job are green on `main`, and **Task 4.0's benchmark gate
      passed** with its A/B artifacts recorded.
- [ ] Land a commit on `main` carrying the footer:

```text
Release-As: 1.0.0-rc.1
```

- [ ] Review the release-please PR. Verify it rewrites the workspace version,
      every `x-release-please` marked pin (especially the `=` pin in
      `hyperdb-api/Cargo.toml`), and `.release-please-manifest.json`.
- [ ] Merge the release PR.
- [ ] **Create the tag and GitHub Release by hand.** Merging does *not* tag:
      `release-please-config.json` sets `"skip-github-release": true` at both
      the root and under `"."`, and `docs/GITHUB_OPERATIONS.md` calls the
      manual tag "a deliberate human checkpoint." Skipping this step
      dispatches the publish workflows against a ref that does not exist.
      `--prerelease` must be passed explicitly for `-rc` tags:

```bash
gh release create v1.0.0-rc.1 --target <merge-sha> --prerelease \
  --title "v1.0.0-rc.1" --notes-from-tag
```

- [ ] *Then* trigger the publish workflows — release-please cannot, due to the
      `GITHUB_TOKEN` limitation documented in `docs/GITHUB_OPERATIONS.md`:

```bash
gh workflow run release.yml -f tag=v1.0.0-rc.1
gh workflow run npm-build-publish.yml -f tag=v1.0.0-rc.1
```

- [ ] Verify the GitHub Release shows `prerelease: true` and that
      `npm dist-tag ls hyperdb-api-node` shows `rc: 1.0.0-rc.1` with `latest`
      still on the previous stable.
- [ ] Note for Task 1.5: two docs contradict the config on this and misled an
      earlier draft of this plan — `CONTRIBUTING.md:216` ("tags the merge
      commit and creates the GitHub Release") and `docs/GITHUB_OPERATIONS.md:26`
      both still claim release-please tags. Fix them.

### Task 4.2 — Public-API audit (the gate on `1.0.0`)

`docs/RUST_GUIDELINES.md` defers several lints explicitly "post-1.0". This is
that audit. **Measure before planning the work** — the guidelines text appears
to overstate the remaining backlog.

- [ ] **Establish the real baseline.** Temporarily promote the deferred lints
      and count, rather than trusting the doc:

```bash
cargo clippy --workspace --all-targets -- \
  -W clippy::must_use_candidate \
  -W clippy::missing_errors_doc \
  -W clippy::missing_panics_doc 2>&1 | rg -c '^warning'
```

      Expectation, from a static read: `missing_errors_doc` and
      `missing_panics_doc` are already effectively clean. They sit at `warn`
      in `[workspace.lints.clippy]` while CI runs `-- -D warnings`, so a large
      open backlog would already be failing CI. Corroborating counts: exactly
      **1** `missing_errors_doc` suppression and **0** `missing_panics_doc`
      suppressions exist across the tree, against 585 `# Errors` sections. If
      that holds, promote both to `deny` — a one-line change, not a docs pass.

- [ ]       **`clippy::must_use_candidate` is the genuine unknown — and it is
      large.** Measured 2026-09-04: **141 sites** (134 methods, 7 functions).
      Every one is an API-judgment call. Decide per site; do not
      blanket-annotate, and budget accordingly — this is the real cost of the
      1.0.0 gate, not the doc lints.
- [ ] **`clippy::multiple_crate_versions`** needs the dep-graph audit its
      Cargo.toml comment describes (rand 0.8/0.9, thiserror 1/2, windows_\*).
      Resolver v3 from Task 1.3 may have already reduced this.
- [ ] Decide, per lint, whether the remaining cosmetic allows
      (`module_name_repetitions`, `too_many_lines`, `doc_markdown`,
      `unreadable_literal`, `items_after_statements`, `match_same_arms`) stay
      permanent or get promoted. Update the Exceptions table either way, and
      correct any stale "post-1.0" wording now that 1.0 has arrived.
- [ ] Sanity-check the public surface against **M-SINGLE-ITEM-PATH**: no
      internal item reachable at two paths. This is the last cheap moment to
      fix it, since after 1.0.0 a path change is a major bump.
- [ ] **Decide whether any capability must be opt-in rather than opt-out.**
      Feature flags for `hyperdb-api` are deferred to post-1.0.0, but the
      semver cost is asymmetric and one half of it expires here: adding a
      **default-on** feature later is non-breaking, while moving existing
      always-on capability behind a **default-off** feature is breaking and
      would cost a major version after 1.0.0. So if anything should be
      genuinely opt-in (TLS? geography? chrono?), it has to be decided now.
      Everything else can safely wait. See the deferred-work section of
      [the design spec](../../specs/2026-09-04-rust-188-edition-2024-design.md).

### Task 4.3 — Promote to `1.0.0`

- [ ] Only after Task 4.2 closes. Additional RCs (`rc.2`, `rc.3`) are cut the
      same way as Task 4.1 if the audit or user feedback forces changes.
- [ ] Land `Release-As: 1.0.0`, merge the release PR, then **create the tag
      and Release by hand** exactly as in Task 4.1 — this time *without*
      `--prerelease`:

```bash
gh release create v1.0.0 --target <merge-sha> --title "v1.0.0" --notes-from-tag
gh workflow run release.yml -f tag=v1.0.0
gh workflow run npm-build-publish.yml -f tag=v1.0.0
```

- [ ] Confirm the npm `latest` dist-tag moves to `1.0.0`.
- [ ] **Remove `bump-minor-pre-major` from
      [`release-please-config.json`](../../../../release-please-config.json)**
      (it appears twice, at the root and under the `"."` package). The flag
      only affects `0.x` and is a no-op once at 1.0.0; leaving it in place
      misleads the next reader into thinking breaking changes still bump minor.
- [ ] Note the permanent consequence in the release notes: from 1.0.0 onward a
      `feat!:` commit means **2.0.0**. The `0.x` era of cheap breaking changes
      is over, which is the point of stabilizing.

---

## Per-phase verification

Beyond the five-command gate above:

- **Phase 0:** `cargo build -p hyperdb-api-node` with the edition override,
  output captured. Go/no-go, not a formality.
- **Phase 1:** `make build`, then `make test`, then the full five-command gate.
  The clippy gate proves Blocker B2 is burned down.
- **Phase 2:** `make test` plus `./run_all_examples.sh`. The pool and MCP
  watcher/server suites matter most, since Task 2.2 changes hook signatures and
  the edition change alters lock-guard drop timing. For Task 2.6, additionally
  `cd hyperdb-api-node && npm install --ignore-scripts && npm run build:debug
  && npm test`, mirroring the `node-bindings` CI job.
- **Phase 3:** `make check-rhel` green locally with captured `rustc --version`
  showing 1.88.0, then the workflow green on a PR.
- **Phase 4:** the five-command gate plus the RHEL job green on `main`, and
  Task 4.0's same-session A/B benchmark showing no unexplained regression,
  both before cutting `1.0.0-rc.1`. Then Task 4.2's measured lint baseline
  captured (not assumed) before promoting to `1.0.0`, and `npm dist-tag ls`
  output confirming `rc` and `latest` point where they should at each step.
  Benchmarks run in release mode only, per AGENTS.md reminder 5.
