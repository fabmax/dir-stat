#!/usr/bin/env python3
"""Schedule comparison table from results/sched.log (median over repetitions)."""
import os
import statistics
import sys
from collections import defaultdict

HOME = os.path.expanduser("~")
rows = []
for line in open(sys.argv[1]):
    if line.startswith("mode=custom"):
        rows.append(dict(t.split("=", 1) for t in line.split() if "=" in t))
g = defaultdict(list)
for r in rows:
    g[(r["dir"].replace(HOME, "~"), r["backend"], int(r["threads"]), r["order"])].append(r)
med = lambda rs, k: statistics.median(float(r[k]) for r in rs)
print("| tree | backend | threads | schedule | wall ms | sys ms | peak queue | top-level done (median, % of scan) | bytes final at 50% of scan |")
print("|---|---|---:|---|---:|---:|---:|---:|---:|")
for k in sorted(g, key=lambda k: (k[0], k[1], k[2], ["steal", "lifo", "fifo"].index(k[3]))):
    rs = g[k]
    print(f"| {k[0]} | {k[1]} | {k[2]} | {k[3]} | {med(rs,'wall_ms'):.0f} | {med(rs,'sys_ms'):.0f} | {med(rs,'max_outstanding'):.0f} | {100*med(rs,'top_done_p50'):.0f}% | {100*med(rs,'final_at_half'):.0f}% |")
