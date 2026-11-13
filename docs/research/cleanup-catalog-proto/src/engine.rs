//! Streaming matcher: indexes the catalog once, then classifies each directory in O(1)-ish
//! time when the scanner has listed it.
//!
//! Information flow (matches the scanner: a directory's listing is produced before any of its
//! children are listed):
//!   * when directory D has been listed, the scanner computes `own = OR(marker_of(entry))` over
//!     D's entries and calls `classify_dir(&parent_ctx, parent_name, d_name, own)`;
//!   * the returned `Ctx` (24 bytes, `Copy`) travels with every child job of D. For the
//!     children it carries D's markers (their *siblings*), D's trie position and zone flags;
//!   * files inside D are classified with `classify_file(&d_ctx, file_name)` (anchored rules
//!     only, e.g. installers in ~/Downloads).
//! Everything a rule may look at - the dir's own name, its parent's name, its path (via the
//! trie), its siblings and its own children - is known exactly when D's listing is done and
//! before any child of D is visited, so the verdict for D is final at that point and
//! "outermost match wins" can be decided top-down without buffering.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::catalog::{CATALOG, Group};
use crate::model::*;

pub type RuleIdx = u16;
pub const NO_RULE: RuleIdx = u16::MAX;
const NO_NODE: u32 = u32::MAX;

const F_NO_HEUR: u8 = 1; // no heuristic rules below (Scope::NoHeuristics)
const F_NO_PROJ: u8 = 2; // no project rules below (Scope::NoProjectRules)
const F_INSIDE: u8 = 4; // below a Target: nothing is evaluated any more
const F_IN_GIT: u8 = 8; // inside a git work tree (some ancestor contains .git)

/// Environment snapshot used to resolve anchored locations (injectable for tests).
#[derive(Clone, Debug)]
pub struct Resolver {
    pub os: Os,
    pub home: String,
    pub vars: HashMap<String, String>,
}

impl Resolver {
    pub fn from_env() -> Resolver {
        let vars: HashMap<String, String> = std::env::vars().collect();
        let os = Os::current();
        let key = if os == Os::WIN { "USERPROFILE" } else { "HOME" };
        let home = vars.get(key).cloned().unwrap_or_default();
        Resolver { os, home, vars }
    }

    fn var(&self, key: &str) -> Option<&str> {
        if self.os == Os::WIN {
            // Windows environment variable names are case-insensitive.
            self.vars.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
        } else {
            self.vars.get(key).map(String::as_str)
        }
    }

    /// Absolute path from an env var; the first entry if it is a list (GOPATH).
    fn var_path(&self, key: &str) -> Option<String> {
        let sep = if self.os == Os::WIN { ';' } else { ':' };
        let first = self.var(key)?.split(sep).next()?.trim();
        is_absolute(first, self.os).then(|| first.to_string())
    }

    pub fn base(&self, b: Base) -> Option<String> {
        let win = self.os == Os::WIN;
        let home = (!self.home.is_empty()).then(|| self.home.clone());
        let under_home = |rel: &str| home.as_ref().map(|h| format!("{h}/{rel}"));
        match b {
            Base::Root => (!win).then(|| "/".to_string()),
            Base::Home => home.clone(),
            Base::MacCaches => (self.os == Os::MAC).then(|| under_home("Library/Caches")).flatten(),
            Base::XdgCache => (!win).then(|| self.var_path("XDG_CACHE_HOME").or_else(|| under_home(".cache"))).flatten(),
            Base::XdgData => (!win).then(|| self.var_path("XDG_DATA_HOME").or_else(|| under_home(".local/share"))).flatten(),
            Base::LocalAppData => win.then(|| self.var_path("LOCALAPPDATA")).flatten(),
            Base::AppData => win.then(|| self.var_path("APPDATA")).flatten(),
            Base::Temp => win.then(|| self.var_path("TEMP").or_else(|| self.var_path("TMP"))).flatten(),
            Base::WinDir => win.then(|| self.var_path("SystemRoot").or_else(|| self.var_path("windir"))).flatten(),
            Base::SystemDrive => win.then(|| self.var("SystemDrive").map(|d| format!("{d}\\"))).flatten(),
            Base::ProgramData => win.then(|| self.var_path("ProgramData")).flatten(),
            Base::Env(key) => self.var_path(key),
        }
    }
}

fn is_absolute(p: &str, os: Os) -> bool {
    if os == Os::WIN {
        let b = p.as_bytes();
        (b.len() >= 2 && b[1] == b':') || p.starts_with("\\\\")
    } else {
        p.starts_with('/')
    }
}

/// Path components; `/` and `\` both separate. `/` -> [], `C:\x` -> ["C:", "x"].
pub fn components(path: &str) -> Vec<String> {
    path.split(['/', '\\']).filter(|c| !c.is_empty()).map(str::to_string).collect()
}

/// `*`-only glob.
pub fn glob_match(pat: &str, name: &str) -> bool {
    let (p, n) = (pat.as_bytes(), name.as_bytes());
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, usize::MAX, 0);
    while ni < n.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if pi < p.len() && p[pi] == n[ni] {
            pi += 1;
            ni += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

#[derive(Default)]
struct Node {
    lit: HashMap<String, u32>,
    globs: Vec<(String, u32)>,
    dir_rules: Vec<RuleIdx>,
    file_rules: Vec<(Pat, RuleIdx)>,
    /// Some anchored rule lives strictly below this node.
    below: bool,
}

/// Classification of one directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    None,
    /// A cleanup candidate. `by_default` = matched only through its container's default rule.
    Target { rule: RuleIdx, by_default: bool },
    /// Well-known cache root; its children are the candidates.
    Container(RuleIdx),
    /// Never suggested.
    Protected(RuleIdx),
    /// Below a candidate (outermost match wins); not suggested separately.
    Inside(RuleIdx),
}

