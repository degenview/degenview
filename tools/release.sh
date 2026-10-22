#!/bin/sh
# Cut a release: bump the version in Cargo.toml/Cargo.lock, commit, sign-tag vX.Y.Z and push.
#
#   tools/release.sh 0.1.3            ask before doing anything
#   tools/release.sh 0.1.3 --yes      no prompt
#   tools/release.sh 0.1.3 --dry-run  print what would happen, change nothing
#   tools/release.sh 0.2.0-rc.1       a release candidate: only the Release Candidate channel gets it
#
# Cargo.toml is the single source of the app's version (Settings > About, the update check, the
# app bundle's Info.plist). Pushing the tag starts the Release workflow, which refuses a tag that
# does not match Cargo.toml, so never tag by hand.
set -eu

cd "$(dirname "$0")/.."

usage() {
    echo "usage: tools/release.sh <X.Y.Z[-(alpha|beta|rc).N]> [--yes] [--dry-run]" >&2
    exit 2
}

version=""
assume_yes=0
dry_run=0
for arg in "$@"; do
    case "$arg" in
        --yes) assume_yes=1 ;;
        --dry-run) dry_run=1 ;;
        -*) usage ;;
        *) [ -z "$version" ] || usage; version="$arg" ;;
    esac
done
[ -n "$version" ] || usage

version="${version#v}"
# SemVer, with dotted numbered prereleases so they order numerically (rc.10 after rc.2): the app's
# update channels read the name (rc) and compare the number.
if ! printf '%s\n' "$version" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$'; then
    echo "error: '$version' is not X.Y.Z or X.Y.Z-(alpha|beta|rc).N (N from 1, no leading zeros)" >&2
    exit 1
fi
tag="v$version"

# The `version` line of the [package] table, however many other tables follow it.
current=$(awk '/^\[/ { in_package = ($0 == "[package]") } in_package && /^version = "/ { gsub(/^version = "|"$/, ""); print; exit }' Cargo.toml)
if [ -z "$current" ]; then
    echo "error: could not read [package] version from Cargo.toml" >&2
    exit 1
fi

# Compare the numeric X.Y.Z core only; the new version must be strictly newer.
core() { printf '%s\n' "${1%%-*}"; }
newest=$(printf '%s\n%s\n' "$(core "$current")" "$(core "$version")" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)
if [ "$(core "$version")" = "$(core "$current")" ]; then
    # Only a pre-release can be followed by its own release (0.2.0-rc.1 -> 0.2.0), or by a later
    # one (rc.1 -> rc.2, rc.9 -> rc.10, alpha -> beta -> rc).
    case "$current" in
        *-*)
            [ "$version" != "$current" ] || { echo "error: already at $current" >&2; exit 1; }
            case "$version" in
                *-*)
                    # name.number: names order alphabetically (alpha < beta < rc), numbers numerically.
                    cur_name="${current#*-}"; cur_name="${cur_name%%.*}"; cur_num="${current##*.}"
                    new_name="${version#*-}"; new_name="${new_name%%.*}"; new_num="${version##*.}"
                    if [ "$new_name" \< "$cur_name" ] || { [ "$new_name" = "$cur_name" ] && [ "$new_num" -le "$cur_num" ]; }; then
                        echo "error: $version is not newer than the current version $current" >&2
                        exit 1
                    fi ;;
            esac ;;
        *) echo "error: $version is not newer than the current version $current" >&2; exit 1 ;;
    esac
elif [ "$newest" != "$(core "$version")" ]; then
    echo "error: $version is not newer than the current version $current" >&2
    exit 1
fi

if [ "$(git rev-parse --abbrev-ref HEAD)" != "main" ]; then
    echo "error: releases are cut from main" >&2
    exit 1
fi
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "error: the working tree has uncommitted changes" >&2
    exit 1
fi
if [ -z "$(git config user.signingkey)" ]; then
    echo "error: git user.signingkey is not set; release tags must be signed (git tag -s)" >&2
    exit 1
fi
# main is PR-only, so the release goes through a pull request opened with gh.
if ! command -v gh >/dev/null 2>&1; then
    echo "error: gh is required (main only accepts changes through a pull request)" >&2
    exit 1
