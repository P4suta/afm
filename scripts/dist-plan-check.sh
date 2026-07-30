#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

manifest=$(mktemp)
trap 'rm -f "$manifest"' EXIT
scripts/cargo-lock-guard.sh dist plan --output-format=json >"$manifest"

if ! jq -e '
  (.ci.github.artifacts_matrix.include // [])
  | all(.[]; (has("container") or has("packages_install")) | not)
' "$manifest" >/dev/null; then
  printf '%s\n' \
    'dist plan reintroduced container or packages_install in the artifact matrix;' \
    'update the hand-maintained release workflow intentionally before proceeding.' \
    >&2
  exit 1
fi

if ! jq -e '.ci.github.pr_run_mode == "plan"' "$manifest" >/dev/null; then
  printf 'dist plan no longer limits pull requests to the plan phase\n' >&2
  exit 1
fi

cat "$manifest"