/// Per-directory matcher state handed from a directory to each of its child jobs.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    states: [u32; 2],
    /// This directory's marker bits (= the siblings of its children).
    pub markers: u64,
    flags: u8,
    /// Target (when inside one) or innermost Protect rule.
    pub enclosing: RuleIdx,
    container_default: RuleIdx,
}

impl Ctx {
    const EMPTY: Ctx = Ctx { states: [NO_NODE, NO_NODE], markers: 0, flags: 0, enclosing: NO_RULE, container_default: NO_RULE };
    pub fn is_inside_target(&self) -> bool {
        self.flags & F_INSIDE != 0
    }
    /// The directory lies inside a git work tree: before suggesting it, verify lazily that it
    /// is ignored and has no tracked files (`git check-ignore`, `git ls-files`).
    pub fn in_git_worktree(&self) -> bool {
        self.flags & F_IN_GIT != 0
    }
}

pub struct Engine {
    rules: Vec<&'static Rule>,
    group_of: Vec<u8>,
    groups: &'static [Group],
    os: Os,
    fold: bool,
    nodes: Vec<Node>,
    by_name: HashMap<String, Vec<(RuleIdx, u8)>>,
    patterned: Vec<(RuleIdx, u8)>,
    contains: Vec<(RuleIdx, u8)>,
    by_id: HashMap<&'static str, RuleIdx>,
}

impl Engine {
    pub fn new(resolver: &Resolver) -> Engine {
        Engine::with_catalog(CATALOG, resolver)
    }

    pub fn with_catalog(groups: &'static [Group], r: &Resolver) -> Engine {
        let mut e = Engine {
            rules: Vec::new(),
            group_of: Vec::new(),
            groups,
            os: r.os,
            fold: r.os == Os::WIN,
            nodes: vec![Node::default()],
            by_name: HashMap::new(),
            patterned: Vec::new(),
            contains: Vec::new(),
            by_id: HashMap::new(),
        };
        for (gi, g) in groups.iter().enumerate() {
            for rule in g.rules {
                e.rules.push(rule);
                e.group_of.push(gi as u8);
            }
        }
        assert!(e.rules.len() < NO_RULE as usize);
        let rules = e.rules.clone();
        for (i, rule) in rules.iter().enumerate() {
            let ri = i as RuleIdx;
            e.by_id.insert(rule.id, ri);
            if !rule.os.has(r.os) {
                continue;
            }
            for (mi, mt) in rule.matchers.iter().enumerate() {
                let mi = mi as u8;
                match *mt {
                    Matcher::At(b, rel) => {
                        if let Some(n) = e.anchor(r, b, rel) {
                            e.nodes[n].dir_rules.push(ri);
                        }
                    }
                    Matcher::FileAt(b, rel, pat) => {
                        if let Some(n) = e.anchor(r, b, rel) {
                            e.nodes[n].file_rules.push((pat, ri));
                        }
                    }
                    Matcher::Dir(Pat::Is(name), _) | Matcher::Under(_, Pat::Is(name), _) => {
                        let key = e.fold_str(name).into_owned();
                        e.by_name.entry(key).or_default().push((ri, mi));
                    }
                    Matcher::Dir(..) | Matcher::Under(..) => e.patterned.push((ri, mi)),
                    Matcher::Contains(_) => e.contains.push((ri, mi)),
                }
            }
        }
        e
    }

