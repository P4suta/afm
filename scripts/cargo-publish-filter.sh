#!/usr/bin/env bash
set -euo pipefail

filter_stderr() {
  awk '
    /^warning: ignoring test `[^`]+` as `tests\/[^`]+\.rs` is not included in the published package$/ {
      next
    }
    /^warning: ignoring benchmark `[^`]+` as `benches\/[^`]+\.rs` is not included in the published package$/ {
      next
    }
    {
      print
    }
  ' "$@"
}

if [[ "${1-}" == "--filter-only" ]]; then
  shift
  filter_stderr "$@"
  exit
fi

if (( $# == 0 )); then
  printf 'usage: %s COMMAND [ARG ...]\n' "$0" >&2
  exit 2
fi

stderr_file=$(mktemp)
trap 'rm -f "$stderr_file"' EXIT

set +e
"$@" 2>"$stderr_file"
status=$?
set -e

filter_stderr "$stderr_file" >&2
exit "$status"
