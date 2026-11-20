#!/bin/sh
# The user-facing changelog: fragments in changes/ become a CHANGELOG.md section at release time, and
# the release notes are cut from CHANGELOG.md. Plain sh, awk and grep, the same on macOS and Linux.
#
#   tools/changelog.sh check                     validate every fragment, change nothing
#   tools/changelog.sh files                     the fragments a release would consume, one per line
#   tools/changelog.sh render <version> <date>   print the section the fragments make
#   tools/changelog.sh insert <version> <date>   write that section at the top of CHANGELOG.md
#   tools/changelog.sh notes <tag> [<prev-tag>]  print the sections of CHANGELOG.md for versions
#                                                after <prev-tag> up to and including <tag>; a
#                                                stable <tag> leaves out the `### Fixed in release
#                                                candidates` subsections
#
# A fragment is changes/<slug>.md holding one change per line, `new: <text>`, `fixed: <text>` or
# `fixed-rc: <text>` (a bug that only release candidates had); changes/README.md says how to write
# one. The same change text twice, in one fragment or two, is refused. The app reads the
# `## <version>` / `### New` / `### Fixed` / `### Fixed in release candidates` shape of the release
# notes (model::changelog), so keep it when changing `render`.
set -eu

cd "$(dirname "$0")/.."
# Sorting and character classes must not depend on the caller's locale.
export LC_ALL=C

changelog=CHANGELOG.md
fragments_dir=changes

usage() {
    echo "usage: tools/changelog.sh check | files | render <version> <date> | insert <version> <date> | notes <tag> [<prev-tag>]" >&2
    exit 2
}

