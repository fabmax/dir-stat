#!/bin/bash
# Streaming / batching / cancellation / wide-directory / bulk-buffer experiments + du verification.
set -u
cd "$(dirname "$0")"
B=target/release/scan-bench
R=results
FIX=/tmp/dirstat-work/probes/fixtures

# 1) verification of the defaults (std backend, work stealing) against du -kx -d 1
for d in /usr /Applications /opt/homebrew "$HOME/.cargo" "$HOME/.rustup" /System/Library; do
  "$B" verify "$d" >> $R/verify.txt 2>&1
  "$B" verify "$d" --backend bulk 2>&1 | head -1 >> $R/verify.txt
done

# 2) bulk buffer size (warm trees, 3 runs each, interleaved)
for rep in 1 2 3; do for d in /System/Library "$HOME/.rustup" /Applications; do for kib in 8 32 64 256; do
  SCAN_BULK_BUF_KIB=$kib "$B" custom "$d" -t 8 --backend bulk --no-model 2>&1 | sed "s/^mode=custom/mode=custom bufkib=$kib/" >> $R/bulkbuf.log
done; done; done

# 3) slow UI -> coalescing; batch interval
for d in "$HOME/.rustup" /System/Library; do
  "$B" custom "$d" 2>&1 | sed 's/^mode=custom/mode=custom ui=normal/' >> $R/ui.log
  "$B" custom "$d" --ui-delay-ms 200 2>&1 | sed 's/^mode=custom/mode=custom ui=slow200ms/' >> $R/ui.log
  "$B" custom "$d" --ui-delay-ms 1000 2>&1 | sed 's/^mode=custom/mode=custom ui=slow1000ms/' >> $R/ui.log
done

# 4) cancellation latency
for d in "$HOME/dev" /System/Library "$HOME/.rustup"; do for ms in 100 1000; do
  "$B" cancel "$d" --after-ms $ms >> $R/cancel.log 2>&1
done; done

# 5) one wide directory (200k files)
[ -d $FIX/wide ] || ../scripts/make_bigdir.sh 200000 > /dev/null
"$B" custom $FIX/wide --no-model > /dev/null 2>&1   # warm-up
for rep in 1 2; do for be in std at bulk; do
  "$B" custom $FIX/wide --backend $be 2>&1 | grep '^mode=' >> $R/wide.log
done; "$B" dua $FIX/wide 2>&1 | grep '^mode=' >> $R/wide.log; done
"$B" cancel $FIX/wide --after-ms 100 >> $R/wide.log 2>&1

# 6) file-size distribution of the other trees (single-threaded walkdir)
for d in /System/Library "$HOME/.rustup" /Applications /opt/homebrew; do "$B" sizes "$d" >> $R/sizes.txt 2>&1; done

# 7) du does not recognise firmlinked aliases
du -skc /Applications /System/Volumes/Data/Applications > $R/du-firmlink.txt 2>&1
echo done
