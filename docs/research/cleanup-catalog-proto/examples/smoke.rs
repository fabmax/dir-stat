//! Read-only smoke test: walk the given roots (no symlinks, same device, depth-limited, no
//! descent into matched targets) and print what the catalog flags.
use cleanup_catalog_proto::engine::{Ctx, Engine, Resolver, Verdict, is_valid_cachedir_tag};
use cleanup_catalog_proto::model::{m, marker_of};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn listing(dir: &Path) -> Option<(Vec<(String, bool)>, u64)> {
    let mut v = Vec::new();
    let mut own = 0u64;
    for ent in std::fs::read_dir(dir).ok()?.flatten() {
        let Ok(ft) = ent.file_type() else { continue };
        let name = ent.file_name().to_string_lossy().into_owned();
        let is_dir = ft.is_dir();
        let mut bit = if ft.is_symlink() { 0 } else { marker_of(&name, is_dir) };
        if bit == m::CACHEDIR_TAG && !is_valid_cachedir_tag(&ent.path()) {
            bit = 0;
        }
        own |= bit;
        v.push((name, is_dir && !ft.is_symlink()));
    }
    Some((v, own))
}

fn walk(e: &Engine, dir: &Path, dev: u64, parent: &Ctx, parent_name: &str, depth: u32, max: u32, out: &mut Vec<(String, String)>, stats: &mut BTreeMap<String, u32>) {
    let Some((entries, own)) = listing(dir) else { return };
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (v, ctx) = e.classify_dir(parent, parent_name, &name, own);
    let label = match v {
        Verdict::Target { rule, by_default } => Some(format!("{}{} [{:?}]", e.rule(rule).id, if by_default { "*" } else { "" }, e.rule(rule).risk().unwrap())),
        Verdict::Protected(r) => Some(format!("protected:{}", e.rule(r).id)),
        Verdict::Container(r) => Some(format!("container:{}", e.rule(r).id)),
        _ => None,
    };
    if let Some(l) = label {
        *stats.entry(l.split(' ').next().unwrap().to_string()).or_default() += 1;
        out.push((dir.display().to_string(), l));
    }
    for (fname, is_dir) in &entries {
        if !is_dir {
            if let Some(r) = e.classify_file(&ctx, fname) {
                out.push((dir.join(fname).display().to_string(), format!("file:{}", e.rule(r).id)));
            }
        }
    }
    if matches!(v, Verdict::Target { .. }) || depth >= max {
        return;
    }
    for (child, is_dir) in entries {
        if !is_dir { continue; }
        let p = dir.join(&child);
        let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
        if md.dev() != dev { continue; }
        walk(e, &p, dev, &ctx, &name, depth + 1, max, out, stats);
    }
}

fn main() {
    let e = Engine::new(&Resolver::from_env());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let max: u32 = args[0].parse().unwrap();
    for root in &args[1..] {
        let root = Path::new(root);
        let dev = std::fs::symlink_metadata(root).unwrap().dev();
        let parent = root.parent().unwrap();
        let (_, pctx) = e.enter(&parent.display().to_string());
        let pname = parent.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut out = Vec::new();
        let mut stats = BTreeMap::new();
        let t = std::time::Instant::now();
        walk(&e, root, dev, &pctx, &pname, 0, max, &mut out, &mut stats);
        println!("=== {} ({} flagged dirs, {:?})", root.display(), out.len(), t.elapsed());
        for (k, n) in &stats { println!("    {n:5}  {k}"); }
        for (p, l) in out.iter().filter(|(_, l)| !l.starts_with("protected:vcs") && !l.starts_with("protected:app-bundles")).take(400) {
            println!("  {l:45} {p}");
        }
    }
}
