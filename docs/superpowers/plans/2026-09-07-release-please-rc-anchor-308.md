# release-please rc-anchor fix (#308) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop release-please from silently miscomputing the next `-rc.N` (regressing to an already-published version) after a manual-tag release cut, without disturbing the manual, gated publish flow.

**Architecture:** Two independent, complementary changes, plus one runbook reorder that makes the first robust.

- **Option C (anchor):** add `release: types: [published]` to the `release-please` workflow trigger. When a maintainer cuts the tag+Release by hand, that same event re-runs release-please with the tag now present, so it anchors on the just-released version and regenerates the (transiently-wrong) next release PR correctly. Lets us keep `skip-github-release: true` and drop routine `Release-As:` footers.
- **Runbook reorder (makes Option C robust — I-1):** the `release: published` re-run fires the instant `gh release create` runs. In today's runbook the maintainer promotes the just-merged release PR's `autorelease: pending` → `autorelease: tagged` label *after* that (a later step). If release-please keys its "untagged, merged release PRs outstanding" abort off the *label* rather than the now-present *git tag*, the re-run would abort instead of correcting the next-rc PR — silently reducing Option C to a no-op. Whether that abort fires is uncertain (at #306's merge-time run a pending, untagged, merged PR did NOT abort — it produced the wrong PR). We sidestep the uncertainty entirely by moving the label promotion to *immediately before* `gh release create`, so the automatic re-run always sees a promoted label **and** the present tag → it anchors and corrects immediately, regardless of release-please's internal label semantics. This is a doc/runbook change (Task 4), no code.
- **Guard B (backstop):** a CI check on the release-please PR that fails if the proposed manifest version moves *backward* relative to `main`. Turns the silent, unrecoverable regression into a loud, pre-merge failure, and survives the 1.0.0 graduation. Independent of Option C: even if the re-run were ever a no-op, the wrong PR self-heals on the next push to `main` (which always has the tag present) and Guard B blocks merging it in the interim.

**Tech Stack:** GitHub Actions YAML, Python 3 (stdlib only — matches `.github/scripts/verify-npm-hyperd-pin.py`), release-please 17.x, Markdown (markdownlint-cli2).

**Spec:** GitHub issue [#308](https://github.com/tableau/hyper-api-rust/issues/308) plus the confirmed mechanism below (this plan is the follow-up to #308 items 2 and 3).

## Confirmed mechanism (from the #308 dry-run, do not re-derive)

Reproduced byte-for-byte in a throwaway repo mirroring this repo's config:

- release-please runs on **every push to main**, including the release-PR merge itself. At that instant the previous rc's tag/Release does **not** yet exist (the maintainer cuts it by hand seconds/minutes later, because `skip-github-release: true`).
- With no matching Release (`releaseIterator` filters drafts via `!!release.tagCommit`; a manual tag not yet visible to the API) **and** no matching tag (`backfillReleasesFromTags`), release-please takes the **manifest-fallback** path: it builds a synthetic latest-release with `sha: ''`, so `commitsAfterSha(commits, undefined)` re-scans **all** history.
- A stale `Release-As: 1.0.0-rc.2` footer in reachable history (commits `a9fe1b0`/`099ad49`/`#254`, all ancestors of `main`) then short-circuits `determineReleaseType` → `CustomVersionUpdate("1.0.0-rc.2")`, forcing the version **backward** to `rc.2` while the changelog "previous" tag stays `rc.3`. Hence `#306`'s `compare/v1.0.0-rc.3...v1.0.0-rc.2`.

Two dry-run legs proved the fix direction:

- **Tag present (even with the GitHub Release deleted)** → `backfillReleasesFromTags` anchors on the tag's sha → only post-rc.3 commits scanned → correct `rc.4`.
- **Draft release** → no tag, filtered from the release query → fallback persists → `rc.2`. (This is why `skip-github-release: false` + `draft: true` is **not** viable and is out of scope.)

So: ensure the tag is present *before* release-please computes the next PR. Option C does that by re-running release-please on the `release: published` event the manual tag emits.

## Global Constraints

- **Python: stdlib only.** No `pip install`. Match the style of `.github/scripts/verify-npm-hyperd-pin.py` (module docstring explaining *why*, `::error::` annotations, `sys.exit(main())`).
- **Do not change the publish trigger.** `release.yml` and `npm-build-publish.yml` must keep firing on `release: [published]`. The manual `gh release create` stays the gated publish moment.
- **Do not touch any crate version, `Cargo.toml`, `Cargo.lock`, or the root `CHANGELOG.md`.** This is a CI + docs change only.
- **No per-crate `CHANGELOG.md` entry.** Per `AGENTS.md` reminder #8, CI/tooling changes that don't alter a publishable crate's public API surface do not get a changelog bullet. (Call this out so review doesn't flag its absence.)
- **markdownlint any Markdown touched** before committing: `npx markdownlint-cli2` (judge new findings against `git show upstream/main:<path>` — there is a pre-existing backlog).
- **Commit conventions:** Conventional Commits, explicit `git add <files>` (never `-A`). Scopes: `ci:` for the workflow/script, `docs:` for the doc updates. `ci:`/`docs:` are patch-or-none for versioning — never `feat:`.
- **Semver precedence must be correct**, including: prerelease < its release (`1.0.0-rc.5 < 1.0.0`), numeric prerelease identifiers compared numerically not lexically (`rc.10 > rc.9`). This is the whole point of Guard B — get it wrong and the guard is worse than nothing.

---

## Task 1: Version-forward guard script (TDD)

A stdlib-only semver comparator + CLI that fails when a proposed manifest version is strictly *behind* the current one.

**Files:**

- Create: `.github/scripts/verify-release-pr-version.py`
- Test: `.github/scripts/test_verify_release_pr_version.py`

**Interfaces:**

- Produces (consumed by Task 2's workflow):
  - CLI: `python3 .github/scripts/verify-release-pr-version.py <base-version> <head-version>` — exit `0` if `head >= base` (forward or equal), exit `1` if `head < base` (backward). Prints an `::error::` annotation on failure.
  - Function: `compare(a: str, b: str) -> int` — returns `-1|0|1` per semver 2.0 precedence (build metadata ignored).

- [ ] **Step 1: Write the failing tests**

```python
# .github/scripts/test_verify_release_pr_version.py
"""Unit tests for the release-PR version-forward guard.

Run: python3 -m unittest .github/scripts/test_verify_release_pr_version.py
(or: cd .github/scripts && python3 -m unittest test_verify_release_pr_version)
"""
import importlib.util
import unittest
from pathlib import Path

# Load the hyphenated module file by path (not importable as a normal name).
_spec = importlib.util.spec_from_file_location(
    "verify_release_pr_version",
    Path(__file__).with_name("verify-release-pr-version.py"),
)
mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(mod)
compare = mod.compare


class TestCompare(unittest.TestCase):
    def test_rc_forward(self):
        self.assertEqual(compare("1.0.0-rc.3", "1.0.0-rc.2"), 1)  # base>head

    def test_rc_backward_is_head_less(self):
        # head rc.2 vs base rc.3 -> head is behind
        self.assertEqual(compare("1.0.0-rc.2", "1.0.0-rc.3"), -1)

    def test_rc_numeric_not_lexical(self):
        # rc.10 must sort ABOVE rc.9 (numeric identifiers)
        self.assertEqual(compare("1.0.0-rc.10", "1.0.0-rc.9"), 1)

    def test_release_beats_prerelease(self):
        # 1.0.0 (no prerelease) > 1.0.0-rc.5
        self.assertEqual(compare("1.0.0", "1.0.0-rc.5"), 1)

    def test_prerelease_below_release(self):
        # 1.0.0-rc.2 < 1.0.0  (the post-graduation backward landmine)
        self.assertEqual(compare("1.0.0-rc.2", "1.0.0"), -1)

    def test_equal(self):
        self.assertEqual(compare("1.0.0-rc.3", "1.0.0-rc.3"), 0)

    def test_triple_bump(self):
        self.assertEqual(compare("1.0.1", "1.0.0"), 1)
        self.assertEqual(compare("2.0.0", "1.9.9"), 1)

    def test_build_metadata_ignored(self):
        self.assertEqual(compare("1.0.0+abc", "1.0.0+xyz"), 0)


class TestForwardRule(unittest.TestCase):
    def test_is_forward(self):
        # head >= base -> not backward
        self.assertTrue(mod.is_forward_or_equal("1.0.0-rc.3", "1.0.0-rc.4"))
        self.assertTrue(mod.is_forward_or_equal("1.0.0-rc.3", "1.0.0-rc.3"))
        self.assertTrue(mod.is_forward_or_equal("1.0.0-rc.5", "1.0.0"))

    def test_is_backward(self):
        self.assertFalse(mod.is_forward_or_equal("1.0.0-rc.3", "1.0.0-rc.2"))
        self.assertFalse(mod.is_forward_or_equal("1.0.0", "1.0.0-rc.2"))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd .github/scripts && python3 -m unittest test_verify_release_pr_version -v`
Expected: FAIL (module file `verify-release-pr-version.py` does not exist yet → `spec_from_file_location` load error / `ModuleNotFoundError`-style failure).

- [ ] **Step 3: Write the guard script**

```python
#!/usr/bin/env python3
"""Guard: a release-please PR must not move the manifest version backward.

release-please computes the next release version from the last release it can
find (a GitHub Release, then a tag, then — as a fallback — the manifest with an
empty sha). Under this repo's manual-tag flow (`skip-github-release: true`), the
release-please run that fires on the release-PR *merge* races ahead of the
hand-cut tag: the tag/Release don't exist yet, release-please takes the
full-history fallback, and a stale `Release-As:` footer left in history can
force the next PR *backward* to an already-published version (issue #308: after
`1.0.0-rc.3`, PR #306 proposed `1.0.0-rc.2`). Publishing that would be
unrecoverable — crates.io never frees a version number.

`release-please.yml` now re-runs on `release: published` so the tag is present
before the next PR is computed (the primary fix). This script is the backstop:
it fails the release-please PR whenever the proposed manifest version is behind
`main`, so a regression can't be merged silently. It survives the 1.0.0
graduation: `1.0.0 > 1.0.0-rc.N`, so graduating forward passes, while a stale
`Release-As: 1.0.0-rc.N` footer trying to drag a post-1.0.0 line back fails.

Usage: verify-release-pr-version.py <base-version> <head-version>
Exit 0 if head >= base (forward or unchanged), exit 1 if head < base.
"""

from __future__ import annotations

import re
import sys

# MAJOR.MINOR.PATCH with an optional -prerelease and +build (semver 2.0).
# Build metadata does not affect precedence and is discarded.
_SEMVER = re.compile(
    r"^(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)"
    r"(?:-(?P<pre>[0-9A-Za-z.-]+))?"
    r"(?:\+[0-9A-Za-z.-]+)?$"
)


def _parse(v: str) -> tuple[int, int, int, tuple[object, ...] | None]:
    m = _SEMVER.match(v.strip())
    if not m:
        raise ValueError(f"not a semver string: {v!r}")
    pre = m.group("pre")
    if pre is None:
        pre_ids: tuple[object, ...] | None = None  # no prerelease sorts highest
    else:
        pre_ids = tuple(
            int(part) if part.isdigit() else part for part in pre.split(".")
        )
    return (int(m.group("major")), int(m.group("minor")), int(m.group("patch")), pre_ids)


def _cmp_pre(a: tuple[object, ...] | None, b: tuple[object, ...] | None) -> int:
    # A version WITHOUT a prerelease has higher precedence than one WITH.
    if a is None and b is None:
        return 0
    if a is None:
        return 1
    if b is None:
        return -1
    for x, y in zip(a, b):
        xi, yi = isinstance(x, int), isinstance(y, int)
        if xi and yi:
            if x != y:
                return 1 if x > y else -1  # numeric compare
        elif xi != yi:
            return -1 if xi else 1  # numeric identifiers < alphanumeric
        else:
            if x != y:
                return 1 if x > y else -1  # ASCII lexical
    # All shared identifiers equal: the longer set has higher precedence.
    if len(a) != len(b):
        return 1 if len(a) > len(b) else -1
    return 0


def compare(a: str, b: str) -> int:
    """Return -1/0/1 for a<b / a==b / a>b per semver 2.0 precedence."""
    pa, pb = _parse(a), _parse(b)
    if pa[:3] != pb[:3]:
        return 1 if pa[:3] > pb[:3] else -1
    return _cmp_pre(pa[3], pb[3])


def is_forward_or_equal(base: str, head: str) -> bool:
    """True when head is not behind base (head >= base)."""
    return compare(head, base) >= 0


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(f"usage: {argv[0]} <base-version> <head-version>", file=sys.stderr)
        return 2
    base, head = argv[1], argv[2]
    try:
        forward = is_forward_or_equal(base, head)
    except ValueError as exc:
        print(f"::error::release-PR version guard could not parse a version — {exc}")
        return 2
    if forward:
        print(f"ok: proposed version {head} is not behind main ({base}).")
        return 0
    print(
        f"::error::release-please proposed a BACKWARD version: PR wants {head}, "
        f"main is already at {base}. This is the #308 regression — do NOT merge. "
        "Re-run release-please after the previous release's tag exists "
        "(it re-runs automatically on `release: published`), or add a "
        f"`Release-As:` footer pinning the correct forward version."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd .github/scripts && python3 -m unittest test_verify_release_pr_version -v`
Expected: PASS (all cases).

- [ ] **Step 5: Smoke the CLI both directions**

Run:

```bash
python3 .github/scripts/verify-release-pr-version.py 1.0.0-rc.3 1.0.0-rc.4; echo "exit=$?"   # forward -> ok, exit=0
python3 .github/scripts/verify-release-pr-version.py 1.0.0-rc.3 1.0.0-rc.2; echo "exit=$?"   # backward (#306) -> ::error::, exit=1
python3 .github/scripts/verify-release-pr-version.py 1.0.0-rc.5 1.0.0;      echo "exit=$?"   # graduation -> ok, exit=0
```

Expected: exit codes `0`, `1`, `0` respectively; the middle one prints the `::error::` line.

- [ ] **Step 6: Commit**

```bash
git add .github/scripts/verify-release-pr-version.py .github/scripts/test_verify_release_pr_version.py
git commit -m "ci: add release-PR backward-version guard script (#308)"
```

---

## Task 2: Guard workflow — run the check on the release-please PR

**Files:**

- Create: `.github/workflows/verify-release-pr-version.yml`

**Interfaces:**

- Consumes: `.github/scripts/verify-release-pr-version.py` (Task 1), `.release-please-manifest.json` (repo root, `{".": "X.Y.Z"}`).

**Design notes (read before writing):**

- Trigger on `pull_request` targeting `main`. **The job always runs on every PR; the meaningful steps are gated** to the release-please branch (`release-please--branches--main`). This is deliberate (I-2): Task 4 Step 8 makes this a *required* status check, and a job-level `if:` skip reports a `skipped` conclusion whose required-check semantics are historically inconsistent — the failure mode is "every normal PR blocked forever waiting on a check that never reports success." An always-running job with gated steps reports a definite `success` on non-release PRs, so it is safe to require.
- The release PR is authored with the `RELEASE_PLEASE_TOKEN` PAT, so `pull_request` workflows *do* fire on it (GITHUB_TOKEN anti-recursion does not apply — see the `release-please.yml` header comment).
- Read the **head** manifest from the checked-out tree; read the **base** manifest from `origin/<base ref>` via `git show` after fetching it.
- Invoke the script as `python3` (matches `verify-hyperd-pin.yml`; bare `python` is not reliably on `ubuntu-latest`'s PATH) — M-4.
- To actually *block* merge this must be added as a required status check in branch protection — a maintainer/settings step, documented in Task 4, not code.

- [ ] **Step 1: Write the workflow**

```yaml
name: verify-release-pr-version

# Backstop for issue #308: a release-please PR must never move the manifest
# version backward (e.g. propose 1.0.0-rc.2 after 1.0.0-rc.3 shipped). The
# primary fix is the `release: published` re-trigger in release-please.yml;
# this guard fails loudly and pre-merge if a regression slips through anyway.
# See docs/GITHUB_OPERATIONS.md and .github/scripts/verify-release-pr-version.py.
on:
  pull_request:
    branches: [main]

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

permissions:
  contents: read

jobs:
  version-forward:
    # The job ALWAYS runs so it reports a definite success on every PR — safe to
    # mark as a required status check. Only the release-please PR carries a
    # version bump worth guarding, so the real work is gated at the step level.
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        if: startsWith(github.head_ref, 'release-please--branches--')
        with:
          fetch-depth: 0

      - name: Compare proposed manifest version against main
        if: startsWith(github.head_ref, 'release-please--branches--')
        env:
          BASE_REF: ${{ github.event.pull_request.base.ref }}
        run: |
          set -euo pipefail
          git fetch --no-tags origin "$BASE_REF"
          BASE=$(git show "origin/${BASE_REF}:.release-please-manifest.json" | jq -r '."."')
          HEAD=$(jq -r '."."' .release-please-manifest.json)
          echo "main manifest:      $BASE"
          echo "release-PR manifest: $HEAD"
          python3 .github/scripts/verify-release-pr-version.py "$BASE" "$HEAD"
```

On a non-release PR both steps are skipped and the job still completes with a `success` conclusion — the property that makes it safe as a required check.

- [ ] **Step 2: Validate the workflow YAML parses**

Run:

```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/verify-release-pr-version.yml')); print('yaml ok')"
```

Expected: `yaml ok`. (If `actionlint` is installed, also run `actionlint .github/workflows/verify-release-pr-version.yml`; it is not a repo dependency, so a parse check is the required gate.)

- [ ] **Step 3: Dry-check the gate logic locally**

Simulate the two manifest values the workflow would read and confirm the script call behaves:

```bash
BASE=1.0.0-rc.3; HEAD=1.0.0-rc.2   # the #306 case
python3 .github/scripts/verify-release-pr-version.py "$BASE" "$HEAD"; echo "exit=$?"   # expect ::error:: + exit=1
```

Expected: exit=1 with the `::error::` annotation (proves the workflow's core step fails the PR on a regression).

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/verify-release-pr-version.yml
git commit -m "ci: fail the release PR on a backward version bump (#308)"
```

---

## Task 3: Anchor fix — re-run release-please on `release: published`

**Files:**

- Modify: `.github/workflows/release-please.yml:40-43` (the `on:` block) and its header comment.

**Interfaces:**

- No code contract. Behavioral: after this change, cutting the manual tag/Release re-runs release-please with the tag present → it anchors on the just-released version (proven by the #308 dry-run: tag present → correct `rc.N+1`).

- [ ] **Step 1: Add the trigger**

Change the `on:` block at `.github/workflows/release-please.yml:40-43` from:

```yaml
on:
  push:
    branches: [main]
  workflow_dispatch: {}
```

to:

```yaml
on:
  push:
    branches: [main]
  # #308: the run that fires on the release-PR *merge* races ahead of the
  # hand-cut tag (skip-github-release: true), so release-please can't see the
  # just-released version and miscomputes the next rc backward. Re-running when
  # the maintainer publishes the Release — the tag now exists — makes it anchor
  # on that version and regenerate the next release PR correctly. The manual-tag
  # runbook promotes the release PR's `autorelease: tagged` label BEFORE
  # `gh release create` so this re-run doesn't hit the outstanding-pending-PR
  # abort (see docs/GITHUB_OPERATIONS.md). The verify-release-pr-version guard
  # backstops any residual lag.
  release:
    types: [published]
  workflow_dispatch: {}
```

- [ ] **Step 2: Update the workflow header comment**

In the top-of-file comment block (`.github/workflows/release-please.yml:1-38`), find the paragraph describing the manual-tag flow (around lines 3-9, "A maintainer creates the v{X.Y.Z} tag and GitHub Release by hand afterwards, which is what triggers the publish workflows."). Append a sentence:

```text
# That same `release: published` event also re-runs this workflow so it can
# anchor on the freshly created tag and correct the next release PR (see #308).
```

- [ ] **Step 3: Validate the workflow YAML parses**

Run:

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-please.yml')); print('yaml ok')"
```

Expected: `yaml ok`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release-please.yml
git commit -m "ci: re-run release-please on release publish to anchor the next rc (#308)"
```

---

## Task 4: Documentation — reconcile the release docs with the fix, and reorder the runbook

`GITHUB_OPERATIONS.md` currently claims the rc counter bumps itself "no footer, no maintainer action" and the open PR "self-corrects in place." #308 proved that false under the manual-tag race; Task 3 makes it *reliably* true and Guard B makes any residual failure loud. This task (a) reconciles the detailed prose in `GITHUB_OPERATIONS.md`, (b) **reorders the manual-tag runbook** so the `autorelease: tagged` label is promoted *before* `gh release create` (the I-1 fix — makes Option C's re-run abort-proof), (c) adds the guard to the workflows table and branch-protection docs, and (d) adds surgical trigger pointers to the two summary docs that defer mechanism detail here.

**Files:**

- Modify: `docs/GITHUB_OPERATIONS.md` (multiple sections — the detailed source of truth)
- Modify: `CONTRIBUTING.md` (one trigger pointer — Step 10)
- Modify: `AGENTS.md` (one trigger pointer — Step 11)

**Files intentionally NOT changed (explicit decision — I-3):**

- `hyperdb-api-node/DEVELOPMENT.md:322-323, 341-343` — its "release-please *publishes* the Release" phrasing is already inaccurate under `skip-github-release: true` (the maintainer publishes), a *pre-existing* defect not introduced by this change; its "triggers off the GitHub Release event" claim for `npm-build-publish.yml` stays true. Out of scope to avoid scope creep; flag for a separate `docs:` cleanup.
- `docs/README.md:15` — a doc-index one-liner ("what runs on every push and PR, what runs on tag pushes"); not materially stale (push/PR/tag triggers all still hold) and too high-level to enumerate the re-trigger.
- `docs/superpowers/specs/2026-07-10-*.md` and `2026-09-04-*.md` — dated design artifacts, not living behavior docs; leave as historical record.
- `docs/CODE_SCANNING_FIXES.md:10` — its "four workflows" count is already stale (there are 6→7) and scoped to a 2026-06-15 CodeQL fix; not touched by this change.

Rationale for the light touch on `CONTRIBUTING.md`/`AGENTS.md`: both explicitly defer the rc mechanism to `GITHUB_OPERATIONS.md → Pre-releases` ("documented once"), and their auto-increment claims become *more* reliable — not falsified — post-fix. Duplicating the mechanism into them would violate their own single-source convention. So each gets only a one-clause trigger pointer, not a mechanism rewrite.

- [ ] **Step 1: Workflows table — bump the count, add the new trigger and the new workflow**

Two edits in this section:

1. **Count (M-1):** `docs/GITHUB_OPERATIONS.md:21` reads "**Six** GitHub Actions workflows live under `.github/workflows/`." Task 2 adds a seventh file, so change "Six" → "Seven". (Confirmed the string appears exactly once in the repo, so this is the only place.)
2. **Table:** at `docs/GITHUB_OPERATIONS.md:26`, the `release-please` row lists its trigger as "`push` to `main`, manual". Update to "`push` to `main`, GitHub Release `published`, manual" and, in the description, note the `release: published` re-run anchors the next rc after the manual tag (#308). Add a new table row for `verify-release-pr-version.yml`: trigger `pull_request` to `main`; role "fail the release-please PR if it proposes a version behind `main` (backward-bump guard, #308)". Keep column alignment; do not reflow other rows (MD060 is disabled — don't let a formatter touch the table).

- [ ] **Step 2: "How it flows" — correct the self-bump claim (lines 226-227)**

Replace the sentence at `docs/GITHUB_OPERATIONS.md:226-227` (shown as literal Markdown — the `#pre-releases` anchors resolve in the target doc, not this plan):

```text
**While the rc line is open**, that version bumps itself to the next `-rc.N` with no footer needed; see [Pre-releases](#pre-releases).
```

with a version that names the mechanism and the backstop. Be precise (M-2): the routine bump works because the *previous* rc's tag already exists on every ordinary push; the `release: published` re-run specifically covers the one window where it doesn't — right after the release PR merges, before the maintainer cuts the tag. Do not imply the re-run is what makes the bump work at all. Suggested text:

```text
**While the rc line is open**, that version bumps itself to the next `-rc.N` with no footer needed: release-please anchors on the previous rc's tag, which exists on every ordinary push. The one gap — right after the release PR merges but before you cut the new tag — is closed by re-running release-please on the `release: published` event when you cut it. A [backward-version guard](#pre-releases) fails the release PR if that anchoring ever slips. See [Pre-releases](#pre-releases).
```

- [ ] **Step 3: "How it flows" step 5/6 — note the re-trigger**

In step 5 (`docs/GITHUB_OPERATIONS.md:236-239`) or step 6 (`:240-244`), add a sentence that `gh release create`'s `release: published` event *also* re-runs release-please, which regenerates the open release PR against the now-existing tag (correcting any transient backward proposal from the merge-time run). Keep it one sentence; the detail lives in Pre-releases.

- [ ] **Step 4: Reorder the manual-tag runbook — promote the label BEFORE `gh release create` (I-1)**

This is the change that makes Option C abort-proof. In the code block at `docs/GITHUB_OPERATIONS.md:268-301`, **swap the order** of the current step 4 (`gh release create`) and step 5 (`gh pr edit` label promotion) so the label is promoted first, and rewrite both comments. Concretely, replace:

```text
# 4. Create the tag + GitHub Release. --target accepts the merge SHA;
#    the positional arg is the tag name. release.yml fires on the
#    resulting `release: published` event and publishes to crates.io.
gh release create vX.Y.Z \
  -R tableau/hyper-api-rust \
  --target <merge-sha> \
  --title "vX.Y.Z" \
  --notes-file /tmp/vX.Y.Z-notes.md \
  --latest    # OR --prerelease for -rc / -alpha / -beta tags

# 5. Promote the release PR's label so future release-please runs don't
#    abort with "untagged, merged release PRs outstanding".
gh pr edit <release-pr-number> -R tableau/hyper-api-rust \
  --remove-label "autorelease: pending" \
  --add-label "autorelease: tagged"
```

with:

```text
# 4. Promote the release PR's label to `autorelease: tagged` BEFORE creating
#    the Release. Creating the Release (next step) fires `release: published`,
#    which re-runs release-please to re-anchor the next -rc.N (#308). Promoting
#    the label first guarantees that re-run won't abort on "untagged, merged
#    release PRs outstanding". This is only a label edit — reversible — so the
#    point of no return is still `gh release create` below.
gh pr edit <release-pr-number> -R tableau/hyper-api-rust \
  --remove-label "autorelease: pending" \
  --add-label "autorelease: tagged"

# 5. Create the tag + GitHub Release. --target accepts the merge SHA; the
#    positional arg is the tag name. Both release.yml and release-please.yml
#    fire on the resulting `release: published` event — release.yml publishes
#    to crates.io, release-please re-anchors the next rc PR (#308).
gh release create vX.Y.Z \
  -R tableau/hyper-api-rust \
  --target <merge-sha> \
  --title "vX.Y.Z" \
  --notes-file /tmp/vX.Y.Z-notes.md \
  --latest    # OR --prerelease for -rc / -alpha / -beta tags
```

Then update the numbered "After the tag is created" list (`:303-313`) and the prose intro (`:255-266`) only where they reference "step 5" / the promotion by number, so cross-references stay correct after the swap. If nothing references those numbers, no further change.

- [ ] **Step 5: "Manual tag step" recovery list — reframe the footer as emergency-only**

At `docs/GITHUB_OPERATIONS.md:315-334`, the "To stop a release" list is still correct, but the surrounding narrative implies footers are a routine safety valve. Leave the three bullets, and add one line noting that (a) since release-please re-runs on `release: published`, a *forgotten* rc bump self-heals when the prior tag is cut — the `Release-As` footer is for pinning a *different* version (skip a burned number, jump to 1.0.0), not for the routine next rc; and (b) because the label is now promoted before the Release is cut (Step 4), stopping *after* promotion but *before* `gh release create` just means re-labeling the PR (`autorelease: snooze`, per the first bullet) — nothing shipped.

- [ ] **Step 6: "Pre-releases" — correct lines 446-447 and 479-481**

- At `:446-447` ("every releasing prefix computes `1.0.0-rc.3` — no footer, no maintainer action"), append that this holds because release-please anchors on the previous rc's tag, and that the tag must exist when release-please runs — which the `release: published` re-trigger (added for #308) guarantees for the post-merge window.
- At `:479-481` ("An already-open release PR **self-corrects in place** whenever anything lands on `main`"), qualify: it self-corrects on the next release-please run; after a manual-tag cut that run is the one the `release: published` event triggers, and because the runbook now promotes the label before cutting the tag (see the manual-tag step), that re-run anchors and corrects immediately rather than aborting. Note the earlier merge-time run may briefly show the wrong rc until the tag is cut — the backward-version guard prevents merging it in that window.

- [ ] **Step 7: "Graduating to 1.0.0" — update the failure-mode table (lines 522-544)**

The "Why this trade is worth taking" table's "How you find out" row says "wrong version in the PR title" (human eyeballs). Update the "After" column / surrounding prose to note that the backward-version guard now makes a *regression* (not just a forgotten graduation) a loud CI failure, and that graduating passes the guard because `1.0.0 > 1.0.0-rc.N`. Do not overclaim: the guard catches *backward* moves, not a forgotten prerelease-key removal that yields `1.0.1-rc` (that is still caught by reading the PR title). Keep both tripwires described.

- [ ] **Step 8: "Branch protection" — require the new check**

In the `## Branch protection` section (around `docs/GITHUB_OPERATIONS.md:781`), add that `verify-release-pr-version` should be a required status check on `main` so a backward-version release PR cannot be merged even by an admin bypass of other gates. Note this is a repo-settings step (not code); that the guard's always-run-job design (Task 2) makes it safe to require (non-release PRs report `success`, not `skipped`); and that until it is marked required, the check is advisory (red X, not a hard block).

- [ ] **Step 9: "Re-verifying the prerelease behaviour" — cross-reference #308**

At the end of `docs/GITHUB_OPERATIONS.md:546-576`, add one sentence: the #308 regression was confirmed with exactly this dry-run harness (scratch repo, `release-pr --dry-run`), and any future change to `skip-github-release`, the prerelease keys, the release triggers, or the label-promotion order should re-run it and confirm a manual-tag cut still computes `rc.(N+1)` forward.

- [ ] **Step 10: `CONTRIBUTING.md` — one trigger pointer (I-3)**

`CONTRIBUTING.md` defers the rc mechanism to `GITHUB_OPERATIONS.md → Pre-releases` and its "no footer" claim is accurate post-fix, so make only a surgical addition. In the "What maintainers do" step 4 (`CONTRIBUTING.md:239-243`, "**The publish workflows fire on their own** from the `release: published` event that `gh release create` emits…"), append one sentence:

> That same `release: published` event also re-runs release-please so it re-anchors the next `-rc.N` against the freshly cut tag (see [`docs/GITHUB_OPERATIONS.md` → Pre-releases](docs/GITHUB_OPERATIONS.md#pre-releases); #308).

Do not touch lines 248-254 (the "no footer" paragraph) — it is accurate and defers detail to `GITHUB_OPERATIONS.md`.

- [ ] **Step 11: `AGENTS.md` — one trigger pointer (I-3)**

Same principle. In the release section, the sentence "[`.github/workflows/release-please.yml`](.github/workflows/release-please.yml) runs on every push to `main` and opens (or updates) a `chore(main): release X.Y.Z` PR, driven by …" — append after that sentence:

> It also re-runs when a GitHub Release is published, which re-anchors the next `-rc.N` after the manual tag cut (see [docs/GITHUB_OPERATIONS.md → Pre-releases](docs/GITHUB_OPERATIONS.md#pre-releases); #308).

Do not touch the "rc counter increments automatically … no `Release-As:` footer needed" sentence — it is accurate post-fix and explicitly says the mechanism is "documented once" in `GITHUB_OPERATIONS.md`.

- [ ] **Step 12: markdownlint every touched Markdown file**

Run:

```bash
npx markdownlint-cli2 docs/GITHUB_OPERATIONS.md CONTRIBUTING.md AGENTS.md
```

Judge findings against baseline: for each file, `git show upstream/main:<path> > /tmp/base-$(basename <path>) && npx markdownlint-cli2 /tmp/base-…` (or diff the counts). Fix only findings your edits introduced (watch MD024 duplicate headings, MD040 bare fences → tag `text`). Do **not** let a formatter reflow tables (MD060 is disabled for that reason).

- [ ] **Step 13: Commit**

```bash
git add docs/GITHUB_OPERATIONS.md CONTRIBUTING.md AGENTS.md
git commit -m "docs: reconcile release docs with the #308 rc-anchor fix and guard"
```

---

## Verification (run after all tasks)

- [ ] `cd .github/scripts && python3 -m unittest test_verify_release_pr_version -v` → all pass.
- [ ] CLI smoke (Task 1 Step 5) → exit codes `0`, `1`, `0`.
- [ ] Both workflow files parse: `python3 -c "import yaml; [yaml.safe_load(open(f)) for f in ['.github/workflows/release-please.yml','.github/workflows/verify-release-pr-version.yml']]; print('ok')"`.
- [ ] `git diff upstream/main --stat` shows only: 2 new `.github/scripts` files, 1 new workflow, 1 modified workflow, and 3 modified docs (`docs/GITHUB_OPERATIONS.md`, `CONTRIBUTING.md`, `AGENTS.md`). No `Cargo.*`, no crate `CHANGELOG.md`, no root `CHANGELOG.md`.
- [ ] `npx markdownlint-cli2 docs/GITHUB_OPERATIONS.md CONTRIBUTING.md AGENTS.md` new-finding count against baseline is zero for each file.
- [ ] Re-read `.github/workflows/release-please.yml`'s `on:` block: `push`, `release: [published]`, `workflow_dispatch` all present; `concurrency`/`permissions`/`jobs` unchanged.
- [ ] Confirm the `docs/GITHUB_OPERATIONS.md` manual-tag code block now promotes the `autorelease: tagged` label (`gh pr edit … --add-label "autorelease: tagged"`) *before* `gh release create`, and the two step comments read in the new order.

## Risks and non-goals

- **Residual API-visibility race:** the `release: published`-triggered release-please run starts ~seconds after the tag is created, so API consistency is near-certain (vs. the sub-second merge-time race that caused #308). The runbook reorder (Task 4 Step 4 — promote the label before cutting the tag) removes the other half of the risk: the re-run now always sees a promoted `autorelease: tagged` label and cannot abort on "untagged, merged release PRs outstanding". Guard B is the backstop if the API ever lags. Accepted.
- **Guard requires branch protection to *block*.** Until a maintainer marks `verify-release-pr-version` required, it is advisory. Documented in Task 4 Step 8. Not code-fixable here.
- **Post-revert edge (M-3):** the guard compares the release PR's proposed version against the *manifest on `main`*. The documented "to stop a release, revert the release PR's commit" recovery (GITHUB_OPERATIONS.md) pushes `main`'s manifest *backward* relative to an already-burned tag. If a maintainer then reopens a release PR, the guard compares against the reverted (lower) manifest, so a version that is forward-of-manifest but ≤ the burned tag could pass the guard. This is a rare, maintainer-initiated recovery path already under human supervision; the guard is a regression tripwire, not a tag-collision detector. Noted in Task 4 Step 5. Not addressed by this change.
- **Cannot be fully E2E-tested without a real release cycle.** The anchoring logic is proven by the #308 dry-run legs (tag present → `rc.N+1`); the first real rc.5 cut should be watched to confirm the re-trigger fires, the label-before-tag order holds, and the PR lands forward.
- **Stale `Release-As:` footers remain in history** and stay latent landmines until they age past release-please's 500-commit scan depth. Option C makes the fallback path unreachable in normal operation; Guard B catches it if it ever becomes reachable. Rewriting published history to remove them is out of scope.
- **Non-goal:** `skip-github-release: false` (Option A′/2) and draft releases — the former breaks the manual publish gate (`release: [published]` fires publish), the latter provides no anchor (confirmed: draft → no tag, filtered from the release query).
- **Follow-up (not in this branch):** post to / close #308 with the confirmed mechanism and this fix once merged.
