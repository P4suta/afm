#!/usr/bin/env bash
set -euo pipefail

if (( $# > 1 )); then
  printf 'usage: %s [COMMA_SEPARATED_SKIP_CRATES]\n' "$0" >&2
  exit 2
fi

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

skip_crates=${1-}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

scripts/test-cargo-publish-filter.sh
scripts/publish-excludes.sh "$skip_crates" >"$tmp/excludes"
mapfile -t excludes <"$tmp/excludes"

metadata=$(cargo metadata --locked --no-deps --format-version 1)
declare -A skipped=()
for ((i = 0; i < ${#excludes[@]}; i += 2)); do
  skipped["${excludes[i + 1]}"]=1
done

declare -a selected=()
declare -A versions=()
jq -r '.packages[] | select(.publish != []) | [.name, .version] | @tsv' \
  <<<"$metadata" >"$tmp/selected.tsv"
while IFS=$'\t' read -r name version; do
  if [[ -z "${skipped[$name]+present}" ]]; then
    selected+=("$name")
    versions["$name"]=$version
  fi
done <"$tmp/selected.tsv"

printf 'package-smoke selected %d crate(s):\n' "${#selected[@]}"
if (( ${#selected[@]} == 0 )); then
  printf '  (none)\n'
  printf 'all publishable crates were skipped; nothing to dry-run\n'
  exit
fi
printf '  %s\n' "${selected[@]}"

marker="$tmp/publish-started"
touch "$marker"
# Cargo stages interdependent workspace members in a temporary registry.
# Isolate CARGO_HOME so verification cannot accidentally compile a same-version
# registry cache instead of the archive built by this dry run.
CARGO_HOME="$tmp/cargo-home" scripts/cargo-publish-filter.sh \
  cargo publish --workspace --dry-run --locked "${excludes[@]}"

for name in "${selected[@]}"; do
  crate="target/package/tmp-crate/${name}-${versions[$name]}.crate"
  if [[ ! -f "$crate" || ! "$crate" -nt "$marker" ]]; then
    printf 'cargo did not freshly build selected archive %s\n' "$crate" >&2
    exit 1
  fi
done

core=aozora-flavored-markdown
if [[ -n "${skipped[$core]+present}" ]]; then
  printf 'core crate was skipped; archive unit-test smoke is not applicable\n'
  exit
fi

core_crate="target/package/tmp-crate/${core}-${versions[$core]}.crate"
mkdir "$tmp/unpacked"
tar -xf "$core_crate" -C "$tmp/unpacked"
cd "$tmp/unpacked/${core}-${versions[$core]}"
CARGO_HOME="$tmp/cargo-home" cargo test --lib --all-features --locked
