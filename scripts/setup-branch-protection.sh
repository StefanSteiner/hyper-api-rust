#!/usr/bin/env bash
#
# One-shot setup for branch protection on `main`. Idempotent — safe to
# re-run when the required-checks list changes. Requires `gh` CLI
# authenticated with the `repo` scope (the default for `gh auth login`)
# and admin access to the repo.
#
# Required checks below are the jobs from ci.yml that run on every PR.
# verify-hyperd-pin.yml's `verify` is intentionally omitted because it's
# paths-filtered and would block unrelated PRs. release.yml's jobs are
# tag-triggered and never run on PRs.

set -euo pipefail

BRANCH="${BRANCH:-main}"
REPO="${REPO:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}"

# Job names must match exactly what appears in the PR "Checks" tab.
# Matrix jobs use `<name> (<matrix-value>)`.
CONTEXTS=(
  "rustfmt"
  "clippy"
  "test (ubuntu-latest)"
  "test (macos-14)"
  "test (windows-latest)"
  "publish dry-run"
)

# Build the contexts JSON array
contexts_json=$(printf '%s\n' "${CONTEXTS[@]}" | jq -R . | jq -s .)

echo "Configuring branch protection on $REPO:$BRANCH"
echo "Required checks:"
printf '  - %s\n' "${CONTEXTS[@]}"
echo

gh api \
  --method PUT \
  "repos/$REPO/branches/$BRANCH/protection" \
  --input - <<EOF
{
  "required_status_checks": {
    "strict": true,
    "contexts": $contexts_json
  },
  "enforce_admins": false,
  "required_pull_request_reviews": null,
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false
}
EOF

echo
echo "Done. Verify at https://github.com/$REPO/settings/branches"
