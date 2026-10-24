#!/usr/bin/env bash
# Interleaved A/B benchmark of the Pine engine between two commits.
#
#   tools/pine-bench.sh <base-rev> [head-rev]
#
# Builds the `pine::bench_tests` harness (src/pine/bench_tests.rs) at both revisions, then runs the
# two test binaries alternately for PINE_BENCH_ROUNDS rounds (default 6) so drift in machine load
# hits both equally, and prints the median per workload, the spread between rounds and the change.
# Without `head-rev` the current working tree is the second build. Both revisions must contain the
# harness. Set PINE_BENCH_FEATURES=alloc-count to add allocation counts and peak memory.
#
# Other knobs: PINE_BENCH_REPS (timed runs per round, default 5), PINE_BENCH_ONLY (workload names),
# PINE_BENCH_TARGET (a cargo target directory to keep between invocations).
set -euo pipefail

base_rev=${1:?usage: tools/pine-bench.sh <base-rev> [head-rev]}
head_rev=${2:-}
rounds=${PINE_BENCH_ROUNDS:-6}
features=${PINE_BENCH_FEATURES:-}
root=$(git rev-parse --show-toplevel)
work=$(mktemp -d "${TMPDIR:-/tmp}/pine-bench.XXXXXX")
target=${PINE_BENCH_TARGET:-$work/target}
cleanup() {
  git -C "$root" worktree remove --force "$work/base" >/dev/null 2>&1 || true
  git -C "$root" worktree remove --force "$work/head" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

# build <dir> <label>: prints the path of the lib test binary built in <dir>.
build() {
  (
    cd "$1"
    export CARGO_TARGET_DIR="$target/$2"
    cargo test --release --lib ${features:+--features "$features"} --no-run \
      --message-format=json 2>/dev/null |
      python3 -I -c '
import json, sys
path = ""
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get("reason") == "compiler-artifact" and message.get("profile", {}).get("test") and message.get("executable"):
        path = message["executable"]
print(path)'
  )
}

git -C "$root" worktree add --detach "$work/base" "$base_rev" >/dev/null
base_bin=$(build "$work/base" base)
if [[ -n "$head_rev" ]]; then
  git -C "$root" worktree add --detach "$work/head" "$head_rev" >/dev/null
  head_bin=$(build "$work/head" head)
else
  head_bin=$(build "$root" head)
fi
if [[ ! -x "$base_bin" || ! -x "$head_bin" ]]; then
  echo "could not build both test binaries" >&2
  exit 1
fi

export PINE_BENCH_REPS=${PINE_BENCH_REPS:-5}
for round in $(seq 1 "$rounds"); do
  echo "round $round/$rounds" >&2
  for side in base head; do
    bin=$base_bin
    if [[ $side == head ]]; then bin=$head_bin; fi
    "$bin" pine::bench_tests --ignored --nocapture --test-threads=1 |
      grep 'BENCH ' | sed "s/^.*BENCH /$side /" >>"$work/results.txt"
  done
done

python3 -I - "$work/results.txt" <<'PY'
import statistics, sys
from collections import defaultdict

medians = defaultdict(lambda: defaultdict(list))  # workload -> side -> median_ns of each round
last = defaultdict(dict)                           # workload -> side -> fields of the last run
for line in open(sys.argv[1]):
    side, name, *fields = line.split()
    fields = dict(field.split("=") for field in fields)
    medians[name][side].append(int(fields["median_ns"]))
    last[name][side] = fields

print(f"{'workload':<20}{'base ms':>10}{'head ms':>10}{'change':>9}   rounds base / head (ms)   checksum")
for name, sides in medians.items():
    base, head = sides["base"], sides["head"]
    b, h = statistics.median(base), statistics.median(head)
    same = last[name]["base"]["checksum"] == last[name]["head"]["checksum"]
    print(
        f"{name:<20}{b / 1e6:>10.2f}{h / 1e6:>10.2f}{(h - b) / b * 100:>+8.2f}%"
        f"   {min(base) / 1e6:.2f}-{max(base) / 1e6:.2f} / {min(head) / 1e6:.2f}-{max(head) / 1e6:.2f}"
        f"   {'same' if same else 'DIFFERENT'}"
    )
if any(int(sides["base"].get("allocs", 0)) for sides in last.values()):
    print()
    print(f"{'workload':<20}{'allocs base':>13}{'allocs head':>13}{'bytes base':>15}{'bytes head':>15}{'peak base':>13}{'peak head':>13}")
    for name, sides in last.items():
        b, h = sides["base"], sides["head"]
        print(f"{name:<20}{b['allocs']:>13}{h['allocs']:>13}{b['bytes']:>15}{h['bytes']:>15}{b['peak']:>13}{h['peak']:>13}")
PY