fi
# The app only auto-installs immutable releases, and a release published while the setting is off
# can never be made immutable, so check before tagging.
immutable=$(gh api repos/:owner/:repo/immutable-releases --jq .enabled 2>/dev/null || echo unknown)
if [ "$immutable" = "false" ]; then
    echo "error: enable Immutable releases first (repo Settings > General > Releases)" >&2
    exit 1
elif [ "$immutable" = "unknown" ]; then
    echo "warning: could not check whether Immutable releases is enabled" >&2
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "error: tag $tag already exists" >&2
    exit 1
fi

case "$version" in
    *-rc.*) channel="Release Candidate channel only" ;;
    *-*) channel="no channel (the app's updater ignores alpha/beta tags)" ;;
    *) channel="Stable and Release Candidate channels" ;;
esac
echo "Release $tag ($channel)"
echo "  1. Cargo.toml / Cargo.lock: $current -> $version"
echo "  2. commit 'Release $tag' on branch release/$tag, open a PR, merge it with a merge commit"
echo "  3. signed tag $tag on the merged main (verified locally), then git push origin $tag"
echo "     (starts the Release workflow)"
if [ "$dry_run" = 1 ]; then
    echo "(dry run: nothing changed)"
    exit 0
fi
if [ "$assume_yes" != 1 ]; then
    printf 'Continue? [y/N] '
    read -r answer
    case "$answer" in y | Y) ;; *) echo "aborted"; exit 1 ;; esac
fi

# Only the [package] version line changes; dependency tables keep their own `version =` keys.
awk -v new="$version" '
    /^\[/ { in_package = ($0 == "[package]") }
    in_package && !done && /^version = "/ { print "version = \"" new "\""; done = 1; next }
    { print }
' Cargo.toml > Cargo.toml.release.tmp
mv Cargo.toml.release.tmp Cargo.toml

# `--workspace` refreshes only the workspace's own packages (here: the root version), which is what
# cargo documents it for after a version bump; every other dependency keeps the locked version the
# tests ran against. Prove it rather than trust it: the lock may differ by exactly one line.
cargo update --offline --workspace >/dev/null
if [ "$(git diff --numstat -- Cargo.lock | cut -f1,2)" != "$(printf '1\t1')" ]; then
    echo "error: Cargo.lock changed by more than the version line; nothing was committed." >&2
    git diff --stat -- Cargo.toml Cargo.lock >&2
    echo "Inspect with: git diff Cargo.lock    Undo with: git checkout -- Cargo.toml Cargo.lock" >&2
    exit 1
fi

branch="release/$tag"
git switch -c "$branch"
git add Cargo.toml Cargo.lock
git commit -m "Release $tag"
git push origin "$branch"
# A merge commit, never squash/rebase: the release commit's SHA is what the tag will cover.
pr=$(gh pr create --base main --head "$branch" --title "Release $tag" \
    --body "Version bump to $version. Merged with a merge commit; tools/release.sh tags main afterwards.")
gh pr merge "$pr" --merge

# Tag only once main has the bump: a refused merge must not leave a tag (and a running workflow).
git switch main
git pull --ff-only origin main
if ! grep -q "^version = \"$version\"" Cargo.toml; then
    echo "error: main does not have version $version after the merge; no tag was created." >&2
    exit 1
fi
git tag -s "$tag" -m "DegenView $tag"
# The app only installs updates whose tag GitHub reports as verified, so catch a bad signature
# here, before pushing. Needs gpg.ssh.allowedSignersFile; without it GitHub is the only check.
if [ -n "$(git config gpg.ssh.allowedSignersFile)" ]; then
    if ! git verify-tag "$tag"; then
        echo "error: $tag does not verify locally; nothing was pushed." >&2
        echo "Undo with: git tag -d $tag" >&2
        exit 1
    fi
else
    echo "warning: gpg.ssh.allowedSignersFile is not set, so $tag was not verified locally" >&2
fi
git push origin "$tag"
git push origin --delete "$branch" || true
git branch -d "$branch" || true
echo "Pushed $tag. Follow the build: gh run watch"
