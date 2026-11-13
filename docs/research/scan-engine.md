# dir-stat: filesystem scanning engine

Research, benchmarks and the recommended design for the scanner module.
Prototype / reference implementation: `/tmp/dirstat-work/probes/scan-bench` (see section 9).

Test machine: Apple Silicon, 16 cores (12 performance + 4 efficiency), 64 GB RAM, internal SSD (APFS),
macOS 26.7.1 (25G241), rustc 1.93.1. Date: 2026-11-02.

---

## 0. TL;DR (recommendation)

1. **Write our own walker. Do not parse `du`, do not build on a walker crate.** ~650 lines of std +
   `libc` (engine + queue + mount table, without comments). It is byte-exact with `du -kx` on all 7
   test trees (5.3M entries in total), 2.4-3.7x faster than `du` with the portable std backend and
   within ~10% of the fastest crate (`dua-core`) with the macOS bulk backend, and it produces
   exactly the event stream the UI needs: one `DirListing` per directory, parents strictly before
   children, every announced directory exactly once, at most 20 UI updates per second.
2. **Shape:** N std worker threads with a **std-only work-stealing queue** (per-worker deques; own
   LIFO, steal FIFO) and an `outstanding` counter for termination. A worker lists one directory
   (`read_dir` + one stat per entry), aggregates its files, sends one `DirListing` to an unbounded
   channel, *then* publishes the subdirectories as new jobs. One batcher thread coalesces listings
   and hands the UI at most one `Update` per 50 ms through a non-blocking sink (coalescing
   backpressure: the scan never waits for the UI).
