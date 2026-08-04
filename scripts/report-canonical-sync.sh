#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
ledger=${LEDGER_PATH:-"${repo_root}/downstream-patches.json"}
report=${1:-"${RUNNER_TEMP:-/tmp}/rrelayer-canonical-sync.md"}

if [[ ! -f "$ledger" ]]; then
  echo "downstream patch ledger is missing: ${ledger}" >&2
  exit 1
fi

repository=${CANONICAL_REPOSITORY:-$(jq -r '.canonical.repository' "$ledger")}
recorded_tag=$(jq -r '.canonical.tag' "$ledger")
recorded_commit=$(jq -r '.canonical.commit' "$ledger")
recorded_tree=$(jq -r '.canonical.tree' "$ledger")

if [[ -z "$repository" || -z "$recorded_tag" ]]; then
  echo "canonical repository and tag must be present in the patch ledger" >&2
  exit 1
fi

comparison_root=$(mktemp -d)
trap 'rm -r -- "$comparison_root"' EXIT
git init --quiet --bare "${comparison_root}/canonical.git"
git -C "${comparison_root}/canonical.git" fetch --quiet --force "$repository" \
  '+refs/tags/*:refs/tags/*'

resolved_recorded_commit=$(
  git -C "${comparison_root}/canonical.git" rev-parse "refs/tags/${recorded_tag}^{commit}"
)
resolved_recorded_tree=$(
  git -C "${comparison_root}/canonical.git" rev-parse "refs/tags/${recorded_tag}^{tree}"
)

if [[ "$resolved_recorded_commit" != "$recorded_commit" || "$resolved_recorded_tree" != "$recorded_tree" ]]; then
  echo "recorded canonical tag no longer resolves to its recorded commit and tree" >&2
  exit 1
fi

latest_tag=$(
  git -C "${comparison_root}/canonical.git" for-each-ref \
    --format='%(refname:strip=2)' refs/tags |
    grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' |
    sort -V |
    tail -n 1
)

if [[ -z "$latest_tag" ]]; then
  echo "canonical repository exposes no stable v<major>.<minor>.<patch> tag" >&2
  exit 1
fi

latest_commit=$(
  git -C "${comparison_root}/canonical.git" rev-parse "refs/tags/${latest_tag}^{commit}"
)
latest_tree=$(
  git -C "${comparison_root}/canonical.git" rev-parse "refs/tags/${latest_tag}^{tree}"
)

review_candidate=no
if [[ "$latest_commit" != "$recorded_commit" ]]; then
  review_candidate=yes
  if ! git -C "${comparison_root}/canonical.git" merge-base --is-ancestor \
    "$recorded_commit" "$latest_commit"; then
    echo "latest canonical tag is not descended from the recorded baseline" >&2
    exit 1
  fi
fi

changed_paths=""
schema_changed=no
dependency_changed=no
api_or_config_changed=no
if [[ "$review_candidate" == yes ]]; then
  changed_paths=$(
    git -C "${comparison_root}/canonical.git" diff --name-only \
      "$recorded_commit" "$latest_commit"
  )
  if grep -Eq '(^|/)(schema|migrations?)(/|$)' <<<"$changed_paths"; then
    schema_changed=yes
  fi
  if grep -Eq '(^|/)(Cargo\.(toml|lock)|rust-toolchain\.toml)$' <<<"$changed_paths"; then
    dependency_changed=yes
  fi
  if grep -Eq '(^|/)(yaml\.rs|startup\.rs|api/|sdk/|documentation/)' <<<"$changed_paths"; then
    api_or_config_changed=yes
  fi
fi

release_url="${repository%.git}/releases/tag/${latest_tag}"
{
  echo '# Canonical rrelayer comparison'
  echo
  echo "- Recorded canonical tag: \`${recorded_tag}\`"
  echo "- Recorded commit: \`${recorded_commit}\`"
  echo "- Recorded tree: \`${recorded_tree}\`"
  echo "- Latest canonical tag: \`${latest_tag}\`"
  echo "- Latest commit: \`${latest_commit}\`"
  echo "- Latest tree: \`${latest_tree}\`"
  echo "- Canonical release notes: ${release_url}"
  echo "- Review candidate: **${review_candidate}**"
  echo

  if [[ "$review_candidate" == no ]]; then
    echo 'The recorded canonical tag is current.'
  else
    echo '## Impact hints'
    echo
    echo "- Schema/migration paths changed: **${schema_changed}**"
    echo "- Dependency/build paths changed: **${dependency_changed}**"
    echo "- API/config/SDK/documentation paths changed: **${api_or_config_changed}**"
    echo
    echo 'These are path-based review hints, not compatibility conclusions. A maintainer must inspect release notes, migrations, API/config changes, and replay decisions before creating a downstream integration branch.'
    echo
    echo '## Canonical commits'
    echo
    git -C "${comparison_root}/canonical.git" log --reverse \
      --pretty='- `%h` %s' "${recorded_commit}..${latest_commit}"
    echo
    echo '## Changed paths'
    echo
    while IFS= read -r path; do
      [[ -n "$path" ]] && printf -- '- `%s`\n' "$path"
    done <<<"$changed_paths"
  fi

  echo
  echo 'No branch, pull request, tag, merge, or deployment was created.'
} >"$report"

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  sed -n '1,400p' "$report" >>"$GITHUB_STEP_SUMMARY"
fi

echo "canonical comparison report written to ${report}"
