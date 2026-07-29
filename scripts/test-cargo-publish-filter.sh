#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
fixtures="$repo_root/tests/fixtures/package-smoke"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

"$repo_root/scripts/cargo-publish-filter.sh" --filter-only \
  "$fixtures/known.stderr" \
  "$fixtures/near.stderr" \
  "$fixtures/ordinary.stderr" \
  >"$tmp/actual-filtered"
diff -u "$fixtures/filtered.stderr" "$tmp/actual-filtered"

set +e
# The single-quoted program is intentionally literal input to the fixture shell.
# shellcheck disable=SC2016
"$repo_root/scripts/cargo-publish-filter.sh" \
  bash -c '
    printf "%s\n" "command stdout"
    printf "%s\n" \
      "warning: ignoring test \`ir_coverage\` as \`tests/ir_coverage.rs\` is not included in the published package" \
      "command stderr" >&2
    exit 23
  ' >"$tmp/actual-stdout" 2>"$tmp/actual-stderr"
status=$?
set -e

if (( status != 23 )); then
  printf 'cargo stderr filter changed exit status: expected 23, got %d\n' "$status" >&2
  exit 1
fi
printf '%s\n' 'command stdout' >"$tmp/expected-stdout"
printf '%s\n' 'command stderr' >"$tmp/expected-stderr"
diff -u "$tmp/expected-stdout" "$tmp/actual-stdout"
diff -u "$tmp/expected-stderr" "$tmp/actual-stderr"