    fn fold_str<'a>(&self, s: &'a str) -> Cow<'a, str> {
        if self.fold { Cow::Owned(s.to_lowercase()) } else { Cow::Borrowed(s) }
    }

    fn anchor(&mut self, r: &Resolver, b: Base, rel: &[&str]) -> Option<usize> {
        let base = r.base(b)?;
        let mut comps = components(&base);
        comps.extend(rel.iter().map(|s| s.to_string()));
        let mut n = 0usize;
        for c in comps {
            self.nodes[n].below = true;
            let c = self.fold_str(&c).into_owned();
            let existing = if c.contains('*') {
                self.nodes[n].globs.iter().find(|(g, _)| *g == c).map(|(_, id)| *id)
            } else {
                self.nodes[n].lit.get(&c).copied()
            };
            let next = match existing {
                Some(id) => id as usize,
                None => {
                    let id = self.nodes.len() as u32;
                    self.nodes.push(Node::default());
                    if c.contains('*') {
                        self.nodes[n].globs.push((c, id));
                    } else {
                        self.nodes[n].lit.insert(c, id);
                    }
                    id as usize
                }
            };
            n = next;
        }
        Some(n)
    }

    pub fn rule(&self, i: RuleIdx) -> &'static Rule {
        self.rules[i as usize]
    }
    pub fn group_title(&self, i: RuleIdx) -> &'static str {
        self.groups[self.group_of[i as usize] as usize].title
    }
    pub fn rule_by_id(&self, id: &str) -> Option<RuleIdx> {
        self.by_id.get(id).copied()
    }
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    fn step(&self, states: [u32; 2], name: &str) -> [u32; 2] {
        let mut out = [NO_NODE; 2];
        let mut k = 0;
        let mut push = |id: u32| {
            if !out.contains(&id) && k < 2 {
                out[k] = id;
                k += 1;
            }
        };
        for &s in states.iter().filter(|&&s| s != NO_NODE) {
            let node = &self.nodes[s as usize];
            if let Some(&c) = node.lit.get(name) {
                push(c);
            }
            for (g, c) in &node.globs {
                if glob_match(g, name) {
                    push(*c);
                }
            }
        }
        out
    }

    /// Context of the filesystem root (Unix `/`, or the virtual root above drives on
    /// Windows) and the root's own verdict.
    pub fn root(&self) -> (Verdict, Ctx) {
        if self.os == Os::WIN {
            (Verdict::None, Ctx { states: [0, NO_NODE], ..Ctx::EMPTY })
        } else {
            self.eval_dir(&Ctx::EMPTY, [0, NO_NODE], "", "", 0)
        }
    }

    /// Walk the anchored rules along an absolute path (no listings needed). Use it to get the
    /// parent context of a scan root that is not `/`.
    pub fn enter(&self, path: &str) -> (Verdict, Ctx) {
        let (mut v, mut ctx) = self.root();
        let mut parent = String::new();
        for c in components(path) {
            let (v2, c2) = self.classify_dir(&ctx, &parent, &c, 0);
            v = v2;
            ctx = c2;
            parent = c;
        }
        (v, ctx)
    }

    /// Classify directory `name` (child of the directory described by `parent`) once its own
    /// listing is known. `own` = OR of `marker_of` over its entries.
    pub fn classify_dir(&self, parent: &Ctx, parent_name: &str, name: &str, own: u64) -> (Verdict, Ctx) {
        if parent.flags & F_INSIDE != 0 {
            return (Verdict::Inside(parent.enclosing), Ctx { states: [NO_NODE; 2], markers: own, ..*parent });
        }
        let fname = self.fold_str(name);
        let states = self.step(parent.states, &fname);
        self.eval_dir(parent, states, parent_name, &fname, own)
    }

    fn eval_dir(&self, parent: &Ctx, states: [u32; 2], parent_name: &str, name: &str, own: u64) -> (Verdict, Ctx) {
        let mut ctx = Ctx { states, markers: own, flags: parent.flags, enclosing: parent.enclosing, container_default: NO_RULE };
        if parent.markers & m::GIT != 0 {
            ctx.flags |= F_IN_GIT; // this dir is a child of a work-tree root
        }

        // 1. Anchored rules at this exact path.
        let mut protect: Option<RuleIdx> = None;
        let mut anchored: Option<RuleIdx> = None;
        let mut on_anchor_path = false;
        for &s in states.iter().filter(|&&s| s != NO_NODE) {
            let node = &self.nodes[s as usize];
            on_anchor_path |= node.below;
            for &r in &node.dir_rules {
                match self.rules[r as usize].kind {
                    Kind::Protect(scope) => {
                        ctx.flags |= scope_flags(scope);
                        protect = Some(protect.map_or(r, |p| p.min(r)));
                    }
                    _ => anchored = Some(anchored.map_or(r, |a| a.min(r))),
                }
            }
        }
        // 2. Protect heuristics (app bundles, .git, media libraries) - always active.
        if protect.is_none() && anchored.is_none() {
            if let Some(r) = self.heuristic(parent, parent_name, name, own, ctx.flags, true) {
                if let Kind::Protect(scope) = self.rules[r as usize].kind {
                    ctx.flags |= scope_flags(scope);
                }
                protect = Some(r);
            }
        }
        if let Some(r) = anchored {
            match self.rules[r as usize].kind {
                Kind::Container { default } => {
                    ctx.container_default = default.and_then(|id| self.rule_by_id(id)).unwrap_or(NO_RULE);
                    return (Verdict::Container(r), ctx);
                }
                _ => {
                    ctx.flags |= F_INSIDE;
                    ctx.enclosing = r;
                    return (Verdict::Target { rule: r, by_default: false }, ctx);
                }
            }
        }
        if let Some(r) = protect {
            ctx.enclosing = r;
            return (Verdict::Protected(r), ctx);
        }
        // 3. Heuristic target rules (not on the path to an anchored rule: anchored knowledge
        //    wins, e.g. ~/Library is never Unity's Library, ~/.gradle never a project .gradle).
        if ctx.flags & F_NO_HEUR == 0 && !on_anchor_path {
            if let Some(r) = self.heuristic(parent, parent_name, name, own, ctx.flags, false) {
                ctx.flags |= F_INSIDE;
                ctx.enclosing = r;
                return (Verdict::Target { rule: r, by_default: false }, ctx);
            }
        }
        // 4. Container default for unmatched children; along anchored paths keep it for
        //    deeper levels (e.g. ~/Library/Caches/Google/Chrome).
        if parent.container_default != NO_RULE {
            if on_anchor_path {
                ctx.container_default = parent.container_default;
            } else {
                let r = parent.container_default;
                ctx.flags |= F_INSIDE;
                ctx.enclosing = r;
                return (Verdict::Target { rule: r, by_default: true }, ctx);
            }
        }
        (Verdict::None, ctx)
    }

    fn heuristic(&self, parent: &Ctx, parent_name: &str, name: &str, own: u64, flags: u8, want_protect: bool) -> Option<RuleIdx> {
        let mut best: Option<RuleIdx> = None;
        let mut consider = |r: RuleIdx, mi: u8| {
            if best.is_some_and(|b| b <= r) {
                return;
            }
            let rule = self.rules[r as usize];
            if matches!(rule.kind, Kind::Protect(_)) != want_protect || !rule.os.has(self.os) {
                return;
            }
            if rule.zone == Zone::Projects && flags & F_NO_PROJ != 0 {
                return;
            }
            let ok = match rule.matchers[mi as usize] {
                Matcher::Dir(pat, when) => self.pat_ok(pat, name) && when_ok(when, parent.markers, own),
                Matcher::Under(pname, pat, when) => {
                    self.fold_str(pname) == self.fold_str(parent_name) && self.pat_ok(pat, name) && when_ok(when, parent.markers, own)
                }
                Matcher::Contains(when) => when_ok(when, parent.markers, own),
                _ => false,
            };
            if ok {
                best = Some(r);
            }
        };
        if let Some(v) = self.by_name.get(name) {
            for &(r, mi) in v {
                consider(r, mi);
            }
        }
        for &(r, mi) in &self.patterned {
            consider(r, mi);
        }
        if own != 0 {
            for &(r, mi) in &self.contains {
                consider(r, mi);
            }
        }
        best
    }

    fn pat_ok(&self, pat: Pat, name: &str) -> bool {
        let f = |s: &'static str| self.fold_str(s);
        match pat {
            Pat::Is(s) => f(s) == name,
            Pat::Prefix(s) => name.starts_with(&*f(s)),
            Pat::Suffix(s) => name.ends_with(&*f(s)),
            Pat::Glob(g) => glob_match(&f(g), name),
            Pat::Ext(exts) => name.rsplit_once('.').is_some_and(|(_, e)| exts.iter().any(|x| x.eq_ignore_ascii_case(e))),
        }
    }

    /// Classify a regular file inside the directory described by `dir` (anchored file rules
    /// only). Returns Target or Protect rules.
    pub fn classify_file(&self, dir: &Ctx, name: &str) -> Option<RuleIdx> {
        if dir.flags & F_INSIDE != 0 {
            return None;
        }
        let fname = self.fold_str(name);
        for &s in dir.states.iter().filter(|&&s| s != NO_NODE) {
            for &(pat, r) in &self.nodes[s as usize].file_rules {
                if self.pat_ok(pat, &fname) {
                    return Some(r);
                }
            }
        }
        None
    }

    /// Cleanup hints with `{name}` / `{path}` filled in.
    pub fn cleanup_for(&self, r: RuleIdx, name: &str, path: &str) -> Vec<String> {
        self.rule(r).cleanup.iter().map(|c| c.replace("{name}", name).replace("{path}", path)).collect()
    }

    /// Static sanity checks of the catalog (run in tests / debug builds).
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        let mut seen = HashMap::new();
        let mut referenced = Vec::new();
        for (i, r) in self.rules.iter().enumerate() {
            if let Some(j) = seen.insert(r.id, i) {
                errs.push(format!("duplicate id {} ({} and {})", r.id, j, i));
            }
            if let Kind::Container { default: Some(d) } = r.kind {
                match self.rule_by_id(d) {
                    Some(di) if matches!(self.rule(di).kind, Kind::Target(_)) => referenced.push(d),
                    _ => errs.push(format!("{}: default {} missing or not a Target", r.id, d)),
                }
            }
            if r.reason.is_empty() || r.reason.chars().count() > 70 || !r.reason.ends_with('.') {
                errs.push(format!("{}: reason must be 1..=70 chars and end with '.' ({} chars)", r.id, r.reason.chars().count()));
            }
        }
        for r in &self.rules {
            if r.matchers.is_empty() && !referenced.contains(&r.id) {
                errs.push(format!("{}: no matchers and not a container default", r.id));
            }
        }
        for (n, node) in self.nodes.iter().enumerate() {
            let self_protect = node.dir_rules.iter().any(|&r| self.rule(r).kind == Kind::Protect(Scope::SelfOnly));
            let target = node.dir_rules.iter().any(|&r| !matches!(self.rule(r).kind, Kind::Protect(_)));
            if self_protect && target {
                errs.push(format!("trie node {n}: SelfOnly protect and a target at the same path"));
            }
        }
        errs
    }
}

