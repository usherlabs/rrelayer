#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
schema="${repo_root}/schemas/downstream-patches.schema.json"
ledger="${repo_root}/downstream-patches.json"
test_root=$(mktemp -d)
trap 'rm -r -- "$test_root"' EXIT

validate() {
  npx --yes ajv-cli@5.0.0 validate \
    --spec=draft2020 \
    --strict=true \
    -s "$schema" \
    -d "$1" >/dev/null 2>&1
}

validate "$ledger"

absorbed="${test_root}/absorbed.json"
jq '
  .patches[0].status = "absorbed"
  | .patches[0].absorbed_by = {
      downstream_disposition: "removed",
      downstream_removal_commit: "1111111111111111111111111111111111111111",
      canonical_commit: "2222222222222222222222222222222222222222",
      canonical_only_regression_evidence: ["https://example.invalid/actions/runs/1"],
      downstream_only_behavior_remaining: false
    }
' "$ledger" >"$absorbed"
validate "$absorbed"

for required_field in \
  downstream_disposition \
  downstream_removal_commit \
  canonical_only_regression_evidence \
  downstream_only_behavior_remaining; do
  invalid="${test_root}/missing-${required_field}.json"
  jq --arg field "$required_field" \
    'del(.patches[0].absorbed_by[$field])' "$absorbed" >"$invalid"
  if validate "$invalid"; then
    echo "absorbed patch unexpectedly permits missing ${required_field}" >&2
    exit 1
  fi
done

missing_revision="${test_root}/missing-canonical-revision.json"
jq 'del(.patches[0].absorbed_by.canonical_commit)' \
  "$absorbed" >"$missing_revision"
if validate "$missing_revision"; then
  echo "absorbed patch unexpectedly permits a missing canonical revision" >&2
  exit 1
fi

remaining_behavior="${test_root}/remaining-behavior.json"
jq '.patches[0].absorbed_by.downstream_only_behavior_remaining = true' \
  "$absorbed" >"$remaining_behavior"
if validate "$remaining_behavior"; then
  echo "absorbed patch unexpectedly permits downstream-only behavior" >&2
  exit 1
fi

echo "downstream patch ledger schema tests passed"
