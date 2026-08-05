# Contributing to rrelayer

This repository is the implementation source of truth for Usher/FIET changes
to rrelayer. Generic changes should also be proposed to the canonical
[`joshstevens19/rrelayer`](https://github.com/joshstevens19/rrelayer)
repository when appropriate, but canonical acceptance is not required before a
reviewed downstream release can be published.

The downstream release flow is:

```text
feature or fix pull request
             |
             | review and required CI
             v
       fiet/v0.14
             |
             | annotated immutable tag
             v
    fiet-v0.14.0-<revision>
             |
             | exact Git subtree snapshot
             v
 usherlabs/fiet-tee/packages/tx-broker/rrelayer
```

## Contribution branch

Target downstream pull requests at `fiet/v0.14`. It is the current default
branch and was created directly from canonical tag `v0.14.0`; downstream
patches are maintained above that exact baseline.

The existing `master` and `develop` branches are historical references. Do not
use them as downstream release targets, reset them, or rewrite their public
history.

Future canonical release lines should start a new branch, such as
`fiet/v0.15`, directly from the corresponding exact canonical tag and replay
only the active patches recorded in `downstream-patches.json`. Do not merge a
new canonical release into an already modified downstream release branch.

Follow the coding, testing, changelog, and security requirements in
[`AGENTS.md`](AGENTS.md) for all changes.

## Protected integration branch

A repository administrator, or a custom role with permission to edit repository
rules, must maintain an active branch ruleset for
`refs/heads/fiet/v0.14`. The ruleset must have no bypass actors and must:

- prevent deletion and force pushes;
- require a pull request;
- require at least one approving review;
- dismiss stale approvals after new changes;
- require approval of the most recent reviewable push;
- require all review conversations to be resolved;
- require the pull request branch to be up to date; and
- require these status checks:
  - `Canonical baseline and patch ledger`;
  - `Rustfmt`;
  - `Clippy`;
  - `Downstream Integrity and Policy`;
  - `E2E Raw Provider`;
  - `x86_64-unknown-linux-gnu (ubuntu-22.04)`;
  - `aarch64-apple-darwin (macos-latest)`;
  - `x86_64-apple-darwin (macos-15-intel)`; and
  - `x86_64-pc-windows-msvc (windows-latest)`.

Do not require conditional release, Docker, or release-PR jobs that are skipped
on ordinary pull requests. Do not make CodeRabbit a required check because its
review can be rate-limited.

The ruleset should not use **Restrict updates** with an empty bypass list for
the branch, because that would also prevent reviewed pull-request merges.
Required pull requests plus force-push protection provide the intended branch
control.

Before protecting a future `fiet/vX.Y` branch, update the workflow triggers so
every required check runs on that branch. A wildcard branch rule must not
require a check that can never start.

## Immutable downstream tags

Downstream tags use this format:

```text
fiet-v<canonical-version>-<downstream-revision>
```

The first reconciled release is `fiet-v0.14.0-1`.

A separate active **tag ruleset** must target `refs/tags/fiet-v*`, have no
bypass actors, and enable:

- **Restrict updates**; and
- **Restrict deletions**.

Leave **Restrict creations** disabled in that immutability ruleset. This allows
an authorized maintainer to create a new release tag while preventing anyone
from moving or deleting it afterward. Blocking only non-fast-forward updates is
not sufficient because it may still permit a tag to advance to a descendant
commit.

If tag creation later becomes CI-only, add a second tag ruleset that restricts
creations and grants creation bypass only to the release GitHub App. Do not add
that App to the update/deletion ruleset: a bypass there would allow it to
rewrite existing releases.

Repository administrators can still change repository rules. If protection
must also be independent of repository administrators, use an organization
ruleset controlled by organization owners instead.

## Downstream release prerequisites

Before creating a downstream tag:

1. Confirm the release commit is on the protected `fiet/v0.14` branch.
2. Confirm its pull request received an independent approval.
3. Confirm every required branch check passed for the final reviewed commit.
4. Confirm `downstream-patches.json` validates and accounts for every
   downstream source difference.
5. Confirm the canonical baseline commit and tree still match the ledger.
6. Confirm the tracked `Cargo.lock` builds and tests with `--locked`.
7. Confirm no secret or environment-specific policy value is present in source
   or release artifacts.
8. Prepare release notes covering the canonical baseline, downstream patches,
   database migrations, downgrade compatibility, and rollback constraints.

## Creating a downstream release

Fetch the protected branch and identify the reviewed merge commit:

```bash
git fetch origin fiet/v0.14
git log -1 --show-signature origin/fiet/v0.14
```

Create an annotated tag at that exact commit. Substitute the reviewed commit
SHA explicitly; do not tag an unresolved local branch name.

```bash
git tag -a fiet-v0.14.0-1 <reviewed-merge-commit-sha> \
  -m "FIET rrelayer v0.14.0 downstream revision 1"
git show --no-patch --show-signature fiet-v0.14.0-1
git push origin refs/tags/fiet-v0.14.0-1
```

The tag push runs the downstream CI matrix again. Wait for it to pass before
publishing GitHub Release metadata. Then publish against the existing tag:

```bash
gh release create fiet-v0.14.0-1 \
  --repo usherlabs/rrelayer \
  --verify-tag \
  --title "FIET rrelayer v0.14.0-1" \
  --notes-file <reviewed-release-notes.md>
```

Record the tag object, peeled commit, and root tree:

```bash
git rev-parse fiet-v0.14.0-1
git rev-parse fiet-v0.14.0-1^{}
git rev-parse fiet-v0.14.0-1^{tree}
git ls-remote --tags origin \
  refs/tags/fiet-v0.14.0-1 \
  refs/tags/fiet-v0.14.0-1^{}
```

The peeled commit and root tree are the identities recorded in the consumer
provenance file.

## Never rewrite a published downstream release

After a `fiet-v*` tag is pushed, do not move, force-update, or delete it. This
applies even if a defect is discovered immediately afterward.

Correct the source on `fiet/v0.14`, repeat review and CI, and publish the next
revision, such as `fiet-v0.14.0-2`. Release notes and the patch ledger should
explain why the prior revision was superseded.

## Consumption by fiet-tee

`usherlabs/fiet-tee` consumes a complete ordinary-file Git subtree snapshot of
the immutable tag. It does not consume a nested submodule or a separately
published binary, and it must not carry local patches inside the rrelayer
subtree.

After the initial reviewed bootstrap, snapshot updates must originate from the
dedicated `fiet-tee` snapshot GitHub App workflow. That workflow imports the
complete tagged tree, updates the adjacent provenance record, and opens a pull
request. A human CODEOWNER reviews and merges it; the App cannot approve,
bypass protection, or merge its own pull request.

The consumer provenance gate verifies:

- the recorded tag peels to the recorded commit;
- the commit has the recorded root tree; and
- the complete `packages/tx-broker/rrelayer` subtree has that exact tree.

Only after the exact snapshot is merged may Admin or Maker update its
`fiet-tee` pin and perform its independent staging and production promotion.

## GitHub rules references

- [Creating repository rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/creating-rulesets-for-a-repository)
- [Available rules for rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets)
- [Managing repository rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/managing-rulesets-for-a-repository)
