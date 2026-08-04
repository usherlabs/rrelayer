#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
ledger="${repo_root}/downstream-patches.json"
baseline=$(jq -r '.canonical.commit' "$ledger")

if ! git -C "$repo_root" cat-file -e "${baseline}^{commit}"; then
  echo "canonical baseline commit is unavailable: ${baseline}" >&2
  exit 1
fi

mapfile -t affected_paths < <(
  jq -r '.patches[] | .affected_paths[]' "$ledger" | sort -u
)

release_metadata=(
  ".github/workflows/canonical-sync.yml"
  ".github/workflows/downstream-governance.yml"
  "downstream-patches.json"
  "schemas/downstream-patches.schema.json"
  "scripts/report-canonical-sync.sh"
  "scripts/test-downstream-patch-coverage.sh"
  "scripts/test-report-canonical-sync.sh"
  "scripts/test-verify-canonical-baseline.sh"
  "scripts/verify-canonical-baseline.sh"
)

is_covered() {
  local changed_path=$1
  local prefix

  for prefix in "${release_metadata[@]}" "${affected_paths[@]}"; do
    if [[ "$changed_path" == "$prefix" || "$changed_path" == "${prefix}/"* ]]; then
      return 0
    fi
  done

  return 1
}

uncovered=()
while IFS= read -r changed_path; do
  if [[ -n "$changed_path" ]] && ! is_covered "$changed_path"; then
    uncovered+=("$changed_path")
  fi
done < <(git -C "$repo_root" diff --name-only "$baseline" HEAD)

if (( ${#uncovered[@]} > 0 )); then
  echo "downstream source paths missing from downstream-patches.json:" >&2
  printf '  %s\n' "${uncovered[@]}" >&2
  exit 1
fi

echo "all downstream source paths are covered by the patch ledger"
