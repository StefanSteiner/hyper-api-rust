# Rust 1.88 / Edition 2024 Uplift

Planning artifacts for raising the workspace MSRV floor to Rust 1.88, migrating
to edition 2024, and adding a RHEL 9.7 `rust-toolset` compatibility gate.

**Status:** approved, not yet implemented
**Branch:** `feat/upgrade-to-188-RHEL97` (base `28c8813`)

## Documents

* [`2026-09-04-rust-188-edition-2024-uplift.md`](2026-09-04-rust-188-edition-2024-uplift.md)
  — the implementation plan. Task-by-task, checkbox-tracked, with the exact
  file paths and line numbers each task touches. Start here when executing.
* [`../../specs/2026-09-04-rust-188-edition-2024-design.md`](../../specs/2026-09-04-rust-188-edition-2024-design.md)
  — the design specification. Decisions D1 through D6 with rationale and
  rejected alternatives, the measured ground truth behind them, blockers B1
  through B6, and explicit non-goals. Read this when questioning *why* the plan
  is shaped the way it is.

The spec lives in `docs/superpowers/specs/` per the convention documented in
[`AGENTS.md`](../../../../AGENTS.md); only the plan and its supporting material
are collected in this folder.

## What this changes

Four phases, executed in order.

* **Phase 0 — de-risk the Node bindings.** A throwaway spike that proves
  `ctor 1.0.5`'s macro-generated bare `#[no_mangle]` survives expansion into an
  edition-2024 cdylib. This is the one migration risk that depends on code the
  repository does not own, so it runs before anything else; a red result
  reshapes the whole migration.
* **Phase 1 — configuration and edition.** MSRV floor to 1.88, `cargo fix
  --edition` *before* the manifest flip, then the hand-fixes that automated
  migration cannot do.
* **Phase 2 — modernization.** Let chains, `async` closures in the connection
  pool, `use<..>` precise capturing on eight public signatures, and the
  `Option::is_none_or` cluster. Includes the `hyperdb-api-node` cast cleanup.
* **Phase 3 — enterprise CI.** A UBI9 container job proving the workspace
  builds with nothing but `dnf install rust-toolset`, plus a local
  `make check-rhel` mirror and a README section documenting the contract.
* **Phase 4 — release as 1.0.0.** Phases 0 through 3 land unreleased, then a
  single `1.0.0-rc.1` covers the whole migration. Promotion to `1.0.0` is
  gated on the public-API audit that closes out the lints
  [`docs/RUST_GUIDELINES.md`](../../../RUST_GUIDELINES.md) defers "post-1.0".
  Needs no new release machinery — the `Release-As:` footer, the
  `x-release-please` version markers, and the npm `rc` dist-tag routing all
  already exist.

## Three things worth knowing before you start

**The "1.81 to 1.88" framing is misleading.** `rust-toolchain.toml` pins
`channel = "stable"`, which resolves to rustc 1.98.0. The repo's 1.81 is a
declared floor that was never enforced, so this is an MSRV *raise* combined
with an edition migration — not a compiler upgrade. Local development stays on
`stable` throughout; see decision D1.

**Two categories the plan deliberately declines.** Strict provenance has
nothing to migrate (zero integer-to-pointer casts in the tree), and
`number::midpoint` has only cosmetic `f64` candidates. Both are recorded as
non-goals with evidence so they are not re-investigated.

**Changelog rules are file-specific.** The root `CHANGELOG.md` is
release-please-generated and never hand-edited; the seven per-crate
`CHANGELOG.md` files carry `## [Unreleased]` sections and are hand-maintained.
Decision D6 works through why `AGENTS.md` and `CONTRIBUTING.md` only appear to
disagree on this.

**Phase 4 ends the cheap-breaking-change era.** `bump-minor-pre-major: true`
only applies below 1.0.0. Once `1.0.0` ships, a `feat!:` commit means `2.0.0`,
so the Task 4.2 API audit is the last inexpensive chance to change public
paths and signatures. Decision D9 covers the sequencing; D7 covers why the
MSRV bump is marked breaking despite **M-MSRV** advising otherwise.
