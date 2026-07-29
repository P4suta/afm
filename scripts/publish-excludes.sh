#!/usr/bin/env bash
set -euo pipefail

requested=${1-}
metadata=$(cargo metadata --locked --no-deps --format-version 1)

declare -A publishable=()
while IFS= read -r name; do
  publishable["$name"]=1
done < <(
  jq -r '.packages[] | select(.publish != []) | .name' <<<"$metadata"
)

declare -A seen=()
IFS=',' read -ra entries <<<"$requested"
for raw in "${entries[@]}"; do
  name=${raw#"${raw%%[![:space:]]*}"}
  name=${name%"${name##*[![:space:]]}"}
  [[ -z "$name" ]] && continue
  if [[ -z "${publishable[$name]+present}" ]]; then
    printf 'cannot skip %q: not a publishable workspace package\n' "$name" >&2
    exit 2
  fi
  if [[ -n "${seen[$name]+present}" ]]; then
    printf 'cannot skip %q twice\n' "$name" >&2
    exit 2
  fi
  seen["$name"]=1
  printf '%s\n%s\n' '--exclude' "$name"
done
