#!/bin/sh
# List git worktrees that are finished with, remove them, and reclaim idle build output.
#
#   tools/clean-worktrees.sh                        list every worktree, its size, and why it stays or can go
#   tools/clean-worktrees.sh --remove-merged        remove the worktrees marked REMOVABLE (asks first)
#   tools/clean-worktrees.sh --clean-idle 14        delete target/debug of worktrees not built for 14 days
#   tools/clean-worktrees.sh --remove-merged --yes  no prompt
#
# Every worktree has its own target/, and cargo never deletes old build output, so finished
# worktrees are where the disk goes. A worktree is REMOVABLE only when its branch's pull request is
# MERGED and the branch has nothing newer than it, it has no uncommitted changes, and nothing is
# running from it. A branch with no pull request is never removable: a worktree you made five
# minutes ago looks exactly like a finished one. Branches are never deleted and `git worktree
# remove` is never forced. Needs `gh` signed in to see pull requests; without it nothing is
# marked REMOVABLE.
#
# --clean-idle keeps the worktrees and deletes only target/debug (a rebuild is a few minutes), in
# every worktree, the main one included, whose build output has not changed for that many days.
# Use --clean-idle 0 to clear them all.
set -eu

cd "$(dirname "$0")/.."

usage() {
    echo "usage: tools/clean-worktrees.sh [--remove-merged | --clean-idle DAYS] [--yes]" >&2
    exit "${1:-2}"
}

mode=list
idle_days=""
assume_yes=0
while [ $# -gt 0 ]; do
    case "$1" in
        --remove-merged) mode=remove ;;
        --clean-idle)
            mode=idle
            shift
            [ $# -gt 0 ] || usage
            idle_days="$1"
            case "$idle_days" in '' | *[!0-9]*) usage ;; esac
            ;;
        --yes) assume_yes=1 ;;
        -h | --help) usage 0 ;;
        *) usage ;;
    esac
    shift
done

here=$(git rev-parse --show-toplevel)
base=origin/main
git rev-parse --verify -q "$base" >/dev/null || base=main
tab=$(printf '\t')
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# One line per worktree: directory, branch (or "(detached)"), flags.
git worktree list --porcelain | awk '
    /^worktree / { if (w != "") print w "\t" b "\t" f; w = substr($0, 10); b = "(detached)"; f = "" }
    /^branch /   { b = $2; sub("refs/heads/", "", b) }
    /^locked/    { f = f "locked " }
    /^prunable/  { f = f "prunable " }
    /^bare/      { f = f "bare " }
    END          { if (w != "") print w "\t" b "\t" f }
' >"$tmp/worktrees"

gigabytes() {
    du -sk "$1" 2>/dev/null | awk '{ printf "%.1f", $1 / 1048576 }'
}

# Whether anything is running from a worktree: a process with its path on the command line, or
# one holding its build lock or its binaries open.
in_use() {
    if ps -axo command 2>/dev/null | grep -F -- "$1/" | grep -v grep >/dev/null; then
        return 0
    fi
    command -v lsof >/dev/null 2>&1 || return 1
    for f in target/debug/.cargo-lock target/release/.cargo-lock target/debug/degenview target/release/degenview; do
        if [ -e "$1/$f" ] && [ -n "$(lsof -t "$1/$f" 2>/dev/null)" ]; then
            return 0
        fi
    done
    return 1
}

# The pull requests for a branch, one "STATE headRefOid" line each. Fails if gh cannot answer.
pr_states() {
    command -v gh >/dev/null 2>&1 || return 1
    gh pr list --state all --head "$1" --json state,headRefOid --jq '.[] | "\(.state) \(.headRefOid)"' 2>/dev/null
}

