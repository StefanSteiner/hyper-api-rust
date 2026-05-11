# GitHub Operations

How this repo uses GitHub: what runs on every push and PR, what runs on
tag pushes, how releases become crates.io publishes and downloadable
binaries, and what maintainers do by hand vs. what the automation does.

Audience: maintainers and contributors who want to know "what happens
when I push", "how do I cut a release", or "where do the pre-built
binaries on the Releases page come from".

## Repository

- **Canonical URL:** https://github.com/tableau/hyper-api-rust
- **Default branch:** `main`
- **License:** dual MIT / Apache-2.0 (see [LICENSE-MIT.txt](../LICENSE-MIT.txt), [LICENSE-APACHE.txt](../LICENSE-APACHE.txt))
- **Governance:** see [CONTRIBUTING.md](../CONTRIBUTING.md) for the
  do-acracy / meritocracy model, PR workflow, and contribution checklist.

## Workflows

Four GitHub Actions workflows live under [`.github/workflows/`](../.github/workflows/):

| Workflow | File | Triggers | Purpose |
|---|---|---|---|
| `ci` | [ci.yml](../.github/workflows/ci.yml) | `push` to `main`, all PRs, manual | fmt, clippy, full test matrix, `cargo deny`, `cargo audit`, `cargo publish --dry-run` |
| `release` | [release.yml](../.github/workflows/release.yml) | tag push matching `v*.*.*` / `v*.*.*-rc.*`, manual | re-run tests, build per-platform binaries, publish 6 Rust crates to crates.io (`hyperdb-api-node` is published separately to npm), attach archives to the GitHub Release |
| `npm-build-publish` | [npm-build-publish.yml](../.github/workflows/npm-build-publish.yml) | GitHub Release published, manual | build npm platform packages with bundled hyperd, publish to npm registry |
| `verify-hyperd-pin` | [verify-hyperd-pin.yml](../.github/workflows/verify-hyperd-pin.yml) | changes to `hyperdb-bootstrap/hyperd-version.toml` or its source, weekly cron, manual | `HEAD` every pinned hyperd release URL to catch Tableau yanks / typos |

### CI (`ci.yml`)

Runs on **every PR** and on **every push to `main`**. Jobs:

- `rustfmt` — `cargo fmt --all --check`.
- `clippy` — `cargo clippy --workspace --all-targets -- -D warnings` (single runner; lints are platform-independent).
- `test` — full workspace test matrix on `ubuntu-latest`, `macos-14`, `windows-latest`.
- `publish-dry-run` — `cargo publish --dry-run` for each publishable crate so a broken publish manifest is caught before a tag is cut.
- `cargo-deny` — license and advisory policy enforcement per [`deny.toml`](../deny.toml).
- `cargo-audit` — RustSec advisories, `--deny warnings`.

In-progress PR CI runs are **cancelled** when a new commit is pushed to
the PR. Main-branch runs always complete. This is set via the
`concurrency` block at the top of [ci.yml](../.github/workflows/ci.yml).

### Release (`release.yml`)

Runs on **tag push** (e.g. `git push origin v0.2.0`) or via manual
`workflow_dispatch` with an explicit tag input. Structure:

```
verify          ← full test suite + hyperd URL check, single-platform
   │
   ├─► build-binaries (matrix × 4 targets)
   │      build release binaries for hyperdb-mcp + hyperdb-bootstrap
   │      package as .tar.gz / .zip, compute per-archive .sha256
   │      upload as per-target GitHub Actions artifacts
   │
   └─► (gate) ──┐
                ▼
              publish          ← crates.io publish in dependency order,
                                 then combine sha256 sidecars, then
                                 create/update GitHub Release with
                                 archives + SHA256SUMS.txt attached
```

**Targets built** (binary archives only — the crates themselves are
architecture-independent source on crates.io):

| Target | Runner | Archive format |
|---|---|---|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `.tar.gz` |
| `aarch64-apple-darwin` | `macos-14` | `.tar.gz` |
| `x86_64-apple-darwin` | `macos-14` (cross) | `.tar.gz` |
| `x86_64-pc-windows-msvc` | `windows-latest` | `.zip` |

