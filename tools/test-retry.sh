#!/bin/bash
# Checks tools/retry.sh: succeeds after N failures, gives up after the attempt limit, passes arguments
# through, and does not repeat a command that already succeeded. Fast: the delays are set to zero.
set -eu
cd "$(dirname "$0")/.."
. ./tools/retry.sh
export RETRY_BASE_DELAY=0 RETRY_MAX_DELAY=0
dir=$(mktemp -d); trap 'rm -rf "$dir"' EXIT
fail() { echo "FAIL: $1" >&2; exit 1; }

# A command that fails twice, then succeeds: three runs, success.
counter="$dir/count"; : > "$counter"
flaky() { echo x >> "$counter"; [ "$(wc -l < "$counter")" -ge 3 ]; }
retry "flaky" flaky || fail "did not succeed after two failures"
[ "$(wc -l < "$counter")" -eq 3 ] || fail "ran $(wc -l < "$counter") times, expected 3"

# One that always fails: exactly RETRY_ATTEMPTS runs, a failing status, and the annotation.
: > "$counter"
always() { echo x >> "$counter"; return 1; }
out=$(RETRY_ATTEMPTS=4 retry "always" always 2>&1) && fail "reported success for a command that always fails"
[ "$(wc -l < "$counter")" -eq 4 ] || fail "ran $(wc -l < "$counter") times, expected 4"
echo "$out" | grep -q '::error::always failed 4 times' || fail "no error annotation: $out"

# One that succeeds at once runs once (a staple that worked is not repeated), and arguments arrive intact.
: > "$counter"
once() { echo "$*" >> "$counter"; }
retry "once" once "a b" c || fail "a passing command failed"
[ "$(cat "$counter")" = "a b c" ] || fail "arguments changed: $(cat "$counter")"

# Under `set -e` a failure inside `if !` must not exit the caller (the validate-is-a-warning pattern).
( set -e; if ! RETRY_ATTEMPTS=2 retry "always" always >/dev/null 2>&1; then echo warned; fi ) | grep -q warned \
    || fail "a failed retry in an if-condition ended the script"
echo "retry.sh: ok"
