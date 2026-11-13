#!/bin/sh
# Builds every negative/*.rs example; each one is EXPECTED to fail. Prints the first error.
cd "$(dirname "$0")"
for f in negative/*.rs; do
  n=$(basename "$f" .rs)
  out=$(cargo build --quiet --example "$n" --features negative 2>&1)
  if [ $? -eq 0 ]; then
    echo "UNEXPECTED PASS: $n"
  else
    echo "== $n: $(echo "$out" | grep -m1 -E '^error')"
    echo "$out" | grep -m3 -E '^\s+= (note|help)|evaluation panicked|not non-capturing|^\s+\|.*\^' | sed 's/^/     /' | head -3
  fi
done
