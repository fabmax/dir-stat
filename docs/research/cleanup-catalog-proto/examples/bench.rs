//! Rough cost of `classify_dir` + marker computation per directory (release build).
use cleanup_catalog_proto::engine::{Engine, Resolver};
use cleanup_catalog_proto::model::marker_of;
use std::time::Instant;

fn main() {
    let e = Engine::new(&Resolver::from_env());
    let names = ["src", "lib", "node_modules", "target", "build", ".git", "tests", "docs", "assets", "a1b2c3", "Contents", "Resources", "__pycache__", ".venv", "cache", "dist"];
    let entries = ["package.json", "Cargo.toml", "README.md", "index.js", "main.rs", "foo.txt", "image.png", "build.gradle.kts", "CACHEDIR.TAG", "x.csproj"];
    let (_, root) = e.enter("/Users/u/dev/project");
    let n = 2_000_000u32;
    let t = Instant::now();
    let mut hits = 0u32;
    for i in 0..n {
        // markers of a 10-entry listing
        let mut own = 0u64;
        for k in 0..10 {
            own |= marker_of(entries[((i as usize) + k * 7) % entries.len()], k % 3 == 0);
        }
        let name = names[(i as usize) % names.len()];
        let (v, ctx) = e.classify_dir(&root, "project", name, own & 0xF0);
        if !matches!(v, cleanup_catalog_proto::engine::Verdict::None) { hits += 1; }
        std::hint::black_box(ctx);
    }
    let el = t.elapsed();
    println!("{} rules; {n} dirs (10 entries each) in {:?}: {:.0} ns/dir incl. markers; {hits} matches", e.rule_count(), el, el.as_nanos() as f64 / n as f64);
}
