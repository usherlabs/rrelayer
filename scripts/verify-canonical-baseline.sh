#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
ledger_file=${DOWNSTREAM_PATCHES_FILE:-"$repository_root/downstream-patches.json"}

fail() {
  echo "canonical baseline verification failed: $*" >&2
  exit 1
}

[[ -f "$ledger_file" ]] || fail "ledger not found at $ledger_file"

canonical_repository=$(jq -er '.canonical.repository | select(length > 0)' "$ledger_file") || \
  fail "canonical.repository is missing"
canonical_tag=$(jq -er '.canonical.tag | select(length > 0)' "$ledger_file") || \
  fail "canonical.tag is missing"
recorded_commit=$(jq -er '.canonical.commit | select(test("^[0-9a-f]{40}$"))' "$ledger_file") || \
  fail "canonical.commit is not a full Git OID"
recorded_tree=$(jq -er '.canonical.tree | select(test("^[0-9a-f]{40}$"))' "$ledger_file") || \
  fail "canonical.tree is not a full Git OID"

remote_refs=$(git ls-remote --tags "$canonical_repository" \
  "refs/tags/$canonical_tag" "refs/tags/$canonical_tag^{}")
remote_commit=$(awk '$2 ~ /\^\{\}$/ { print $1 }' <<< "$remote_refs")
if [[ -z "$remote_commit" ]]; then
  remote_commit=$(awk -v ref="refs/tags/$canonical_tag" '$2 == ref { print $1 }' <<< "$remote_refs")
fi
[[ -n "$remote_commit" ]] || fail "canonical tag $canonical_tag was not found"
[[ "$remote_commit" == "$recorded_commit" ]] || \
  fail "tag $canonical_tag resolves to $remote_commit, expected $recorded_commit"

git fetch --quiet --no-tags "$canonical_repository" "refs/tags/$canonical_tag"
fetched_commit=$(git rev-parse 'FETCH_HEAD^{commit}')
fetched_tree=$(git rev-parse 'FETCH_HEAD^{tree}')

[[ "$fetched_commit" == "$recorded_commit" ]] || \
  fail "fetched tag commit $fetched_commit does not match $recorded_commit"
[[ "$fetched_tree" == "$recorded_tree" ]] || \
  fail "fetched tag tree $fetched_tree does not match $recorded_tree"

git cat-file -e "$recorded_commit^{commit}" 2>/dev/null || \
  fail "recorded canonical commit is absent from local history"
local_tree=$(git rev-parse "$recorded_commit^{tree}")
[[ "$local_tree" == "$recorded_tree" ]] || \
  fail "local baseline tree $local_tree does not match $recorded_tree"

git merge-base --is-ancestor "$recorded_commit" HEAD || \
  fail "HEAD does not descend from the exact recorded canonical commit"

echo "canonical baseline verified: $canonical_tag $recorded_commit $recorded_tree"
