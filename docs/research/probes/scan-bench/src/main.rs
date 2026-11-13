//! scan-bench: benchmark + verification harness for the dir-stat scanning engine.
//!
//! ```text
//! scan-bench custom  <dir> [-t N] [--backend std|at|bulk] [--order steal|lifo|fifo] [--no-model]
//!                          [--threshold BYTES] [--interval-ms N] [--ui-delay-ms N]
//! scan-bench walkdir <dir>
//! scan-bench jwalk   <dir> [-t N]
//! scan-bench ignore  <dir> [-t N]
//! scan-bench dua     <dir> [-t N] [--completion]
//! scan-bench du      <dir>                  (runs `du -kx -d 0`, parses the total)
//! scan-bench verify  <dir> [-t N] [--backend ..]   (custom vs `du -kx -d 1`, root + children)
//! scan-bench cancel  <dir> [--after-ms N] [-t N]   (cancellation latency)
//! scan-bench sizes   <dir>                  (file-size distribution, for the memory policy)
//! scan-bench backends <dir>                 (list one directory with std/at/bulk, diff metadata)
//! ```
//! Every mode prints one `key=value` line (easy to grep/collect from bench.sh).

mod alt;
mod listing;
mod model;
mod mounts;
mod queue;
mod scanner;
#[cfg(test)]
mod tests;

use listing::Backend;
use model::TreeModel;
use scanner::{Options, Schedule, Update};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Args {
    mode: String,
    dir: PathBuf,
    threads: usize,
    backend: Backend,
    schedule: Schedule,
    model: bool,
    threshold: u64,
    interval_ms: u64,
    ui_delay_ms: u64,
    after_ms: u64,
    completion: bool,
    top: usize,
}

fn parse_args() -> Args {
    let mut it = std::env::args().skip(1);
    let usage = "usage: scan-bench <custom|walkdir|jwalk|ignore|dua|du|verify|cancel|sizes> <dir> [options]";
    let mode = it.next().expect(usage);
    let dir = PathBuf::from(it.next().expect(usage));
    let d = Options::default();
    let mut a = Args {
        mode,
        dir,
        threads: d.threads,
        backend: d.backend,
        schedule: d.schedule,
        model: true,
        threshold: d.large_file_threshold,
        interval_ms: d.batch_interval.as_millis() as u64,
        ui_delay_ms: 0,
        after_ms: 200,
        completion: false,
        top: 0,
    };
    while let Some(flag) = it.next() {
        let mut val = || it.next().unwrap_or_else(|| panic!("missing value for {flag}"));
        match flag.as_str() {
            "-t" | "--threads" => a.threads = val().parse().expect("threads"),
            "--backend" => a.backend = Backend::parse(&val()).expect("backend: std|at|bulk"),
            "--order" => {
                a.schedule = match val().as_str() {
                    "lifo" => Schedule::SharedLifo,
                    "fifo" => Schedule::SharedFifo,
                    "steal" => Schedule::WorkStealing,
                    o => panic!("unknown order {o} (lifo|fifo|steal)"),
                }
            }
            "--no-model" => a.model = false,
            "--threshold" => a.threshold = val().parse().expect("threshold"),
            "--interval-ms" => a.interval_ms = val().parse().expect("interval"),
            "--ui-delay-ms" => a.ui_delay_ms = val().parse().expect("ui delay"),
            "--after-ms" => a.after_ms = val().parse().expect("after"),
            "--completion" => a.completion = true,
            "--top" => a.top = val().parse().expect("top"),
            f => panic!("unknown flag {f}"),
        }
    }
    a
}

fn ms(t: libc::timeval) -> f64 {
    t.tv_sec as f64 * 1e3 + t.tv_usec as f64 / 1e3
}

/// (user ms, sys ms, peak RSS MiB) of this process (`who = RUSAGE_SELF`) or its children.
fn rusage(who: libc::c_int) -> (f64, f64, f64) {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(who, &mut ru) };
    // ru_maxrss: bytes on macOS, KiB on Linux
    let rss_mib = if cfg!(target_os = "macos") { ru.ru_maxrss as f64 / 1048576.0 } else { ru.ru_maxrss as f64 / 1024.0 };
    (ms(ru.ru_utime), ms(ru.ru_stime), rss_mib)
}