# Sets `files` to the fragments, in name order. A name outside lowercase slug form is refused, so
# the list can be passed to awk and git unquoted.
files=""
collect_fragments() {
    files=""
    for path in "$fragments_dir"/*.md; do
        [ -e "$path" ] || continue
        name=${path#"$fragments_dir"/}
        [ "$name" != README.md ] || continue
        case "$name" in
            [a-z0-9]*.md) ;;
            *) echo "error: $path: a fragment is named like its slug, lowercase letters, digits and hyphens" >&2; exit 1 ;;
        esac
        case "${name%.md}" in
            *[!a-z0-9-]*) echo "error: $path: a fragment is named like its slug, lowercase letters, digits and hyphens" >&2; exit 1 ;;
        esac
        files="$files $path"
    done
}

# The section for `version` dated `date` from the fragments. Fails with file:line on a malformed
# line, before printing anything.
render() {
    collect_fragments
    heading="## $1 ($2)"
    if [ -z "$files" ]; then
        printf '%s\n' "$heading"
        return
    fi
    # shellcheck disable=SC2086 # `files` holds checked slug names, no spaces.
    awk -v heading="$heading" '
        # Remember where a change text first appeared; a second appearance is a duplicate entry.
        function seen(text) {
            if (text in where) {
                printf "error: %s:%d: the same change as %s\n", FILENAME, FNR, where[text] > "/dev/stderr"
                failed = 1
            } else {
                where[text] = FILENAME ":" FNR
            }
        }
        { sub(/\r$/, ""); sub(/[ \t]+$/, "") }
        /^$/ { next }
        /^new: [^ \t]/ { seen(substr($0, 6)); new[++news] = substr($0, 6); next }
        /^fixed: [^ \t]/ { seen(substr($0, 8)); fixed[++fixes] = substr($0, 8); next }
        /^fixed-rc: [^ \t]/ { seen(substr($0, 11)); fixedrc[++rcfixes] = substr($0, 11); next }
        {
            printf "error: %s:%d: expected `new: <text>`, `fixed: <text>` or `fixed-rc: <text>`\n", FILENAME, FNR > "/dev/stderr"
            failed = 1
        }
        END {
            if (failed) exit 1
            print heading
            if (news) {
                print ""
                print "### New"
                print ""
                for (i = 1; i <= news; i++) print "- " new[i]
            }
            if (fixes) {
                print ""
                print "### Fixed"
                print ""
                for (i = 1; i <= fixes; i++) print "- " fixed[i]
            }
            if (rcfixes) {
                print ""
                print "### Fixed in release candidates"
                print ""
                for (i = 1; i <= rcfixes; i++) print "- " fixedrc[i]
            }
        }
    ' $files
}

# `version` without a leading `v`, refusing anything that is not a plain version.
bare_version() {
    version=${1#v}
    case "$version" in
        [0-9]*.[0-9]*.[0-9]* ) ;;
        *) echo "error: '$1' is not a version" >&2; exit 1 ;;
    esac
    case "$version" in
        *[!0-9A-Za-z.+-]*) echo "error: '$1' is not a version" >&2; exit 1 ;;
    esac
}

[ $# -ge 1 ] || usage
command=$1
shift
case "$command" in
    check)
        [ $# -eq 0 ] || usage
        render 0.0.0 1970-01-01 >/dev/null
        ;;
    files)
        [ $# -eq 0 ] || usage
        collect_fragments
        for path in $files; do
            printf '%s\n' "$path"
        done
        ;;
    render)
        [ $# -eq 2 ] || usage
        bare_version "$1"
        render "$version" "$2"
        ;;
    insert)
        [ $# -eq 2 ] || usage
        bare_version "$1"
        [ -f "$changelog" ] || { echo "error: $changelog is missing" >&2; exit 1; }
        if awk -v version="$version" '
            /^## / { token = $2; sub(/^v/, "", token); if (token == version) found = 1 }
            END { exit !found }
        ' "$changelog"; then
            echo "error: $changelog already has a section for $version" >&2
            exit 1
        fi
        section=$(render "$version" "$2")
        # The intro stays on top; the new section goes above the newest one, one blank line apart.
        printf '%s\n' "$section" | awk -v changelog="$changelog" '
            { section[++lines] = $0 }
            END {
                while ((getline line < changelog) > 0) {
                    if (!started && line ~ /^## /) started = 1
                    if (started) rest[++rests] = line; else head[++heads] = line
                }
                while (heads > 0 && head[heads] == "") heads--
                for (i = 1; i <= heads; i++) print head[i]
                if (heads > 0) print ""
                for (i = 1; i <= lines; i++) print section[i]
                if (rests > 0) print ""
                for (i = 1; i <= rests; i++) print rest[i]
            }
        ' > "$changelog.tmp"
        mv "$changelog.tmp" "$changelog"
        ;;
    notes)
        [ $# -ge 1 ] && [ $# -le 2 ] || usage
        bare_version "$1"
        tag_version=$version
        previous=""
        if [ -n "${2:-}" ]; then
            bare_version "$2"
            previous=$version
        fi
        [ -f "$changelog" ] || exit 0
        # Bugs that only release candidates had mean nothing to someone coming from a stable
        # release, so a stable tag's notes leave their subsections out; a candidate's keep them.
        stable=1
        case "${tag_version%%+*}" in
            *-*) stable=0 ;;
        esac
        # From the tag's heading down to, not including, the previous release's heading (or the end
        # when the previous release has none); nothing when the tag has no section. A section is
        # held until its end: in a stable tag's notes one whose only entries were release-candidate
        # fixes is left with a bare heading, so it is dropped.
        awk -v tag="$tag_version" -v previous="$previous" -v stable="$stable" '
            function keep(line) {
                held[++lines] = line
                if (line ~ /^[-*] /) entries++
            }
            function flush(   at) {
                if (lines > 0 && !(removed && entries == 0)) {
                    at = 1
                    # Nothing before the first section: its separating blank lines are not output.
                    while (!printed && at <= lines && held[at] == "") at++
                    for (; at <= lines; at++) print held[at]
                    printed = 1
                }
                lines = 0; entries = 0; removed = 0
            }
            { sub(/\r$/, "") }
            /^## / {
                flush()
                token = $2
                sub(/^v/, "", token)
                if (token == tag) active = 1
                else if (active && previous != "" && token == previous) active = 0
                else if (!active) next
            }
            active && stable {
                if (tolower($0) ~ /^### +fixed in release candidates[ \t]*$/) { skipping = 1; removed = 1; next }
                if (skipping && /^#/) skipping = 0
                if (skipping) next
            }
            active {
                if ($0 == "") { blanks++; next }
                while (blanks > 0) { keep(""); blanks-- }
                keep($0)
            }
            !active { blanks = 0 }
            END { flush() }
        ' "$changelog"
        ;;
    *) usage ;;
esac
