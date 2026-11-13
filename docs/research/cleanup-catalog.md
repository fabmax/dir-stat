# dir-stat: cleanup-suggestion catalog

Research deliverable for dir-stat (Rust + iced 0.14). Verified 2026-11-02 on macOS 26.7.1 (Apple silicon), plus Linux and Windows rules checked against vendor documentation.

**Companion prototype** (compiles with no dependencies, 9 tests pass): `/tmp/dirstat-work/research/cleanup-catalog-proto/`

| File | Contents |
|---|---|
| `src/model.rs` | Data model (`Rule`, `Matcher`, `Cond`, `Risk`, …), marker bits, `marker_of()` |
| `src/catalog.rs` | The static table: **178 rules in 14 groups** |
| `src/engine.rs` | Index + streaming matcher (`Engine::classify_dir/classify_file/enter`) and invariant tests (macOS, Linux and Windows trees) |
| `examples/catalog_md.rs` | Generates the tables in §4. They are pasted from its output, so the doc and the code can't drift |
| `examples/smoke.rs` | Read-only check on real directories (no symlinks, one device, depth-limited) |
| `examples/bench.rs` | Microbenchmark |

```
cd /tmp/dirstat-work/research/cleanup-catalog-proto
cargo test                                   # catalog invariants + matcher behaviour
cargo run --example catalog_md               # regenerate the §4 tables
cargo run --release --example smoke -- 7 ~/dev /Applications
```

Sibling research I aligned with (read-only): `scan-engine.md` §7 (worker lists a directory, then publishes its children: parents strictly before children) and `ux-spec.md` §7 and §16.2 (cleanup panel, per-match fields).

---

## 0. TL;DR

1. **Three risk levels plus "protect".**
   - **Safe**: derived or cached data that the owning tool recreates on demand.
   - **Low**: restorable, but only by re-downloading, rebuilding or running a tool.
   - **Review**: may contain user data or state that can't be recreated.
   - **Protect**: never suggested. It can also switch heuristics off below it.

   Every rule also carries:
   - a removal mode: `item`, `contents`, or `tool` (= don't delete files, show the command);
   - a ≤ 70-character reason (enforced by a test);
   - optional caution text;
   - cleanup commands or UI paths;
   - a `shares_space` flag for hard-link/clone stores;
   - evidence bits.
2. **Two kinds of matchers.**
   - **Anchored** well-known locations (`~/Library/Caches/Homebrew`, `$CARGO_HOME/registry/src`, `%SystemRoot%\SoftwareDistribution\Download`). These are resolved once at startup into a component trie, including env-var overrides.
   - **Heuristics** evaluated per directory: its own name, its parent's name, *marker bits* of its siblings (= the parent's entries), and marker bits of its own entries.

   **Anchored knowledge always wins.** A heuristic never fires on a directory that lies on the path to an anchored rule. So `~/Library` is never Unity's `Library`, and `~/.gradle` is never a project `.gradle`.
3. **Streaming fit.** Everything a rule may look at is known exactly when a directory's own listing is done, before any of its children are listed. So each directory is classified **once, in the scan worker, right after `read_dir`**:
   - the 24-byte `Ctx` travels with each child job;
   - cost ≈ **0.2 µs per directory**, including the marker computation for a 10-entry listing (release build, M-series);
   - that is negligible next to the one `fstatat` per entry the scanner already does.
