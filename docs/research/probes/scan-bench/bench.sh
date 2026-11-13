#!/bin/bash
# Benchmark matrix for scan-bench. Appends one key=value line per run to results/raw.log.
# Warm cache: every configuration is run REPS times right after each other; the aggregation
# (aggregate.py) uses the median of the runs. Cold cache cannot be forced reliably on macOS
# without root (`purge` needs sudo), so all numbers are warm-cache numbers.
set -u
cd "$(dirname "$0")"
B=target/release/scan-bench
LOG=results/raw.log
REPS=${REPS:-3}
run() { for i in $(seq "$REPS"); do "$B" "$@" 2>&1 | grep '^mode=' >> "$LOG"; done; }

DIRS=${DIRS:-"/System/Library $HOME/.rustup /opt/homebrew /Applications"}
THREADS=${THREADS:-"1 2 4 8 12 16"}
BACKENDS=${BACKENDS:-"std at bulk"}

case "${1:-all}" in
  scaling)
    for d in $DIRS; do
      "$B" custom "$d" -t 8 --no-model >/dev/null 2>&1   # warm-up
      for t in $THREADS; do for be in $BACKENDS; do run custom "$d" -t "$t" --backend "$be" --no-model; done; done
    done ;;
  alts)
    for d in $DIRS; do
      "$B" custom "$d" -t 8 --no-model >/dev/null 2>&1
      run du "$d"
      run walkdir "$d"
      for t in ${ALT_THREADS:-8 16}; do
        run jwalk "$d" -t "$t"; run ignore "$d" -t "$t"; run dua "$d" -t "$t"; run dua "$d" -t "$t" --completion
        run custom "$d" -t "$t" --backend at; run custom "$d" -t "$t" --backend bulk
      done
    done ;;
  sched)
    # queue discipline comparison, interleaved (rep-major) to spread background-load noise evenly
    # NB: kern.maxvnodes is 524288 on this machine; switching between trees of ~450k entries evicts
    # the previous tree's vnodes, so every tree gets a warm-up run in every repetition and the
    # order of the schedules rotates between repetitions.
    orders=("steal lifo fifo" "lifo fifo steal" "fifo steal lifo")
    for i in $(seq "$REPS"); do
      for d in $DIRS; do
        "$B" custom "$d" -t 8 --no-model >/dev/null 2>&1   # warm-up (vnode cache)
        for t in ${SCHED_THREADS:-4 8}; do for be in ${SCHED_BACKENDS:-at bulk}; do
          for o in ${orders[$(( (i - 1) % 3 ))]}; do "$B" custom "$d" -t "$t" --backend "$be" --order "$o" 2>&1 | grep '^mode=' >> "$LOG"; done
        done; done
      done
    done ;;
  *) echo "usage: bench.sh scaling|alts|sched   (env: REPS, DIRS, THREADS, BACKENDS, ALT_THREADS)"; exit 1 ;;
esac