#[allow(clippy::too_many_arguments)]
fn print_line(mode: &str, a: &Args, wall: Duration, dirs: u64, files: u64, alloc: u64, apparent: u64, errors: u64, extra: &str) {
    let (user, sys, rss) = rusage(libc::RUSAGE_SELF);
    let entries = dirs + files;
    let secs = wall.as_secs_f64();
    println!(
        "mode={mode} dir={} threads={} wall_ms={:.1} dirs={dirs} files={files} entries={entries} eps={:.0} \
         alloc_kib={} apparent={apparent} errors={errors} user_ms={user:.0} sys_ms={sys:.0} cpu_pct={:.0} maxrss_mib={rss:.1}{extra}",
        a.dir.display(),
        a.threads,
        secs * 1e3,
        entries as f64 / secs,
        alloc.div_ceil(1024),
        (user + sys) / (secs * 1e3) * 100.0,
    );
}

struct CustomRun {
    model: TreeModel,
    summary: scanner::Summary,
    wall: Duration,
    batches: u64,
    max_batch: usize,
    apply_total: Duration,
    apply_max: Duration,
    first_batch: Duration,
    /// Peak number of queued + in-flight directories (sampled once per batch).
    max_outstanding: u64,
    /// Completion time of each child of the root (for the LIFO vs FIFO comparison).
    top_done: Vec<(scanner::DirId, Duration)>,
}

fn run_custom(a: &Args) -> CustomRun {
    let opts = Options {
        threads: a.threads,
        backend: a.backend,
        schedule: a.schedule,
        large_file_threshold: a.threshold,
        batch_interval: Duration::from_millis(a.interval_ms),
    };
    let t0 = Instant::now();
    let (tx, rx) = mpsc::sync_channel::<Update>(1);
    let handle = scanner::start(&a.dir, opts, tx).expect("start scan");
    let mut model = TreeModel::new(handle.root.clone());
    let (mut batches, mut max_batch) = (0u64, 0usize);
    let (mut apply_total, mut apply_max) = (Duration::ZERO, Duration::ZERO);
    let mut first_batch = Duration::ZERO;
    let mut max_outstanding = 0u64;
    let mut top_done: Vec<(scanner::DirId, Duration)> = Vec::new();
    let mut top_seen: std::collections::HashSet<scanner::DirId> = Default::default();
    let summary = loop {
        match rx.recv().expect("scanner hung up without Finished") {
            Update::Batch(b) => {
                if batches == 0 {
                    first_batch = t0.elapsed();
                }
                batches += 1;
                max_batch = max_batch.max(b.listings.len());
                max_outstanding = max_outstanding.max(b.progress.outstanding);
                let t = Instant::now();
                if a.model {
                    for l in b.listings {
                        model.apply(l);
                    }
                }
                let dt = t.elapsed();
                if a.model {
                    let root = model.root();
                    for c in root.first_child..root.first_child + root.child_count {
                        if model.nodes[c as usize].state == model::NodeState::Done && top_seen.insert(c) {
                            top_done.push((c, t0.elapsed()));
                        }
                    }
                }
                apply_total += dt;
                apply_max = apply_max.max(dt);
                if a.ui_delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(a.ui_delay_ms)); // simulate a slow UI
                }
            }
            Update::Finished(s) => break s,
        }
    };
    let wall = t0.elapsed();
    handle.join();
    CustomRun { model, summary, wall, batches, max_batch, apply_total, apply_max, first_batch, max_outstanding, top_done }
}

/// (median completion time of the root's children as a fraction of the scan time,
///  share of the final total that was already final at 50% of the scan time)
fn completion_profile(r: &CustomRun) -> (f64, f64) {
    let wall = r.wall.as_secs_f64();
    let mut fr: Vec<f64> = r.top_done.iter().map(|(_, t)| t.as_secs_f64() / wall).collect();
    fr.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = if fr.is_empty() { f64::NAN } else { fr[fr.len() / 2] };
    let total = r.model.root().total.alloc.max(1) as f64;
    let early: u64 = r
        .top_done
        .iter()
        .filter(|(_, t)| t.as_secs_f64() <= wall * 0.5)
        .map(|(c, _)| r.model.nodes[*c as usize].total.alloc)
        .sum();
    (p50, early as f64 / total)
}

