## 3. Rust data model (excerpt of `src/model.rs` / `src/engine.rs`)

The whole table is `const`-constructed, using `const fn` builders. Promotion of `&[...]` inside `static` initializers works on stable Rust (checked with 1.93.1, edition 2024).

```rust
pub enum Risk { Safe, Low, Review }
pub enum Category { Cache, BuildArtifacts, Dependencies, PackageCache, DeveloperTools, Logs, Trash,
                    Installers, VirtualMachines, Backups, Temporary, SystemUpdates, Models, Protected }
pub struct Os(pub u8);                       // MAC | LINUX | WIN bit set
pub enum Base { Root, Home, MacCaches, XdgCache, XdgData, LocalAppData, AppData, Temp, WinDir,
                SystemDrive, ProgramData, Env(&'static str) }
pub enum Pat { Is(&'static str), Prefix(&'static str), Suffix(&'static str), Glob(&'static str),
               Ext(&'static [&'static str]) }

/// One conjunction over marker bits: sib_* = parent's entries, own_* = the dir's entries.
pub struct Cond { pub sib_all: u64, pub sib_any: u64, pub sib_none: u64,
                  pub own_all: u64, pub own_any: u64, pub own_none: u64 }
pub type When = &'static [Cond];             // OR of conjunctions; &[] = always

pub enum Matcher {
    At(Base, &'static [&'static str]),       // anchored dir; components may be `*` globs
    FileAt(Base, &'static [&'static str], Pat), // regular files directly in an anchored dir
    Dir(Pat, When),                          // heuristic: own name + markers
    Under(&'static str, Pat, When),          // heuristic: parent name + own name (+ markers)
    Contains(When),                          // heuristic: own entries only (pyvenv.cfg, CACHEDIR.TAG)
}
pub enum Scope { SelfOnly, NoProjectRules, NoHeuristics }
pub enum Kind { Target(Risk), Container { default: Option<&'static str> }, Protect(Scope) }
pub enum Zone { Anywhere, Projects }         // Projects: off inside NoProjectRules zones
pub enum Removal { Item, Contents, ToolOnly }

pub struct Rule {
    pub id: &'static str,                    // stable, e.g. "cargo-target"
    pub name: &'static str,                  // group title in the panel
    pub category: Category, pub os: Os, pub kind: Kind, pub zone: Zone,
    pub matchers: &'static [Matcher],        // any-of
    pub removal: Removal,
    pub reason: &'static str,                // <= 70 chars, ends with '.'
    pub caution: Option<&'static str>,
    pub cleanup: &'static [&'static str],    // commands, or UI paths containing '▸'; {name}/{path}
    pub shares_space: bool,                  // hard links / APFS clones: size overstates gain
    pub evidence: u8,                        // ev::DOCS | SOURCE | LOCAL | SECONDARY
}
```

Example entries, verbatim from `src/catalog.rs`:

```rust
Rule::target("cargo-target", "Cargo target", BuildArtifacts, Safe, Os::ALL,
    &[Dir(Is("target"), &[Cond::sib(m::CARGO_TOML), Cond::has(m::CACHEDIR_TAG | m::RUSTC_INFO)])])
    .why("Rust build output; cargo build recreates it (full rebuild).")
    .cleanup(&["cargo clean"]).projects().ev(ev::SOURCE | ev::LOCAL),

Rule::target("node-modules-pnpm", "node_modules (pnpm)", Dependencies, Low, Os::ALL,
    &[Dir(Is("node_modules"), &[Cond::sib(m::PACKAGE_JSON).and_has_any(m::PNPM_MODULES)])])
    .why("Packages linked by pnpm; pnpm install restores them.")
    .caution("Files are hard links/clones of the pnpm store: little space is freed until `pnpm store prune`.")
    .cleanup(&["pnpm install", "pnpm store prune"]).shares_space().projects(),

Rule::target("cmake-build", "CMake build tree", BuildArtifacts, Low, Os::ALL,
    &[Contains(&[Cond::has(m::CMAKE_CACHE).lacks(m::CMAKE_LISTS | m::GIT)])])   // never an in-source build
    .why("Out-of-source CMake build tree; regenerate with cmake and rebuild.").projects(),

Rule::container("mac-caches", "App caches (~/Library/Caches)", Cache, Os::MAC, Some("mac-app-cache"),
    &[At(MacCaches, &[])]),
Rule::target("mac-app-cache", "App cache", Cache, Low, Os::MAC, &[])          // the container default
    .why("Cache of one app; it is rebuilt, possibly by re-downloading.").caution("Quit the app first."),

Rule::target("rustup-toolchain", "Rust toolchain", DeveloperTools, Low, Os::ALL,
    &[At(Env("RUSTUP_HOME"), &["toolchains", "*"]), At(Home, &[".rustup", "toolchains", "*"])])
    .cleanup(&["rustup toolchain list", "rustup toolchain uninstall {name}"]).removal(Removal::ToolOnly),

Rule::protect("app-bundles", "Application bundle", Os::ALL, Scope::NoHeuristics,
    &[Dir(Suffix(".app"), BUNDLE), /* .framework .bundle .plugin .appex .xpc .kext */])
// const BUNDLE: When = &[Cond::has_any(m::BUNDLE_CONTENTS | m::BUNDLE_VERSIONS | m::INFO_PLIST)];
```

