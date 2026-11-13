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