fn custom(a: &Args) {
    let r = run_custom(a);
    let p = &r.summary.progress;
    if a.model {
        r.model.check_complete().expect("model incomplete");
        let root = r.model.root();
        assert_eq!(root.total.alloc, p.alloc, "model total != scanner total");
        assert_eq!(root.total_dirs, p.dirs_listed, "model dirs != scanner dirs");
    }
    let order = match a.schedule {
        Schedule::SharedLifo => "lifo",
        Schedule::SharedFifo => "fifo",
        Schedule::WorkStealing => "steal",
    };
    let (top_p50, final_at_half) = completion_profile(&r);
    let extra = format!(
        " backend={} order={order} model={} batches={} max_batch={} apply_total_ms={:.1} apply_max_ms={:.2} first_batch_ms={:.1} hl_dedup={} \
         max_outstanding={} top_done_p50={top_p50:.2} final_at_half={final_at_half:.2} nodes={} files_kept={} \
         node_bytes={} file_entry_bytes={} listing_bytes={} scanner_elapsed_ms={:.1}",
        a.backend.name(),
        a.model,
        r.batches,
        r.max_batch,
        r.apply_total.as_secs_f64() * 1e3,
        r.apply_max.as_secs_f64() * 1e3,
        r.first_batch.as_secs_f64() * 1e3,
        r.summary.hardlinks_deduped,
        r.max_outstanding,
        r.model.nodes.len(),
        r.model.nodes.iter().map(|n| n.large_files.len()).sum::<usize>(),
        std::mem::size_of::<model::Node>(),
        std::mem::size_of::<scanner::FileEntry>(),
        std::mem::size_of::<scanner::DirListing>(),
        p.elapsed.as_secs_f64() * 1e3,
    );
    print_line("custom", a, r.wall, p.dirs_listed, p.files, p.alloc, r.model.root().total.apparent, p.errors, &extra);
    if a.top > 0 && a.model {
        print_top(&r.model, a.top);
    }
}

/// `--top N`: what the UI shows first - the biggest folders below the root and the biggest files.
fn print_top(m: &TreeModel, n: usize) {
    let mib = |b: u64| b as f64 / 1048576.0;
    println!("largest folders in {} ({} directories):", m.root_path.display(), m.root().total_dirs);
    for c in m.children_by_size(scanner::ROOT_ID).into_iter().take(n) {
        let node = &m.nodes[c as usize];
        println!("  {:>10.1} MiB  {:>8} files  {}", mib(node.total.alloc), node.total.files, m.path_of(c).display());
    }
    let mut files: Vec<(u64, std::path::PathBuf)> = Vec::new();
    for (i, node) in m.nodes.iter().enumerate() {
        for f in &node.large_files {
            files.push((f.alloc.max(f.apparent), m.path_of(i as u32).join(&*f.name)));
        }
    }
    files.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    println!("largest files:");
    for (bytes, path) in files.into_iter().take(n) {
        println!("  {:>10.1} MiB  {}", mib(bytes), path.display());
    }
    let unreadable: u32 = m.nodes.iter().map(|n| n.entry_errors).sum::<u32>()
        + m.nodes.iter().filter(|n| n.error.is_some()).count() as u32;
    println!("unreadable entries/folders: {unreadable}");
}

fn alt(a: &Args) {
    let t0 = Instant::now();
    let t = match a.mode.as_str() {
        "walkdir" => alt::walkdir(&a.dir),
        "jwalk" => alt::jwalk(&a.dir, a.threads),
        "ignore" => alt::ignore(&a.dir, a.threads),
        "dua" => alt::dua(&a.dir, a.threads, !a.completion),
        _ => unreachable!(),
    };
    let wall = t0.elapsed();
    let extra = format!(" hl_dedup={}{}", t.hardlinks_deduped, if a.mode == "dua" && a.completion { " order=completion" } else { "" });
    print_line(&a.mode, a, wall, t.dirs, t.files, t.alloc, t.apparent, t.errors, &extra);
}

/// Runs `du -kx -d <depth> <dir>` and returns path -> KiB, plus the number of stderr lines
/// (permission warnings) and the wall time.
fn run_du(dir: &Path, depth: u32) -> (HashMap<PathBuf, u64>, usize, Duration) {
    let t0 = Instant::now();
    let out = Command::new("du").arg("-kx").arg("-d").arg(depth.to_string()).arg(dir).output().expect("run du");
    let wall = t0.elapsed();
    let mut map = HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((kib, path)) = line.split_once('\t') {
            map.insert(PathBuf::from(path), kib.trim().parse().unwrap_or(0));
        }
    }
    let warnings = String::from_utf8_lossy(&out.stderr).lines().count();
    (map, warnings, wall)
}