4. **Outermost match wins.** Nothing below a candidate is evaluated, so totals never double-count and nested `node_modules` don't show up as separate items. The exception is **containers** (`~/Library/Caches`, `~/.cache`, `/var/cache`, `$CARGO_HOME/registry`, JetBrains caches): they are not candidates themselves, and each direct child is one, matched by a specific rule or else by the container's default rule.
5. **Safety zones found necessary on this machine.** Each of these would have been a wrong "safe" label without a zone:
   - `/Applications/Visual Studio Code.app/Contents/Resources/app/node_modules` has a sibling `package.json`.
   - `/opt/homebrew/lib/node_modules/npm/node_modules` (npm's own dependencies) has one too.
   - Tracked test fixtures in the JetBrains monorepo (`testData/**/Pods`, `testResources/**/vendor`, `testData/**/.build`, …).
   - Two folders in that repo that are **git-tracked**: `…/typeDefinitions/node_modules` and `…/execute-on-changed-ant-task/target`.

   The fix:
   - protect zones for app bundles (only with a real `Contents/`, `Versions/` or `Info.plist`), package-manager prefixes, OS folders, VCS dirs and test-fixture dirs;
   - "no project rules" inside `~/Library`, `~/.*` and `AppData`;
   - a cheap `in_git_worktree` flag, so the UI can verify lazily that a candidate is ignored and contains no tracked files before calling it safe (§5.6).
6. **"Reclaimable" can be overstated.** These cases are flagged per rule (`shares_space`, cautions):
   - pnpm/bun/uv use APFS clones on macOS. The pnpm `node_modules` here has `nlink = 1` but shares blocks with the store.
   - Xcode parallel-test simulator clones: one report measured 307 GB with `du` and freed 0 GB.
   - Docker.raw is sparse.
   - Simulator runtimes are mounted volumes.
7. **The biggest wins on this Mac** (du, allowed locations only):
   - Rust `target` dirs under `~/dev`: 94 G + 22 G + 6.6 G + 4.1 G + 1.9 G (all carry Cargo's `CACHEDIR.TAG` and `.rustc_info.json`);
   - `~/.m2/repository`: 34 G;
   - `~/.gradle`: 12 G;
   - `~/.rustup/toolchains`: 10 G (8 toolchains);
   - `~/.cargo/registry`: 1.8 G;
   - `~/.npm`: 1.5 G (1.3 G of it npx).

---

## 1. Risk levels and how the UI should use them

| Level | Meaning (catalog contract) | Typical examples | ux-spec §7 section |
|---|---|---|---|
| **Safe** | Pure cache or derived data, recreated on demand by the owning tool. No user data. | DerivedData, Cargo `target`, `__pycache__`, Homebrew/pip/npm download caches, logs | "Safe to delete" |
| **Low** | Restorable, but costs a re-download, a rebuild or an explicit tool command. Offline work may break until then. | `node_modules`, `.venv`, `~/.m2/repository`, Gradle caches, rustup toolchains, simulator runtimes, HF models | "Safe to delete", sorted after Safe. Show the reason or caution, e.g. "Reinstalled by npm". A product decision may move it to "Review" instead. |
| **Review** | May contain user data or state that can't be recreated automatically. | Trash, Xcode Archives (dSYMs), iOS backups, Docker disk, VMs, simulator devices, AVDs, Mail attachment copies, Windows.old, installers in Downloads | "Review first" |
| Protect | Never suggested. *Self* protects only the folder itself; the stronger scopes also switch off heuristics below it. | `/`, `~`, `~/Documents`, app bundles, `/opt/homebrew`, `.git`, iCloud Drive, WinSxS, debconf | not listed (optional "System" pill) |

**Removal mode** (`Del.` column in §4):
- **`item`**: move the matched folder or file to the Trash.
- **`contents`**: trash its children but keep the folder. Use this for tool- or OS-owned folders such as `~/Library/Logs` or `SoftwareDistribution\Download`.
- **`tool`**: do **not** offer Move to Trash. Show the cleanup command or UI path instead. Typical reasons:
  - the files are read-only (Go module cache);
  - the vendor says never to touch them directly (uv cache, simulator runtimes);
  - the folder holds data a tool must manage itself (Docker disk, snaps, journal, Windows.old, iOS backups, conda pkgs);
  - emptying is the operation (Trash).

**Mapping to the ux-spec §16.2 contract:**

| ux-spec field | Catalog source |
|---|---|
| `rule_id` | `Rule::id` |
| `group_title` | `Rule::name` |
| `short_label` | `Category::pill()`: "Cache", "Build", "Deps", "Logs", "Trash", "Installer", "VM", "Backup", "Tools", "Temp", "Update", "Model" |
| `risk` | Safe/Low → `safe`, Review → `review`. Protect is never emitted. |
| `reason` | `Rule::reason`: ≤ 70 chars, sentence case, ends with a period (checked by `validate()`) |
| `how_to_clean` | First `cleanup` entry: "Or run \`…\`." for commands, "Prefer …" for UI paths (entries containing `▸`). Long commands should go into the tooltip. |
| *(new)* `caution` | `Rule::caution`. Please show it next to the trash action, e.g. "Quit Xcode first." or "Stop Gradle daemons first." |
| *(new)* `shares_space` | Show "may free less than shown". |
| `project_last_modified` | Only for rules tagged *projects only*. Use the newest mtime of the sibling marker files (`package.json`, `Cargo.toml`, …) or of the project dir. Compute it lazily in the UI: a few hundred stats when the panel opens. |

`cleanup` strings may contain `{name}` / `{path}` placeholders (`rustup toolchain uninstall {name}`); `Engine::cleanup_for()` fills them in. `<id>`, `<name>`-style placeholders are for the user to fill.

---

## 2. Matching model

### 2.1 What is known when

The scanner (scan-engine.md §7.4) lists directory **D** with `read_dir` + one stat per entry, sends D's `DirListing`, and *then* publishes D's subdirectories as jobs. So when D's listing is done, the worker knows:

| Fact | Source | Available? |
|---|---|---|
| D's name and its parent's name | job path | yes |
| D's absolute path → trie state | parent's `Ctx.states` + one step | yes |
| D's **siblings** (= parent's entries) | parent's `Ctx.markers` (computed when the parent was listed) | yes, as marker bits |
| D's **own entries** (names + file/dir type) | this listing | yes |
| Grandchildren, e.g. `ProjectSettings/ProjectVersion.txt` as seen from a Unity `Library` (its own entries such as `node_modules/.modules.yaml` *are* available) | later listings | **no**: no rule depends on them |
| Grandparent's entries | | **no** (rules use the parent *name* at most) |
| File contents | | only `CACHEDIR.TAG`. Read 43 bytes when the bit is set (rare) to check the signature. |

The parent's listing arrives first, but its markers are only half of what D's classification needs; D's own markers come with D's listing. So **classification happens once, when D's listing completes**. No pre-classification at announcement time is needed. The UI could show a provisional pill from name + siblings, but that is not worth the complexity: a listing takes microseconds.

Because D's verdict is final before any child job exists, *outermost match wins* is just a flag copied into the children's `Ctx`. No buffering, no re-evaluation.

### 2.2 Per-directory algorithm (`Engine::eval_dir`)

1. **Inside a target?** Then return `Inside(rule)`. No work at all for the millions of files under `node_modules`/`target`.
2. **Trie step** (one hash lookup per state, plus the few glob edges at that node; at most 2 live states). Then the **anchored rules at exactly this path**:
   - `Protect` rules add zone flags.
   - `Target` or `Container` rules are remembered.
3. **Protect heuristics.** These are always active:
   - app bundles (`*.app`/`.framework`/… only with a real `Contents/`, `Versions/` or `Info.plist`, so a cache folder named `com.example.app` is not mistaken for one);
   - `.git` / `.hg` / `.svn`;
   - test-fixture dirs (`testData`, `testdata`, `fixtures`, …);
   - `*.photoslibrary` / `*.musiclibrary`.
4. **Pick the result of steps 2-3.** An anchored Target or Container returns now; otherwise a Protect match returns `Protected`.
5. **Heuristic targets** run only if all of these hold:
   - the dir is not in a *NoHeuristics* zone;
   - it is not on the path to an anchored rule;
   - for rules tagged *projects only*: it is not in a *NoProjectRules* zone.

   Candidates are checked in this order:
   1. rules for this exact name (hash map: `node_modules`, `target`, `build`, `.gradle`, …);
   2. name-pattern rules (about 12: `*.pvm`, `.Trash-*`, …);
   3. `Contains` rules, only if the dir has marker bits (about 6: `pyvenv.cfg`, `CACHEDIR.TAG`, `CMakeCache.txt` …).

   **The first rule in table order wins**, so specific rules beat generic ones. For example, a virtualenv (which virtualenv and uv also tag with `CACHEDIR.TAG`) is a `python-venv`, not a generic tagged cache.
6. **Container default** for a direct child of a container that matched nothing. If the child lies on an anchored path (e.g. `~/Library/Caches/Google`, which leads to `Google/AndroidStudio*`), the default is carried one level deeper instead. That way `~/Library/Caches/Google/Chrome` still gets "App cache".

Files: `classify_file(&dir_ctx, name)` checks only anchored file rules: installers in `~/Downloads`, `C:\pagefile.sys`, `MEMORY.DMP`. Dirs without file rules cost nothing.

**Complexity.** Markers cost O(entries): one `match` on the name, plus one on the extension or stem. Per directory the cost is O(1) expected: one trie step, one `by_name` lookup and about 20 integer mask tests. Measured: **207 ns per directory including markers for a 10-entry listing**, 178 rules, release build (`examples/bench.rs`). The index is a few KB; `Ctx` is 24 bytes per pending job.

### 2.3 Zones (Protect scopes)

| Scope | Effect below the folder | Used for |
|---|---|---|
| `SelfOnly` | none (only the folder itself is never suggested) | `/`, `/Users`, `/Applications`, `/Volumes/*`, `~`, `~/Desktop`, `~/Documents`, `~/Downloads`, …, `C:\`, `C:\Users` |
| `NoProjectRules` | project rules (node_modules, target, build, .venv, …) disabled. App-cache rules (Chromium, CACHEDIR.TAG, VM bundles) and anchored rules still apply. | `~/Library`, every hidden home dir `~/.*` (tools: `~/.vscode/extensions`, `~/.npm-global`, `~/.local/pipx`, `~/.config/*`), `%USERPROFILE%\AppData`, test-fixture dirs |
| `NoHeuristics` | all heuristics disabled; anchored rules still apply | macOS `/System`, `/usr`, `/bin`, `/sbin`, `/Library`, `/private`, `/cores`; Linux `/usr`, `/etc`, `/opt`, `/var/lib`, `/snap`, `/nix`, …; `C:\Windows`, Program Files, ProgramData; `/opt/homebrew`, `/usr/local/Cellar`; app bundles; `.git`; iCloud Drive; `~/Library/Mail` and `Messages`; uv/pipx tool venvs; `/var/cache/debconf`; WinSxS; `C:\Windows\Installer`; `ProgramData\Package Cache` |

So anchored rules still work inside protected trees (`C:\Windows\SoftwareDistribution\Download`, `/Library/Developer/CoreSimulator/Images`, `/var/lib/docker`). Name heuristics never reach into installed software.

### 2.4 Resolution of anchored locations

`Resolver` snapshots the environment once and resolves these bases:
- **`Home`**: `$HOME` or `%USERPROFILE%`.
- **`MacCaches`**: `~/Library/Caches`.
- **`XdgCache`**: `$XDG_CACHE_HOME`, else `~/.cache`. On macOS too, because uv, zig, Hugging Face, Puppeteer, pre-commit, … use it there.
- **`XdgData`**: `$XDG_DATA_HOME`, else `~/.local/share`.
- **Windows**: `LocalAppData`, `AppData`, `Temp`, `WinDir`, `SystemDrive`, `ProgramData`.
- **`Env(var)`**: the path from an env var.

Relative XDG values are ignored, as the XDG spec requires. For path lists (`GOPATH`) the first entry is used. Windows compares case-insensitively.

Tool overrides are additional matchers: `CARGO_HOME`, `RUSTUP_HOME`, `GRADLE_USER_HOME`, `GOCACHE`, `GOMODCACHE`, `GOPATH`, `npm_config_cache`, `YARN_CACHE_FOLDER`, `PNPM_HOME`, `BUN_INSTALL_CACHE_DIR`, `PIP_CACHE_DIR`, `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `POETRY_CACHE_DIR`, `HF_HOME`, `HF_HUB_CACHE`, `HF_XET_CACHE`, `OLLAMA_MODELS`, `PUB_CACHE`, `ANDROID_HOME`, `ANDROID_SDK_ROOT`, `ANDROID_AVD_HOME`, `ANDROID_USER_HOME`, `NUGET_PACKAGES`, `NUGET_HTTP_CACHE_PATH`, `CCACHE_DIR`, `SCCACHE_DIR`, `COURSIER_CACHE`, `KONAN_DATA_DIR`, `DENO_DIR`, `CYPRESS_CACHE_FOLDER`, `PLAYWRIGHT_BROWSERS_PATH`, `HOMEBREW_CACHE`, `VAGRANT_HOME`, `COMPOSER_CACHE_DIR`, `BAZELISK_HOME`.

**A GUI app started from Finder/Dock does not see variables exported in `~/.zshrc`.** It inherits launchd's environment, so overrides are often invisible. The defaults still match. An optional enrichment is to ask the tools once, in the background, with a timeout, and add the returned paths:
- `brew --cache`
- `go env GOCACHE GOMODCACHE`
- `npm config get cache`
- `pip3 cache dir`
- `pnpm store path`
- `yarn cache dir`

Only run *path-printing* commands. Lesson from this session: `bazel --version` through Bazelisk downloaded a Bazel binary.

---
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

#### macOS: system and user folders

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `mac-caches` | App caches (~/Library/Caches) | container (default `mac-app-cache`) | Cache | mac | `~/Library/Caches` | Per-app caches; apps must be able to rebuild them. |  | – | DL |
| `mac-app-cache` | App cache | **Low** | Cache | mac | (container default only) | Cache of one app; it is rebuilt, possibly by re-downloading. ⚠ Quit the app first. |  | item | D |
| `mac-sandbox-cache` | Sandboxed app cache | **Low** | Cache | mac | `~/Library/Containers/*/Data/Library/Caches` | Cache of a sandboxed (App Store) app; rebuilt on demand. ⚠ Quit the app first. macOS 14+ asks before reading other apps' containers. |  | contents | – |
| `mac-logs` | Logs (~/Library/Logs) | **Safe** | Logs | mac | `~/Library/Logs` | Log files and crash reports; only useful for troubleshooting. |  | contents | DL |
| `mac-system-logs` | System logs (/Library/Logs) | **Safe** | Logs | mac | `/Library/Logs` | System-wide logs and crash reports; deleting needs admin rights. |  | contents | DL |
| `mac-trash` | Trash | **Review** | Trash | mac | `~/.Trash`<br>`/Volumes/*/.Trashes` | Files you moved to the Trash; emptying deletes them for good. ⚠ Measuring ~/.Trash may require Full Disk Access. | Finder ▸ Empty Trash | tool | L |
| `mac-mail-downloads` | Mail attachment copies | **Review** | Cache | mac | `~/Library/Containers/com.apple.mail/Data/Library/Mail Downloads`<br>`~/Library/Mail Downloads` | Copies of attachments opened in Mail; edited ones may hold changes. | Mail ▸ Settings ▸ General ▸ Remove unedited downloads | contents | D2 |
| `ios-backups` | iPhone/iPad backups | **Review** | Backups | mac, win | `~/Library/Application Support/MobileSync/Backup`<br>`~/Apple/MobileSync/Backup`<br>`%APPDATA%\Apple Computer\MobileSync\Backup` | Local device backups; a deleted backup cannot be recovered. ⚠ Needs Full Disk Access to measure on macOS. | Finder ▸ (device) ▸ General ▸ Manage Backups… | tool | D |
| `ios-firmware` | iOS firmware downloads (.ipsw) | **Low** | Installers | mac, win | `~/Library/iTunes/iPhone Software Updates`<br>`~/Library/iTunes/iPad Software Updates`<br>`~/Library/iTunes/iPod Software Updates`<br>`%APPDATA%\Apple Computer\iTunes\iPhone Software Updates`<br>`%APPDATA%\Apple Computer\iTunes\iPad Software Updates` | Device firmware for updates/restores; downloaded again if needed. |  | contents | 2 |
| `mac-os-installer` | macOS installer app | **Low** | Installers | mac | `/Applications/Install macOS *.app` | macOS installer app (12+ GB); it can be downloaded again. | `softwareupdate --fetch-full-installer` | item | – |
| `downloads-installers` | Installers in Downloads | **Review** | Installers | all | files `*.dmg, *.pkg, *.mpkg, *.iso, *.xip, *.msi, *.msix, *.exe, *.deb, *.rpm, *.appimage` in `~/Downloads` | Installer or disk image you downloaded; rarely needed after install. |  | item | – |
| `homebrew-cache` | Homebrew downloads | **Safe** | Package-manager cache | mac, linux | `$HOMEBREW_CACHE`<br>`~/Library/Caches/Homebrew`<br>`~/.cache/Homebrew` | Downloaded bottles and casks; installed software doesn't need them. | `brew cleanup --prune=all` | contents | DL |

#### Apple development (Xcode, simulators, Swift, CocoaPods, Carthage)

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `xcode-derived-data` | Xcode DerivedData | **Safe** | Build artifacts | mac | `~/Library/Developer/Xcode/DerivedData` | Xcode build products and indexes; rebuilt on next build. ⚠ Quit Xcode first. |  | contents | 2 |
| `xcode-archives` | Xcode Archives | **Review** | Backups | mac | `~/Library/Developer/Xcode/Archives` | Archived app builds with dSYMs, needed to symbolicate crashes. ⚠ Apple: "You must retain the Xcode archive for each build of your app you distribute." | Xcode ▸ Window ▸ Organizer ▸ Archives | item | D |
| `xcode-device-support` | Xcode device support | **Low** | Developer tools | mac | `~/Library/Developer/Xcode/iOS DeviceSupport`<br>`~/Library/Developer/Xcode/watchOS DeviceSupport`<br>`~/Library/Developer/Xcode/tvOS DeviceSupport`<br>`~/Library/Developer/Xcode/visionOS DeviceSupport` | Debug symbols per device OS version; recopied when a device connects. ⚠ Quit Xcode first. Re-copying needs the device and takes minutes. |  | contents | 2 |
| `xcode-previews` | SwiftUI preview simulators | **Safe** | Cache | mac | `~/Library/Developer/Xcode/UserData/Previews` | Simulators and build data used for SwiftUI previews. | `xcrun simctl --set previews delete all` | tool | 2 |
| `xcode-cache` | Xcode cache | **Safe** | Cache | mac | `~/Library/Caches/com.apple.dt.Xcode` | Xcode's own cache; rebuilt automatically. ⚠ Quit Xcode first. |  | item | – |
| `coresim-devices` | Simulator devices | **Review** | Developer tools | mac | `~/Library/Developer/CoreSimulator/Devices` | Simulator devices with the apps and data installed in them. ⚠ `delete unavailable` only removes simulators whose runtime is gone. | `xcrun simctl delete unavailable`<br>`xcrun simctl erase all` | tool | 2 |
| `xctest-devices` | Test simulator clones | **Safe** | Developer tools | mac | `~/Library/Developer/XCTestDevices` | Simulator clones for parallel testing; recreated by test runs. ⚠ Quit Xcode and stop running tests first. APFS clones: deleting may free far less than shown. ⧉ *Size may overstate what deleting frees (shared blocks).* | `xcrun simctl --set ~/Library/Developer/XCTestDevices delete all` | tool | 2 |
| `coresim-caches` | Simulator caches | **Safe** | Cache | mac | `~/Library/Developer/CoreSimulator/Caches` | Simulator dyld caches; regenerated when a simulator boots. |  | contents | 2 |
| `coresim-runtimes` | Simulator runtimes | **Low** | Developer tools | mac | `/Library/Developer/CoreSimulator/Images`<br>`/Library/Developer/CoreSimulator/Profiles/Runtimes` | Simulator runtimes (several GB each); re-downloadable in Xcode. ⚠ Remove only via Xcode or simctl. Mounted runtime volumes (…/CoreSimulator/Volumes) are separate file systems. | Xcode ▸ Settings ▸ Components<br>`xcrun simctl runtime list`<br>`xcrun simctl runtime delete <id>` | tool | D2 |
| `xcode-extra` | Additional Xcode versions | **Review** | Developer tools | mac | `/Applications/Xcode-*.app`<br>`/Applications/Xcode_*.app` | Another Xcode installation (10+ GB); keep it if a project needs it. |  | item | – |
| `swiftpm-build` | SwiftPM .build | **Safe** | Build artifacts | all | dir `.build` (next to Package.swift)<br>*projects only* | Swift package build output; rebuilt by swift build. | `swift package clean`<br>`swift package reset` | item | L |
| `swiftpm-cache` | SwiftPM cache | **Safe** | Package-manager cache | mac, linux | `~/Library/Caches/org.swift.swiftpm`<br>`~/.cache/org.swift.swiftpm` | Cached Swift package repositories; fetched again when needed. | `swift package purge-cache` | item | L |
| `cocoapods-cache` | CocoaPods cache | **Safe** | Package-manager cache | mac | `~/Library/Caches/CocoaPods` | Downloaded pods; CocoaPods fetches them again when needed. | `pod cache clean --all` | item | D |
| `cocoapods-pods` | CocoaPods Pods | **Low** | Dependencies | mac | dir `Pods` (next to Podfile)<br>*projects only* | Pods installed into the project; pod install restores them. ⚠ Some teams commit Pods/ to git. | `pod install` | item | – |
| `cocoapods-repos` | CocoaPods spec repos | **Low** | Package-manager cache | mac | `~/.cocoapods/repos` | Local clones of CocoaPods spec repos (the CDN is the default). | `pod repo list`<br>`pod repo remove <name>` | tool | D |
| `carthage` | Carthage checkouts and builds | **Low** | Dependencies | mac | dir `Carthage` (next to Cartfile)<br>*projects only* | Carthage dependencies; carthage bootstrap restores them. ⚠ With --use-submodules, Carthage/Checkouts are git submodules. | `carthage bootstrap` | item | – |
| `carthage-cache` | Carthage cache | **Safe** | Package-manager cache | mac | `~/Library/Caches/org.carthage.CarthageKit` | Carthage download cache; fetched again when needed. |  | item | – |

#### JavaScript / TypeScript

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `node-modules-pnpm` | node_modules (pnpm) | **Low** | Dependencies | all | dir `node_modules` (next to package.json, containing .modules.yaml\|.pnpm/)<br>*projects only* | Packages linked by pnpm; pnpm install restores them. ⚠ Files are hard links/clones of the pnpm store: little space is freed until `pnpm store prune`. ⧉ *Size may overstate what deleting frees (shared blocks).* | `pnpm install`<br>`pnpm store prune` | item | DL |
| `node-modules` | node_modules | **Low** | Dependencies | all | dir `node_modules` (next to package.json)<br>*projects only* | Installed npm/Yarn/Bun packages; reinstalled by npm ci or yarn. | `npm ci`<br>`yarn install`<br>`bun install` | item | L |
| `npm-cache` | npm cache | **Safe** | Package-manager cache | all | `$npm_config_cache`<br>`~/.npm`<br>`%LOCALAPPDATA%\npm-cache` | npm download cache and npx packages; refetched on demand. | `npm cache clean --force`<br>`npm cache npx rm` | contents | DL |
| `yarn-cache` | Yarn 1 cache | **Safe** | Package-manager cache | all | `$YARN_CACHE_FOLDER`<br>`~/Library/Caches/Yarn`<br>`~/.cache/yarn`<br>`%LOCALAPPDATA%\Yarn\Cache` | Yarn classic's global package cache; refetched on demand. | `yarn cache clean` | item | L2 |
| `yarn-berry-cache` | Yarn 2+ global cache | **Safe** | Package-manager cache | all | `~/.yarn/berry/cache` | Yarn 2+ shared package archives; refetched on demand. | `yarn cache clean --mirror` | item | D |
| `yarn-project-cache` | Project .yarn/cache | **Review** | Dependencies | all | dir `.yarn/cache`<br>*projects only* | Project-local Yarn archives; Zero-Install repos commit them to git. |  | item | D |
| `pnpm-store` | pnpm store | **Low** | Package-manager cache | all | `$PNPM_HOME/store`<br>`~/.local/share/pnpm/store`<br>`~/Library/pnpm/store`<br>`%LOCALAPPDATA%\pnpm\store`<br>dir `.pnpm-store` | pnpm's package store shared by all projects; refetched on demand. ⚠ Projects using the global virtual store need `pnpm install` afterwards. ⧉ *Size may overstate what deleting frees (shared blocks).* | `pnpm store prune` | item | DL |
| `pnpm-cache` | pnpm metadata cache | **Safe** | Package-manager cache | all | `~/Library/Caches/pnpm`<br>`~/.cache/pnpm`<br>`%LOCALAPPDATA%\pnpm-cache` | pnpm package metadata and dlx cache; refetched on demand. |  | item | DL |
| `bun-cache` | Bun cache | **Low** | Package-manager cache | all | `$BUN_INSTALL_CACHE_DIR`<br>`~/.bun/install/cache` | Bun's global package cache; refetched on demand. ⚠ Projects using Bun's global store need `bun install` afterwards. ⧉ *Size may overstate what deleting frees (shared blocks).* | `bun pm cache rm` | tool | D |
| `next-build` | Next.js .next | **Safe** | Build artifacts | all | dir `.next` (next to package.json or next.config.*)<br>*projects only* | Next.js build output and cache; recreated by next dev/build. ⚠ Not on a server that is running this app. |  | item | D |
| `nuxt-build` | Nuxt .nuxt / .output | **Safe** | Build artifacts | all | dir `.nuxt`, `.output` (next to nuxt.config.*)<br>*projects only* | Nuxt generated files and build output; recreated on build. | `npx nuxt cleanup` | item | D |
| `sveltekit` | SvelteKit .svelte-kit | **Safe** | Build artifacts | all | dir `.svelte-kit` (next to package.json or svelte.config.*)<br>*projects only* | SvelteKit generated files; recreated by vite dev / build. |  | item | – |
| `angular-cache` | Angular .angular | **Safe** | Cache | all | dir `.angular` (next to angular.json)<br>*projects only* | Angular CLI build cache (.angular/cache). | `ng cache clean` | item | D |
| `turbo-cache` | Turborepo .turbo | **Safe** | Cache | all | dir `.turbo` (next to package.json or turbo.json)<br>*projects only* | Turborepo local task cache and run logs. |  | item | DL |
| `parcel-cache` | Parcel cache | **Safe** | Cache | all | dir `.parcel-cache` (next to package.json)<br>*projects only* | Parcel bundler cache; rebuilt on next build. |  | item | – |
| `electron-cache` | Electron downloads | **Safe** | Package-manager cache | all | `~/Library/Caches/electron`<br>`~/.cache/electron`<br>`%LOCALAPPDATA%\electron\Cache` | Electron binaries downloaded by @electron/get. |  | item | D |
| `node-gyp-cache` | node-gyp headers | **Safe** | Developer tools | all | `~/Library/Caches/node-gyp`<br>`~/.cache/node-gyp`<br>`%LOCALAPPDATA%\node-gyp\Cache`<br>`~/.node-gyp` | Node.js headers for building native add-ons; downloaded on demand. |  | item | L |
| `playwright-browsers` | Playwright browsers | **Low** | Developer tools | all | `$PLAYWRIGHT_BROWSERS_PATH`<br>`~/Library/Caches/ms-playwright`<br>`~/.cache/ms-playwright`<br>`%LOCALAPPDATA%\ms-playwright` | Browser builds downloaded by Playwright for tests. | `npx playwright uninstall --all` | item | D |
| `cypress-cache` | Cypress binaries | **Low** | Developer tools | all | `$CYPRESS_CACHE_FOLDER`<br>`~/Library/Caches/Cypress`<br>`~/.cache/Cypress`<br>`%LOCALAPPDATA%\Cypress\Cache` | Cypress app binaries, one per version. | `npx cypress cache prune`<br>`npx cypress cache clear` | item | D |
| `puppeteer-cache` | Puppeteer browsers | **Low** | Developer tools | all | `~/.cache/puppeteer` | Browsers downloaded by Puppeteer for automation/tests. | `npx puppeteer browsers install` | item | D |
| `deno-cache` | Deno cache | **Safe** | Package-manager cache | all | `$DENO_DIR`<br>`~/Library/Caches/deno`<br>`~/.cache/deno`<br>`%LOCALAPPDATA%\deno` | Deno's module cache; refetched on demand. | `deno clean` | item | D |

#### Rust

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `cargo-target` | Cargo target | **Safe** | Build artifacts | all | dir `target` (next to Cargo.toml; OR containing CACHEDIR.TAG + .rustc_info.json)<br>*projects only* | Rust build output; cargo build recreates it (full rebuild). | `cargo clean` | item | SL |
| `cargo-target-dir` | Cargo target (custom dir) | **Safe** | Build artifacts | all | any dir (containing CACHEDIR.TAG + .rustc_info.json) | Rust build output (custom target dir); recreated by cargo build. |  | item | SL |
| `cargo-registry` | Cargo registry | container | Package-manager cache | all | `$CARGO_HOME/registry`<br>`~/.cargo/registry` | Crates from registries; Cargo 1.88+ prunes unused ones itself. |  | – | DL |
| `cargo-registry-src` | Cargo registry sources | **Safe** | Package-manager cache | all | `$CARGO_HOME/registry/src`<br>`~/.cargo/registry/src` | Unpacked crate sources; re-extracted from registry/cache offline. ⚠ `cargo cache` is the third-party cargo-cache tool (cargo install cargo-cache). | `cargo cache --autoclean` | item | DL |
| `cargo-registry-cache` | Cargo .crate downloads | **Low** | Package-manager cache | all | `$CARGO_HOME/registry/cache`<br>`~/.cargo/registry/cache` | Downloaded .crate archives; re-downloaded when a build needs them. |  | item | DL |
| `cargo-registry-index` | Cargo registry index | **Low** | Package-manager cache | all | `$CARGO_HOME/registry/index`<br>`~/.cargo/registry/index` | Registry index metadata; fetched again when needed. |  | item | DL |
| `cargo-git` | Cargo git dependencies | container | Package-manager cache | all | `$CARGO_HOME/git`<br>`~/.cargo/git` | Git dependencies of Rust projects. |  | – | DL |
| `cargo-git-checkouts` | Cargo git checkouts | **Safe** | Package-manager cache | all | `$CARGO_HOME/git/checkouts`<br>`~/.cargo/git/checkouts` | Working copies checked out from git/db; recreated offline. ⚠ `cargo cache` is the third-party cargo-cache tool (cargo install cargo-cache). | `cargo cache --autoclean` | item | DL |
| `cargo-git-db` | Cargo git clones | **Low** | Package-manager cache | all | `$CARGO_HOME/git/db`<br>`~/.cargo/git/db` | Bare clones of git dependencies; cloned again when needed. |  | item | DL |
| `rustup-toolchain` | Rust toolchain | **Low** | Developer tools | all | `$RUSTUP_HOME/toolchains/*`<br>`~/.rustup/toolchains/*` | One installed Rust toolchain (~1-1.5 GB); reinstallable with rustup. ⚠ Keep your default toolchain and those pinned by rust-toolchain.toml. | `rustup toolchain list`<br>`rustup toolchain uninstall {name}` | tool | L |
| `rustup-tmp` | rustup leftovers | **Safe** | Temporary files | all | `$RUSTUP_HOME/tmp`<br>`$RUSTUP_HOME/downloads`<br>`~/.rustup/tmp`<br>`~/.rustup/downloads` | Leftover rustup downloads and temp files. |  | contents | L |

#### Python and ML

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `python-venv` | Python virtualenv | **Low** | Dependencies | all | any dir (containing pyvenv.cfg)<br>*projects only* | Python virtual environment; recreate it from your requirements. ⚠ If its packages aren't recorded anywhere, run `pip freeze > requirements.txt` first. |  | item | DL |
| `pycache` | __pycache__ | **Safe** | Build artifacts | all | dir `__pycache__`<br>*projects only* | Compiled Python bytecode; regenerated on import. |  | item | – |
| `mypy-cache` | mypy cache | **Safe** | Cache | all | dir `.mypy_cache`<br>*projects only* | mypy's incremental type-check cache. |  | item | S |
| `pytest-cache` | pytest cache | **Safe** | Cache | all | dir `.pytest_cache`<br>*projects only* | pytest cache (last-failed tests and similar). |  | item | S |
| `ruff-cache` | Ruff cache | **Safe** | Cache | all | dir `.ruff_cache`<br>*projects only* | Ruff linter cache. |  | item | S |
| `tox-envs` | tox environments | **Safe** | Dependencies | all | dir `.tox` (next to tox.ini or pyproject.toml\|setup.py\|setup.cfg)<br>*projects only* | Virtualenvs created by tox; recreated on the next tox run. |  | item | S |
| `nox-envs` | nox environments | **Safe** | Dependencies | all | dir `.nox` (next to noxfile.py)<br>*projects only* | Virtualenvs created by nox; recreated on the next nox run. |  | item | – |
| `pip-cache` | pip cache | **Safe** | Package-manager cache | all | `$PIP_CACHE_DIR`<br>`~/Library/Caches/pip`<br>`~/.cache/pip`<br>`%LOCALAPPDATA%\pip\Cache` | pip's wheel and HTTP cache; refetched on demand. | `pip cache purge` | item | DL |
| `uv-cache` | uv cache | **Safe** | Package-manager cache | all | `$UV_CACHE_DIR`<br>`~/.cache/uv`<br>`%LOCALAPPDATA%\uv\cache` | uv's package cache; refetched on demand. ⚠ uv docs: never modify the cache directly; use `uv cache clean`. ⧉ *Size may overstate what deleting frees (shared blocks).* | `uv cache clean`<br>`uv cache prune` | tool | DL |
| `uv-python` | uv-managed Pythons | **Low** | Developer tools | all | `$UV_PYTHON_INSTALL_DIR`<br>`~/.local/share/uv/python`<br>`%APPDATA%\uv\data\python` | Python interpreters installed by uv; reinstallable. | `uv python list --only-installed`<br>`uv python uninstall <version>` | tool | – |
| `poetry-cache` | Poetry cache | **Low** | Package-manager cache | all | `$POETRY_CACHE_DIR`<br>`~/Library/Caches/pypoetry`<br>`~/.cache/pypoetry`<br>`%LOCALAPPDATA%\pypoetry\Cache` | Poetry's cache, incl. its virtualenvs; poetry install recreates them. | `poetry cache clear pypi --all` | item | D |
| `conda-pkgs` | conda package cache | **Low** | Package-manager cache | all | dir `pkgs` (next to conda-meta/)<br>`~/.conda/pkgs` | Downloaded and extracted conda packages. ⚠ Deleting by hand can break environments that link into it; use conda clean. | `conda clean --all` | tool | D |
| `hf-hub` | Hugging Face models/datasets | **Low** | ML models | all | `$HF_HUB_CACHE`<br>`$HF_HOME/hub`<br>`~/.cache/huggingface/hub`<br>`~/.cache/huggingface/hub` | Downloaded Hugging Face models/datasets; re-downloaded on next use. | `hf cache ls`<br>`hf cache rm <repo>`<br>`hf cache prune` | item | D |
| `hf-xet` | Hugging Face upload cache | **Safe** | Cache | all | `$HF_XET_CACHE`<br>`~/.cache/huggingface/xet`<br>`~/.cache/huggingface/xet` | Xet shard cache used to speed up uploads. |  | item | D |
| `ollama-models` | Ollama models | **Review** | ML models | all | `$OLLAMA_MODELS`<br>`~/.ollama/models`<br>`/usr/share/ollama/.ollama/models` | Local LLM weights; pulled models can be downloaded again. ⚠ Models made with `ollama create` may not be reproducible. | `ollama list`<br>`ollama rm <model>` | tool | 2 |

#### JVM (Gradle, Maven, Kotlin, Scala) and JetBrains IDEs

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `gradle-caches` | Gradle caches | **Low** | Package-manager cache | all | `$GRADLE_USER_HOME/caches`<br>`~/.gradle/caches` | Gradle dependency and build caches; Gradle prunes unused entries. ⚠ Stop Gradle daemons first (`gradle --stop`). |  | contents | DL |
| `gradle-wrapper-dists` | Gradle distributions | **Low** | Developer tools | all | `$GRADLE_USER_HOME/wrapper/dists`<br>`~/.gradle/wrapper/dists` | Gradle versions downloaded by gradlew; unused ones auto-deleted. |  | item | DL |
| `gradle-jdks` | Gradle-provisioned JDKs | **Low** | Developer tools | all | `$GRADLE_USER_HOME/jdks`<br>`~/.gradle/jdks` | JDKs auto-downloaded by Gradle toolchains; fetched again if needed. |  | item | DL |
| `gradle-daemon` | Gradle daemon logs | **Safe** | Logs | all | `$GRADLE_USER_HOME/daemon`<br>`~/.gradle/daemon` | Gradle daemon registry and logs. ⚠ Stop Gradle daemons first (`gradle --stop`). |  | contents | DL |
| `gradle-project-cache` | Gradle project .gradle | **Safe** | Build artifacts | all | dir `.gradle` (next to build.gradle[.kts] or settings.gradle[.kts] or gradlew)<br>*projects only* | Per-project Gradle state (incremental build, config cache). |  | item | DL |
| `gradle-build` | Gradle build | **Safe** | Build artifacts | all | dir `build` (next to build.gradle[.kts])<br>*projects only* | Gradle build output of this project or module. | `./gradlew clean` | item | DL |
| `kotlin-project-dir` | Kotlin .kotlin | **Safe** | Build artifacts | all | dir `.kotlin` (next to build.gradle[.kts] or settings.gradle[.kts])<br>*projects only* | Kotlin Gradle plugin's per-project data (sessions, errors). |  | item | DL |
| `android-cxx` | Android .cxx | **Safe** | Build artifacts | all | dir `.cxx` (next to build.gradle[.kts])<br>*projects only* | CMake/ndk-build intermediates of an Android module. |  | item | – |
| `maven-repo` | Maven local repository | **Low** | Dependencies | all | `~/.m2/repository` | Downloaded Maven artifacts plus ones you installed locally. ⚠ Artifacts you only built locally or installed by hand (mvn install:install-file) are lost. |  | item | DL |
| `maven-wrapper` | Maven wrapper distributions | **Low** | Developer tools | all | `~/.m2/wrapper` | Maven versions downloaded by mvnw; fetched again if needed. |  | item | L |
| `maven-target` | Maven target | **Safe** | Build artifacts | all | dir `target` (next to pom.xml)<br>*projects only* | Maven build output; mvn package recreates it. | `mvn clean` | item | – |
| `sbt-target` | sbt target | **Safe** | Build artifacts | all | dir `target` (next to build.sbt)<br>*projects only* | sbt build output; recreated on the next build. | `sbt clean` | item | – |
| `coursier-cache` | Coursier cache | **Low** | Package-manager cache | all | `$COURSIER_CACHE`<br>`~/Library/Caches/Coursier`<br>`~/.cache/coursier`<br>`%LOCALAPPDATA%\Coursier\Cache` | Scala/sbt dependency cache; re-downloaded on demand. |  | item | D |
| `konan` | Kotlin/Native toolchains | **Low** | Developer tools | all | `$KONAN_DATA_DIR`<br>`~/.konan` | Kotlin/Native compilers and toolchains; re-downloaded by builds. ⚠ Never cleaned automatically; the next Kotlin/Native build is slower. |  | item | DL2 |
| `jetbrains-caches` | JetBrains IDE caches | container (default `jetbrains-system`) | Cache | all | `~/Library/Caches/JetBrains`<br>`~/.cache/JetBrains`<br>`%LOCALAPPDATA%\JetBrains` | One system folder per JetBrains IDE version. |  | – | DL |
| `jetbrains-system` | JetBrains IDE system folder | **Low** | Cache | all | (container default only) | IDE caches, indexes and Local History of one IDE version. ⚠ Close the IDE first. Local History is lost; indexes are rebuilt. | IDE ▸ Help ▸ Delete Leftover IDE Directories…<br>IDE ▸ File ▸ Invalidate Caches… | item | D |
| `android-studio-system` | Android Studio system folder | **Low** | Cache | all | `~/Library/Caches/Google/AndroidStudio*`<br>`~/.cache/Google/AndroidStudio*`<br>`%LOCALAPPDATA%\Google\AndroidStudio*` | Android Studio caches, indexes and Local History (one per version). ⚠ Close Android Studio first. Local History is lost. | Android Studio ▸ Help ▸ Delete Leftover IDE Directories… | item | – |

#### Go

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `go-build-cache` | Go build cache | **Safe** | Cache | all | `$GOCACHE`<br>`~/Library/Caches/go-build`<br>`~/.cache/go-build`<br>`%LOCALAPPDATA%\go-build` | Go build and test cache; trimmed automatically, rebuilt as needed. | `go clean -cache` | item | D |
| `go-mod-cache` | Go module cache | **Low** | Dependencies | all | `$GOMODCACHE`<br>`$GOPATH/pkg/mod`<br>`~/go/pkg/mod` | Downloaded Go modules; re-downloaded on demand. ⚠ Files are read-only, so plain deletion fails; use `go clean -modcache`. | `go clean -modcache` | tool | D |

#### Mobile (Android, Flutter/Dart)

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `android-system-images` | Android emulator images | **Low** | Developer tools | all | `$ANDROID_HOME/system-images`<br>`$ANDROID_SDK_ROOT/system-images`<br>`~/Library/Android/sdk/system-images`<br>`~/Android/Sdk/system-images`<br>`%LOCALAPPDATA%\Android\Sdk\system-images` | Emulator system images (1-3 GB each); re-downloadable. ⚠ Emulators (AVDs) that use a removed image stop working. | `sdkmanager --list_installed`<br>`sdkmanager --uninstall "system-images;android-<api>;<tag>;<abi>"` | item | D |
| `android-ndk` | Android NDKs | **Low** | Developer tools | all | `$ANDROID_HOME/ndk`<br>`$ANDROID_SDK_ROOT/ndk`<br>`~/Library/Android/sdk/ndk`<br>`~/Android/Sdk/ndk`<br>`%LOCALAPPDATA%\Android\Sdk\ndk` | Installed Android NDK versions (~1 GB each); re-downloadable. | `sdkmanager --uninstall "ndk;<version>"` | item | – |
| `android-avds` | Android emulators (AVDs) | **Review** | VMs/containers | all | `$ANDROID_AVD_HOME`<br>`$ANDROID_USER_HOME/avd`<br>`~/.android/avd` | Emulator devices with their data and snapshots. | Android Studio ▸ Device Manager<br>`avdmanager delete avd -n <name>` | tool | D |
| `flutter-build` | Flutter build | **Safe** | Build artifacts | all | dir `build` (next to pubspec.yaml)<br>*projects only* | Flutter/Dart build output; recreated on the next build. | `flutter clean` | item | – |
| `dart-tool` | Dart .dart_tool | **Safe** | Build artifacts | all | dir `.dart_tool` (next to pubspec.yaml)<br>*projects only* | Dart tool state; regenerated by dart pub get. | `flutter clean` | item | – |
| `pub-cache` | Dart/Flutter pub cache | **Low** | Package-manager cache | all | `$PUB_CACHE`<br>`~/.pub-cache`<br>`%LOCALAPPDATA%\Pub\Cache` | Packages shared by all Dart/Flutter projects; refetched on demand. ⚠ Also removes globally activated tools (dart pub global activate). | `dart pub cache clean` | tool | D |

#### Native and other ecosystems

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `cmake-build` | CMake build tree | **Low** | Build artifacts | all | any dir (containing CMakeCache.txt, not containing CMakeLists.txt or .git)<br>*projects only* | Out-of-source CMake build tree; regenerate with cmake and rebuild. |  | item | – |
| `ccache` | ccache | **Safe** | Cache | all | `$CCACHE_DIR`<br>`~/.ccache`<br>`~/Library/Caches/ccache`<br>`~/.cache/ccache`<br>`%LOCALAPPDATA%\ccache` | ccache compiler cache (default limit 5 GiB). ⚠ ccache.conf in this folder is configuration; `ccache -C` keeps it. | `ccache -C`<br>`ccache -c` | tool | DS |
| `sccache` | sccache | **Safe** | Cache | all | `$SCCACHE_DIR`<br>`~/.cache/sccache`<br>`~/Library/Caches/Mozilla.sccache`<br>`%LOCALAPPDATA%\Mozilla\sccache` | sccache compiler cache (default limit 10 GB). ⚠ Stop the server first (`sccache --stop-server`). |  | contents | D |
| `bazel-output-root` | Bazel output root | **Low** | Build artifacts | all | `~/.cache/bazel`<br>`~/Library/Caches/bazel`<br>`/private/var/tmp/_bazel_*`<br>`~/_bazel_*` | Bazel build outputs and external repos of all workspaces. ⚠ Run `bazel shutdown` first; prefer `bazel clean --expunge` per workspace. | `bazel shutdown`<br>`bazel clean --expunge` | item | DL |
| `bazel-output-base` | Bazel output base | **Low** | Build artifacts | all | any dir (containing execroot/ + external/) | Build outputs and external repos of one Bazel workspace. ⚠ Run `bazel shutdown` in the workspace first. | `bazel clean --expunge` | item | D |
| `bazelisk-cache` | Bazelisk downloads | **Safe** | Developer tools | all | `$BAZELISK_HOME`<br>`~/Library/Caches/bazelisk`<br>`~/.cache/bazelisk`<br>`%LOCALAPPDATA%\bazelisk` | Bazel binaries downloaded by Bazelisk; fetched again on demand. |  | item | L |
| `zig-cache` | Zig cache / zig-out | **Safe** | Build artifacts | all | dir `.zig-cache`, `zig-cache`, `zig-out` (next to build.zig)<br>*projects only* | Zig build cache and output; rebuilt by zig build. |  | item | L |
| `zig-global-cache` | Zig global cache | **Safe** | Cache | all | `~/.cache/zig`<br>`%LOCALAPPDATA%\zig` | Zig's global build cache; rebuilt as needed. |  | item | L |
| `dotnet-bin-obj` | .NET bin / obj | **Safe** | Build artifacts | all | dir `bin`, `obj` (next to *.csproj\|*.fsproj\|*.vbproj)<br>*projects only* | .NET project build output; recreated by dotnet build. | `dotnet clean` | item | – |
| `nuget-packages` | NuGet global packages | **Low** | Package-manager cache | all | `$NUGET_PACKAGES`<br>`~/.nuget/packages` | NuGet packages for all projects; restore downloads them again. ⚠ Close IDEs first, then restore your projects. | `dotnet nuget locals global-packages --clear` | tool | D |
| `nuget-http-cache` | NuGet HTTP cache | **Safe** | Package-manager cache | all | `$NUGET_HTTP_CACHE_PATH`<br>`~/.local/share/NuGet/v3-cache`<br>`~/.local/share/NuGet/http-cache`<br>`%LOCALAPPDATA%\NuGet\v3-cache` | Cached NuGet feed responses and packages. | `dotnet nuget locals http-cache --clear` | item | D |
| `stack-work` | Haskell .stack-work | **Safe** | Build artifacts | all | dir `.stack-work` (next to stack.yaml or *.cabal\|cabal.project)<br>*projects only* | Stack build output; recreated by stack build. | `stack clean --full` | item | D |
| `cabal-dist` | Cabal dist-newstyle | **Safe** | Build artifacts | all | dir `dist-newstyle` (next to *.cabal\|cabal.project)<br>*projects only* | Cabal build output; recreated by cabal build. | `cabal clean` | item | – |
| `elixir-build` | Elixir _build | **Safe** | Build artifacts | all | dir `_build` (next to mix.exs)<br>*projects only* | Mix build output; recreated by mix compile. |  | item | – |
| `elixir-deps` | Elixir deps | **Low** | Dependencies | all | dir `deps` (next to mix.exs)<br>*projects only* | Mix dependencies; mix deps.get restores them. | `mix deps.get` | item | – |
| `bundler-vendor` | Bundler vendor/bundle | **Low** | Dependencies | all | dir `vendor/bundle` (containing ruby/)<br>*projects only* | Gems installed into the project; bundle install restores them. | `bundle install` | item | – |
| `php-vendor` | Composer vendor | **Low** | Dependencies | all | dir `vendor` (next to composer.json + composer.lock, not next to go.mod)<br>*projects only* | Composer packages; composer install restores them from the lock. ⚠ Some deployments commit vendor/ instead of running composer. | `composer install` | item | – |
| `composer-cache` | Composer cache | **Safe** | Package-manager cache | all | `$COMPOSER_CACHE_DIR`<br>`~/Library/Caches/composer`<br>`~/.cache/composer`<br>`~/.composer/cache`<br>`%LOCALAPPDATA%\Composer` | Composer download cache; refetched on demand. | `composer clear-cache` | item | D |
| `terraform` | Terraform .terraform | **Low** | Dependencies | all | dir `.terraform` (next to *.tf\|*.tf.json\|.terraform.lock.hcl)<br>*projects only* | Provider/module cache and workspace pointer; terraform init redoes it. ⚠ Not during a backend migration. State files sit next to it; never delete those. | `terraform init` | item | D |
| `terragrunt-cache` | Terragrunt cache | **Safe** | Cache | all | dir `.terragrunt-cache`<br>*projects only* | Terragrunt's copy of modules; recreated on the next run. |  | item | – |
| `unity-library` | Unity Library | **Low** | Build artifacts | all | dir `Library` (next to Assets/ + ProjectSettings/)<br>*projects only* | Unity's import cache; rebuilt when the project is next opened. ⚠ Close the project in Unity first. Re-import and recompile can take long. |  | item | D |
| `unity-temp` | Unity Temp / obj | **Safe** | Build artifacts | all | dir `Temp`, `obj` (next to Assets/ + ProjectSettings/)<br>*projects only* | Unity temp and C# intermediate files. ⚠ Close the project in Unity first. |  | item | D |
| `unreal-intermediate` | Unreal Intermediate | **Safe** | Build artifacts | all | dir `Intermediate` (next to *.uproject)<br>*projects only* | Unreal Engine intermediate build files. |  | item | – |
| `unreal-ddc` | Unreal DerivedDataCache | **Low** | Cache | all | dir `DerivedDataCache` (next to *.uproject)<br>*projects only* | Unreal's project derived-data cache; rebuilt on demand (slow). |  | item | – |
| `godot-cache` | Godot import cache | **Safe** | Build artifacts | all | dir `.godot`, `.import` (next to project.godot)<br>*projects only* | Godot import cache; regenerated when the project opens. |  | item | – |
| `cdk-out` | AWS CDK cdk.out | **Safe** | Build artifacts | all | dir `cdk.out` (next to cdk.json)<br>*projects only* | Synthesized CloudFormation templates and assets. |  | item | – |
| `elm-stuff` | Elm elm-stuff | **Safe** | Build artifacts | all | dir `elm-stuff` (next to elm.json)<br>*projects only* | Elm build artifacts; recreated by elm make. |  | item | – |

#### Containers and virtual machines

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `docker-desktop-vm` | Docker Desktop disk image | **Review** | VMs/containers | mac, win | `~/Library/Containers/com.docker.docker/Data/vms`<br>`%LOCALAPPDATA%\Docker\wsl`<br>`%ProgramData%\DockerDesktop\vm-data` | All Docker images, containers, build cache and volumes. ⚠ Deleting it deletes every container and volume (databases!). Finder shows the maximum size of a sparse file. | `docker system df -v`<br>`docker system prune`<br>`docker builder prune` | tool | D |
| `docker-engine` | Docker data (Linux) | **Review** | VMs/containers | linux | `/var/lib/docker` | All Docker images, containers, build cache and volumes. ⚠ `docker volume prune` deletes data; volumes are never pruned by default. | `docker system df -v`<br>`docker system prune`<br>`docker builder prune` | tool | D |
| `podman-machine` | Podman machines | **Review** | VMs/containers | all | `~/.local/share/containers/podman/machine` | Podman VM disks with their images, containers and volumes. | `podman system df`<br>`podman system prune`<br>`podman machine rm <name>` | tool | L |
| `podman-storage` | Podman storage (rootless) | **Review** | VMs/containers | linux | `~/.local/share/containers/storage` | Rootless Podman images, containers and volumes. | `podman system prune` | tool | – |
| `colima-lima` | Colima / Lima VMs | **Review** | VMs/containers | mac, linux | `~/.colima`<br>`~/.lima` | Linux VMs (and their disks) managed by Colima or Lima. | `colima delete`<br>`limactl delete <name>` | tool | – |
| `vm-bundles` | Virtual machine | **Review** | VMs/containers | all | dir `*.pvm`, `*.utm`, `*.vmwarevm` | A virtual machine (Parallels/UTM/VMware) with its own OS and files. | `Delete the VM from inside Parallels, UTM or VMware` | item | – |
| `virtualbox-vm` | VirtualBox VM | **Review** | VMs/containers | all | any dir (containing *.vbox) | A VirtualBox VM folder with its virtual disks. | `VBoxManage unregistervm <name> --delete` | item | – |
| `vagrant-boxes` | Vagrant boxes | **Low** | VMs/containers | all | `$VAGRANT_HOME/boxes`<br>`~/.vagrant.d/boxes` | Downloaded Vagrant base boxes; re-downloadable. | `vagrant box list`<br>`vagrant box prune` | tool | – |
| `wsl-distro` | WSL distribution disk | **Review** | VMs/containers | win | files `ext4.vhdx` in `%LOCALAPPDATA%\Packages\*\LocalState`<br>`%LOCALAPPDATA%\wsl\*` | The whole disk of a WSL Linux distribution. | `wsl --list --verbose`<br>`wsl --unregister <distro>` | tool | – |

#### Linux system

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `xdg-cache` | User cache (~/.cache) | container (default `xdg-cache-entry`) | Cache | mac, linux | `~/.cache` | Per-user non-essential (cached) data, per the XDG spec. |  | – | DL |
| `xdg-cache-entry` | Program cache | **Low** | Cache | mac, linux | (container default only) | Cache of one program; rebuilt or re-downloaded on demand. ⚠ Close the program first. A few tools keep login tokens here. |  | item | D |
| `thumbnails` | Thumbnails | **Safe** | Cache | linux | `~/.cache/thumbnails` | Image thumbnails; regenerated by the file manager. |  | item | – |
| `var-cache` | System cache (/var/cache) | container (default `var-cache-entry`) | Cache | linux | `/var/cache` | System-wide application caches (FHS: must be regenerable). |  | – | D |
| `var-cache-entry` | System program cache | **Review** | Cache | linux | (container default only) | System cache of one program; prefer its own cleanup command. ⚠ Needs root. Some packages keep state here despite the FHS. |  | item | D |
| `debconf` | debconf database | protect (no heuristics inside) | Protected | linux | `/var/cache/debconf` | Debian's package configuration database lives here; never delete. |  | never | D |
| `apt-cache` | APT package cache | **Safe** | Package-manager cache | linux | `/var/cache/apt/archives` | Downloaded .deb packages; not needed after installation. | `sudo apt clean` | tool | – |
| `dnf-cache` | DNF/YUM cache | **Safe** | Package-manager cache | linux | `/var/cache/dnf`<br>`/var/cache/yum` | Downloaded RPM packages and repository metadata. | `sudo dnf clean all` | tool | – |
| `pacman-cache` | pacman package cache | **Low** | Package-manager cache | linux | `/var/cache/pacman/pkg` | Downloaded packages; old versions allow downgrades. | `sudo paccache -rk2`<br>`sudo pacman -Sc` | tool | – |
| `journal` | systemd journal | **Safe** | Logs | linux | `/var/log/journal` | Persistent system logs; vacuum old ones with journalctl. | `journalctl --disk-usage`<br>`sudo journalctl --vacuum-size=500M`<br>`sudo journalctl --vacuum-time=2weeks` | tool | D |
| `linux-trash` | Trash | **Review** | Trash | linux | `~/.local/share/Trash`<br>dir `.Trash-*`, `.Trash` | Files moved to the trash; emptying deletes them for good. | `gio trash --empty` | tool | D |
| `snap-revisions` | Snap packages | **Low** | System updates | linux | `/var/lib/snapd/snaps` | Installed snaps, incl. disabled old revisions kept for rollback. ⚠ Never delete .snap files by hand. | `snap list --all`<br>`sudo snap remove <name> --revision=<rev>`<br>`sudo snap set system refresh.retain=2` | tool | 2 |
| `flatpak` | Flatpak apps and runtimes | **Low** | System updates | linux | `/var/lib/flatpak`<br>`~/.local/share/flatpak` | Installed Flatpak apps and runtimes; unused runtimes can go. | `flatpak uninstall --unused` | tool | D |
| `flatpak-app-cache` | Flatpak app cache | **Safe** | Cache | linux | `~/.var/app/*/cache` | Cache of one Flatpak app (its XDG_CACHE_HOME). |  | contents | D |
| `coredumps` | Crash dumps | **Safe** | Logs | linux | `/var/lib/systemd/coredump`<br>`/var/crash` | Crash dumps; only needed to debug crashes. | `coredumpctl list` | contents | – |

#### Windows system

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `win-temp` | Temporary files | **Low** | Temporary files | win | `%TEMP%`<br>`%SystemRoot%\Temp` | Temporary files of apps and installers. ⚠ Skip files in use; finish pending installs/restarts first. | Settings ▸ System ▸ Storage ▸ Temporary files | contents | – |
| `win-old` | Previous Windows installation | **Review** | System updates | win | `C:\Windows.old` | Previous Windows install, needed to roll back; removed after 10 days. | Settings ▸ System ▸ Storage ▸ Temporary files ▸ Previous version of Windows<br>Disk Cleanup ▸ Clean up system files | tool | D |
| `win-update-cache` | Windows Update downloads | **Low** | System updates | win | `%SystemRoot%\SoftwareDistribution\Download` | Downloaded updates; not needed once they are installed. ⚠ Not while updates install or a restart is pending; stop wuauserv and bits first. | Disk Cleanup ▸ Clean up system files ▸ Windows Update Cleanup | contents | 2 |
| `win-setup-temp` | Windows upgrade leftovers | **Review** | System updates | win | `C:\$WINDOWS.~BT`<br>`C:\$WINDOWS.~WS` | Temporary files of a Windows upgrade. | Disk Cleanup ▸ Clean up system files ▸ Temporary Windows installation files | tool | 2 |
| `win-recycle-bin` | Recycle Bin | **Review** | Trash | win | `C:\$Recycle.Bin`<br>dir `$Recycle.Bin` | Files you deleted; emptying removes them for good. | Recycle Bin ▸ Empty Recycle Bin | tool | – |
| `win-crash-dumps` | App crash dumps | **Safe** | Logs | win | `%LOCALAPPDATA%\CrashDumps` | Crash dumps of applications; only for debugging. |  | contents | – |
| `win-minidumps` | System crash dumps | **Low** | Logs | win | `%SystemRoot%\Minidump`<br>files `MEMORY.DMP` in `%SystemRoot%` | Blue-screen dumps; only needed to analyze system crashes. |  | item | – |
| `win-system` | Windows / Program Files | protect (no heuristics inside) | Protected | win | `%SystemRoot%`<br>`C:\Program Files`<br>`C:\Program Files (x86)`<br>`%ProgramData%` | Windows and installed programs; uninstall via Settings ▸ Apps. |  | never | – |
| `win-winsxs` | Component store (WinSxS) | protect (no heuristics inside) | Protected | win | `%SystemRoot%\WinSxS` | Never delete; shrink it with DISM component cleanup instead. | `Dism.exe /online /Cleanup-Image /StartComponentCleanup` | never | D |
| `win-installer-cache` | Windows Installer cache | protect (no heuristics inside) | Protected | win | `%SystemRoot%\Installer`<br>`%ProgramData%\Package Cache` | Needed to repair, update or uninstall programs; never delete. |  | never | – |
| `win-system-files` | Page/hibernation file | protect (self) | Protected | win | files `pagefile.sys` in `C:`<br>files `hiberfil.sys` in `C:`<br>files `swapfile.sys` in `C:` | Managed by Windows; `powercfg /h off` removes hiberfil.sys. | `powercfg /h off` | never | – |

#### Generic heuristics (evaluated after all specific rules)

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `chromium-cache` | Browser / Electron app cache | **Safe** | Cache | all | dir `Cache`, `Code Cache`, `GPUCache`, `DawnCache`, `DawnGraphiteCache`, `DawnWebGPUCache`, `GrShaderCache` (next to Local Storage/ + GPUCache/) | Web cache of a Chromium browser or Electron app (Slack, VS Code…). ⚠ Quit the app first. |  | item | – |
| `service-worker-cache` | Website offline data | **Low** | Cache | all | dir `Service Worker/CacheStorage`<br>dir `Service Worker/ScriptCache` | Offline data of websites and web apps; downloaded again online. ⚠ Quit the app first. |  | item | – |
| `cachedir-tag` | Tagged cache (CACHEDIR.TAG) | **Low** | Cache | all | any dir (containing CACHEDIR.TAG) | Its creator tagged it as a regenerable cache (CACHEDIR.TAG). ⚠ Tags are hints, not guarantees (the spec: "never the final word"). |  | item | DL |

#### Protected locations (never suggested)

| id | Name | Risk | Category | OS | Match | Explanation | Cleanup | Del. | Ev. |
|---|---|---|---|---|---|---|---|---|---|
| `top-level` | Top-level folder | protect (self) | Protected | all | `/`<br>`/Users`<br>`/Applications`<br>`/home`<br>`/Volumes/*`<br>`~`<br>`~/Desktop`<br>`~/Documents`<br>`~/Downloads`<br>`~/Pictures`<br>`~/Movies`<br>`~/Music`<br>`~/Videos`<br>`~/Public`<br>`C:`<br>`C:\Users` | Top-level folder; never suggested as a whole. |  | never | – |
| `mac-library` | ~/Library | protect (no project rules inside) | Protected | mac | `~/Library` | App data and settings; only known caches inside are suggested. |  | never | – |
| `home-dotdirs` | Hidden tool folder | protect (no project rules inside) | Protected | all | `~/.*`<br>`~/AppData` | Tool settings and data; only known caches inside are suggested. |  | never | – |
| `icloud-drive` | iCloud Drive | protect (no heuristics inside) | Protected | mac | `~/Library/Mobile Documents` | iCloud Drive: deleting here deletes on all your devices. |  | never | – |
| `mail-messages` | Mail and Messages | protect (no heuristics inside) | Protected | mac | `~/Library/Mail`<br>`~/Library/Messages` | Your mail and message history. |  | never | – |
| `media-libraries` | Photos / Music library | protect (no heuristics inside) | Protected | mac | dir `*.photoslibrary`, `*.musiclibrary` | Media library; delete items inside its app instead. |  | never | – |
| `app-bundles` | Application bundle | protect (no heuristics inside) | Protected | all | dir `*.app`, `*.framework`, `*.bundle`, `*.plugin`, `*.appex`, `*.xpc`, `*.kext` (containing Contents/ or Info.plist or Versions/) | Inside an app bundle; uninstall the app instead. |  | never | – |
| `test-fixtures` | Test fixtures | protect (no project rules inside) | Protected | all | dir `testData`, `testdata`, `test-data`, `test_data`, `testResources`, `test-resources`, `fixtures`, `__fixtures__` | Test data checked into a repository; artifacts inside may be fixtures. |  | never | – |
| `vcs-data` | Version-control data | protect (no heuristics inside) | Protected | all | dir `.git`, `.hg`, `.svn` | Repository history; not a cache. |  | never | – |
| `homebrew-prefix` | Homebrew prefix | protect (no heuristics inside) | Protected | mac, linux | `/opt/homebrew`<br>`/usr/local/Cellar`<br>`/usr/local/Caskroom`<br>`/usr/local/Homebrew`<br>`/home/linuxbrew/.linuxbrew` | Homebrew-managed software; let brew remove old versions. | `brew cleanup`<br>`brew autoremove` | never | DL |
| `python-tools` | uv / pipx tools | protect (no heuristics inside) | Protected | all | `~/.local/share/uv/tools`<br>`~/.local/share/pipx`<br>`~/.local/pipx` | Command-line tools installed with uv or pipx. | `uv tool uninstall <name>`<br>`pipx uninstall <name>` | never | – |
| `mac-system` | macOS system | protect (no heuristics inside) | Protected | mac | `/System`<br>`/usr`<br>`/bin`<br>`/sbin`<br>`/Library`<br>`/private`<br>`/cores` | Managed by macOS; only specific caches inside are suggested. |  | never | – |
| `linux-system` | Linux system | protect (no heuristics inside) | Protected | linux | `/usr`<br>`/bin`<br>`/sbin`<br>`/lib`<br>`/lib64`<br>`/etc`<br>`/boot`<br>`/opt`<br>`/snap`<br>`/nix`<br>`/srv`<br>`/var/lib`<br>`/proc`<br>`/sys`<br>`/dev`<br>`/run` | Operating-system or package-managed files. |  | never | – |

---

## 5. Safety notes (why the rules look the way they do)

### 5.1 False-positive traps found on this machine

| Trap | What a naive rule does | Guard in the catalog |
|---|---|---|
| `/Applications/Visual Studio Code.app/Contents/Resources/app/node_modules` (sibling `package.json`) | "node_modules, reinstall with npm" → breaks VS Code | `app-bundles` protect (NoHeuristics). Smoke test: 378 bundles in /Applications, 0 candidates. |
| `/opt/homebrew/lib/node_modules/npm/node_modules` (sibling `package.json`) | breaks npm | `homebrew-prefix` protect. Smoke test: /opt/homebrew → 0 candidates. |
| Tracked fixtures in the JetBrains monorepo (`~/dev/ultimate/**/testData/**/{Pods,.build,target}`, `testResources/**/vendor`) | suggests deleting repository content | `test-fixtures` protect (NoProjectRules). 1268 fixture dirs below depth 7 of `~/dev`. |
| `…/wechat-miniprogram/resources/typeDefinitions/node_modules`, `…/jps-bootstrap/execute-on-changed-ant-task/target` | both contain **git-tracked** files and are not ignored (checked with `git ls-files` / `git check-ignore`) | `in_git_worktree` flag + lazy git verification (§5.6) |
| Home dir containing `Assets/` + `ProjectSettings/` → `~/Library` looks like a Unity project | would label the whole user Library "Unity cache" | anchored-knowledge rule (`~/Library` is on anchored paths) + `mac-library` protect |
| `~/.gradle` holds `gradle.properties` / `init.d` (credentials, config); a stray `~/build.gradle` makes it look like a project `.gradle` | deletes user config | anchored-knowledge rule; only `caches`, `wrapper/dists`, `jdks`, `daemon` are candidates |
| Cache folder named like a bundle id, e.g. `~/Library/Caches/com.example.app` | protect heuristic hides a real cache | bundle protect requires `Contents/`, `Versions/` or `Info.plist` |
| In-source CMake builds (`CMakeCache.txt` next to `CMakeLists.txt`) | deletes the source tree | `cmake-build` requires *no* `CMakeLists.txt` and *no* `.git` in the same dir |
| Go `vendor/` (often committed), Yarn `.yarn/releases`, `plugins`, `sdks`, `patches` (committed) | deletes committed files | `php-vendor` requires `composer.json` + `composer.lock` and *no* `go.mod`; only `.yarn/cache` is matched, and as **Review** (Zero-Installs commit it) |
| `~/.cache/huggingface/token`, other tools' login tokens in `~/.cache` | container default deletes a credential | container defaults apply to **directories only**; HF hub/xet are specific rules; the caution on `xdg-cache-entry` mentions tokens |
| `/var/cache/debconf/{config,passwords,templates}.dat` | FHS says /var/cache is regenerable, but this is Debian's configuration database | `debconf` protect |
| `~/Library/Caches/JetBrains/MonorepoBazel` is a **live** Bazel `output_user_root` (bazel server running for 4 h 42 min) | deleting under a running server | IDE system folders are Low with "close the IDE first"; Bazel rules say "run `bazel shutdown` first" |
| `~/.cargo/bin`, uv/pipx tool venvs, `~/.local/share/uv/tools` | `pyvenv.cfg` / `node_modules` heuristics would break installed CLIs | hidden-home zone (NoProjectRules) + `python-tools` protect |
| `~/.sdkman/tmp` | SDKMAN docs: deleting it manually "will break SDKMAN!" | not a candidate (hidden-home zone, no rule); only `sdk flush` would be correct |

### 5.2 Outermost match and containers: decisions
- **Outermost wins** for every `Target`. Descendants get `Inside(rule)`, so the tree can show them dimmed ("inside node_modules"), and the "reclaimable" total is a plain sum over targets, never double-counted.
- **Containers** are roots of independent caches where the root is the wrong unit: mixed risks, apps keep running, and some children are worth a specific command.
  - `~/Library/Caches`, `~/.cache`, `/var/cache`: each child is either a specific rule (`Homebrew` → `brew cleanup --prune=all`, `pip` → `pip cache purge`, …) or gets the container default (Low/Review).
  - `$CARGO_HOME/registry` and `git` are split so that `registry/src` and `git/checkouts` can be **Safe** (re-extracted offline), while `registry/cache`, `index` and `git/db` are **Low** (network).
  - JetBrains caches: one IDE version per child.
- Defaults never apply to files, and never to a child that lies on the path to a more specific anchored rule. Such a child carries the default one level down instead (`~/Library/Caches/Google/Chrome` → App cache, `…/Google/AndroidStudio2025.1` → its own rule).

### 5.3 CACHEDIR.TAG
- Spec (bford.info/cachedir), quoted precisely:
  - the tag is a *regular file*;
  - its first 43 bytes are `Signature: 8a477f597d28d172789f06886806bc55`, case-sensitive, with nothing before it;
  - it marks the *whole* directory tree as cache data that "can be re-generated";
  - tools must treat tags as "hints, never as 'the final word'", because a planted tag could hide data.
- Confirmed writers, by tool source or on this machine:
  - **Cargo**: every `target/` (all 6 here), plus `~/.cargo/registry/` and `~/.cargo/git/`. On macOS Cargo also sets the Time Machine exclusion.
  - **Gradle ≥ 8.1**: `~/.gradle/{caches, wrapper/dists, daemon, jdks}`.
  - **uv**: its cache, and every venv it creates.
  - **virtualenv**: every venv.
  - **pytest** (`.pytest_cache`), **mypy** (`.mypy_cache`), **ruff** (`.ruff_cache`), **tox** (`.tox`).
  - **Hugging Face hub cache**.
  - **fontconfig**: `/opt/homebrew/var/cache/fontconfig`.
  - **ccache**: only in the 16 first-level subdirectories, not in the cache root, because the root holds `ccache.conf`.
- So the generic `cachedir-tag` rule is **Low**, comes last, works in app-data zones but not in NoHeuristics zones, and is preceded by specific rules. Tagged virtualenvs get the venv explanation; ccache gets its root rule.

### 5.4 Measurement caveats that affect "reclaimable"
- **Allocated, not apparent size** (scan-engine already uses `st_blocks*512`). Docker Desktop's `Docker.raw` is a sparse file: Finder shows its maximum (e.g. 64 GB), while the actual allocation is lower (Docker FAQ).
- **Hard links** (pnpm/bun on Linux and Windows; uv venvs on Linux) are counted once per scan. A candidate whose files are also linked from a store frees less. The scanner could add up bytes with `nlink > 1` per subtree as "shared".
- **APFS clones can't be detected with stat.**
  - pnpm and bun default to `clonefile` on macOS, and uv uses clones there too. Here the pnpm `node_modules` files have `nlink = 1`.
  - Xcode's parallel-test clones (`XCTestDevices`): one report has 81 clones measured at 307 GB by `du` that freed about 0 GB.
  - These rules carry `shares_space` (⧉).
- **Simulator runtimes** are disk images mounted under `/Library/Developer/CoreSimulator/Volumes`. Those are separate file systems, so the scanner must not cross them; `du` of the mounted content overstates the image size.
- Space comes back only after **the Trash is emptied**. APFS **local snapshots** (`tmutil listlocalsnapshots /`) can keep deleted blocks alive. Say so in the confirmation dialog.

### 5.5 Permissions (TCC and admin) that matter for these rules
- **Prompt on first access**: `~/Desktop`, `~/Documents`, `~/Downloads` (needed for `downloads-installers`).
- **EPERM without Full Disk Access**: `~/Library/Mail`, `~/Library/Messages`, Safari data, `~/Library/Application Support/MobileSync/Backup` (`ios-backups`), and usually `~/.Trash`. Show "needs Full Disk Access" instead of a size (scan-engine keeps errno for this).
- **App Data prompt (macOS 14+)** when reading other apps' `~/Library/Containers/*` and `Group Containers`. This affects `mac-sandbox-cache`, `mac-mail-downloads` and `docker-desktop-vm`.
- **Admin or root needed**:
  - to delete `/Library/Logs`, `/Library/Developer/CoreSimulator`, Linux `/var/cache/*`, `/var/lib/*` and journal;
  - Windows `C:\Windows\*` and `Windows.old` (owned by TrustedInstaller).

  These are `contents` or `tool` removals.

### 5.6 Git verification (recommended before labelling project artifacts Safe/Low)
The matcher sets `Ctx::in_git_worktree()` when any ancestor contains `.git`. This costs nothing, because the GIT marker bit is part of each listing.

For such candidates, verify lazily, e.g. when the cleanup panel shows them, batched per repository:
1. `git -C <worktree> check-ignore -q -- <rel>`: the folder must be ignored.
2. `git -C <worktree> ls-files -- <rel>`: the output must be empty (no tracked files).

Outcome:
- tracked files → drop the suggestion;
- not ignored → downgrade it to Review.

This is the same rule dua-cli's `clean` applies. Reading the index with the `gix` crate avoids subprocesses. **On macOS, `/usr/bin/git` without the Command Line Tools opens an install dialog.** Check `xcode-select -p`, or use gix.

---

## 6. What exists on this machine (verification log)

Only non-recursive `stat`/`ls -d` was used inside `~/Library`. `du -sh` was run only in `~/.cargo`, `~/.rustup`, `~/.npm`, `~/.gradle`, `~/.m2`, `~/.cache`, `/opt/homebrew` and on `node_modules`/`target` dirs under `~/dev`. Nothing was deleted.

| Location | Size / state | Rule |
|---|---|---|
| `~/dev/air-agent-host/agent-host/target` | **94 G**; `CACHEDIR.TAG` + `.rustc_info.json` | cargo-target (Safe) |
| `~/dev/agent-tester/target`, `alacritty/target`, `air-log-viewer/src-tauri/target`, `elizacp/symposium-acp/target`, `alacritty/alacritty_vte/target` | 22 G, 6.6 G, 4.1 G, 1.9 G, 175 M; all tagged | cargo-target |
| `~/.m2/repository` (+ `wrapper` 19 M) | **34 G** | maven-repo (Low) |
| `~/.gradle` | 12 G: `caches` 8.4 G (versions 8.5 … 9.8.0), `wrapper/dists` 1.0 G (6 distributions), `jdks` 1.0 G, `daemon` 15 M, `native` 68 M. `CACHEDIR.TAG` in caches, wrapper/dists, daemon, jdks | gradle-* |
| `~/.rustup/toolchains` | **10 G**, 8 toolchains (1.84.0 … 1.99.1 + stable); `downloads` 0 B, `tmp` 464 K | rustup-toolchain |
| `~/.cargo/registry` | 1.8 G: `src` 1.4 G, `cache` 290 M, `index` 95 M; `git` 13 M; tagged | cargo-registry-* |
| `~/.npm` | 1.5 G: `_npx` **1.3 G**, `_cacache` 142 M | npm-cache |
| `node_modules` under `~/dev` | 2.2 G (`jcp-air`, pnpm: `.modules.yaml`, files `nlink = 1` = clones), 649 M, 381 M, 141 M, 92 M, 80 M | node-modules(-pnpm) |
| `~/.cache` | 166 M: `uv` (tagged, `.gitignore` = `*`), `zig` 127 M, 10 other tool caches | xdg-cache container |
| `/opt/homebrew` | 5.9 G (Cellar 3.9 G). `brew --cache` = `~/Library/Caches/Homebrew` | homebrew-prefix (protect, `brew cleanup`) |
| Present (stat only) | `~/Library/Caches/{Homebrew, pip, Yarn, pnpm, JetBrains, Google, node-gyp, bazelisk, bazel, typescript, Firefox}`, `~/Library/Logs/{DiagnosticReports, Homebrew, JetBrains}`, `~/Library/pnpm/store` (`pnpm store path` → `…/store/v11`), `~/Library/Android/sdk` (no system-images/ndk), `~/.android/cache`, `~/.konan` (6 Kotlin/Native versions), `~/.sdkman/candidates`, `~/.local/share/containers/podman/machine`, `~/.vscode/extensions`, `~/.Trash`; Docker.app and UTM.app installed | — |
| Absent | Xcode.app (CLT only), so no DerivedData/Archives/DeviceSupport/CoreSimulator; `~/go`, `~/.pub-cache`, `~/.nuget`, conda, `~/.bun`, `~/.yarn`, `~/.ollama`, `~/.cocoapods` | — |

**Matcher smoke test** (`examples/smoke.rs`: read-only, depth ≤ 7, no descent into matches, single thread):

| Root | Time | Result |
|---|---|---|
| `~/dev` | 12.7 s | 7 Cargo targets, 226 node_modules (2 pnpm), 46 Gradle `build`, 21 `.gradle`, 16 `.kotlin`, 3 `.turbo`, 4 Zig, 2 venvs (`.venv` with Python 3.13+'s `.gitignore`, plain `venv`), 5 CEF caches; 1268 fixture dirs and 22 bundles protected |
| `/Applications` | 0.35 s | 378 bundles protected, **0 candidates** |
| `/opt/homebrew` | 0.46 s | **0 candidates** |
| `~/.cargo`, `~/.rustup`, `~/.gradle`, `~/.m2`, `~/.npm`, `~/.cache` | — | exactly the expected anchored rules |

Side effect to know about: running `bazel --version` (Bazelisk) to check the version made Bazelisk try to download Bazel 9.3.1 into `~/Library/Caches/bazelisk`. Only a cache was written; nothing was deleted or modified elsewhere.

---

## 7. Deliberately not included, and open questions

**Not included (risk of a wrong "safe")**:
- `.vercel` and `.netlify`: they hold the project link.
- Generic `dist`, `out`, `build` without a project marker, and `coverage`.
- `.ipynb_checkpoints`: previous notebook versions.
- Unreal `Saved/` (saves and config) and Visual Studio `.vs` (user state).
- `.vagrant` project dirs: they point to VMs.
- `/private/var/folders` and `/Library/Updates`: OS-managed.
- WinSxS and the Windows Installer cache: protect rules, with the DISM hint.
- pyenv/nvm/rbenv versions and conda envs: Review candidates that could be added later, with tool commands.
- Old kernels, Steam libraries, Time Machine snapshots (not folders).
- Delivery Optimization cache: not verified.

**Open questions**:
1. Is Low shown under "Safe to delete" (ux-spec mock) or "Review first"? The catalog supports both.
2. Version-aware hints (keep the default rustup toolchain, the current JetBrains/Android Studio version, DeviceSupport for attached devices) need a tool call (`rustup default`) or a comparison of sibling version dirs. Not done in v1.
3. Git verification: git CLI vs `gix`, and where to run it (UI background task vs scanner).
4. Staleness ("8 mo", ux-spec): the mtime of the marker file vs the mtime of the project dir.
5. **Unverified entries** (Ev. "–") to double-check before release:
   - sandboxed-app caches;
   - XCTestDevices `--set` usage (secondary sources only);
   - uv Windows Python dir;
   - Chromium cache folder names;
   - VM bundle suffixes, Colima/Lima paths, WSL disk paths;
   - Windows Temp/Recycle Bin/crash dumps.

---

## 8. Sources

**Specs**
- Cache Directory Tagging Specification: https://bford.info/cachedir/
- XDG Base Directory: https://specifications.freedesktop.org/basedir/latest/
- FHS 3.0 /var/cache: https://refspecs.linuxfoundation.org/FHS_3.0/fhs/ch05s05.html
- freedesktop Trash spec: https://specifications.freedesktop.org/trash/latest/

**Rust**
- Cargo home: https://doc.rust-lang.org/cargo/guide/cargo-home.html
- Cargo config `[cache]` / `build.target-dir`: https://doc.rust-lang.org/cargo/reference/config.html
- Rust 1.88 (automatic cache cleaning): https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/
- Cargo source, `exclude_from_backups` (CACHEDIR.TAG + Time Machine): https://github.com/rust-lang/cargo/blob/master/crates/cargo-util/src/paths.rs
- Local help: `cargo clean --help`, `rustup toolchain --help`

**JavaScript**
- npm cache: local `npm help cache` / `npm help folders` (npm 11.6.2); https://docs.npmjs.com/cli/v11/commands/npm-cache
- Yarn: https://yarnpkg.com/configuration/yarnrc, https://yarnpkg.com/cli/cache/clean, https://classic.yarnpkg.com/en/docs/cli/cache, https://github.com/actions/cache/pull/70
- pnpm: https://pnpm.io/settings/store, https://github.com/pnpm/pnpm.io/blob/main/docs/settings/node-modules.md, https://github.com/pnpm/pnpm.io/blob/main/docs/settings/other.md; local `pnpm store --help`
- Bun: https://bun.com/docs/install/cache, https://bun.com/docs/pm/cli/pm
- Next.js: https://nextjs.org/docs/app/guides/ci-build-caching, https://nextjs.org/docs/pages/api-reference/config/next-config-js/distDir
- Angular: https://angular.dev/cli/cache
- Turborepo 2.0: https://turbo.build/blog/turbo-2-0, https://turbo.build/repo/docs/reference/configuration
- Nuxt: https://nuxt.com/docs/4.x/api/commands/cleanup
- Playwright: https://playwright.dev/docs/browsers
- Cypress: https://docs.cypress.io/app/references/advanced-installation
- Puppeteer: https://pptr.dev/guides/configuration
- @electron/get: https://github.com/electron/get
- Deno: https://docs.deno.com/runtime/reference/cli/clean/

**Python and ML**
- pip: https://pip.pypa.io/en/stable/topics/caching/
- uv: https://docs.astral.sh/uv/concepts/cache/; source https://github.com/astral-sh/uv/blob/main/crates/uv-virtualenv/src/virtualenv.rs
- venv: https://docs.python.org/3/library/venv.html
- Tool sources that write CACHEDIR.TAG:
  - virtualenv: https://github.com/pypa/virtualenv/blob/main/src/virtualenv/create/creator.py
  - pytest: https://github.com/pytest-dev/pytest/blob/main/src/_pytest/cacheprovider.py
  - mypy: https://github.com/python/mypy/blob/master/mypy/build.py
  - ruff: https://github.com/astral-sh/ruff/blob/main/crates/ruff/src/cache.rs
  - tox: https://github.com/tox-dev/tox/blob/main/src/tox/util/path.py
- Poetry: https://python-poetry.org/docs/configuration/
- conda clean: https://docs.conda.io/projects/conda/en/latest/commands/clean.html
- Hugging Face cache: https://huggingface.co/docs/huggingface_hub/guides/manage-cache
- Ollama paths (secondary): https://github.com/ollama/ollama/issues/733

**JVM**
- Gradle directory layout (cleanup, CACHEDIR.TAG): https://docs.gradle.org/current/userguide/directory_layout.html
- Maven repositories: https://maven.apache.org/guides/introduction/introduction-to-repositories.html
- Kotlin 2.0 `.kotlin` dir: https://kotlinlang.org/docs/whatsnew20.html
- Kotlin/Native `~/.konan`: https://kotlinlang.org/docs/native-libraries.html; https://slack-chats.kotlinlang.org/t/27080030 (secondary)
- JetBrains directories: https://www.jetbrains.com/help/idea/directories-used-by-the-ide-to-store-settings-caches-plugins-and-logs.html
- Coursier: https://get-coursier.io/docs/cache

**Go**
- https://pkg.go.dev/cmd/go
- https://go.dev/ref/mod

**Mobile**
- Android env vars, AVD location: https://developer.android.com/tools/variables
- Dart pub cache: https://dart.dev/tools/pub/cmd/pub-cache, https://dart.dev/tools/pub/glossary

**Apple**
- Debug information / retain archives: https://developer.apple.com/documentation/xcode/building-your-app-to-include-debugging-information
- Missing dSYM: https://developer.apple.com/documentation/xcode/locating-a-missing-debug-symbol-file
- Xcode components: https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components
- File System Programming Guide (Caches): https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/MacOSXDirectories/MacOSXDirectories.html
- iOS backups: https://support.apple.com/en-us/108809
- Mail settings: https://support.apple.com/guide/mail/change-general-settings-cpmlprefgen/mac
- DerivedData (forum): https://developer.apple.com/forums/thread/81265
- Simulator runtimes: https://furbo.org/2022/11/09/managing-xcode-downloads/, https://github.com/Fullex26/MacCleaner/issues/112 and /107
- XCTestDevices: https://cleanmydev.com/blog/xctestdevices-cleanup/
- IPSW location: https://osxdaily.com/2010/12/08/ipsw-file-location-mac-and-windows/
- CocoaPods commands: https://guides.cocoapods.org/terminal/commands.html

**Homebrew**
- Local `brew cleanup --help` and `brew.1` manpage (`HOMEBREW_CACHE` defaults, 120-day rule); https://docs.brew.sh/Manpage

**Containers**
- Docker Desktop for Mac FAQ (Docker.raw): https://docs.docker.com/desktop/troubleshoot-and-support/faqs/macfaqs/
- Docker pruning: https://docs.docker.com/engine/manage-resources/pruning/
- Docker WSL disk (secondary): https://www.dash0.com/faq/where-are-docker-images-stored-on-disk

**Build tools**
- NuGet folders: https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders
- Bazel output layout: https://bazel.build/remote/output-directories
- ccache: https://ccache.dev/manual/latest.html; source `src/ccache/storage/local/localstorage.cpp`
- sccache: https://github.com/mozilla/sccache/blob/main/docs/Local.md
- Terraform init: https://developer.hashicorp.com/terraform/cli/init
- Stack: https://docs.haskellstack.org/en/stable/topics/stack_work/
- Unity: https://docs.unity3d.com/6000.5/Documentation/Manual/default-directories.html
- Composer: https://getcomposer.org/doc/06-config.md
- SDKMAN: https://sdkman.io/usage

**Linux**
- debconf.conf: https://manpages.debian.org/testing/debconf-doc/debconf.conf.5.en.html
- journalctl: https://www.freedesktop.org/software/systemd/man/latest/journalctl.html
- Flatpak: https://docs.flatpak.org/en/latest/using-flatpak.html, https://docs.flatpak.org/en/latest/conventions.html
- snap retain (secondary): https://www.omgubuntu.co.uk/2026/01/remove-snap-revisions-ubuntu-disk-space

**Windows**
- Windows.old: https://support.microsoft.com/en-us/windows/deployment/install-upgrade/delete-your-previous-version-of-windows
- WinSxS: https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/clean-up-the-winsxs-folder
- SoftwareDistribution (Q&A, secondary): https://learn.microsoft.com/en-us/answers/questions/785704/c-windowssoftwaredistributiondownload-taking-up-to