Other `hyperdb-api-node` triples (aarch64 Linux, musl Linux) are intentionally
**not** built — `hyperd` itself is not supported on those platforms today,
so a `hyperdb-bootstrap` binary for them would be a trap.

**Dependency-ordered crates.io publish** (per-crate `sleep 45` between
each so the crates.io index has time to settle before the next crate's
verification step resolves the just-published dep):

1. `hyperdb-api-core`
2. `hyperdb-api-salesforce`
3. `hyperdb-api`
4. `hyperdb-mcp`
5. `hyperdb-bootstrap`
6. `sea-query-hyperdb`

`hyperdb-api-node` is **not** on the crates.io list (its `Cargo.toml` has
`publish = false`) — it ships as npm `hyperdb-api-node` through napi-rs's
own pipeline, which is outside this workflow today.

**Pre-release vs. stable:** the GitHub Release is marked `prerelease: true`
automatically for tags containing `-rc.`, `-alpha.`, or `-beta.` — they
show up on the Releases page but are not flagged as "Latest release".

**Concurrency:** only one release workflow runs at a time (the `concurrency:
release` group at the top of the file); a second tag push during a
release will queue, not clobber.

### npm-publish (`npm-build-publish.yml`)

Builds and publishes npm packages for `hyperdb-mcp` and `hyperdb-api-node`
with the `hyperd` database engine bundled into each platform package. This
lets end users run `npx hyperdb-mcp` or `npm install hyperdb-api-node`
without needing Rust toolchains or manual hyperd setup.

**Triggers:**

- **GitHub Release published** — fires automatically after `release.yml`
  creates/updates a GitHub Release.
- **Manual `workflow_dispatch`** — tag/branch input is optional; leave
  empty to build from the default branch HEAD (useful for testing the
  pipeline without tagging).

**Structure:**

```
verify-ci       ← checks that CI passed for this commit (gh api commit status)
   │
   └─► build-npm (matrix × 4 platforms)
          build hyperdb-mcp + hyperdb-api-node native binaries
          download hyperd via curl with SHA256 verification
          assemble platform packages (binary + hyperd + LICENSE-HYPERD)
          upload as GitHub Actions artifacts (7-day retention)
              │
              └─► publish-npm
                    publish platform packages then main packages to npm
```

**Platform matrix:**

| Platform | Runner | Rust target | hyperd source |
|---|---|---|---|
| `darwin-arm64` | `macos-14` | `aarch64-apple-darwin` | `macos-arm64` |
| `darwin-x64` | `macos-13` | `x86_64-apple-darwin` | `macos-x86_64` |
| `linux-x64-gnu` | `ubuntu-latest` | `x86_64-unknown-linux-gnu` | `linux-x86_64` |
| `win32-x64-msvc` | `windows-latest` | `x86_64-pc-windows-msvc` | `windows-x86_64` |

**npm packages published:**

| Package | Type | Contents |
|---|---|---|
| `hyperdb-mcp` | Main (bin shim) | `bin.js` — detects platform, sets `HYPERD_PATH`, spawns native binary |
| `hyperdb-mcp-darwin-arm64` | Platform | `hyperdb-mcp` + `hyperd` + `LICENSE-HYPERD` |
| `hyperdb-mcp-darwin-x64` | Platform | same, Intel macOS |
| `hyperdb-mcp-linux-x64-gnu` | Platform | same, Linux x64 |
| `hyperdb-mcp-win32-x64-msvc` | Platform | same, Windows x64 |
| `hyperdb-api-node` | Main (napi-rs) | JS bindings + `getHyperdPath()` helper |
| `hyperdb-api-node-*` | Platform | `.node` addon + `hyperd` + `LICENSE-HYPERD` |

**CI gate:** The `verify-ci` job checks that the combined commit status
is `success` before building. If CI hasn't passed (e.g., someone
triggers a manual dispatch on a broken commit), the workflow aborts
immediately. Note: this does **not** prevent tagging — git tags can be
created regardless of CI status. Use GitHub Rulesets (repo Settings →
Rules) to enforce tag-creation restrictions if needed.

