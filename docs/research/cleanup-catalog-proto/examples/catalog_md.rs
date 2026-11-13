//! Prints the catalog as Markdown (one table per group) so the doc never drifts from the code.
use cleanup_catalog_proto::catalog::CATALOG;
use cleanup_catalog_proto::model::*;

fn base(b: Base) -> (&'static str, bool) {
    match b {
        Base::Root => ("", false),
        Base::Home => ("~", false),
        Base::MacCaches => ("~/Library/Caches", false),
        Base::XdgCache => ("~/.cache", false),
        Base::XdgData => ("~/.local/share", false),
        Base::LocalAppData => ("%LOCALAPPDATA%", true),
        Base::AppData => ("%APPDATA%", true),
        Base::Temp => ("%TEMP%", true),
        Base::WinDir => ("%SystemRoot%", true),
        Base::SystemDrive => ("C:", true),
        Base::ProgramData => ("%ProgramData%", true),
        Base::Env(v) => (v, false),
    }
}

fn path(b: Base, rel: &[&str]) -> String {
    let (s, win) = base(b);
    let sep = if win { "\\" } else { "/" };
    let head = match b {
        Base::Env(v) => format!("${v}"),
        Base::Root => String::new(),
        _ => s.to_string(),
    };
    if rel.is_empty() {
        if head.is_empty() { "/".into() } else { head }
    } else {
        format!("{head}{sep}{}", rel.join(sep))
    }
}

fn pat(p: Pat) -> String {
    match p {
        Pat::Is(s) => s.to_string(),
        Pat::Prefix(s) => format!("{s}*"),
        Pat::Suffix(s) => format!("*{s}"),
        Pat::Glob(s) => s.to_string(),
        Pat::Ext(e) => e.iter().map(|x| format!("*.{x}")).collect::<Vec<_>>().join(", "),
    }
}

fn cond(c: &Cond) -> String {
    let mut parts = Vec::new();
    let names = |mask: u64, sep: &str| m::describe(mask).replace(" | ", sep);
    if c.sib_all != 0 { parts.push(format!("next to {}", names(c.sib_all, " + "))); }
    if c.sib_any != 0 { parts.push(format!("next to {}", names(c.sib_any, " or "))); }
    if c.sib_none != 0 { parts.push(format!("not next to {}", names(c.sib_none, " or "))); }
    if c.own_all != 0 { parts.push(format!("containing {}", names(c.own_all, " + "))); }
    if c.own_any != 0 { parts.push(format!("containing {}", names(c.own_any, " or "))); }
    if c.own_none != 0 { parts.push(format!("not containing {}", names(c.own_none, " or "))); }
    parts.join(", ")
}

fn when(w: When) -> String {
    if w.is_empty() { String::new() } else { format!(" ({})", w.iter().map(cond).collect::<Vec<_>>().join("; OR ")) }
}

fn matcher(mt: &Matcher) -> String {
    match *mt {
        Matcher::At(b, rel) => format!("`{}`", path(b, rel)),
        Matcher::FileAt(b, rel, p) => format!("files `{}` in `{}`", pat(p), path(b, rel)),
        Matcher::Dir(p, w) => format!("dir `{}`{}", pat(p), when(w)),
        Matcher::Under(parent, p, w) => format!("dir `{parent}/{}`{}", pat(p), when(w)),
        Matcher::Contains(w) => format!("any dir{}", when(w)),
    }
}

fn esc(s: &str) -> String {
    s.replace('|', "\\|")
}

fn main() {
    for g in CATALOG {
        println!("\n#### {}\n", g.title);
        println!("| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |");
        println!("|---|---|---|---|---|---|---|---|---|---|");
        for r in g.rules {
            let risk = match r.kind {
                Kind::Target(Risk::Safe) => "**Safe**".to_string(),
                Kind::Target(Risk::Low) => "**Low**".to_string(),
                Kind::Target(Risk::Review) => "**Review**".to_string(),
                Kind::Container { default } => match default {
                    Some(d) => format!("container (default `{d}`)"),
                    None => "container".to_string(),
                },
                Kind::Protect(Scope::SelfOnly) => "protect (self)".into(),
                Kind::Protect(Scope::NoProjectRules) => "protect (no project rules inside)".into(),
                Kind::Protect(Scope::NoHeuristics) => "protect (no heuristics inside)".into(),
            };
            // Group consecutive `Dir` matchers with the same condition: "dir `a`, `b` (cond)".
            let mut lines: Vec<String> = Vec::new();
            let mut i = 0;
            while i < r.matchers.len() {
                if let Matcher::Dir(p0, w0) = r.matchers[i] {
                    let mut names = vec![format!("`{}`", pat(p0))];
                    let mut j = i + 1;
                    while j < r.matchers.len() {
                        match r.matchers[j] {
                            Matcher::Dir(pj, wj) if format!("{wj:?}") == format!("{w0:?}") => { names.push(format!("`{}`", pat(pj))); j += 1; }
                            _ => break,
                        }
                    }
                    lines.push(format!("dir {}{}", names.join(", "), when(w0)));
                    i = j;
                } else {
                    lines.push(matcher(&r.matchers[i]));
                    i += 1;
                }
            }
            let mut matchers = lines.join("<br>");
            if matchers.is_empty() { matchers = "(container default only)".into(); }
            if r.zone == Zone::Projects { matchers.push_str("<br>*projects only*"); }
            let mut expl = r.reason.to_string();
            if let Some(c) = r.caution { expl.push_str(&format!(" ⚠ {c}")); }
            if r.shares_space { expl.push_str(" ⧉ *Size may overstate what deleting frees (shared blocks).*"); }
            let cleanup = r.cleanup.iter().map(|c| if c.contains('▸') { c.to_string() } else { format!("`{c}`") }).collect::<Vec<_>>().join("<br>");
            let del = match (r.kind, r.removal) {
                (Kind::Protect(_), _) => "never",
                (Kind::Container { .. }, _) => "–",
                (_, Removal::Item) => "item",
                (_, Removal::Contents) => "contents",
                (_, Removal::ToolOnly) => "tool",
            };
            let mut evs = Vec::new();
            if r.evidence & ev::DOCS != 0 { evs.push("D"); }
            if r.evidence & ev::SOURCE != 0 { evs.push("S"); }
            if r.evidence & ev::LOCAL != 0 { evs.push("L"); }
            if r.evidence & ev::SECONDARY != 0 { evs.push("2"); }
            let evs = if evs.is_empty() { "–".to_string() } else { evs.join("") };
            println!(
                "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                r.id, esc(r.name), risk, r.category.title(), r.os.label(), esc(&matchers), esc(&expl), esc(&cleanup), del, evs
            );
        }
    }
}
