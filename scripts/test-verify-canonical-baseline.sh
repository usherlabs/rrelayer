#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
verifier="$repository_root/scripts/verify-canonical-baseline.sh"

if ! bash "$verifier"; then
  echo "expected the recorded canonical baseline to validate" >&2
  exit 1
fi

test_directory=$(mktemp -d)
trap 'rm -rf "$test_directory"' EXIT

tampered_ledger="$test_directory/downstream-patches.json"
jq '.canonical.tree = "0000000000000000000000000000000000000000"' \
  "$repository_root/downstream-patches.json" > "$tampered_ledger"

if DOWNSTREAM_PATCHES_FILE="$tampered_ledger" bash "$verifier"; then
  echo "expected a tampered canonical tree to be rejected" >&2
  exit 1
fi

echo "canonical baseline verifier tests passed"
