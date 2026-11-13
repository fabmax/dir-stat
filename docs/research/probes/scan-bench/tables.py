#!/usr/bin/env python3
"""Compact markdown tables for scan-engine.md from the raw logs (medians)."""
import os
import statistics
import sys
from collections import defaultdict

HOME = os.path.expanduser("~")
TREES = ["/System/Library", "~/.rustup", "/opt/homebrew", "/Applications"]


def load(path):
    rows = []
    for line in open(path):
        if line.startswith("mode="):
            rows.append(dict(t.split("=", 1) for t in line.split() if "=" in t))
    for r in rows:
        r["dir"] = r["dir"].replace(HOME, "~")
    return rows


def med(rs, k):
    return statistics.median(float(r[k]) for r in rs)


def scaling(path):
    g = defaultdict(list)
    for r in load(path):
        g[(r["dir"], int(r["threads"]), r["backend"])].append(r)
    print("| threads | " + " | ".join(f"`{t}` std / at / bulk" for t in TREES) + " |")
    print("|---:|" + "---:|" * len(TREES))
    for t in [1, 2, 4, 8, 12, 16]:
        cells = []
        for d in TREES:
            vals = [g.get((d, t, b)) for b in ("std", "at", "bulk")]
            best = min(med(v, "wall_ms") for v in vals if v)
            cells.append(" / ".join(("**%d**" if med(v, "wall_ms") == best else "%d") % med(v, "wall_ms") for v in vals))
        print(f"| {t} | " + " | ".join(cells) + " |")
    print()
    print("System CPU (ms, `at` backend): " + "; ".join(
        f"`{d}` " + ", ".join(f"{t}t={med(g[(d, t, 'at')], 'sys_ms') / 1000:.1f}s" for t in [1, 4, 8, 16]) for d in TREES))


def alts(path):
    g = defaultdict(list)
    for r in load(path):
        m = r["mode"]
        if m == "custom":
            m = f"custom/{r['backend']}"
        if m == "dua" and r.get("order") == "completion":
            m = "dua-core (Completion order)"
        elif m == "dua":
            m = "dua-core (ParentFirst)"
        t = 1 if r["mode"] in ("du", "walkdir") else int(r["threads"])
        g[(m, t, r["dir"])].append(r)
    names = ["du", "walkdir", "jwalk", "ignore", "dua-core (ParentFirst)", "dua-core (Completion order)", "custom/at", "custom/bulk"]
    print("| walker | threads | " + " | ".join(f"`{t}`" for t in TREES) + " | peak RSS MiB (`~/.rustup`) |")
    print("|---|---:|" + "---:|" * (len(TREES) + 1))
    for n in names:
        for t in sorted({k[1] for k in g if k[0] == n}):
            cells = []
            for d in TREES:
                rs = g.get((n, t, d))
                cells.append(f"{med(rs, 'wall_ms'):.0f}" if rs else "-")
            rs = g.get((n, t, "~/.rustup"))
            rss = f"{med(rs, 'maxrss_mib'):.0f}" if rs else "-"
            print(f"| {n} | {t} | " + " | ".join(cells) + f" | {rss} |")