# Sets `verdict` (REMOVABLE or KEEP) and `reason` for one worktree.
assess() {
    wt=$1 br=$2 fl=$3 first=$4
    verdict=KEEP
    if [ "$first" = 1 ]; then reason="main worktree"; return; fi
    if [ "$wt" = "$here" ]; then reason="you are here"; return; fi
    case "$fl" in
        *locked*) reason="locked"; return ;;
        *bare*) reason="bare"; return ;;
        *prunable*) reason="directory missing (run git worktree prune)"; return ;;
    esac
    if in_use "$wt"; then reason="in use"; return; fi
    if [ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ]; then
        reason="uncommitted changes"
        return
    fi
    head=$(git -C "$wt" rev-parse HEAD)
    ahead=$(git -C "$wt" rev-list --count "$base..HEAD" 2>/dev/null || echo 0)
    if [ "$br" = "(detached)" ]; then
        # At the tip of the base it may be brand new; anything older is stale.
        if [ "$ahead" = 0 ] && [ "$head" != "$(git rev-parse "$base")" ]; then
            verdict=REMOVABLE
            reason="detached, already in $base"
        else
            reason="detached, not in $base"
        fi
        return
    fi
    if ! prs=$(pr_states "$br"); then
        reason="pull request state unknown (gh)"
        return
    fi
    if [ -z "$prs" ]; then reason="no pull request"; return; fi
    merged=0
    other=""
    while IFS=' ' read -r state oid; do
        case "$state" in
            OPEN) reason="open pull request"; return ;;
            MERGED) if [ "$oid" = "$head" ] || [ "$ahead" = 0 ]; then merged=1; else other="merged, but $ahead newer commit(s)"; fi ;;
            *) other="pull request closed, not merged" ;;
        esac
    done <<EOF
$prs
EOF
    if [ "$merged" = 1 ]; then
        verdict=REMOVABLE
        reason="pull request merged"
    else
        reason="${other:-no merged pull request}"
    fi
}

confirm() {
    [ "$assume_yes" = 1 ] && return 0
    if [ ! -t 0 ] && [ ! -r /dev/tty ]; then
        echo "Not a terminal: pass --yes to go ahead." >&2
        return 1
    fi
    printf '%s [y/N] ' "$1"
    read -r reply </dev/tty
    case "$reply" in y | Y | yes) return 0 ;; *) return 1 ;; esac
}

: >"$tmp/removable"
printf '%-34s %-34s %7s  %s\n' WORKTREE BRANCH "GB" VERDICT
first=1
total=0
while IFS="$tab" read -r wt br fl; do
    assess "$wt" "$br" "$fl" "$first"
    first=0
    size=$(gigabytes "$wt")
    printf '%-34s %-34s %7s  %s (%s)\n' "$(basename "$wt")" "$br" "$size" "$verdict" "$reason"
    if [ "$verdict" = REMOVABLE ]; then
        echo "$wt" >>"$tmp/removable"
        total=$(awk -v a="$total" -v b="$size" 'BEGIN { printf "%.1f", a + b }')
    fi
done <"$tmp/worktrees"
count=$(wc -l <"$tmp/removable" | tr -d ' ')
echo
echo "$count removable, about $total GB."

case "$mode" in
list)
    if [ "$count" -gt 0 ]; then
        echo "Run with --remove-merged to remove them."
    fi
    ;;
remove)
    [ "$count" -gt 0 ] || exit 0
    confirm "Remove $count worktree(s), freeing about $total GB?" || exit 1
    while IFS= read -r wt; do
        if git worktree remove "$wt"; then
            echo "removed $wt"
        else
            echo "could not remove $wt (left as it was)" >&2
        fi
    done <"$tmp/removable"
    git worktree prune
    ;;
idle)
    : >"$tmp/idle"
    idle_total=0
    while IFS="$tab" read -r wt br fl; do
        [ -d "$wt/target/debug" ] || continue
        if in_use "$wt"; then continue; fi
        # Anything built or touched within the window means it is still being worked on.
        if [ -n "$(find "$wt/target/debug/deps" -maxdepth 1 -mtime "-$idle_days" 2>/dev/null | head -n 1)" ]; then
            continue
        fi
        echo "$wt" >>"$tmp/idle"
        idle_total=$(awk -v a="$idle_total" -v b="$(gigabytes "$wt/target/debug")" 'BEGIN { printf "%.1f", a + b }')
    done <"$tmp/worktrees"
    idle_count=$(wc -l <"$tmp/idle" | tr -d ' ')
    if [ "$idle_count" -eq 0 ]; then
        echo "No worktree has been idle for $idle_days day(s)."
        exit 0
    fi
    sed 's/^/  /' "$tmp/idle"
    confirm "Delete target/debug in $idle_count worktree(s), freeing about $idle_total GB?" || exit 1
    while IFS= read -r wt; do
        rm -rf "$wt/target/debug"
        echo "cleaned $wt/target/debug"
    done <"$tmp/idle"
    ;;
esac