fn du(a: &Args) {
    let (map, warnings, wall) = run_du(&a.dir, 0);
    let kib = map.values().copied().max().unwrap_or(0);
    let (u, s, rss) = rusage(libc::RUSAGE_CHILDREN);
    println!(
        "mode=du dir={} threads=1 wall_ms={:.1} alloc_kib={kib} errors={warnings} user_ms={u:.0} sys_ms={s:.0} cpu_pct={:.0} maxrss_mib={rss:.1}",
        a.dir.display(),
        wall.as_secs_f64() * 1e3,
        (u + s) / (wall.as_secs_f64() * 1e3) * 100.0
    );
}

/// Compare the scanner (through the UI model) with `du -kx -d 1`.
fn verify(a: &Args) {
    // du before and after: if they differ, the tree changed while we looked (e.g. ~/dev builds).
    let (du_before, warnings, _) = run_du(&a.dir, 1);
    let r = run_custom(a);
    let (du_after, _, _) = run_du(&a.dir, 1);
    let m = &r.model;
    m.check_complete().expect("model incomplete");
    let canon = std::fs::canonicalize(&a.dir).unwrap();
    let du_root = |map: &HashMap<PathBuf, u64>| map.get(&a.dir).or_else(|| map.get(&canon)).copied().unwrap_or(0);
    let ours = m.root().total.alloc.div_ceil(1024);
    let (b, af) = (du_root(&du_before), du_root(&du_after));
    let verdict = if ours == b || ours == af {
        "MATCH"
    } else if b != af {
        "CHURN(du changed between its two runs)"
    } else {
        "MISMATCH"
    };
    println!(
        "verify dir={} ours_kib={ours} du_kib={b} du_after_kib={af} diff_kib={} {verdict} du_warnings={warnings} our_errors={} hl_dedup={}",
        a.dir.display(),
        ours as i64 - b as i64,
        r.summary.progress.errors,
        r.summary.hardlinks_deduped,
    );
    for (p, k) in r.summary.error_samples.iter().take(5) {
        println!("  error sample: {} ({:?}, errno {:?})", p.display(), k.kind, k.errno);
    }
    // per child of the root
    let (mut matched, mut mismatched) = (0, 0);
    let root = m.root();
    for c in root.first_child..root.first_child + root.child_count {
        let node = &m.nodes[c as usize];
        if matches!(node.state, model::NodeState::Skipped(_)) {
            continue;
        }
        let du_path = a.dir.join(&*node.name);
        let Some(&du_kib) = du_before.get(&du_path) else { continue };
        let ours = node.total.alloc.div_ceil(1024);
        if ours == du_kib || du_after.get(&du_path) == Some(&ours) {
            matched += 1;
        } else {
            mismatched += 1;
            if mismatched <= 10 {
                println!("  child {} ours_kib={ours} du_kib={du_kib} diff_kib={}", du_path.display(), ours as i64 - du_kib as i64);
            }
        }
    }
    println!("  children matched={matched} mismatched={mismatched}");
}

/// Cancellation latency: time from cancel() to `Finished` and to all threads joined.
fn cancel(a: &Args) {
    let opts = Options { threads: a.threads, backend: a.backend, schedule: a.schedule, ..Options::default() };
    let (tx, rx) = mpsc::sync_channel::<Update>(1);
    let handle = scanner::start(&a.dir, opts, tx).expect("start");
    let deadline = Instant::now() + Duration::from_millis(a.after_ms);
    let mut listed = 0usize;
    while Instant::now() < deadline {
        if let Ok(Update::Batch(b)) = rx.recv_timeout(Duration::from_millis(5)) {
            listed += b.listings.len();
        }
    }
    let t0 = Instant::now();
    handle.cancel();
    let mut summary = None;
    for u in rx.iter() {
        match u {
            Update::Batch(b) => listed += b.listings.len(),
            Update::Finished(s) => {
                summary = Some(s);
                break;
            }
        }
    }
    let to_finished = t0.elapsed();
    handle.join();
    let to_joined = t0.elapsed();
    let s = summary.expect("Finished");
    println!(
        "cancel dir={} after_ms={} cancelled={} listed={listed} to_finished_ms={:.2} to_joined_ms={:.2}",
        a.dir.display(),
        a.after_ms,
        s.cancelled,
        to_finished.as_secs_f64() * 1e3,
        to_joined.as_secs_f64() * 1e3
    );
}

