#!/bin/bash
# Creates synthetic trees under /tmp/dirstat-work/probes/fixtures (our own sandbox):
#  wide/   one directory with N files of 0..1 KiB (wide-directory behaviour, cancellation inside a dir)
set -eu
N=${1:-200000}
F=/tmp/dirstat-work/probes/fixtures
mkdir -p "$F/wide"
cd "$F/wide"
perl -e 'my $n = shift; for my $i (1..$n) { open(my $f, ">", "f$i") or die; print $f ("x" x ($i % 1024)); close $f }' "$N"
ls -f | wc -l
