#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
test_root=$(mktemp -d)
trap 'rm -r -- "$test_root"' EXIT

canonical="${test_root}/canonical"
ledger="${test_root}/downstream-patches.json"
report="${test_root}/report.md"

git init --quiet "$canonical"
git -C "$canonical" config user.name "Canonical Sync Test"
git -C "$canonical" config user.email "sync@example.invalid"
mkdir -p "${canonical}/crates/core/src/schema"
printf '[workspace]\n' >"${canonical}/Cargo.toml"
printf 'baseline\n' >"${canonical}/crates/core/src/schema/v1.rs"
git -C "$canonical" add --all
git -C "$canonical" commit --quiet -m "release: v0.14 baseline"
git -C "$canonical" tag -a v0.14.0 -m v0.14.0

baseline_commit=$(git -C "$canonical" rev-parse 'v0.14.0^{commit}')
baseline_tree=$(git -C "$canonical" rev-parse 'v0.14.0^{tree}')

printf '[workspace]\nresolver = "3"\n' >"${canonical}/Cargo.toml"
printf 'migration\n' >"${canonical}/crates/core/src/schema/v2.rs"
git -C "$canonical" add --all
git -C "$canonical" commit --quiet -m "feat: add schema migration"
git -C "$canonical" tag -a v0.15.0 -m v0.15.0

printf 'bug fix\n' >"${canonical}/fix.txt"
git -C "$canonical" add --all
git -C "$canonical" commit --quiet -m "fix: canonical bug"
git -C "$canonical" tag -a v0.15.1 -m v0.15.1

jq -n \
  --arg repository "$canonical" \
  --arg commit "$baseline_commit" \
  --arg tree "$baseline_tree" \
  '{
    schema_version: 1,
    canonical: {
      repository: $repository,
      tag: "v0.14.0",
      commit: $commit,
      tree: $tree
    },
    patches: []
  }' >"$ledger"

LEDGER_PATH="$ledger" "${repo_root}/scripts/report-canonical-sync.sh" "$report"

grep -F 'Recorded canonical tag: `v0.14.0`' "$report" >/dev/null
grep -F 'Latest canonical tag: `v0.15.1`' "$report" >/dev/null
grep -F 'Review candidate: **yes**' "$report" >/dev/null
grep -F 'Schema/migration paths changed: **yes**' "$report" >/dev/null
grep -F 'Dependency/build paths changed: **yes**' "$report" >/dev/null
grep -F 'feat: add schema migration' "$report" >/dev/null
grep -F 'fix: canonical bug' "$report" >/dev/null
grep -F 'No branch, pull request, tag, merge, or deployment was created.' "$report" >/dev/null

latest_commit=$(git -C "$canonical" rev-parse 'v0.15.1^{commit}')
latest_tree=$(git -C "$canonical" rev-parse 'v0.15.1^{tree}')
jq \
  --arg commit "$latest_commit" \
  --arg tree "$latest_tree" \
  '.canonical.tag = "v0.15.1" | .canonical.commit = $commit | .canonical.tree = $tree' \
  "$ledger" >"${ledger}.next"
mv "${ledger}.next" "$ledger"

LEDGER_PATH="$ledger" "${repo_root}/scripts/report-canonical-sync.sh" "$report"
grep -F 'Review candidate: **no**' "$report" >/dev/null
grep -F 'The recorded canonical tag is current.' "$report" >/dev/null

echo "canonical sync report tests passed"