/// File size distribution: how many files / bytes are >= threshold (memory policy for files).
fn sizes(a: &Args) {
    let th: [u64; 6] = [4 << 10, 64 << 10, 256 << 10, 1 << 20, 10 << 20, 100 << 20];
    let (mut cnt, mut bytes) = ([0u64; 6], [0u64; 6]);
    let (mut files, mut dirs, mut total, mut name_bytes, mut dir_name_bytes) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut per_dir: HashMap<PathBuf, u64> = HashMap::new();
    for e in walkdir::WalkDir::new(&a.dir).follow_links(false).same_file_system(true).into_iter().flatten() {
        let Ok(m) = e.metadata() else { continue };
        let meta = listing::meta_from_std(&m);
        let nlen = e.file_name().len() as u64;
        if e.file_type().is_dir() {
            dirs += 1;
            dir_name_bytes += nlen;
            continue;
        }
        files += 1;
        name_bytes += nlen;
        total += meta.alloc;
        if let Some(p) = e.path().parent() {
            *per_dir.entry(p.to_path_buf()).or_default() += 1;
        }
        for i in 0..th.len() {
            if meta.alloc.max(meta.apparent) >= th[i] {
                cnt[i] += 1;
                bytes[i] += meta.alloc;
            }
        }
    }
    let max_per_dir = per_dir.values().copied().max().unwrap_or(0);
    print!(
        "sizes dir={} dirs={dirs} files={files} alloc_mib={} avg_file_name={:.1} avg_dir_name={:.1} max_files_in_one_dir={max_per_dir}",
        a.dir.display(),
        total >> 20,
        name_bytes as f64 / files.max(1) as f64,
        dir_name_bytes as f64 / dirs.max(1) as f64
    );
    for i in 0..th.len() {
        print!(
            " ge{}k={}({:.2}%files,{:.1}%bytes)",
            th[i] >> 10,
            cnt[i],
            cnt[i] as f64 * 100.0 / files.max(1) as f64,
            bytes[i] as f64 * 100.0 / total.max(1) as f64
        );
    }
    println!();
}

fn main() {
    let a = parse_args();
    match a.mode.as_str() {
        "custom" => custom(&a),
        "walkdir" | "jwalk" | "ignore" | "dua" => alt(&a),
        "du" => du(&a),
        "verify" => verify(&a),
        "cancel" => cancel(&a),
        "sizes" => sizes(&a),
        "backends" => compare_backends(&a.dir),
        m => panic!("unknown mode {m}"),
    }
}

/// Debug helper: list one directory (non-recursively) with every backend and diff the metadata.
#[allow(dead_code)]
pub fn compare_backends(dir: &Path) {
    use listing::{list_dir, Kind, Meta};
    use std::ffi::OsString;
    let mut all: Vec<(&str, HashMap<OsString, (Kind, Option<Meta>)>)> = Vec::new();
    let backends: Vec<Backend> = ["std", "at", "bulk"].iter().filter_map(|b| Backend::parse(b)).collect();
    for b in backends {
        let mut m = HashMap::new();
        let r = list_dir(b, dir, &mut |e| {
            m.insert(e.name.to_owned(), (e.kind, e.meta.ok()));
            true
        });
        println!("backend={} entries={} result={r:?}", b.name(), m.len());
        all.push((b.name(), m));
    }
    let (base_name, base) = &all[0];
    let mut shown = 0;
    for (name, (kind, meta)) in base {
        for (other_name, other) in &all[1..] {
            let Some((k2, m2)) = other.get(name) else {
                println!("  {name:?}: missing in {other_name}");
                continue;
            };
            let (a, b) = (meta.unwrap_or_default(), m2.unwrap_or_default());
            let same = kind == k2 && a.dev == b.dev && a.ino == b.ino && a.nlink == b.nlink && a.alloc == b.alloc && a.apparent == b.apparent;
            if !same && shown < 15 {
                shown += 1;
                println!("  {name:?} {base_name}: {kind:?} {a:?}\n  {name:?} {other_name}: {k2:?} {b:?}");
            }
        }
    }
}