def sched(path):
    g = defaultdict(list)
    for r in load(path):
        g[(r["dir"], r["backend"], int(r["threads"]), r["order"])].append(r)
    O = ["steal", "lifo", "fifo"]
    print("| tree | wall ms, `at` 4t (steal / lifo / fifo) | wall ms, `at` 8t | wall ms, `bulk` 8t | top-level dirs done, median (% of scan) | share of bytes final at 50% of scan | peak queue |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for d in TREES:
        f = lambda be, t, k, fmt="%d": " / ".join(fmt % med(g[(d, be, t, o)], k) for o in O)
        pct = lambda be, t, k: " / ".join("%d%%" % round(100 * med(g[(d, be, t, o)], k)) for o in O)
        print(f"| `{d}` | {f('at', 4, 'wall_ms')} | {f('at', 8, 'wall_ms')} | {f('bulk', 8, 'wall_ms')} | {pct('bulk', 8, 'top_done_p50')} | {pct('bulk', 8, 'final_at_half')} | {f('bulk', 8, 'max_outstanding')} |")




def dev(path):
    g = defaultdict(list)
    for r in load(path):
        name = r["mode"] if r["mode"] != "custom" else f"custom/{r['backend']} {r['order']}"
        g[(name, int(r["threads"]) if r["mode"] not in ("du", "walkdir") else 1)].append(r)
    print("| walker | threads | runs | wall s (median) | wall s (min-max) | sys CPU s | peak RSS MiB |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for (n, t) in sorted(g, key=lambda k: (k[0], k[1])):
        rs = g[(n, t)]
        w = [float(r["wall_ms"]) / 1000 for r in rs]
        print(f"| {n} | {t} | {len(rs)} | {statistics.median(w):.1f} | {min(w):.1f}-{max(w):.1f} | {med(rs, 'sys_ms') / 1000:.0f} | {med(rs, 'maxrss_mib'):.0f} |")


def verify(path, *more):
    """verify.txt from bench_misc.sh: per tree a full std-backend verify, then the bulk root line."""
    rows, seen = [], defaultdict(int)
    cur = None
    for p in (path,) + more:
        for line in open(p):
            if line.startswith("verify "):
                d = dict(t.split("=", 1) for t in line.split() if "=" in t)
                d["verdict"] = next((t for t in line.split() if t in ("MATCH", "MISMATCH") or t.startswith("CHURN")), "?")
                d["dir"] = d["dir"].replace(HOME, "~")
                seen[d["dir"]] += 1
                d["backend"] = "bulk" if "dev-verify" in p or seen[d["dir"]] > 1 else "std"
                rows.append(d)
                cur = d
            elif line.strip().startswith("children matched=") and cur is not None:
                cur["children"] = line.split("matched=")[1].split()[0] + " / " + line.split("mismatched=")[1].strip()
    print("| tree | backend | ours KiB | `du -kx` KiB | diff | verdict | errors (ours / du warnings) | extra hard links | children matching du (yes / no) |")
    print("|---|---|---:|---:|---:|---|---:|---:|---:|")
    for d in rows:
        print(f"| `{d['dir']}` | {d['backend']} | {int(d['ours_kib']):,} | {int(d['du_kib']):,} | {d['diff_kib']} | {d['verdict']} | {d['our_errors']} / {d['du_warnings']} | {int(d['hl_dedup']):,} | {d.get('children', '-')} |")


def mem(path):
    """dev-mem.log: thresholds 0 / 1 MiB / max, then --no-model, then the /usr/bin/time run."""
    labels = ["UI tree + every file kept individually (`--threshold 0`)",
              "UI tree + files >= 1 MiB (default)",
              "UI tree, no individual files (`--threshold max`)",
              "scanner only, no UI tree (`--no-model`)",
              "default again, under `/usr/bin/time -l`"]
    print("| configuration (`~/dev`, bulk, 8 threads) | files kept | tree nodes | peak RSS MiB | wall s |")
    print("|---|---:|---:|---:|---:|")
    for lab, r in zip(labels, load(path)):
        print(f"| {lab} | {int(r.get('files_kept', 0)):,} | {int(r.get('nodes', 0)) if r.get('model') == 'true' else 0:,} | {float(r['maxrss_mib']):.0f} | {float(r['wall_ms']) / 1000:.1f} |")


def kv(line):
    return dict(t.split("=", 1) for t in line.split() if "=" in t)


def misc(rdir):
    out = []
    # slow UI
    rows = [kv(l) for l in open(f"{rdir}/ui.log") if l.startswith("mode=")]
    out.append("| tree | UI | wall ms | batches | max listings per batch |")
    out.append("|---|---|---:|---:|---:|")
    for r in rows:
        out.append(f"| `{r['dir'].replace(HOME, '~')}` | {r['ui']} | {float(r['wall_ms']):.0f} | {r['batches']} | {r['max_batch']} |")
    out.append("")
    # cancellation
    rows = [kv(l) for l in open(f"{rdir}/cancel.log") if l.startswith("cancel ")]
    out.append("| tree | cancel after | listings received | cancel -> `Finished` | cancel -> all threads joined |")
    out.append("|---|---:|---:|---:|---:|")
    for r in rows:
        out.append(f"| `{r['dir'].replace(HOME, '~')}` | {r['after_ms']} ms | {r['listed']} | {float(r['to_finished_ms']):.1f} ms | {float(r['to_joined_ms']):.1f} ms |")
    out.append("")
    # wide directory
    rows = [kv(l) for l in open(f"{rdir}/wide.log")]
    g = defaultdict(list)
    canc = None
    for r in rows:
        if "mode" in r:
            g[r["mode"] if r["mode"] != "custom" else f"custom/{r['backend']}"].append(r)
        elif "after_ms" in r:
            canc = r
    out.append("| one directory with 200,000 files | wall ms (median) | batches | sys ms |")
    out.append("|---|---:|---:|---:|")
    for k, rs in g.items():
        b = rs[0].get("batches", "-")
        out.append(f"| {k} | {med(rs, 'wall_ms'):.0f} | {b} | {med(rs, 'sys_ms'):.0f} |")
    if canc:
        out.append(f"\nCancel after {canc['after_ms']} ms inside that directory: `Finished` after {float(canc['to_finished_ms']):.1f} ms.")
    print("\n".join(out))


def bulkbuf(path, devpath=None):
    g = defaultdict(list)
    for l in open(path):
        if l.startswith("mode=custom"):
            r = kv(l)
            g[(r["dir"].replace(HOME, "~"), int(r["bufkib"]))].append(r)
    dirs = []
    for (d, _) in g:
        if d not in dirs:
            dirs.append(d)
    print("| tree (bulk, 8 threads) | " + " | ".join(f"{k} KiB buffer" for k in (8, 32, 64, 256)) + " |")
    print("|---|" + "---:|" * 4)
    for d in dirs:
        print(f"| `{d}` | " + " | ".join(f"{med(g[(d, k)], 'wall_ms'):.0f} ms" if (d, k) in g else "-" for k in (8, 32, 64, 256)) + " |")
    if devpath:
        h = defaultdict(list)
        for l in open(devpath):
            if l.startswith("mode="):
                r = kv(l)
                name = r["mode"] if r["mode"] != "custom" else (f"custom/bulk {r['bufkib']} KiB" if "bufkib" in r else f"custom/{r['backend']}")
                h[name].append(float(r["wall_ms"]) / 1000)
        print("\n`~/dev`, 8 threads, interleaved: " + "; ".join(f"{k}: {', '.join(f'{v:.1f}' for v in vs)} s" for k, vs in h.items()))


if __name__ == "__main__":
    {"scaling": scaling, "alts": alts, "sched": sched, "dev": dev, "verify": verify, "mem": mem,
     "misc": misc, "bulkbuf": bulkbuf}[sys.argv[1]](*sys.argv[2:])