**Markers** are 51 bits of `u64`, e.g.:
- files: `package.json`, `Cargo.toml`, `CACHEDIR.TAG`, `.rustc_info.json`, `pyvenv.cfg`, `pom.xml`, `build.gradle[.kts]`, `Package.swift`, `Podfile`, `pubspec.yaml`, `*.csproj`, `*.tf`, `CMakeCache.txt`, `next.config.*`, `.modules.yaml`;
- dirs: `Assets/` + `ProjectSettings/` (Unity), `conda-meta/`, `execroot/` + `external/` (Bazel), `Local Storage/` + `GPUCache/` (Chromium), `.git`, `Contents/` …

The full list is `m::NAMES`. `marker_of(name, is_dir) -> u64` is a single `match`; the scanner ORs it over the entries of each listing. For `CACHEDIR.TAG` it must clear the bit unless `is_valid_cachedir_tag(path)` holds: a regular file, not a symlink, whose first 43 bytes are `Signature: 8a477f597d28d172789f06886806bc55`.

**Engine API** (what the scanner and UI call):

```rust
let engine = Engine::new(&Resolver::from_env());      // once; Arc-share with the scan workers
let (_, parent_ctx) = engine.enter(scan_root_parent);  // anchored rules along the path, no I/O
// in the worker, after read_dir(D) + stats, before publishing D's children:
let own: u64 = entries.iter().fold(0, |a, e| a | marker_of(&e.name, e.is_dir));
let (verdict, d_ctx) = engine.classify_dir(&job.ctx, parent_name, d_name, own);
//   Verdict::{None, Target{rule, by_default}, Container(rule), Protected(rule), Inside(rule)}
//   d_ctx: Copy, 24 bytes -> stored in each child Job; d_ctx.in_git_worktree() for §5.6
for f in files { if let Some(rule) = engine.classify_file(&d_ctx, &f.name) { /* installer etc. */ } }
```

Suggested scanner changes (scan-engine.md §7.2):
- `Job` gains `ctx: Ctx`;
- `DirListing` gains `verdict: Verdict` (4 bytes) and an `in_git` bit;
- large files (≥ 1 MiB, already listed individually) may carry a file-rule id.

The UI tree stores the verdict per node. The cleanup panel is a filter over nodes with `Verdict::Target` (sizes come from the tree, as ux-spec requires).

---

## 4. The catalog

Columns:
- **Risk**: see §1. "container" = its children are the candidates. "protect" = never suggested.
- **Match**:
  - `~` = home; `~/.cache` = `$XDG_CACHE_HOME` or `~/.cache`; `$VAR` = env-var override (only if set and absolute); `*` = glob component.
  - "next to X" = the parent contains X. "containing X" = the folder itself contains X. Within one condition all parts must hold; "OR" separates alternatives.
  - *projects only* = disabled inside `~/Library`, hidden home dirs, `AppData` and test fixtures.
- **Del.**: removal mode (§1).
- **Ev.** (evidence):
  - **D** = vendor documentation
  - **S** = tool source code read
  - **L** = observed on this Mac
  - **2** = secondary sources (forums, guides)
  - **–** = from knowledge only (verify before relying on it)
- ⚠ = caution text. ⧉ = `shares_space`.

Order matters only among heuristic rules: the first match wins, and the generic group comes last.
