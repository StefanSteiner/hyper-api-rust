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