fn scope_flags(scope: Scope) -> u8 {
    match scope {
        Scope::SelfOnly => 0,
        Scope::NoProjectRules => F_NO_PROJ,
        Scope::NoHeuristics => F_NO_HEUR | F_NO_PROJ,
    }
}

fn when_ok(when: When, sib: u64, own: u64) -> bool {
    when.is_empty() || when.iter().any(|c| c.ok(sib, own))
}

/// Check a `CACHEDIR.TAG` candidate: a regular file (not a symlink) whose first 43 bytes are
/// the signature. Call only when the marker bit is set (rare), clear the bit otherwise.
pub fn is_valid_cachedir_tag(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(meta) = std::fs::symlink_metadata(path) else { return false };
    if !meta.file_type().is_file() {
        return false;
    }
    let mut buf = [0u8; 43];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut buf)).is_ok() && &buf == CACHEDIR_SIGNATURE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct T {
        dirs: BTreeMap<String, T>,
        files: Vec<String>,
    }

    /// Paths ending in '/' are directories, others files.
    fn tree(paths: &[&str]) -> T {
        let mut root = T::default();
        for p in paths {
            let is_dir = p.ends_with('/');
            let comps = components(p);
            let mut cur = &mut root;
            for (i, c) in comps.iter().enumerate() {
                if i + 1 == comps.len() && !is_dir {
                    cur.files.push(c.clone());
                } else {
                    cur = cur.dirs.entry(c.clone()).or_default();
                }
            }
        }
        root
    }

    fn own_markers(t: &T) -> u64 {
        t.dirs.keys().map(|n| marker_of(n, true)).chain(t.files.iter().map(|n| marker_of(n, false))).fold(0, |a, b| a | b)
    }

    fn walk(e: &Engine, paths: &[&str]) -> HashMap<String, Verdict> {
        fn rec(e: &Engine, t: &T, path: &str, name: &str, ctx: &Ctx, out: &mut HashMap<String, Verdict>) {
            for f in &t.files {
                if let Some(r) = e.classify_file(ctx, f) {
                    let v = if matches!(e.rule(r).kind, Kind::Protect(_)) { Verdict::Protected(r) } else { Verdict::Target { rule: r, by_default: false } };
                    out.insert(format!("{path}/{f}"), v);
                }
            }
            for (n, child) in &t.dirs {
                let (v, c) = e.classify_dir(ctx, name, n, own_markers(child));
                let p = format!("{path}/{n}");
                out.insert(p.clone(), v);
                rec(e, child, &p, n, &c, out);
            }
        }
        let t = tree(paths);
        let mut out = HashMap::new();
        let (v, ctx) = e.root();
        out.insert(String::new(), v);
        rec(e, &t, "", "", &ctx, &mut out);
        out
    }

    fn mac() -> Engine {
        Engine::new(&Resolver { os: Os::MAC, home: "/Users/u".into(), vars: HashMap::new() })
    }

    fn id(e: &Engine, v: Option<&Verdict>) -> String {
        match v {
            Some(Verdict::Target { rule, by_default }) => format!("{}{}", e.rule(*rule).id, if *by_default { "*" } else { "" }),
            Some(Verdict::Container(r)) => format!("container:{}", e.rule(*r).id),
            Some(Verdict::Protected(r)) => format!("protected:{}", e.rule(*r).id),
            Some(Verdict::Inside(r)) => format!("inside:{}", e.rule(*r).id),
            Some(Verdict::None) => "-".into(),
            None => "missing".into(),
        }
    }

    #[test]
    fn catalog_is_valid() {
        let e = mac();
        let errs = e.validate();
        assert!(errs.is_empty(), "{errs:#?}");
        assert_eq!(std::mem::size_of::<Ctx>(), 24);
    }

    #[test]
    fn projects_and_exclusions() {
        let e = mac();
        let v = walk(
            &e,
            &[
                "/Users/u/dev/web/package.json",
                "/Users/u/dev/web/node_modules/left-pad/package.json",
                "/Users/u/dev/web/node_modules/left-pad/node_modules/",
                "/Users/u/dev/pn/package.json",
                "/Users/u/dev/pn/node_modules/.modules.yaml",
                "/Users/u/dev/rs/Cargo.toml",
                "/Users/u/dev/rs/target/CACHEDIR.TAG",
                "/Users/u/dev/rs/target/.rustc_info.json",
                "/Users/u/dev/custom-target/CACHEDIR.TAG",
                "/Users/u/dev/custom-target/.rustc_info.json",
                "/Users/u/dev/py/.venv/pyvenv.cfg",
                "/Users/u/dev/py/.venv/CACHEDIR.TAG",
                "/Users/u/dev/py/.venv/lib/",
                "/Users/u/dev/cpp/CMakeLists.txt",
                "/Users/u/dev/cpp/build/CMakeCache.txt",
                "/Users/u/dev/insource/CMakeLists.txt",
                "/Users/u/dev/insource/CMakeCache.txt",
                "/Users/u/dev/game/Assets/",
                "/Users/u/dev/game/ProjectSettings/",
                "/Users/u/dev/game/Library/",
                "/Users/u/dev/kt/build.gradle.kts",
                "/Users/u/dev/kt/build/",
                "/Users/u/dev/kt/.gradle/",
                "/Users/u/dev/kt/.kotlin/",
                "/Users/u/dev/plain/build/",
                "/Users/u/dev/repo/.git/node_modules/",
                "/Users/u/dev/repo/package.json",
                "/Applications/Visual Studio Code.app/Contents/Resources/app/package.json",
                "/Applications/Visual Studio Code.app/Contents/Resources/app/node_modules/",
                "/opt/homebrew/lib/node_modules/npm/package.json",
                "/opt/homebrew/lib/node_modules/npm/node_modules/",
                "/Users/u/.local/pipx/venvs/black/pyvenv.cfg",
                "/Users/u/.vscode/extensions/ext/package.json",
                "/Users/u/.vscode/extensions/ext/node_modules/",
                "/Users/u/Assets/",
                "/Users/u/ProjectSettings/",
                "/Users/u/build.gradle",
                "/Users/u/.gradle/caches/",
                "/Users/u/.gradle/gradle.properties",
                "/Users/u/Library/Application Support/Slack/Local Storage/",
                "/Users/u/Library/Application Support/Slack/GPUCache/",
                "/Users/u/Library/Application Support/Slack/Cache/",
                "/Users/u/Library/Application Support/Slack/IndexedDB/",
                "/Users/u/Library/Application Support/Foo/package.json",
                "/Users/u/Library/Application Support/Foo/node_modules/",
            ],
        );
        let g = |p: &str| id(&e, v.get(p));
        assert_eq!(g("/Users/u/dev/web/node_modules"), "node-modules");
        assert_eq!(g("/Users/u/dev/web/node_modules/left-pad/node_modules"), "inside:node-modules");
        assert_eq!(g("/Users/u/dev/pn/node_modules"), "node-modules-pnpm");
        assert_eq!(g("/Users/u/dev/rs/target"), "cargo-target");
        assert_eq!(g("/Users/u/dev/custom-target"), "cargo-target-dir");
        assert_eq!(g("/Users/u/dev/py/.venv"), "python-venv", "specific rule beats the generic CACHEDIR.TAG rule");
        assert_eq!(g("/Users/u/dev/cpp/build"), "cmake-build");
        assert_eq!(g("/Users/u/dev/insource"), "-", "in-source CMake build is never a candidate");
        assert_eq!(g("/Users/u/dev/game/Library"), "unity-library");
        assert_eq!(g("/Users/u/dev/kt/build"), "gradle-build");
        assert_eq!(g("/Users/u/dev/kt/.gradle"), "gradle-project-cache");
        assert_eq!(g("/Users/u/dev/kt/.kotlin"), "kotlin-project-dir");
        assert_eq!(g("/Users/u/dev/plain/build"), "-", "build without a project marker");
        assert_eq!(g("/Users/u/dev/repo/.git"), "protected:vcs-data");
        assert_eq!(g("/Users/u/dev/repo/.git/node_modules"), "-");
        assert_eq!(g("/Applications/Visual Studio Code.app"), "protected:app-bundles");
        assert_eq!(g("/Applications/Visual Studio Code.app/Contents/Resources/app/node_modules"), "-");
        assert_eq!(g("/opt/homebrew"), "protected:homebrew-prefix");
        assert_eq!(g("/opt/homebrew/lib/node_modules/npm/node_modules"), "-");
        assert_eq!(g("/Users/u/.local/pipx/venvs/black"), "-");
        assert_eq!(g("/Users/u/.vscode/extensions/ext/node_modules"), "-");
        assert_eq!(g("/Users/u/Library"), "protected:mac-library", "never Unity's Library");
        assert_eq!(g("/Users/u/.gradle"), "protected:home-dotdirs", "never a project .gradle");
        assert_eq!(g("/Users/u/.gradle/caches"), "gradle-caches");
        assert_eq!(g("/Users/u/Library/Application Support/Slack/Cache"), "chromium-cache");
        assert_eq!(g("/Users/u/Library/Application Support/Slack/GPUCache"), "chromium-cache");
        assert_eq!(g("/Users/u/Library/Application Support/Slack/IndexedDB"), "-");
        assert_eq!(g("/Users/u/Library/Application Support/Foo/node_modules"), "-", "no project rules in app data");
    }

    #[test]
    fn anchored_containers_and_files() {
        let e = mac();
        let v = walk(
            &e,
            &[
                "/Users/u/Library/Caches/Homebrew/downloads/",
                "/Users/u/Library/Caches/com.example.app/",
                "/Users/u/Library/Caches/JetBrains/IntelliJIdea2026.2/",
                "/Users/u/Library/Caches/Google/AndroidStudio2025.1/",
                "/Users/u/Library/Caches/Google/Chrome/",
                "/Users/u/Library/Caches/pip/",
                "/Users/u/.cache/uv/CACHEDIR.TAG",
                "/Users/u/.cache/tagged/CACHEDIR.TAG",
                "/Users/u/.cache/plain/",
                "/Users/u/.cache/huggingface/hub/",
                "/Users/u/.cache/huggingface/token",
                "/Users/u/.cache/huggingface/assets/",
                "/Users/u/.cargo/registry/src/",
                "/Users/u/.cargo/registry/cache/",
                "/Users/u/.cargo/registry/CACHEDIR.TAG",
                "/Users/u/.cargo/bin/",
                "/Users/u/.rustup/toolchains/1.84.0-aarch64-apple-darwin/",
                "/Users/u/Downloads/Foo.dmg",
                "/Users/u/Downloads/notes.txt",
                "/Users/u/Library/Developer/Xcode/DerivedData/App-abc/",
                "/Users/u/Library/Developer/Xcode/Archives/2026-01-01/",
                "/Users/u/Library/Containers/com.docker.docker/Data/vms/0/data/",
                "/Users/u/Library/Containers/com.example.sandboxed/Data/Library/Caches/",
                "/Users/u/Library/Mobile Documents/com~apple~CloudDocs/proj/package.json",
                "/Users/u/Library/Mobile Documents/com~apple~CloudDocs/proj/node_modules/",
                "/Users/u/Documents/proj/package.json",
                "/Users/u/Documents/proj/node_modules/",
                "/Applications/Install macOS Tahoe.app/Contents/",
                "/Applications/Xcode-16.4.app/",
            ],
        );
        let g = |p: &str| id(&e, v.get(p));
        assert_eq!(g(""), "protected:top-level");
        assert_eq!(g("/Users/u"), "protected:top-level");
        assert_eq!(g("/Users/u/Library/Caches"), "container:mac-caches");
        assert_eq!(g("/Users/u/Library/Caches/Homebrew"), "homebrew-cache");
        assert_eq!(g("/Users/u/Library/Caches/Homebrew/downloads"), "inside:homebrew-cache");
        assert_eq!(g("/Users/u/Library/Caches/com.example.app"), "mac-app-cache*");
        assert_eq!(g("/Users/u/Library/Caches/JetBrains"), "container:jetbrains-caches");
        assert_eq!(g("/Users/u/Library/Caches/JetBrains/IntelliJIdea2026.2"), "jetbrains-system*");
        assert_eq!(g("/Users/u/Library/Caches/Google"), "-", "on an anchored path: keep looking deeper");
        assert_eq!(g("/Users/u/Library/Caches/Google/AndroidStudio2025.1"), "android-studio-system");
        assert_eq!(g("/Users/u/Library/Caches/Google/Chrome"), "mac-app-cache*", "container default carried down");
        assert_eq!(g("/Users/u/Library/Caches/pip"), "pip-cache");
        assert_eq!(g("/Users/u/.cache"), "container:xdg-cache");
        assert_eq!(g("/Users/u/.cache/uv"), "uv-cache");
        assert_eq!(g("/Users/u/.cache/tagged"), "cachedir-tag");
        assert_eq!(g("/Users/u/.cache/plain"), "xdg-cache-entry*");
        assert_eq!(g("/Users/u/.cache/huggingface"), "-");
        assert_eq!(g("/Users/u/.cache/huggingface/hub"), "hf-hub");
        assert_eq!(g("/Users/u/.cache/huggingface/assets"), "xdg-cache-entry*");
        assert_eq!(v.get("/Users/u/.cache/huggingface/token"), None, "files never get container defaults");
        assert_eq!(g("/Users/u/.cargo/registry"), "container:cargo-registry");
        assert_eq!(g("/Users/u/.cargo/registry/src"), "cargo-registry-src");
        assert_eq!(g("/Users/u/.cargo/registry/cache"), "cargo-registry-cache");
        assert_eq!(g("/Users/u/.cargo/bin"), "-");
        let tc = "/Users/u/.rustup/toolchains/1.84.0-aarch64-apple-darwin";
        assert_eq!(g(tc), "rustup-toolchain");
        if let Some(Verdict::Target { rule, .. }) = v.get(tc) {
            assert_eq!(e.cleanup_for(*rule, "1.84.0-aarch64-apple-darwin", tc)[1], "rustup toolchain uninstall 1.84.0-aarch64-apple-darwin");
        }
        assert_eq!(g("/Users/u/Downloads"), "protected:top-level");
        assert_eq!(g("/Users/u/Downloads/Foo.dmg"), "downloads-installers");
        assert_eq!(v.get("/Users/u/Downloads/notes.txt"), None);
        assert_eq!(g("/Users/u/Library/Developer/Xcode/DerivedData"), "xcode-derived-data");
        assert_eq!(g("/Users/u/Library/Developer/Xcode/Archives"), "xcode-archives");
        assert_eq!(g("/Users/u/Library/Containers/com.docker.docker/Data/vms"), "docker-desktop-vm");
        assert_eq!(g("/Users/u/Library/Containers/com.example.sandboxed/Data/Library/Caches"), "mac-sandbox-cache");
        assert_eq!(g("/Users/u/Library/Mobile Documents"), "protected:icloud-drive");
        assert_eq!(g("/Users/u/Library/Mobile Documents/com~apple~CloudDocs/proj/node_modules"), "-");
        assert_eq!(g("/Users/u/Documents"), "protected:top-level");
        assert_eq!(g("/Users/u/Documents/proj/node_modules"), "node-modules", "SelfOnly keeps rules alive inside");
        assert_eq!(g("/Applications/Install macOS Tahoe.app"), "mac-os-installer", "anchored beats the bundle protect");
        assert_eq!(g("/Applications/Xcode-16.4.app"), "xcode-extra");
    }

    #[test]
    fn more_heuristics() {
        let e = mac();
        let v = walk(
            &e,
            &[
                "/Users/u/.Trash/old.txt",
                "/Volumes/Ext/.Trashes/501/",
                "/Volumes/Ext/proj/package.json",
                "/Volumes/Ext/proj/node_modules/",
                "/Users/u/dev/goproj/go.mod",
                "/Users/u/dev/goproj/composer.json",
                "/Users/u/dev/goproj/composer.lock",
                "/Users/u/dev/goproj/vendor/",
                "/Users/u/dev/php/composer.json",
                "/Users/u/dev/php/composer.lock",
                "/Users/u/dev/php/vendor/",
                "/Users/u/dev/berry/package.json",
                "/Users/u/dev/berry/.yarn/cache/",
                "/Users/u/dev/berry/.yarn/releases/",
                "/Users/u/.yarn/berry/cache/",
                "/Users/u/dev/custom-bazel/ob/execroot/",
                "/Users/u/dev/custom-bazel/ob/external/",
                "/Users/u/VMs/Ubuntu/Ubuntu.vbox",
                "/Users/u/Library/Containers/com.utmapp.UTM/Data/Documents/Linux.utm/config.plist",
                "/Users/u/dev/tf/main.tf",
                "/Users/u/dev/tf/.terraform/providers/",
                "/Users/u/dev/rb/vendor/bundle/ruby/3.3.0/gems/",
                "/Users/u/dev/elec/Local Storage/",
                "/Users/u/dev/elec/GPUCache/",
                "/Users/u/dev/elec/Cache/",
            ],
        );
        let g = |p: &str| id(&e, v.get(p));
        assert_eq!(g("/Users/u/.Trash"), "mac-trash");
        assert_eq!(g("/Volumes/Ext"), "protected:top-level");
        assert_eq!(g("/Volumes/Ext/.Trashes"), "mac-trash");
        assert_eq!(g("/Volumes/Ext/proj/node_modules"), "node-modules");
        assert_eq!(g("/Users/u/dev/goproj/vendor"), "-", "Go vendor/ is often committed");
        assert_eq!(g("/Users/u/dev/php/vendor"), "php-vendor");
        assert_eq!(g("/Users/u/dev/berry/.yarn/cache"), "yarn-project-cache");
        assert_eq!(g("/Users/u/dev/berry/.yarn/releases"), "-");
        assert_eq!(g("/Users/u/.yarn/berry/cache"), "yarn-berry-cache");
        assert_eq!(g("/Users/u/dev/custom-bazel/ob"), "bazel-output-base");
        assert_eq!(g("/Users/u/VMs/Ubuntu"), "virtualbox-vm");
        assert_eq!(g("/Users/u/Library/Containers/com.utmapp.UTM/Data/Documents/Linux.utm"), "vm-bundles");
        assert_eq!(g("/Users/u/dev/tf/.terraform"), "terraform");
        assert_eq!(g("/Users/u/dev/rb/vendor/bundle"), "bundler-vendor");
        assert_eq!(g("/Users/u/dev/elec/Cache"), "chromium-cache");
    }

    #[test]
    fn fixtures_and_git() {
        let e = mac();
        let v = walk(
            &e,
            &[
                "/Users/u/dev/ide/.git/",
                "/Users/u/dev/ide/swift/testData/proj/Podfile",
                "/Users/u/dev/ide/swift/testData/proj/Pods/",
                "/Users/u/dev/ide/web/package.json",
                "/Users/u/dev/ide/web/node_modules/",
            ],
        );
        let g = |p: &str| id(&e, v.get(p));
        assert_eq!(g("/Users/u/dev/ide/swift/testData"), "protected:test-fixtures");
        assert_eq!(g("/Users/u/dev/ide/swift/testData/proj/Pods"), "-");
        assert_eq!(g("/Users/u/dev/ide/web/node_modules"), "node-modules");
        let (_, root) = e.enter("/Users/u/dev");
        let (_, ide) = e.classify_dir(&root, "dev", "ide", m::GIT);
        let (_, web) = e.classify_dir(&ide, "ide", "web", m::PACKAGE_JSON);
        let (_, nm) = e.classify_dir(&web, "web", "node_modules", 0);
        assert!(!ide.in_git_worktree() && web.in_git_worktree() && nm.in_git_worktree());
    }

    #[test]
    fn enter_mid_tree() {
        let e = mac();
        let (v, ctx) = e.enter("/Users/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f");
        assert_eq!(id(&e, Some(&v)), "inside:cargo-registry-src");
        assert!(ctx.is_inside_target());
        let (v, _) = e.enter("/Users/u/dev");
        assert_eq!(v, Verdict::None);
    }

    #[test]
    fn linux_paths() {
        let e = Engine::new(&Resolver { os: Os::LINUX, home: "/home/u".into(), vars: HashMap::new() });
        let v = walk(
            &e,
            &[
                "/var/cache/apt/archives/",
                "/var/cache/debconf/config.dat",
                "/var/cache/fontconfig/CACHEDIR.TAG",
                "/var/cache/man/",
                "/var/lib/docker/overlay2/",
                "/var/lib/dpkg/",
                "/var/log/journal/",
                "/home/u/.local/share/Trash/files/",
                "/home/u/.cache/pip/",
                "/home/u/.cache/JetBrains/PyCharm2026.1/",
                "/home/u/.var/app/org.gnome.Maps/cache/",
                "/usr/lib/node_modules/npm/package.json",
                "/usr/lib/node_modules/npm/node_modules/",
                "/media/usb/.Trash-1000/",
            ],
        );
        let g = |p: &str| id(&e, v.get(p));
        assert_eq!(g("/var/cache"), "container:var-cache");
        assert_eq!(g("/var/cache/apt"), "-");
        assert_eq!(g("/var/cache/apt/archives"), "apt-cache");
        assert_eq!(g("/var/cache/debconf"), "protected:debconf");
        assert_eq!(g("/var/cache/fontconfig"), "cachedir-tag");
        assert_eq!(g("/var/cache/man"), "var-cache-entry*");
        assert_eq!(g("/var/lib/docker"), "docker-engine");
        assert_eq!(g("/var/lib/dpkg"), "-");
        assert_eq!(g("/var/log/journal"), "journal");
        assert_eq!(g("/home/u/.local/share/Trash"), "linux-trash");
        assert_eq!(g("/home/u/.cache/pip"), "pip-cache");
        assert_eq!(g("/home/u/.cache/JetBrains/PyCharm2026.1"), "jetbrains-system*");
        assert_eq!(g("/home/u/.var/app/org.gnome.Maps/cache"), "flatpak-app-cache");
        assert_eq!(g("/usr/lib/node_modules/npm/node_modules"), "-");
        assert_eq!(g("/media/usb/.Trash-1000"), "linux-trash");
    }

    #[test]
    fn windows_paths() {
        let vars: HashMap<String, String> = [
            ("LOCALAPPDATA", r"C:\Users\u\AppData\Local"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
            ("TEMP", r"C:\Users\u\AppData\Local\Temp"),
            ("SystemRoot", r"C:\WINDOWS"),
            ("SystemDrive", "C:"),
            ("ProgramData", r"C:\ProgramData"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let e = Engine::new(&Resolver { os: Os::WIN, home: r"C:\Users\u".into(), vars });
        assert!(e.validate().is_empty());
        let v = walk(
            &e,
            &[
                "C:/Users/u/AppData/Local/Temp/x/",
                "C:/Windows/SoftwareDistribution/Download/abc/",
                "C:/Windows/WinSxS/x/",
                "C:/Windows/Installer/",
                "C:/Windows/System32/app/package.json",
                "C:/Windows/System32/app/node_modules/",
                "C:/Windows.old/",
                "C:/pagefile.sys",
                "C:/Users/u/AppData/Local/npm-cache/_cacache/",
                "C:/Users/u/AppData/Roaming/npm/node_modules/x/package.json",
                "C:/Users/u/AppData/Roaming/npm/node_modules/x/node_modules/",
                "C:/Users/u/source/repos/App/App.csproj",
                "C:/Users/u/source/repos/App/bin/",
                "C:/Users/u/source/repos/App/obj/",
                "C:/Users/u/Downloads/setup.EXE",
            ],
        );
        let g = |p: &str| id(&e, v.get(p));
        assert_eq!(g("/C:/Users/u/AppData/Local/Temp"), "win-temp");
        assert_eq!(g("/C:/Windows/SoftwareDistribution/Download"), "win-update-cache", "case-insensitive, inside a protected tree");
        assert_eq!(g("/C:/Windows/WinSxS"), "protected:win-winsxs");
        assert_eq!(g("/C:/Windows/Installer"), "protected:win-installer-cache");
        assert_eq!(g("/C:/Windows/System32/app/node_modules"), "-");
        assert_eq!(g("/C:/Windows.old"), "win-old");
        assert_eq!(g("/C:/pagefile.sys"), "protected:win-system-files");
        assert_eq!(g("/C:/Users/u/AppData/Local/npm-cache"), "npm-cache");
        assert_eq!(g("/C:/Users/u/AppData/Roaming/npm/node_modules/x/node_modules"), "-");
        assert_eq!(g("/C:/Users/u/source/repos/App/bin"), "dotnet-bin-obj");
        assert_eq!(g("/C:/Users/u/source/repos/App/obj"), "dotnet-bin-obj");
        assert_eq!(g("/C:/Users/u/Downloads/setup.EXE"), "downloads-installers");
    }

    #[test]
    fn env_overrides_and_markers() {
        let vars: HashMap<String, String> =
            [("CARGO_HOME", "/data/cargo"), ("GOPATH", "/work/go:/other"), ("XDG_CACHE_HOME", "relative/ignored")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        let e = Engine::new(&Resolver { os: Os::LINUX, home: "/home/u".into(), vars });
        let v = walk(&e, &["/data/cargo/registry/src/", "/work/go/pkg/mod/", "/home/u/.cache/pip/"]);
        let g = |p: &str| id(&e, v.get(p));
        assert_eq!(g("/data/cargo/registry/src"), "cargo-registry-src");
        assert_eq!(g("/work/go/pkg/mod"), "go-mod-cache");
        assert_eq!(g("/home/u/.cache/pip"), "pip-cache", "relative XDG_CACHE_HOME is ignored per spec");
        assert_eq!(marker_of("next.config.mjs", false), m::NEXT_CONFIG);
        assert_eq!(marker_of("main.tf", false), m::TF_FILES);
        assert_eq!(marker_of("vars.tf.json", false), m::TF_FILES);
        assert_eq!(marker_of("App.csproj", false), m::DOTNET_PROJ);
        assert_eq!(marker_of("Assets", false), 0, "Assets must be a directory");
        assert!(glob_match("Install macOS *.app", "Install macOS Tahoe.app"));
        assert!(glob_match(".*", ".cache") && !glob_match(".*", "cache"));
        assert!(glob_match("_bazel_*", "_bazel_max") && !glob_match("Xcode-*.app", "Xcode.app"));
    }
}
