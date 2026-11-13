#!/bin/bash
# A/B: do the per-thread iopolicies (dataless off, atime off) cost anything? Interleaved, warm.
set -u
cd "$(dirname "$0")"
B=target/release/scan-bench
for d in "$HOME/.rustup" /Applications /opt/homebrew; do
  "$B" custom "$d" --no-model > /dev/null 2>&1   # warm-up
  for rep in 1 2 3; do
    "$B" custom "$d" --no-model 2>&1 | sed 's/^mode=custom/mode=custom iopol=on/' >> results/iopol.log
    SCAN_NO_IOPOL=1 "$B" custom "$d" --no-model 2>&1 | sed 's/^mode=custom/mode=custom iopol=off/' >> results/iopol.log
    "$B" custom "$d" --no-model --backend bulk 2>&1 | sed 's/^mode=custom/mode=custom iopol=on/' >> results/iopol.log
    SCAN_NO_IOPOL=1 "$B" custom "$d" --no-model --backend bulk 2>&1 | sed 's/^mode=custom/mode=custom iopol=off/' >> results/iopol.log
  done
done
echo done