3. **Defaults:** `threads = min(available_parallelism, 8)`: the walk is kernel-bound and on macOS
   more than 8 threads is *slower* (system volume: more than 4). Listing backend =
   `std::fs::read_dir` + `DirEntry::metadata()`: since Rust 1.9x that already is
   `fstatat(dirfd, name)` on macOS (`statx` on Linux); a hand-written libc variant was not faster.
   **`getattrlistbulk`** (what Apple's own `du`/fts uses) is the one optimisation worth doing for
   macOS: 1.1-1.7x faster at 8 threads, up to 3.6x single-threaded, prototype included and verified
   against du -> **v1.1**.
4. **Size semantics = `du -kx`:** allocated bytes (`st_blocks * 512`), hard links counted once per
   scan, directory inodes counted, symlinks never followed, apparent size (`st_size`) as a toggle.
5. **Mount policy:** stay on the root's device **and** treat every mount-table entry below the root as
   a boundary ("separate volume, not scanned" rows in the tree). This is what fixes the APFS firmlink
   problem: `/System/Volumes/Data` has the *same* `st_dev` as `/` (volume group), so a pure `st_dev`
   check (`du -x`, `ncdu -x`, `dua -x`) counts all user data twice when scanning `/`; the mount table
   lists it, while the firmlinks (`/Users`, `/Applications`, ...) are not mounts, so user data is
   counted once and appears at the paths the user knows (gdu does the same).
6. **Memory:** keep every directory (~130 B node), keep only files >= 1 MiB individually (~1% of the
   files on `~/dev`, ~3/4 of the bytes) plus an aggregated "N smaller files" entry per
   directory; list a folder's small files on demand (one `read_dir`) when the user opens it.
   Measured on `~/dev` (533k dirs, 3.5M files): 21 MiB for the engine, ~160 MiB peak with the full
   tree; keeping every file instead would add ~245 MiB.
7. **macOS specifics:** per worker thread `setiopolicy_np` *materialize dataless files: off* (never
   download iCloud/File Provider content, like du) and *atime updates: off*; keep `errno` with
   errors so the UI can tell EPERM (privacy/TCC -> "grant Full Disk Access") from EACCES.
8. **Expect seconds, not milliseconds, for real home folders:** a 4M-entry tree does not fit the
   macOS vnode cache (`kern.maxvnodes` = 524,288), so every scan re-reads metadata from the SSD
   (~30k 4 KiB reads/s): `~/dev` (4.08M entries) takes 15-22 s with 8 threads (du: 80 s). Streaming
   the tree while scanning is therefore essential, and the work-stealing order makes the top-level
   folders fill in early.

---

## 1. Options compared

| | `du` subprocess + parse | walkdir 2.5 | jwalk 0.9 | ignore 0.4 (WalkParallel) | dua-core 6.0 | **custom (prototype)** |
|---|---|---|---|---|---|---|
| **Speed** (2.4, 2.5) | single-threaded (per core it is fast: Apple's fts uses `getattrlistbulk`); 2.4-6x slower than custom@8 | single-threaded | parallel (rayon), full-path lstat | parallel, full-path lstat | parallel work-stealing, `getattrlistbulk` on macOS, parallel stat of wide dirs | parallel; fastest or tied |
| **Streaming for a live tree** | post-order lines (a dir is printed after all its children): the tree cannot be shown until subtrees finish; per-file info only with `-a` (millions of lines) | DFS iterator per entry | per-entry stream, optionally sorted | per-entry callback, no dir ids, no ordering contract | per-entry iterator with dense `DirectoryId` + parent id, `Order::ParentFirst`: tree-friendly | one `DirListing` per directory, parent-before-child and exactly-once guaranteed, dense ids, batched |
| **Progress** | nothing until a subtree finishes | DIY | DIY | DIY | DIY | counters every batch + per-subtree completion |
| **Cancellation** | kill the process | stop iterating | drop iterator | `WalkState::Quit` | `next_cancellable` / drop (joins workers) | `AtomicBool`, latency <= one directory (2.7) |
| **Portability** | BSD vs GNU flags (`-d` vs `--max-depth`, `-A` vs `--apparent-size`), BSD du cannot NUL-separate (newline in a filename breaks parsing), no du on Windows | all | all | all | macOS/Linux/Windows | std backend everywhere; libc only for optional macOS/unix bits |
| **Hard-link dedupe** | yes | DIY | DIY | DIY | DIY (dua-cli has it, not core) | yes, global `(dev, ino)` set for `nlink > 1` |
| **Errors** | localized text on stderr | per entry | per entry | per entry | typed, with path | per directory (`errno` kept) + capped sample list |
| **Symlinks / mounts / exclusions** | `-P` default; `-x` = `st_dev` only (firmlink double count from `/`) | `follow_links(false)`, `same_file_system` | callback | `same_file_system`, gitignore filters (must be disabled) | `descend` predicate, `st_dev` | never follows; `st_dev` + mount table; exclusion = one more set lookup |
| **Dependency weight / risk** | none, but process management + parsing | tiny, mature | rayon + crossbeam; **deprecated** ("Use `dua-core` instead") | crossbeam-deque, globset, regex-automata, memchr, same-file, walkdir, log | crossbeam, libc, windows-sys; **created 2026-08-05, 6 major versions in 3 months** (3.3 -> 6.0), ~5k lines with FFI-heavy macOS/Windows modules | libc |

Verdict:
* `du` gives correct totals but the wrong shape (post-order, no live tree, no progress inside big
  subtrees), is single-threaded and non-portable. Rejected.
* walkdir is the safe single-threaded baseline but 4-16x slower than a parallel walker on this machine.
* jwalk is deprecated by its own author.
* ignore is a gitignore-filtering walker; all its features are things we must turn off.
* dua-core is the most interesting crate (fast, tree-friendly ids, bulk metadata), but it is brand
  new with weekly breaking releases, and we would still have to write the per-directory
  aggregation, batching, completion tracking, mount policy and hard-link handling on top of a
  per-entry iterator. With the bulk backend the custom walker is within ~0-10% of it (2.4, 2.5) with a
  fraction of the code and no API-churn risk. Worth re-evaluating once it has stabilised.

---

## 2. Benchmarks

### 2.1 Method

* Binary: `scan-bench` (release, thin LTO). Every configuration was run 3x back-to-back; the tables
  show the **median**. All runs are **warm cache**: cold cache cannot be forced without root
  (`purge` needs sudo) and the first run of each tree was used as warm-up. Caveat: trees larger than
  the kernel's caches (`~/dev`, 4M entries) re-read metadata from the SSD on every scan, so for them
  "warm" and "cold" are nearly the same (first-ever run 24 s, later runs 15-32 s, see 2.5).
* "entries" = directories + files (+ symlinks etc.); `k entries/s` = entries / wall time.
* CPU and peak RSS from `getrusage(RUSAGE_SELF)` (same numbers as `/usr/bin/time -l`), for `du` from
  `RUSAGE_CHILDREN`.
* The machine was not idle: macOS Storage Management (`ApplicationsStorageExtension`,
  `StorageManagementService`, ~0.5 core each, scanning the same disk), IDEs and other build jobs
  (load average 7-30) were running; expect +-10% noise on the small trees, more on `~/dev`.
* All walkers compute the same thing (du -kx semantics) and agree to the KiB (section 3).

Test trees (all on the internal APFS volume group):

| tree | dirs | files | allocated | files/dir | notes |
|---|---:|---:|---:|---:|---|
| `/System/Library` | 160,711 | 289,448 | 15.5 GiB | 1.8 | sealed system volume, 66 dirs EACCES, 22k hard links |
| `~/.rustup` | 16,045 | 437,808 | 10.1 GiB | 27 | Data volume, many small doc files |
| `/opt/homebrew` | 18,802 | 163,745 | 5.9 GiB | 8.7 | |
| `/Applications` | 11,127 | 64,038 | 16.9 GiB | 5.8 | app bundles |
| `~/.cargo` | 12,456 | 62,598 | 1.8 GiB | 5.0 | |
| `/usr` | 1,199 | 22,050 | 0.8 GiB | 18 | 2.9k hard links |
| `~/dev` | 533,523 | 3,549,233 | 281 GiB | 6.7 | realistic "home directory" load: git repos, node_modules, cargo targets; 479k redundant hard links |

### 2.2 Custom walker: thread scaling and listing backends

Backends: `std` = `std::fs::read_dir` + `DirEntry::metadata()`; `at` = hand-written
`open/fdopendir/readdir/fstatat` (same syscalls, no per-entry allocation); `bulk` = macOS
`getattrlistbulk` (no per-entry syscall). Wall time in ms (median of 3):

| threads | `/System/Library` std / at / bulk | `~/.rustup` std / at / bulk | `/opt/homebrew` std / at / bulk | `/Applications` std / at / bulk |
|---:|---:|---:|---:|---:|
| 1 | 9465 / 11219 / **8811** | 5112 / 5142 / **1424** | 913 / 916 / **682** | 474 / 527 / **396** |
| 2 | 5561 / 5173 / **4795** | 2013 / 2228 / **680** | 477 / 482 / **407** | 254 / 253 / **209** |
| 4 | 3185 / 2844 / **2517** | 567 / 659 / **388** | 358 / 307 / **278** | 203 / 195 / **168** |
| 8 | 3307 / 3173 / **3045** | 510 / 518 / **303** | 304 / 434 / **235** | 159 / 154 / **125** |
| 12 | 4918 / 4679 / **4600** | 588 / 551 / **443** | 498 / 427 / **265** | 170 / 174 / **142** |
| 16 | **5564** / 5696 / 6057 | 641 / 644 / **452** | 438 / 437 / **303** | 183 / 175 / **139** |

System CPU time (`at` backend): `/System/Library` 1t=8.5s, 4t=9.0s, 8t=21.5s, 16t=75.2s; `~/.rustup` 1t=2.6s, 4t=2.2s, 8t=3.5s, 16t=8.0s; `/opt/homebrew` 1t=0.9s, 4t=1.2s, 8t=1.5s, 16t=5.2s; `/Applications` 1t=0.5s, 4t=0.7s, 8t=1.2s, 16t=1.9s

(shared-LIFO queue, `--no-model`, median of 3; bold = fastest backend in the row)

Findings:
* **More than 8 threads is always slower on macOS**, and on the sealed system volume more than 4 is.
  Wall time goes *up* while system CPU explodes (`/System/Library` with `at`: 9.0 s sys at 4 threads,
  21 s at 8, 51 s at 12, 75 s at 16): the walker is kernel-bound (user CPU is < 5% of the total) and
  macOS VFS/APFS lock contention dominates. Our own queue lock is not the bottleneck (see the dua-core
  comparison in 2.4, which uses lock-free work stealing and shows the same curve). dua-cli reached
  the same conclusion and defaults to 8 threads on macOS (`DEFAULT_THREADS = 8`, "without saturating
  macOS with syscalls").
* **Default: `min(available_parallelism, 8)`.** 8 is best or within noise of best on every Data-volume
  tree (where user data lives); only the read-only system volume prefers 4 (8 is ~10% slower there).
* **`std` == `at`** within noise everywhere: since Rust 1.9x, `DirEntry::metadata()` on Apple targets is
  `fstatat(dirfd, name, AT_SYMLINK_NOFOLLOW)` (no full-path resolution), so a hand-written libc
  backend buys nothing. Use std in v1.
* **`bulk` (getattrlistbulk) wins** wherever directories contain many files: 1.7x at 8 threads and 3.6x
  single-threaded on `~/.rustup`, 1.2-1.3x on bundles/homebrew, ~1.05-1.1x on `/System/Library`
  (1.8 files/dir: per-directory open/read overhead dominates there).


### 2.3 Queue discipline (custom walker)

`scan-bench custom --order steal|lifo|fifo`, median of 3, interleaved with a warm-up per tree (the
vnode cache, `kern.maxvnodes = 524288`, cannot hold two of these trees at once) and rotated order.
"top-level dirs done" = median time at which the root's children became complete; "bytes final at
50%" = share of the final total whose subtree was already complete halfway through the scan
(values for `bulk`, 8 threads, steal / lifo / fifo):

| tree | wall ms, `at` 4t (steal / lifo / fifo) | wall ms, `at` 8t | wall ms, `bulk` 8t | top-level dirs done, median (% of scan) | share of bytes final at 50% of scan | peak queue |
|---|---:|---:|---:|---:|---:|---:|
| `/System/Library` | 2452 / 2471 / 2711 | 2622 / 2873 / 2929 | 2556 / 2739 / 2970 | 18% / 78% / 8% | 35% / 33% / 18% | 2863 / 2785 / 43567 |
| `~/.rustup` | 713 / 767 / 781 | 607 / 564 / 526 | 321 / 343 / 341 | 16% / 15% / 15% | 0% / 0% / 0% | 678 / 177 / 4526 |
| `/opt/homebrew` | 322 / 327 / 333 | 305 / 298 / 306 | 234 / 243 / 248 | 21% / 82% / 20% | 9% / 24% / 3% | 360 / 441 / 4198 |
| `/Applications` | 173 / 175 / 185 | 139 / 158 / 153 | 115 / 122 / 124 | 87% / 82% / 100% | 47% / 15% / 7% | 264 / 266 / 2586 |

* **Work stealing** (per-worker deques, own LIFO, steal FIFO; `queue.rs`) is as fast or faster than
  the shared LIFO (up to 9-12% at 8 threads on `/System/Library` and `/Applications`) and gives the
  best progressive picture: workers spread over the top-level folders immediately and each finishes
  its subtree depth-first.
* Shared LIFO leaves most top-level folders untouched until the end (they sit at the bottom of the
  stack): median top-level completion at ~80% of the scan time.
* FIFO (breadth-first) completes small folders early but big subtrees only at the very end, and its
  queue grows to 45k directories.

### 2.4 Alternatives and `du`

Wall time in ms, median of 3, warm cache. All rows compute identical totals. `custom/*` rows here
used the shared-LIFO queue and also built the full UI tree model (hence their RSS); the
work-stealing queue (2.3) is another 0-10% faster at 8 threads.

| walker | threads | `/System/Library` | `~/.rustup` | `/opt/homebrew` | `/Applications` | peak RSS MiB (`~/.rustup`) |
|---|---:|---:|---:|---:|---:|---:|
| du | 1 | 8029 | 1790 | 886 | 570 | 5 |
| walkdir | 1 | 11003 | 4952 | 1037 | 583 | 2 |
| jwalk | 4 | 3083 | 1162 | 526 | 178 | 28 |
| jwalk | 8 | 3139 | 818 | 452 | 145 | 31 |
| jwalk | 16 | 6112 | 704 | 363 | 166 | 52 |
| ignore | 4 | 2435 | 584 | 426 | 198 | 95 |
| ignore | 8 | 2616 | 521 | 317 | 148 | 87 |
| ignore | 16 | 5531 | 656 | 350 | 160 | 86 |
| dua-core (ParentFirst) | 4 | 2240 | 445 | 521 | 366 | 3 |
| dua-core (ParentFirst) | 8 | 2821 | 334 | 491 | 123 | 4 |
| dua-core (ParentFirst) | 16 | 6298 | 516 | 414 | 145 | 6 |
| dua-core (Completion order) | 4 | 2498 | 418 | 552 | 161 | 3 |
| dua-core (Completion order) | 8 | 2891 | 347 | 434 | 123 | 4 |
| dua-core (Completion order) | 16 | 5940 | 526 | 308 | 146 | 6 |
| custom/at | 4 | 2905 | 681 | 489 | 202 | 9 |
| custom/at | 8 | 3351 | 490 | 336 | 148 | 8 |
| custom/at | 16 | 5479 | 706 | 394 | 170 | 8 |
| custom/bulk | 4 | 2359 | 404 | 505 | 155 | 8 |
| custom/bulk | 8 | 3100 | 302 | 335 | 128 | 11 |
| custom/bulk | 16 | 5772 | 466 | 274 | 151 | 12 |

Findings:
* `du` is not slow per core: Apple's `fts` (Libc-1752, `gen/fts.c:757`) itself enumerates with
  `getattrlistbulk`. It is just single-threaded: the parallel walkers are 2.6-3.7x faster on the
  system volume, 5.9x on `~/.rustup` and ~5x on `~/dev` (du: 79.6 s, I/O-bound at 47% CPU).
* walkdir (single-threaded, path-based lstat) is the slowest everywhere.
* Every parallel walker shows the same kernel-bound curve (4-8 threads best, 16 threads 2x slower
  on the system volume) - including dua-core's lock-free work-stealing pool, so this is not a
  property of our queue.
* dua-core (bulk metadata + work stealing) and custom/bulk are the fastest, within ~10% of each other;
  ignore and custom/std(at) follow; jwalk is the slowest parallel option (user CPU 3-10x higher).
* Peak RSS of ignore reaches 95 MiB on `~/.rustup` (it queues `DirEntry`s per thread) - for a
  totals-only walk.

### 2.5 A home-directory-sized tree (`~/dev`, 4.08M entries)

`~/dev` (533,523 directories, 3,549,233 files, 281 GiB: git checkouts, `node_modules`, cargo
`target` dirs) is the realistic "scan my home folder" case. It is 8x larger than the vnode cache
(`kern.maxvnodes` = 524,288; `kern.num_recycledvnodes` climbs by millions per scan) and larger than
what APFS keeps cached: during every scan `iostat` shows 25-38k reads/s of 4 KiB (100-150 MB/s) from
the SSD, so for trees of this size "warm" and "cold" converge (the very first scan took 24 s).

Median of 2 runs (min-max), 8 threads unless noted; `custom/*` runs include building the full UI
tree; bulk runs used a 256 KiB `getattrlistbulk` buffer:

| walker | threads | runs | wall s (median) | wall s (min-max) | sys CPU s | peak RSS MiB |
|---|---:|---:|---:|---:|---:|---:|
| custom/at steal | 8 | 2 | 22.3 | 21.2-23.3 | 97 | 129 |
| custom/bulk lifo | 8 | 2 | 17.2 | 15.3-19.0 | 67 | 165 |
| custom/bulk steal | 4 | 2 | 22.3 | 20.9-23.6 | 41 | 114 |
| custom/bulk steal | 8 | 2 | 25.1 | 17.9-32.2 | 115 | 130 |
| custom/bulk steal | 12 | 2 | 19.0 | 18.0-20.0 | 100 | 164 |
| custom/std lifo | 8 | 2 | 22.2 | 20.3-24.1 | 102 | 126 |
| custom/std steal | 4 | 2 | 29.1 | 28.5-29.6 | 64 | 159 |
| custom/std steal | 8 | 2 | 21.5 | 20.9-22.2 | 98 | 130 |
| custom/std steal | 12 | 2 | 21.4 | 20.4-22.3 | 148 | 161 |
| du | 1 | 1 | 79.6 | 79.6-79.6 | 36 | 92 |
| dua | 8 | 2 | 15.5 | 15.3-15.7 | 72 | 19 |
| ignore | 8 | 1 | 18.0 | 18.0-18.0 | 92 | 473 |
| jwalk | 8 | 1 | 25.5 | 25.5-25.5 | 120 | 807 |

The five memory runs of 2.6 are additional custom/bulk steal 8-thread runs (15.1-17.2 s); over all
7 runs of that configuration the median is **16.5 s** (the 32.2 s run is an outlier).

Findings:
* Throughput falls from 1-1.5M entries/s (cached trees) to 200-270k entries/s; the walk is now partly
  I/O-latency-bound and the kernel burns 40-170 s of system CPU (vnode recycling, metadata I/O).
* 8 threads is still the sweet spot: 4 threads is clearly slower (fewer I/Os in flight), 12 is not
  faster and costs more system CPU.
* Run-to-run variance is large (e.g. custom/bulk at 8 threads: 17.9 s and 32.2 s; macOS Storage
  Management was scanning the same disk in the background): treat differences below ~20% as noise.
* Ranking: dua-core 15.5 s ~ custom/bulk 16.5 s < ignore 18.0 s < custom/std 21.5 s ~ custom/at
  22.3 s < jwalk 25.5 s << du 79.6 s. dua-core's edge on this uncached tree comes from an adaptive
  trick (`crates/dua-lib/src/lib.rs`, `read_dir_native`): it times the first bulk read of each
  directory against thread CPU time and, if the read was I/O-bound, re-lists the directory without
  metadata and spreads the stat calls over idle workers in chunks of 4 - more I/O in flight. A
  possible later optimisation for us; not needed for v1.
* Memory of the totals-only crates on this tree: jwalk 807 MiB and ignore 473 MiB peak RSS (they
  buffer entries per directory/thread), dua-core 18 MiB; our scanner *including the complete UI tree
  of 533k nodes* stays at 100-165 MiB (2.6).

### 2.6 Memory

Peak RSS of the whole process (scanner + batcher + UI tree model in the same process, like the app),
`~/dev` with 533,523 directories and 3,549,233 files:

| configuration (`~/dev`, bulk, 8 threads) | files kept | tree nodes | peak RSS MiB | wall s |
|---|---:|---:|---:|---:|
| UI tree + every file kept individually (`--threshold 0`) | 3,069,871 | 533,523 | 405 | 16.5 |
| UI tree + files >= 1 MiB (default) | 31,458 | 533,523 | 162 | 17.2 |
| UI tree, no individual files (`--threshold max`) | 0 | 533,523 | 160 | 15.1 |
| scanner only, no UI tree (`--no-model`) | 0 | 0 | 21 | 16.5 |
| default again, under `/usr/bin/time -l` | 31,458 | 533,523 | 164 | 16.4 |

* The engine itself is light: **21 MiB** for 4M entries (hard-link set, queues, in-flight batches).
* The UI tree of 533k directories adds ~140 MiB at *peak*: ~70 MiB of `Node`s plus names, and the
  rest is the transient copy when the single `Vec<Node>` doubles. The app should use a chunked arena
  (e.g. `Vec<Box<[Node; 4096]>>`) or reserve capacity, and can shrink `Node` to ~80 B (u32 counters,
  `Option<Box<[FileEntry]>>`) if needed.
* Keeping files >= 1 MiB costs ~2 MiB (31k files); keeping **every** file costs **+245 MiB** for
  3.07M files (~80 B per file including name and vector slack).

In-memory sizes (64-bit): tree `Node` 128 B (136 B in the final prototype, which also keeps
`entry_errors`) + name allocation; `FileEntry` 32 B + name allocation;
`DirListing` message 104 B + its vectors. The hard-link set holds one `(dev, ino)` per hard-linked
inode (`~/dev` has 479k redundant links; the whole engine including this set peaks at 21 MiB).

File-size distribution of `~/dev` (`scan-bench sizes`, every name counted, so hard-linked bytes
are counted several times here): 3,549,233 files; average file name 31.7 bytes, directory name
12.3 bytes; the widest directory holds **215,073 files**.

| file size >= | 4 KiB | 64 KiB | 256 KiB | **1 MiB** | 10 MiB | 100 MiB |
|---|---:|---:|---:|---:|---:|---:|
| share of files | 99.1% | 10.4% | 4.4% | **1.31%** | 0.12% | 0.01% |
| share of bytes | 100% | 93.6% | 86.5% | **73.3%** | 40.4% | 18.1% |

Files >= 1 MiB on the other trees: `/System/Library` 0.86% of the files / 81% of the bytes,
`/Applications` 1.7% / 88%, `/opt/homebrew` 0.29% / 71%, `~/.rustup` 0.20% / 50%. A 1 MiB threshold
therefore keeps ~1% of the files and shows where 50-90% of the space is.

### 2.7 Streaming, batching, cancellation, wide directories (measured)

* **Batch rate:** on every `~/dev` run the UI received 18-20 batches/s (306-641 batches per scan),
  the first one 50 ms after start (it already contains the root's children), at most 2.2-3.3k
  listings per batch.
* **UI-side cost:** applying all 533k listings to the tree model took 30-56 ms of UI-thread time in
  total per scan (0.15% of the scan time); the most expensive single batch took 0.3-0.75 ms. The
  model is not a bottleneck; rendering will be.
* **Progressive picture on `~/dev`** (18 runs): with work stealing half of the top-level folders are
  complete after 4-11% of the scan time and 15-18% of the bytes are final at half time; with the
  shared LIFO the same happens only after 60-92% of the scan time (3% final at half time).

**Slow UI / coalescing backpressure** (`--ui-delay-ms`: the consumer sleeps after every batch;
default configuration). The batcher merges everything that accumulates while the UI is busy, so the
UI gets fewer, bigger batches and the scan is not slowed down (with a 1 s sleep the wall time is
dominated by the consumer's own sleeps until it has taken the final batch; these runs alternated
between the two trees, hence the higher absolute times than in 2.2):

| tree | UI | wall ms | batches | max listings per batch |
|---|---|---:|---:|---:|
| `~/.rustup` | normal | 1742 | 35 | 1614 |
| `~/.rustup` | slow200ms | 1467 | 7 | 4307 |
| `~/.rustup` | slow1000ms | 3065 | 3 | 11120 |
| `/System/Library` | normal | 2934 | 59 | 3158 |
| `/System/Library` | slow200ms | 2918 | 14 | 14973 |
| `/System/Library` | slow1000ms | 5077 | 5 | 64190 |

**Cancellation** (`scan-bench cancel`, 8 threads): time from `cancel()` until `Finished` arrived and
until every thread was joined.

| tree | cancel after | listings received | cancel -> `Finished` | cancel -> all threads joined |
|---|---:|---:|---:|---:|
| `~/dev` | 100 ms | 2323 | 9.7 ms | 9.7 ms |
| `~/dev` | 1000 ms | 23505 | 99.4 ms | 99.5 ms |
| `/System/Library` | 100 ms | 4494 | 8.5 ms | 8.6 ms |
| `/System/Library` | 1000 ms | 59379 | 0.8 ms | 0.8 ms |
| `~/.rustup` | 100 ms | 2440 | 21.1 ms | 21.1 ms |
| `~/.rustup` | 1000 ms | 9114 | 15.9 ms | 16.0 ms |

The worst case (99 ms, `~/dev`) is a worker inside a large directory whose metadata is not cached
(each stat ~100 us): the flag is checked every N entries, so the prototype now checks every 256
instead of 1024 entries. A syscall that blocks (dead network mount, TCC prompt) cannot be
interrupted, hence "UI treats cancel as immediate" (7.6).

**One wide directory** (`fixtures/wide`, 200,000 files of 0-1 KiB in one directory, warm): a single
worker handles it; progress-only batches keep arriving (20/s) while it does.

| one directory with 200,000 files | wall ms (median) | batches | sys ms |
|---|---:|---:|---:|
| custom/std | 515 | 10 | 484 |
| custom/at | 490 | 10 | 476 |
| custom/bulk | 336 | 7 | 335 |
| dua | 377 | - | 360 |

Cancel after 100 ms inside that directory: `Finished` after 2.8 ms.

Even a million-entry directory is a matter of 2-3 s for one worker; splitting the stat work of
wide directories across workers (dua-core does) is not needed for v1.

---

## 3. Correctness: totals vs `du -kx`

`scan-bench verify <dir>` runs `du -kx -d 1 <dir>` before and after the scan, applies the scanner's
batches to the UI-side tree model (`model.rs`), and compares the root total and every child:

| tree | backend | ours KiB | `du -kx` KiB | diff | verdict | errors (ours / du warnings) | extra hard links | children matching du (yes / no) |
|---|---|---:|---:|---:|---|---:|---:|---:|
| `/usr` | std | 792,180 | 792,180 | 0 | MATCH | 1 / 1 | 2,903 | 3 / 4 |
| `/usr` | bulk | 792,180 | 792,180 | 0 | MATCH | 1 / 1 | 2,903 | - |
| `/Applications` | std | 17,745,336 | 17,745,336 | 0 | MATCH | 0 / 0 | 0 | 38 / 0 |
| `/Applications` | bulk | 17,745,336 | 17,745,336 | 0 | MATCH | 0 / 0 | 0 | - |
| `/opt/homebrew` | std | 6,194,028 | 6,194,028 | 0 | MATCH | 0 / 0 | 2 | 24 / 0 |
| `/opt/homebrew` | bulk | 6,194,028 | 6,194,028 | 0 | MATCH | 0 / 0 | 2 | - |
| `~/.cargo` | std | 1,926,224 | 1,926,224 | 0 | MATCH | 0 / 0 | 0 | 3 / 0 |
| `~/.cargo` | bulk | 1,926,224 | 1,926,224 | 0 | MATCH | 0 / 0 | 0 | - |
| `~/.rustup` | std | 10,639,920 | 10,639,920 | 0 | MATCH | 0 / 0 | 0 | 4 / 0 |
| `~/.rustup` | bulk | 10,639,920 | 10,639,920 | 0 | MATCH | 0 / 0 | 0 | - |
| `/System/Library` | std | 16,248,740 | 16,248,740 | 0 | MATCH | 66 / 66 | 22,447 | 122 / 36 |
| `/System/Library` | bulk | 16,248,740 | 16,248,740 | 0 | MATCH | 66 / 66 | 22,447 | - |
| `~/dev` | bulk | 294,716,696 | 294,716,696 | 0 | MATCH | 0 / 0 | 479,362 | 30 / 0 |

(std = default configuration: std backend + work stealing; bulk = getattrlistbulk backend; the
`/System/Library` total grew by 88 KiB between the earlier benchmark runs and this verification
because the system wrote caches - both tools agree on the current state.)

Root totals match to the KiB on every tree (also with the `bulk` backend), and our error count equals
the number of `du` warnings (66 unreadable dirs in `/System/Library`, 1 in `/usr`).

Explained per-directory discrepancies:

* **Hard links (the only per-child differences seen).** A hard-linked inode is counted once, in the
  directory where it is seen *first*. `du` (fts, sequential) sees links in a fixed order; a parallel
  walker sees them in a nondeterministic order. Examples: `/usr/libexec` -7,432 KiB and `/usr/sbin`
  +7,444 KiB vs du (binaries hard-linked between them); in `/System/Library` 36 of 158 children differ
  by a few KiB to 0.5 MiB in either direction (e.g. `CoreServices` -496 KiB, `Audio` +128 KiB). The
  parent total is identical. This matters on developer machines: `~/dev` has 479k redundant links
  (typical sources: cargo hard-links build outputs from `target/*/deps` into `target/*`, pnpm
  hard-links packages from its store), although there all 30 children still matched du. See 7.9 for a
  deterministic alternative (attribute each hard-linked inode to the lowest common ancestor of its
  links = the directory whose deletion actually frees it).
* **Directory inodes' own blocks.** du adds `st_blocks` of every directory. On APFS this is always 0
  (directory data lives in the filesystem B-tree), on ext4 it is >= 4 KiB per directory (1.2 GiB for
  300k dirs). **Decision: count them** (it is free: we stat every subdirectory anyway for `st_dev`),
  so we stay du-compatible on Linux.
* **Apparent size of directories.** `du -A` / `du --apparent-size` adds the directories' `st_size`
  (on APFS a synthetic value, roughly 32 bytes per entry). We deliberately sum `st_size` of
  non-directories only (apparent size of `~/.cargo`: ours 1,820,876,485 vs 1,824,075,365 with dir
  sizes). Allocated sizes are unaffected.
* **Errors.** Unreadable directories contribute nothing beyond what was read (du does the same and
  exits 1). A directory whose listing fails part-way keeps the entries read so far and is flagged.
* **Files changing during the scan** (e.g. builds in `~/dev`) make any two tools differ; `verify`
  reports this as CHURN when du itself changes between its two runs.

---

## 4. Fast stat

* **macOS / std:** in Rust 1.93.1 `DirEntry::metadata()` is `fstatat(dirfd(dir), name,
  AT_SYMLINK_NOFOLLOW)` for `target_vendor = "apple"` (std `sys/fs/unix.rs`); in Rust <= 1.89 it was
  `lstat(full path)` on macOS. So yes, one stat syscall per entry, but relative to the open directory.
  `DirEntry::file_type()` is free (`d_type`, filled by APFS/HFS+/ext4/btrfs/xfs); std falls back to a
  stat only for `DT_UNKNOWN`. We need the stat anyway (size, `st_dev`), so we take the type from
  `st_mode`.
* **Per-entry overhead of std** (a `CString` + `Arc` clone per entry): unmeasurable next to the
  syscall (`std` == `at`, 2.2).
* **getattrlistbulk:** one syscall returns name + attributes for dozens of entries. Measured gain
  1.1-1.7x at 8 threads on cached trees (up to 3.6x single-threaded), ~1.3x on `~/dev`, a 200k-file
  directory 1.5x, and markedly lower system CPU. It is what Apple's own fts/du uses (Libc
  `gen/fts.c`). Gotchas found while prototyping (all handled in `listing.rs`, totals verified
  against du):
  * the record layout varies per entry: directory attributes are absent for files and file attributes
    for directories, **even with `FSOPT_PACK_INVAL_ATTRS`** (measured), so the parser must follow the
    `returned` bitmap field by field;
  * mount points report the attributes of the *covered* directory (wrong `st_dev`): request
    `ATTR_DIR_MOUNTSTATUS` and check `DIR_MNTSTATUS_MNTPOINT`;
  * firmlinks report the firmlink, not the target (`SF_FIRMLINK` in `ATTR_CMN_FLAGS`): harmless for a
    walker that opens subdirectories by path;
  * `ATTR_CMN_ERROR` is not exported by the `libc` crate (0.2.191);
  * `ATTR_FILE_ALLOCSIZE` equals `st_blocks * 512` for every file we compared (and totals match du);
  * XNU (`bsd/vfs/vfs_attrlist.c`, xnu-12377): APFS implements `VNOP_GETATTRLISTBULK` natively;
    requesting `ATTR_CMN_UUID`/`GRPUUID`/`EXTENDED_SECURITY` or omitting `ATTR_CMN_OBJTYPE` forces the
    slow generic emulation (one lookup + getattr per entry); vnodes created during a bulk call are
    marked for rapid aging (`UT_KERN_RAGE_VNODES`), i.e. bulk enumeration pollutes the vnode cache less;
  * buffer size hardly matters (table below); Apple's fts uses 32 KiB, dua-core 64 KiB, the
    prototype 32 KiB (`SCAN_BULK_BUF_KIB` overrides it).
  Recommendation: not in v1 (keep v1 portable and boring), add as a `cfg(target_os = "macos")` backend
  in v1.1 behind the same `list_dir` function; the prototype is ~150 lines.

  Buffer size, `bulk`, 8 threads, median of 3 (these runs alternated between trees, so they include
  cross-tree cache eviction and are slower than in 2.2):

  | tree (bulk, 8 threads) | 8 KiB buffer | 32 KiB buffer | 64 KiB buffer | 256 KiB buffer |
  |---|---:|---:|---:|---:|
  | `/System/Library` | 2612 ms | 2570 ms | 2515 ms | 2624 ms |
  | `~/.rustup` | 1341 ms | 1203 ms | 1276 ms | 1284 ms |
  | `/Applications` | 199 ms | 156 ms | 154 ms | 150 ms |

  `~/dev` (interleaved, 2 runs each): custom/bulk 32 KiB 18.0 / 16.8 s, custom/bulk 256 KiB
  18.8 / 17.8 s, custom/std 22.5 / 21.6 s, dua-core 15.3 / 15.2 s.
* **Linux:** std already uses `statx(dirfd, name, AT_SYMLINK_NOFOLLOW | AT_STATX_SYNC_AS_STAT,
  STATX_BASIC_STATS | STATX_BTIME)` with an `fstatat` fallback. Nothing to gain for v1 (io_uring
  batching of statx would be the next step, not worth it).
* **Windows (nice-to-have):** std's `metadata()` comes from `FindNextFileW` data (no extra syscall),
  but there is no allocated size (needs `GetCompressedFileSizeW` per file) and no link count: use the
  logical size and skip hard-link dedupe there.

---

## 5. Mount points, APFS firmlinks, special files

### 5.1 What this machine looks like

```
$ stat -f '%d %i %N' / /Users /Applications /System/Volumes/Data /System/Volumes/Data/Users
16777232 2 /
16777232 16991 /Users
16777232 250744365 /Applications
16777232 1152921500311879682 /System/Volumes/Data
16777232 16991 /System/Volumes/Data/Users
$ stat -f '%d %N' /dev /System/Volumes/VM /System/Volumes/Preboot /System/Volumes/Data/home
664207251 /dev                       (devfs)
16777233  /System/Volumes/VM         (separate APFS volume: swap)
16777234  /System/Volumes/Preboot
889192451 /System/Volumes/Data/home  (autofs)
$ mount
/dev/disk3s3s1 on / (apfs, sealed, local, read-only, journaled)
/dev/disk3s1 on /System/Volumes/Data (apfs, local, journaled, nobrowse, protect, root data)
map auto_home on /System/Volumes/Data/home (autofs, automounted, nobrowse)
... VM, Preboot, Update, xarts, iSCPreboot, Hardware, Update/mnt1, Update/SFR/mnt1
$ cat /usr/share/firmlinks     # Data-volume folders grafted into the read-only system tree
/AppleInternal /Applications /Library /System/Library/Caches /System/Library/Assets
/System/Library/PreinstalledAssets /System/Library/AssetsV2 /System/Library/PreinstalledAssetsV2
/System/Library/CoreServices/CoreTypes.bundle/Contents/Library /System/Library/Speech
/Users /Volumes /cores /opt /pkg /private /usr/local /usr/libexec/cups /usr/share/snmp
```

* The sealed System volume and the Data volume form an APFS **volume group that reports one
  `st_dev`** (16777232). System-volume inode numbers are offset by `0x0FFFFFFF00000000` so the two
  volumes' inode spaces do not collide.
* **Firmlinks are fully transparent to path lookup and `lstat`:** `/Users` and
  `/System/Volumes/Data/Users` are the *same* `(dev, ino)`. Only `getattrlistbulk` shows the firmlink
  itself (`SF_FIRMLINK`).
* Consequences when scanning `/`: "don't cross devices" (st_dev) does **not** stop at
  `/System/Volumes/Data` -> everything reachable through the firmlinks (practically all user data:
  `/Users`, `/Applications`, `/Library`, `/private`, `/opt`, ...) is counted twice, once via the
  firmlinks and once via `/System/Volumes/Data`. "Cross everything" additionally walks into VM/Preboot/Update/devfs/autofs.
  Verified harmlessly with `du -skc /Applications /System/Volumes/Data/Applications`: 17,745,336 KiB
  each, **35,490,672 KiB total** - du does not recognise the alias (it only dedupes hard-linked
  files), so a walk from `/` that reaches both paths counts the data twice.

### 5.2 How other tools handle it (read from source)

| tool | `-x` implementation | scanning `/` on macOS (inferred from the source and the `st_dev`s above; `/` was not scanned) |
|---|---|---|
| BSD `du` (Apple file_cmds-479, `du/du.c:258`) | `FTS_XDEV` = `st_dev` compare | traverses `/System/Volumes/Data` too -> double count; only HFS+ *directory hard links* are deduped (`dirlinkchk`, via `ATTR_DIR_LINKCOUNT`) |
| ncdu 2.9.2 (`src/scan.zig:227`) | `stat.dev != dir.dev` | same; the man page says "Directory hard links and firmlinks (MacOS) are not supported" |
| dua-cli 3.0 (`src/crossdev.rs`) | `metadata.dev() == root dev` | no firmlink-specific policy for `/`; dua-core re-stats firmlinks only to get their target's metadata |
| gdu (`pkg/device/dev.go`, `mountpoint_alias_darwin.go`) | reads the **mount table** and ignores every nested mount point (plus its `/System/Volumes/Data/...` alias) | with `-x`: skips `/System/Volumes/Data`, follows the firmlinks -> counted once. Without `-x` it only ignores `/proc /dev /sys /run` -> double count |

Of these, only gdu with `-x` avoids the double count, and it does so with the mount table, not with
`st_dev`. dir-stat should do the same, by default.

### 5.3 Policy (implemented in the prototype)

1. Canonicalize the root (`/tmp` -> `/private/tmp`), remember its `st_dev`.
2. A subdirectory is **not descended** (but shown, with a reason) if
   * its `st_dev` differs from the root's -> `Skip::OtherFilesystem` (external disks, VM/Preboot
     volumes, devfs, autofs, network mounts, Linux /proc /sys /dev /run, btrfs subvolumes - e.g.
     Fedora's `/home` then shows up as a separate volume to scan on its own), or
   * its path is a **mount-table entry** strictly below the root (`getmntinfo(MNT_NOWAIT)` on macOS,
     `/proc/self/mountinfo` on Linux) -> `Skip::MountPoint`. This catches macOS
     `/System/Volumes/Data` (same `st_dev`) and Linux bind mounts.
3. Firmlinked folders are not mounts, so they are scanned normally and user data appears once, under
   `/Users`, `/Applications`, ... The only thing lost is Data-volume content not reachable through a
   firmlink - typically root-only system folders such as `.Spotlight-V100`, `.fseventsd` or
   `.DocumentRevisions-V100` that a normal app cannot read anyway. (`/Volumes/Macintosh HD` is a
   symlink to `/` and is not followed either.)
4. If the user explicitly picks a skipped folder (or `/System/Volumes/Data`) as root, it is scanned
   normally: the rules only apply strictly below the root.
5. UI: show skipped mount points as greyed leaf rows ("separate volume - not scanned", optionally
   with its `statfs` used size as information, not added to totals) with a "Scan this volume" action.
6. Never follow symlinks (lstat semantics everywhere; subdirectories are opened with `O_NOFOLLOW` in
   the libc backends, so a directory swapped for a symlink mid-scan is not followed). A symlink counts
   as an entry with its own (usually 0) blocks. Same for sockets/FIFOs/devices.
7. No other exclusions in v1. A user exclusion list would be one more set lookup next to the mount
   check.

---

## 6. Memory

Estimate for the target size in the brief (**~300k directories, ~3M files**), from the measured
struct sizes and the `~/dev` RSS numbers (2.6):

| what is kept | per item | ~300k dirs / ~3M files |
|---|---:|---:|
| every directory (`Node` ~130 B + name; measured ~260 B at peak incl. `Vec` doubling) | 150-260 B | **~45-80 MB** |
| every file individually (`FileEntry` 32 B + name + `Vec` slack; measured ~80 B) | ~80 B | **~240 MB** |
| only files >= 1 MiB (~1% of files, ~3/4 of the bytes) | ~80 B | **~2-3 MB** |
| top-N (e.g. 10) largest per directory | ~80 B | ~15-25 MB (most dirs have < N files) |
| no individual files | 0 | 0 |

Recommendation:
* **Keep every directory** (needed for the tree; ~150 B each is cheap; use a chunked arena so the
  peak does not double when the vector grows).
* **Keep files with `max(alloc, apparent) >= 1 MiB` individually** and fold the rest into the
  directory's `files` aggregate, shown as one row "N smaller files - X MB". Large files are exactly
  what a "free disk space" user looks for, their count is bounded by disk size / 1 MiB, and on real
  data they are ~1% of the files but most of the bytes. A fixed threshold is simpler and more
  predictable than top-N (top-N keeps noise in tiny directories and drops the 11th 4 GB video in a
  big one). Make the threshold a constant, not a setting.
* **On demand:** when the user expands a directory's "smaller files" row, list that one directory
  again (one `read_dir` + stats, milliseconds) instead of keeping 3M names in memory.
* Keeping every file is possible (~240 MB for 3M files; measured +245 MiB for 3.07M files on
  `~/dev`) but buys nothing the on-demand listing doesn't, and makes every batch and the model bigger.
* Not kept at all: per-file timestamps, owners, modes. If "last modified" is wanted later, keep a
  per-directory `max(mtime)` (8 bytes) rather than per-file data.

---

## 7. Recommended architecture for the scanner module

The prototype's `src/scanner.rs`, `src/queue.rs`, `src/listing.rs`, `src/mounts.rs` (engine) and
`src/model.rs` (UI-side tree) are written as the reference implementation; this section is their
specification.

### 7.1 Module layout

```
src/scan/mod.rs      start(), Options, ScanHandle, events (DirListing, Batch, Update, Summary), workers, batcher
src/scan/queue.rs    work-stealing job queue + termination counter          (prototype: queue.rs)
src/scan/listing.rs  list_dir(dir, callback): std backend (+ macOS bulk backend in v1.1)
src/scan/mounts.rs   mount_points_below(root)          (macOS getmntinfo, Linux /proc/self/mountinfo)
src/tree.rs          TreeModel: arena of nodes, apply(DirListing), completion, sorting, path_of()
```
Copy the prototype files (`scanner.rs`, `queue.rs`, `listing.rs`, `mounts.rs`, `model.rs`) as the
starting point; they compile on their own (`probes/engine-check`) and come with invariant tests
(`tests.rs`). Drop the `at` backend and the shared-queue variants unless wanted for comparison.
The engine has no dependency on iced or any async runtime; it talks to the UI through a small
`UpdateSink` trait (7.12).

### 7.2 Public API

```rust
pub type DirId = u32;                       // root = 0, dense
pub struct Options {
    threads: usize,              // min(available_parallelism, 8)
    backend: Backend,            // Std (v1) | Bulk (macOS, v1.1)
    schedule: Schedule,          // WorkStealing
    large_file_threshold: u64,   // 1 MiB
    batch_interval: Duration,    // 50 ms
}
pub fn start(root: &Path, opts: Options, sink: impl UpdateSink) -> io::Result<ScanHandle>;
impl ScanHandle { pub fn cancel(&self); }   // Drop = cancel (threads are not joined on the UI thread)

pub enum Update { Batch(Batch), Finished(Summary) }       // Finished is always last
pub struct Batch { listings: Vec<DirListing>, progress: Progress }
pub struct DirListing {
    id: DirId,
    dir_alloc: u64,               // the directory inode's own blocks (du counts them)
    files: Usage,                 // all non-directories directly inside: alloc, apparent, count
    large_files: Vec<FileEntry>,  // subset with max(alloc, apparent) >= threshold: name, alloc, apparent
    first_child: DirId,           // subdirs[i] has id first_child + i
    subdirs: Vec<Subdir>,         // name + skip: Option<Skip::{OtherFilesystem, MountPoint}>
    error: Option<ScanError>,     // listing failed / incomplete: io::ErrorKind + raw errno
    entry_errors: u32,            // entries that could not be stat'ed
}
pub struct Progress { dirs_listed, files, alloc, errors, outstanding, elapsed }
pub struct Summary { progress, cancelled, hardlinks_deduped, error_samples: Vec<(PathBuf, ScanError)> }
```
All event types derive `Clone` because iced `Message`s used with widgets must be `Clone`
(they are never actually cloned on the hot path).

### 7.3 Threads and queue

* `threads` workers (default `min(available_parallelism, 8)`), named, 256 KiB stacks (the walker is
  iterative), plus 1 batcher thread. No rayon, no async runtime.
* **Work-stealing queue, std only** (`queue.rs`, ~120 lines): one `Mutex<VecDeque<Job>>` per worker.
  A worker pushes the subdirectories it found to its own deque and pops from the back (depth-first
  inside its own subtree); when its deque is empty it steals from the *front* of another worker's
  deque (oldest = shallowest = biggest remaining subtree). `Job = { id, path: PathBuf, dir_alloc }`.
* Idle workers sleep on one `Condvar` guarded by an epoch counter: a worker registers in `sleepers`,
  fences, re-checks all deques, and only then waits until the epoch changes; a producer pushes,
  fences, and bumps the epoch / notifies only if `sleepers > 0` and it pushed more than the one child
  it will take itself (Dekker-style handshake, so a wake-up cannot be lost; even if it were, no job
  can be stranded because its owner drains its own deque before sleeping).
* Why work stealing rather than one shared `Mutex<VecDeque>`: not because of lock contention (a job is
  10-100 us of syscalls, the queue op ~0.1 us), but because of *where* the workers are in the tree.
  With a shared LIFO all idle workers grab the newest jobs = siblings in the same directory, and the
  root's other children sit at the bottom of the stack until the very end. Work stealing spreads the
  workers over different top-level folders (each finished depth-first): 0-12% faster at 8 threads and
  a much better progressive picture (2.3). A shared FIFO is the worst of both (slower, 45k-entry queue,
  big subtrees complete last).
* The simple shared-queue variant (`Schedule::SharedLifo`) is kept in the prototype for comparison;
  it is a valid fallback if the stealing code ever needs to be simplified.
* Paths, not directory fds, in jobs: with thousands of pending directories we cannot keep their fds
  open. One path resolution per *directory* (not per entry) is negligible.

### 7.4 Event flow and id assignment (parents before children)

Worker loop:
```
while let Some(job) = queue.pop(me, &cancelled) {  // blocks; None = complete or cancelled
    let listing = list_one(&job, &mut children);   // read_dir + stat each entry, aggregate
    tx.send(listing);                              // (1) publish the listing ...
    queue.complete(me, &mut children);             // (2) ... then make the children visible
}
```
* `list_one` reserves ids for all subdirectories with **one** `next_id.fetch_add(n)`; children get
  `first_child + i`. The child's own listing can only be produced after (2), which happens after (1),
  and the channel preserves that happens-before order, so **the UI always receives a directory's
  listing before any of its children's**. No reordering buffer is needed.
* A listing does not repeat its own name or parent id: both were announced by the parent's listing
  (`subdirs[i]` at id `first_child + i`). That keeps messages small and lets the UI show every
  subfolder immediately as a "scanning..." row, before its own listing arrives.
* Ids are dense but blocks may arrive out of order (worker A reserves 100..110, worker B 110..120, B's
  listing arrives first): the UI arena grows with `resize_with` and fills the short-lived vacant
  slots when A's listing arrives.
* **Invariant (exactly once):** every subdirectory announced with `skip == None` gets exactly one
  `DirListing` later (possibly with `error`), unless cancelled. Skipped subdirectories get an id (so
  the UI can show them) but never a listing.
* One message per directory, not per file: ~50k-100k messages/s at full speed through an unbounded
  std mpsc channel is negligible. Workers never block on the UI.

### 7.5 Batching and backpressure

* The batcher drains the channel (`recv_timeout` + `try_iter`) and every `batch_interval` (50 ms)
  calls `sink.try_deliver(Update::Batch { listings, progress })`.
* The UI side is a **bounded channel of capacity 1**. If the UI has not consumed the previous batch,
  `try_deliver` returns `Busy(update)` and the batcher keeps appending to that batch: **coalescing
  backpressure**. The UI gets at most 20 updates/s, each containing everything since the last one;
  the scan never slows down because of the UI; memory is bounded by the tree itself.
* Progress-only batches are sent when no directory finished in the interval but the file counter
  moved (a directory with a million entries produces no listing for seconds; workers publish their
  file count every 256 entries).
* `Finished(Summary)` and the final batch use `deliver_final` (may block/retry); `Finished` is
  always the last update, also after cancellation.

### 7.6 Cancellation and shutdown

* `ScanHandle::cancel()` (and `Drop`) sets an `AtomicBool`, then bumps the queue's epoch under its
  lock and `notify_all`s, so no sleeping worker can miss it (a worker checks the flag under the same
  lock right before it waits).
* Workers check the flag before every job and every 256 entries inside a directory; they exit
  without draining the queue. The last exiting worker drops the last `Sender`; the batcher sees
  `Disconnected`, delivers what it has (the partial tree is still useful: "Stop" keeps results) and
  `Finished { cancelled: true }`.
* If the receiver is dropped, the batcher's delivery fails with `Closed` -> it cancels the scan.
* The UI must not join scanner threads and should treat the scan as cancelled immediately: a worker
  blocked in a syscall on a dead network mount (or on a TCC consent dialog) cannot be interrupted;
  it simply exits when the syscall returns.

### 7.7 Termination detection

`outstanding` (an `AtomicUsize`) = jobs queued + jobs being processed, starting at 1 (the root).
`complete()` first adds the children (`outstanding += n`, then pushes them) and only then retires
the finished job (`outstanding -= 1`), so it can never reach 0 while work remains; whoever brings
it to 0 bumps the epoch and wakes everybody; idle workers see `outstanding == 0` and exit; the
channel disconnect tells the batcher the scan is complete. No separate "done" message between
threads, no polling.

### 7.8 Per-subtree completion tracking (UI side)

Derived entirely from the listings, no extra events:
* On `DirListing(id)`: create child nodes `first_child..` with names (state `Pending`, or `Skipped`),
  set `unfinished = number of non-skipped children`, state `Scanning`; add `own` sizes to the node and
  all ancestors (O(depth)).
* If `unfinished == 0`: mark `Done`, decrement the parent's `unfinished`; if it reaches 0 (the parent
  is necessarily already listed), mark the parent `Done`, and so on upwards.
* A folder shows a spinner while `Pending`/`Scanning`; its size is final once `Done`. The root is
  `Done` exactly when the scan completes (asserted by the benchmark on every run).

### 7.9 Size semantics

* `alloc` = `st_blocks * 512` (default; what frees disk space, what du shows, correct for APFS
  compressed files, sparse files and iCloud "dataless" files (0)). `apparent` = `st_size` (toggle).
* Directory total = `dir_alloc + files.alloc + sum(children totals)`.
* Hard links: a global `Mutex<HashSet<(dev, ino)>>` touched only for `nlink > 1`; first sighting
  counts. Totals are exact; attribution is order-dependent (3). Deterministic refinement for later:
  report hard-linked files separately and let the model attribute each inode to the **lowest common
  ancestor** of all its sightings (moving it up when a new link is seen, O(depth)). That makes the
  result order-independent and equal to "space freed by deleting this folder".
* APFS clones (Finder duplicate, `cp -c`) share blocks but each reports full `st_blocks`, so (like du)
  we over-count reclaimable space. APFS can report the unshared part (`ATTR_CMNEXT_PRIVATESIZE`); a
  possible later "reclaimable size" mode.
* Totals will not equal `df` used space: APFS snapshots (Time Machine local snapshots), purgeable
  space, other volumes (VM swap, Preboot), filesystem metadata and unreadable folders are invisible
  to a walker. The UI should say so.

### 7.10 Errors

* Directory cannot be opened/read: `DirListing { error: Some(ScanError { kind, errno }) }`; the node
  is complete with an error badge (size = what was read).
* Entry cannot be stat'ed: `entry_errors += 1`.
* Counters in `Progress`, up to 200 `(path, ScanError)` samples in `Summary` for a
  "N folders could not be read" dialog.
* Keep `errno`: Rust maps both EPERM and EACCES to `PermissionDenied`, but on macOS EPERM on a
  directory read usually means privacy protection (TCC) or a sandbox denial -> hint "grant Full Disk
  Access" with a link to
  `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles`; EACCES means Unix
  permissions (root-owned).
* Recommended UI filter: ENOENT/ENOTDIR/ELOOP on entries or directories = file vanished or swapped
  during the scan: count as "changed during scan", do not show as errors.

### 7.11 macOS specifics

* **I/O policies per worker thread** (`setiopolicy_np` with `IOPOL_SCOPE_THREAD` = 1; not in the
  `libc` crate, declare it; no privileges needed; thread scope keeps the rest of the app unaffected):
  * `IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES` (3) = `IOPOL_MATERIALIZE_DATALESS_FILES_OFF` (1): never
    download iCloud Drive / File Provider content because we looked at it. Apple's du does the same via
    the `vfs.nspace.prevent_materialization` sysctl (`du.c:323`), gdu via `setiopolicy_np` at process
    scope (Apple TN3150).
  * `IOPOL_TYPE_VFS_ATIME_UPDATES` (2) = `IOPOL_ATIME_UPDATES_OFF` (1): do not write access times for
    the millions of directories we read (setiopolicy_np(3): "useful for applications which access a
    large number of files to reduce the metadata I/O writes").
  * Cost: none measurable (A/B with `SCAN_NO_IOPOL=1`, interleaved, 3 trees x 2 backends x 3 runs,
    `results/iopol.log`: with the policies the medians were between 3% slower and 17% faster, i.e.
    noise).
* **TCC:** reading `~/Desktop`, `~/Documents`, `~/Downloads`, removable/network volumes and iCloud
  Drive shows a consent prompt the first time (attributed to the app bundle, or to Terminal when run
  from a terminal); Full-Disk-Access categories (Mail, Messages, Safari, other apps' containers) fail
  with EPERM without a prompt. Scanning `~` or `/` will trigger these; that is expected and should be
  explained in the UI, but never done implicitly (only after the user picked the root). The blocked
  worker does not stall the other workers.
* Firmlinks / volume group: section 5.

### 7.12 iced 0.14 integration (checked against the iced 0.14.0 sources)

```rust
// engine side: the sink is a futures channel; iced re-exports `futures` (without the executor feature)
impl UpdateSink for iced::futures::channel::mpsc::Sender<Update> {
    fn try_deliver(&mut self, u: Update) -> Delivery {
        match self.try_send(u) {
            Ok(()) => Delivery::Accepted,
            Err(e) if e.is_full() => Delivery::Busy(e.into_inner()),
            Err(_) => Delivery::Closed,
        }
    }
    fn deliver_final(&mut self, mut u: Update) {      // retry instead of block_on
        while let Err(e) = self.try_send(u) {
            if !e.is_full() { return }                // UI gone
            u = e.into_inner();
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
// app side, in update():
Message::StartScan(root) => {
    let (tx, rx) = iced::futures::channel::mpsc::channel(1);
    self.scan = Some(scan::start(&root, Options::default(), tx)?);   // drop = cancel
    self.tree = TreeModel::new(root);
    let (task, handle) = Task::run(rx, Message::Scan).abortable();
    self.scan_task = Some(handle.abort_on_drop());
    task
}
Message::Scan(Update::Batch(b)) => { for l in b.listings { self.tree.apply(l) } self.progress = b.progress; ... }
Message::Scan(Update::Finished(s)) => { self.summary = Some(s); self.scan = None; ... }
```
Sorting by size is a view concern: sort the children of the expanded nodes when rendering (or cache
the order per node with a dirty flag set by `apply`); never keep 500k children vectors sorted
incrementally. Progress: counts + bytes + elapsed + the path of the last listed directory
(`tree.path_of(id)`); a determinate bar is possible only for whole-volume roots (scanned entries /
`statfs` used inodes of the volume(s), capped at 99%).

### 7.13 Testing

Already in the prototype (`src/tests.rs`, `cargo test --release`, ~2 s), for all 3 backends x 3
schedules x 1/3/8 threads:
* a fixture tree with hard links within and across subdirectories, a directory symlink (must not be
  followed), a dangling symlink, an empty directory, an unreadable directory (`chmod 000`, must yield
  `errno` EACCES and count as one error), a 2 MiB file (the only "large file"), a 3,000-file directory;
* a test sink that asserts the streaming invariants while applying batches: every listing's id was
  announced by an earlier listing (parent before child), no listing twice, every announced
  directory listed exactly once, nothing after `Finished`; then model total == independent lstat
  walk == summary total, every node `Done`;
* cancellation: `Finished { cancelled: true }` arrives within 1 s and all threads join; dropping the
  receiver terminates the scan.

To add in the app: deep nesting (thousands of levels), symlink loops, a directory removed during the
scan, and a Linux CI job that runs the tests plus a `du -skx` comparison.

---

## 8. Open questions / later

* `getattrlistbulk` backend (v1.1, prototype exists and is verified; keep the 32 KiB buffer).
* Wide directories are listed by one worker; dua-core additionally splits the stat work of wide
  directories across workers (2.7 has the single-directory numbers). Only worth it if users hit
  such directories regularly.
* Deterministic hard-link attribution (lowest common ancestor, 7.9) and an APFS clone-aware
  "reclaimable size" (`ATTR_CMNEXT_PRIVATESIZE`).
* The std backend's `read_dir(path)` follows a symlink if a directory is replaced by one between its
  parent's listing and its own (TOCTOU race); the libc backends open with `O_NOFOLLOW`. Harmless in
  practice (worst case: one symlinked tree counted), but a reason to prefer `at`/`bulk` eventually.
* Linux/Windows were only compile-checked, not run: the engine modules build for
  x86_64/aarch64-unknown-linux-gnu, x86_64-pc-windows-msvc and x86_64-unknown-freebsd
  (`probes/engine-check`, `RUSTC_BOOTSTRAP=1 cargo check -Zbuild-std=std,panic_abort --target ...`).
  The invariant tests and a `du -skx` comparison should run on a Linux CI runner.
* Windows: allocated size via `GetCompressedFileSizeW`, hard links need a handle per file,
  junction/reparse-point policy (std reports junctions as symlinks -> not followed, which is right).
* Rescans / incremental updates (FSEvents on macOS, inotify on Linux) are out of scope for v1;
  "rescan this folder" = start a scan with that folder as root and splice the result into the tree.

---

## 9. Reproducing

```
cd /tmp/dirstat-work/probes/scan-bench
cargo build --release && cargo test --release        # invariant tests (all backends x schedules)
target/release/scan-bench custom <dir> [-t N] [--backend std|at|bulk] [--order steal|lifo|fifo] [--threshold BYTES] [--top N]
target/release/scan-bench verify <dir> [--backend ..]   # vs du -kx -d 1 (root + children)
target/release/scan-bench du|walkdir|jwalk|ignore|dua <dir> [-t N]
target/release/scan-bench cancel <dir> --after-ms 300   # cancellation latency
target/release/scan-bench sizes <dir>                   # file-size distribution
target/release/scan-bench backends <dir>                # std/at/bulk metadata diff for one directory
./bench.sh scaling|alts|sched ; ./bench_dev.sh ; ./bench_misc.sh ; ./bench_dev_bulkbuf.sh
python3 -I tables.py scaling|alts|sched|dev|verify|mem results/<file>.log
```

Files:
* `src/scanner.rs` - engine (reference implementation): options, events, workers, batcher, sink.
* `src/queue.rs` - work queues: std-only work stealing (default) and the shared LIFO/FIFO variants.
* `src/listing.rs` - directory listing backends: `std` (v1), `at` (libc fstatat), `bulk` (macOS
  getattrlistbulk).
* `src/mounts.rs` - mount table (macOS `getmntinfo`, Linux `/proc/self/mountinfo`).
* `src/model.rs` - UI-side tree model: incremental apply, size propagation, completion, sorting, paths.
* `src/tests.rs` - invariant tests on a fixture tree (hard links, symlinks, unreadable dir, wide dir).
* `src/alt.rs`, `src/main.rs` - crate alternatives and the benchmark harness.
* `results/*.log` - raw measurements (one `key=value` line per run), `results/*.txt` - verify/sizes.
* `/tmp/dirstat-work/probes/engine-check` - compiles only the engine modules, for cross-target checks.
* `/tmp/dirstat-work/probes/scripts/make_bigdir.sh` - recreates the 200k-file `fixtures/wide`
  directory (deleted after the measurements; `bench_misc.sh` recreates it on demand).
* Reference sources read (not built): `/tmp/dirstat-work/ext/{dua-cli,gdu,ncdu,file_cmds,jwalk,libc-apple,xnu}`.
