#!/bin/bash
# ~/dev: getattrlistbulk buffer size (32 KiB default vs 256 KiB) next to dua-core, interleaved.
set -u
cd "$(dirname "$0")"
B=target/release/scan-bench
D=$HOME/dev
for rep in 1 2; do
  for kib in 32 256; do
    SCAN_BULK_BUF_KIB=$kib "$B" custom "$D" -t 8 --backend bulk 2>&1 | grep '^mode=' | sed "s/^mode=custom/mode=custom bufkib=$kib/" >> results/dev-bulkbuf.log
  done
  "$B" custom "$D" -t 8 --backend std 2>&1 | grep '^mode=' >> results/dev-bulkbuf.log
  "$B" dua "$D" -t 8 2>&1 | grep '^mode=' >> results/dev-bulkbuf.log
done
echo done
