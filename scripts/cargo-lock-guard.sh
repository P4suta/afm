#!/usr/bin/env bash
set -euo pipefail

if (( $# == 0 )); then
  printf 'usage: %s COMMAND [ARG ...]\n' "$0" >&2
  exit 2
fi

before=$(mktemp)
trap 'rm -f "$before"' EXIT

# The compiler, not a TOML reader, proves that the checked-in resolution is
# usable both before and after release tooling runs.
cargo metadata --locked --no-deps --format-version 1 >/dev/null
cp Cargo.lock "$before"

"$@"

cargo metadata --locked --no-deps --format-version 1 >/dev/null
if ! cmp -s "$before" Cargo.lock; then
  printf 'release tooling changed Cargo.lock:\n' >&2
  git --no-pager diff -- Cargo.lock >&2
  exit 1
fi
