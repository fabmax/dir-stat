#!/bin/bash
# ~/dev experiments (4.08M entries, larger than the vnode cache): speed, verify vs du, memory, sizes.
set -u
cd "$(dirname "$0")"
B=target/release/scan-bench
D=$HOME/dev
"$B" custom "$D" -t 8 --backend bulk --no-model >/dev/null 2>&1   # warm-up
for rep in 1 2; do
  for cfg in "--backend bulk --order steal -t 8" "--backend std --order steal -t 8" "--backend at --order steal -t 8" \
             "--backend bulk --order lifo -t 8" "--backend std --order lifo -t 8" \
             "--backend bulk --order steal -t 4" "--backend bulk --order steal -t 12" \
             "--backend std --order steal -t 4" "--backend std --order steal -t 12"; do
    "$B" custom "$D" $cfg 2>&1 | grep '^mode=' >> results/dev.log
  done
  "$B" dua "$D" -t 8 2>&1 | grep '^mode=' >> results/dev.log
done
"$B" jwalk "$D" -t 8 2>&1 | grep '^mode=' >> results/dev.log
"$B" ignore "$D" -t 8 2>&1 | grep '^mode=' >> results/dev.log
"$B" du "$D" 2>&1 | grep '^mode=' >> results/dev.log
"$B" verify "$D" -t 8 --backend bulk > results/dev-verify.txt 2>&1
for th in 0 1048576 18446744073709551615; do
  "$B" custom "$D" -t 8 --backend bulk --threshold "$th" 2>&1 | grep '^mode=' >> results/dev-mem.log
done
"$B" custom "$D" -t 8 --backend bulk --no-model 2>&1 | grep '^mode=' >> results/dev-mem.log
/usr/bin/time -l "$B" custom "$D" -t 8 --backend bulk >> results/dev-mem.log 2> results/dev-time-l.txt
"$B" sizes "$D" > results/dev-sizes.txt 2>&1
echo done
