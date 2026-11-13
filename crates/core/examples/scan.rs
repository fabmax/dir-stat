//! Scans a folder into a `Tree` and prints totals and the biggest children, for comparison
//! with `du -skx <dir>` (KiB).
//!
//! ```text
//! cargo run -p dirstat-core --release --example scan -- ~/.cargo [--threads N] [--cancel-after-ms N]
//! ```

use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use dirstat_core::catalog::{Engine, Resolver};
use dirstat_core::scan::{self, NodeId, Options, ScanRequest, Update};
use dirstat_core::tree::Tree;

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut root, mut threads, mut cancel_after) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--threads" => threads = args.next().and_then(|n| n.parse().ok()),
            "--cancel-after-ms" => {
                cancel_after = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .map(Duration::from_millis)
            }
            _ => root = Some(PathBuf::from(arg)),
        }
    }
    let Some(root) = root else {
        eprintln!("usage: scan <dir> [--threads N] [--cancel-after-ms N]");
        std::process::exit(2);
    };

    dirstat_core::platform::init_process();
    let mut opts = Options::new(Arc::new(Engine::new(&Resolver::from_env())));
    if let Some(n) = threads {
        opts.threads = n;
    }
    let started = Instant::now();
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = scan::start(ScanRequest::new(&root), opts, tx);
    let mut tree = Tree::opening(root);
    let mut cancel_sent = None;
    let summary = loop {
        if let (Some(after), None) = (cancel_after, cancel_sent)
            && started.elapsed() >= after
        {
            handle.cancel();
            cancel_sent = Some(Instant::now());
        }
        let update = match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(update) => update,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("scan ended without Finished"),
        };
        match update {
            Update::Started(info) => tree.start(info),
            Update::Failed(e) => {
                eprintln!("can't scan: {}", e.message);
                std::process::exit(1);
            }
            Update::Batch(batch) => {
                for listing in batch.listings {
                    tree.apply(listing);
                }
            }
            Update::Finished(summary) => break summary,
        }
    };
    let elapsed = started.elapsed();
    tree.finish(&summary);
    handle.join();

    let root = tree.node(NodeId::ROOT);
    let total = root.total();
    println!("root        {}", tree.root_path().display());
    println!("fs type     {}", tree.root_fs_type());
    println!(
        "total       {} KiB ({} bytes allocated, {} apparent)",
        total.alloc / 1024,
        total.alloc,
        total.apparent
    );
    println!(
        "items       {} ({} dirs listed)",
        total.items,
        tree.listed()
    );
    println!(
        "errors      {} ({} vanished, {} internal)",
        summary.progress.errors, summary.progress.vanished, summary.internal_errors
    );
    println!(
        "hard links  {} deduped, {} owners kept",
        summary.hardlinks_deduped,
        tree.links().len()
    );
    println!("boundaries  {} skipped below the root", total.boundaries);
    println!(
        "state       root {:?}, cancelled {}",
        root.state(),
        summary.cancelled
    );
    println!(
        "consistent  tree total == summary: {}",
        total.alloc == summary.progress.alloc
    );
    println!("time        {:.2} s", elapsed.as_secs_f64());
    if let Some(sent) = cancel_sent {
        println!(
            "cancel      Finished {} ms after cancel()",
            sent.elapsed().as_millis()
        );
    }
    println!("peak RSS    {:.1} MB", peak_rss_bytes() as f64 / 1e6);

    let mut children: Vec<NodeId> = tree.children(NodeId::ROOT).collect();
    children.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).total().alloc));
    println!("\nlargest children:");
    for child in children.into_iter().take(10) {
        let node = tree.node(child);
        println!("{:>12} KiB  {:?}", node.total().alloc / 1024, node.name());
    }
}

#[cfg(unix)]
fn peak_rss_bytes() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    // ru_maxrss is in bytes on macOS and in KiB on Linux.
    let max = usage.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        max
    } else {
        max * 1024
    }
}

#[cfg(not(unix))]
fn peak_rss_bytes() -> u64 {
    0
}