**Downloading artifacts without publishing:** Since `publish-npm`
requires `NPM_TOKEN`, you can trigger a manual dispatch to test the
build — the build jobs will succeed and upload downloadable artifacts,
while `publish-npm` fails harmlessly.

```bash
# Trigger build from current main (no tag needed)
gh workflow run npm-build-publish.yml

# Trigger build for a specific tag
gh workflow run npm-build-publish.yml --field tag=v0.1.0

# Download artifacts after the run completes
gh run download <run-id> --name npm-darwin-arm64
```

**Local builds:** Use `make npm-pack` to build the current platform's
npm packages locally without CI. This produces `.tgz` files you can
share directly:

```bash
make npm-pack
npm install -g ./hyperdb-mcp/npm/hyperdb-mcp-darwin-arm64-*.tgz \
               ./hyperdb-mcp/npm/hyperdb-mcp-*.tgz
```

### verify-hyperd-pin (`verify-hyperd-pin.yml`)

Independently checks that the per-platform URLs baked into
[`hyperdb-bootstrap/hyperd-version.toml`](../hyperdb-bootstrap/hyperd-version.toml)
still resolve (via `hyperdb-bootstrap verify`). Runs:

- On any PR that touches the pin file or `hyperdb-bootstrap/src/**` (early-warn before the pin change lands).
- On push to `main` for the same paths (covers the merge).
- Every Monday at 12:00 UTC regardless of PR traffic (catches Tableau
  yanking a release out from under us).
- Manually via `workflow_dispatch`.

## Cutting a release

End-to-end recipe for a maintainer:

1. **Bump workspace versions to match the tag.** All 7 publishable crates
   (6 Rust + `hyperdb-api-node` on npm) are kept in lockstep via their
   `Cargo.toml` `version = ...` field (and `package.json` `version` for
   `hyperdb-api-node`). The `publish` job fails fast if `hyperdb-api-core`'s
   version doesn't match the tag, so CI will catch a forgotten bump.
