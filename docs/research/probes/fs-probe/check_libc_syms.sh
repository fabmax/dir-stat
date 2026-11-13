#!/bin/bash
# usage: check_libc_syms.sh <libc-src-dir> <subdir...> -- <symbols...>
D="$1"; shift
dirs=()
while [ "$1" != "--" ]; do dirs+=("$D/$1"); shift; done; shift
for s in "$@"; do
  hits=$(grep -rlw -- "$s" "${dirs[@]}" 2>/dev/null | sed "s#$D/##" | tr '\n' ' ')
  printf '%-44s %s\n' "$s" "${hits:-MISSING}"
done
