#!/usr/bin/env python3
"""Aggregate results/raw.log (key=value lines from scan-bench) into markdown tables.

Usage: python3 -I aggregate.py results/raw.log [scaling|alts]
Median over repetitions of the same (mode, dir, threads, backend, order) configuration.
"""
import os
import statistics
import sys
from collections import defaultdict

HOME = os.path.expanduser("~")


def parse(path):
    rows = []
    for line in open(path):
        line = line.strip()
        if not line.startswith("mode="):
            continue
        d = {}
        # values never contain spaces except dir paths, which have none in our test set
        for tok in line.split(" "):
            if "=" in tok:
                k, v = tok.split("=", 1)
                d[k] = v
        rows.append(d)
    return rows


def key(r):
    mode = r["mode"]
    if mode == "custom":
        mode = f"custom/{r.get('backend')}"
    if mode == "dua" and r.get("order") == "completion":
        mode = "dua/completion"
    return (r["dir"].replace(HOME, "~"), mode, int(r.get("threads", 1)))


def med(rows, k):
    vals = [float(r[k]) for r in rows if k in r]
    return statistics.median(vals) if vals else float("nan")


def main():
    rows = parse(sys.argv[1])
    what = sys.argv[2] if len(sys.argv) > 2 else "scaling"
    groups = defaultdict(list)
    for r in rows:
        groups[key(r)].append(r)

    dirs = []
    for (d, _, _) in groups:
        if d not in dirs:
            dirs.append(d)

    if what == "scaling":
        modes = sorted({m for (_, m, _) in groups if m.startswith("custom/")})
        threads = sorted({t for (_, m, t) in groups if m.startswith("custom/")})
        for d in dirs:
            any_row = next(iter(groups[(d, modes[0], threads[0])]), None) if (d, modes[0], threads[0]) in groups else None
            if any_row is None:
                continue
            print(f"\n**{d}** ({any_row['entries']} entries, {int(any_row['alloc_kib'])/1048576:.1f} GiB)\n")
            print("| threads | " + " | ".join(f"{m.split('/')[1]} wall ms" for m in modes) + " | " + " | ".join(f"{m.split('/')[1]} k entries/s" for m in modes) + " | at CPU % | at sys ms |")
            print("|---:|" + "---:|" * (2 * len(modes) + 2))
            for t in threads:
                cells = []
                for m in modes:
                    g = groups.get((d, m, t), [])
                    cells.append(f"{med(g, 'wall_ms'):.0f}" if g else "-")
                for m in modes:
                    g = groups.get((d, m, t), [])
                    cells.append(f"{med(g, 'eps')/1000:.0f}" if g else "-")
                g = groups.get((d, "custom/at", t), [])
                cells.append(f"{med(g, 'cpu_pct'):.0f}" if g else "-")
                cells.append(f"{med(g, 'sys_ms'):.0f}" if g else "-")
                print(f"| {t} | " + " | ".join(cells) + " |")
    else:
        for d in dirs:
            entries = next((g[0]["entries"] for (dd, m, t), g in groups.items() if dd == d and "entries" in g[0]), None)
            print(f"\n**{d}** ({entries} entries)\n")
            print("| walker | threads | wall ms | k entries/s | CPU % | user ms | sys ms | peak RSS MiB | total KiB |")
            print("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
            items = sorted(((m, t, g) for (dd, m, t), g in groups.items() if dd == d), key=lambda x: (x[0] != "du", x[0], x[1]))
            for m, t, g in items:
                eps = f"{med(g, 'eps')/1000:.0f}" if "eps" in g[0] else "-"
                print(f"| {m} | {t} | {med(g, 'wall_ms'):.0f} | {eps} | {med(g, 'cpu_pct'):.0f} | {med(g, 'user_ms'):.0f} | {med(g, 'sys_ms'):.0f} | {med(g, 'maxrss_mib'):.1f} | {g[0]['alloc_kib']} |")


if __name__ == "__main__":
    main()