2. **Promote each crate's `## [Unreleased]` section to a dated `## [X.Y.Z] - YYYY-MM-DD`
   section** in its `CHANGELOG.md`. The recommended pattern: keep an empty
   `## [Unreleased]` heading at the top, then add `## [X.Y.Z] - YYYY-MM-DD`
   below it with all the bullets that accumulated under `[Unreleased]` since
   the last release. Contributors maintain `[Unreleased]` per the
   [Authoring changes](../CONTRIBUTING.md#authoring-changes-every-contributor)
   guide. (A [release-please-config.json](../release-please-config.json)
   exists but is not currently wired to a workflow — this step is manual today.)
3. **Open a PR and merge to `main`.** CI must be green.
4. **Tag the merge commit:**
   ```bash
   git fetch origin
   git tag -a v0.2.0 -m "v0.2.0" origin/main
   git push origin v0.2.0
   ```
   For pre-releases, use `v0.2.0-rc.1` etc.
5. **Watch the `release` workflow run.** Expected time: ~30-50 min total
   (verify + 4 parallel binary builds + publish with 45 s gaps ×7).
6. **Verify the release page.** Once the workflow goes green:
   - https://github.com/tableau/hyper-api-rust/releases should list the
     new tag with 8 `.tar.gz` / `.zip` archives (4 targets × 2 binaries)
     plus `SHA256SUMS.txt`.
   - Each crate should appear on crates.io under its new version.

### Re-running a partial failure

The `release` workflow is mostly idempotent but there are two sharp edges:

- **crates.io is append-only.** If the workflow publishes `hyperdb-api-core v0.2.0`
  and then fails on `hyperdb-api`, you cannot republish `hyperdb-api-core v0.2.0`
  — that version is burned. Bump the patch version, re-tag, and run again.
  This is why `build-binaries` runs **before** `publish` — if the binary
  build fails, nothing has hit crates.io yet and you can fix and re-tag
  without a version bump.
- **`softprops/action-gh-release@v2` appends assets.** If an asset was
  uploaded on a failed run, the re-run will **not** overwrite it with the
  new one. Delete stale assets by hand first:
  ```bash
  gh release delete-asset v0.2.0 <filename> --yes
  ```
  Then re-run the failed workflow from the GitHub Actions UI.

### Manual release dispatch

For a release where the tag already exists in `origin` (e.g. you want to
rerun the release workflow after a CI fix that didn't change the tag
contents), use the Actions UI:

1. Actions → `release` → "Run workflow".
2. Enter the existing tag name in the `tag` input (e.g. `v0.2.0`).
3. Click Run.

The workflow's same regex validator rejects malformed tag names, and
`concurrency: release` still prevents racing with an in-flight run.

## Secrets

| Secret | Used by | Scope |
|---|---|---|
| `CRATES_IO_TOKEN` | [release.yml](../.github/workflows/release.yml) `publish` job | `cargo publish` to crates.io |
| `NPM_TOKEN` | [npm-build-publish.yml](../.github/workflows/npm-build-publish.yml) `publish-npm` job | `npm publish` to npmjs.org |
| `GITHUB_TOKEN` | Every workflow | Auto-provided by GitHub Actions; used to fetch protoc, post releases, download artifacts, verify CI status |

No Apple Developer ID cert is configured today — macOS binaries are
unsigned. See the "macOS Gatekeeper" note in the README's
[Pre-built binaries](../README.md#pre-built-binaries) section for the
user-side workaround. Proper `codesign` + `notarytool` would require a
`APPLE_DEVELOPER_ID_CERT` + `APPLE_DEVELOPER_ID_CERT_PASSWORD` +
`APPLE_API_KEY_ID` / `APPLE_API_KEY_ISSUER_ID` / `APPLE_API_KEY` set of
secrets and is a future task.

## Issue & PR templates

There are no `.github/ISSUE_TEMPLATE/` or `.github/pull_request_template.md`
files today; Issues and PRs use GitHub defaults. Contributors still
follow the [Contribution Checklist](../CONTRIBUTING.md#contribution-checklist)
manually.

## Branch protection

Branch protection rules on `main` are configured via GitHub's repo
settings (not in this repo as config-as-code). The expected invariants:

- All PRs require at least one approval.
- `ci` must pass before merge.
- Force-push and deletion are blocked.
- Tags matching `v*.*.*` can only be pushed by maintainers (enforced via
  tag protection rules, separate from branch protection).

Check the actual live settings under
**Settings → Branches** and **Settings → Tags** on the GitHub UI.

## When something breaks

- **CI failures on `main`:** investigate and fix forward. The cancel-on-new-push
  concurrency only applies to PRs; main-branch runs always complete, so a
  broken main is a real signal.
- **`release` workflow failure before `publish`:** fix and re-tag
  (deleting the stale tag locally and on `origin` first,
  `git push origin :v0.2.0-rc.1`).
- **`release` workflow failure during `publish`:** do *not* re-tag — the
  already-published crates are burned. Bump to the next patch version
  and try again.
- **`verify-hyperd-pin` failure:** the pinned hyperd release URL 404'd.
  Check the Tableau releases page, update
  [`hyperdb-bootstrap/hyperd-version.toml`](../hyperdb-bootstrap/hyperd-version.toml)
  with the new version + fresh SHA-256s, and open a PR.
- **Newly-flagged `cargo-audit` advisory on `main`:** open a PR with a
  dep bump (or, if no fix is yet available, document the waiver in
  [`deny.toml`](../deny.toml) with an expiration date).

## Related docs

- [CONTRIBUTING.md](../CONTRIBUTING.md) — governance model, PR workflow, contribution checklist.
- [docs/RUST_GUIDELINES.md](RUST_GUIDELINES.md) — coding standards enforced by `ci.yml`.
- [AGENTS.md](../AGENTS.md) — codebase architecture and build commands for contributors.
- [deny.toml](../deny.toml) — `cargo deny` policy (licenses, advisories).
- [README.md#pre-built-binaries](../README.md#pre-built-binaries) — user-side install snippet for the binaries this workflow produces.
