# dir-stat v1: design

Status: revised after review by five critics (fs-safety, iced, scan-perf, ux, implementability). Every finding and what was done with it is in [`review-log.md`](review-log.md). Date: 2026-11-13.

dir-stat is a Rust + iced 0.14 desktop app that shows where disk space goes and which large folders are safe to delete. This document is the blueprint for v1. It decides what the research left open and points to the research for details. Research reports are in `docs/research/` (index: `docs/research/README.md`). Paths inside them that start with `/tmp/dirstat-work/research/` now live in `docs/research/`, and paths that start with `/tmp/dirstat-work/probes/` now live in `docs/research/probes/`.

Short names for the reports used in citations below:

| Short name | File |
|---|---|
| **API** | `docs/research/iced-api.md` |
| **SCAN** | `docs/research/scan-engine.md` |
| **FS** | `docs/research/fs-semantics.md` |
| **UX** | `docs/research/ux-spec.md` |
| **CAT** | `docs/research/cleanup-catalog.md` |

Reference code to start from (copy it, don't depend on it):
- `docs/research/probes/scan-bench/src/{scanner,queue,listing,mounts,model,tests}.rs`: the scanner and its tree model.
- `docs/research/cleanup-catalog-proto/src/{model,catalog,engine}.rs`: the catalog and its matcher.
- `docs/research/probes/iced-probe/src/{main,scan,rows,style,platform,treemap}.rs`: iced patterns that compile.

---

## 0. Summary of the main decisions

1. **Two crates in one workspace.** `dirstat-core` holds the scanner, tree, catalog, git verification, platform code, delete guards and trash executor, and has no iced dependency. `dir-stat` is the iced app. The split keeps the engine testable without building iced (382 crates) and enforces the "engine knows nothing about the UI" seam (SCAN §7.1).
2. **The scanner is a port of the scan-bench reference implementation**, with additive changes. It uses `min(cores, 8)` std worker threads, a work-stealing queue, one `DirListing` per directory (parents before children, each exactly once), and a batcher that sends at most 20 updates per second with coalescing backpressure (SCAN §7). Directory listing uses the std backend (`read_dir` + `DirEntry::metadata`). Additions:
   - opening the root (canonicalize, stat, mount snapshot, root context) happens on the scan's own thread, never on the UI thread;
   - catalog classification runs in the worker;
   - inode numbers, the newest mtime, hard-linked bytes and cloud-only items are reported;
   - iCloud dataless folders are not read;
   - mount points are skipped *before* they are stat'ed;
   - a panicking job is caught, so a scan always terminates.
3. **The UI thread owns the tree. Nothing is shared and there are no locks.** The tree is a chunked arena of directory nodes indexed by a dense `NodeId(u32)`. Files of 1 MiB or more are kept per directory and smaller files only as a total. Applying batches costs about 0.15% of scan time (SCAN §2.7).
4. **Catalog: a port of the prototype's 178 rules and its streaming matcher.** Each directory is classified once, in the worker, right after it is listed, and the outermost match wins (CAT §2). **Risk maps to UI sections as follows (user decision D1): Safe → "Safe to delete"; Low and Review → "Review first"; Protect is never shown.** Candidates inside a git work tree are checked with git before they get a section. One pure function, `catalog::display_section(risk, git)`, decides the section for the cleanup pane, the tree pills and the trash dialog alike.
5. **iced app: one `App` state, one `Message` enum, and plain `update`/`view` functions.**
   - Scan updates arrive through an abortable `Task::run` over a futures channel, tagged with a scan id.
   - A single 50 ms tick subscription runs only while something is live.
   - Row virtualization is manual, with 28 px rows. The row list is rebuilt synchronously for user actions and throttled only for scan-driven changes; every rebuild keeps the scroll anchor.
   - Sorting is calm: values update live, order changes on a 1.5 s tick that waits while the user interacts, with hysteresis buckets (UX §5.4).
6. **Delete means Move to Trash only.** Every trash goes through a confirmation dialog. Protected paths, folders containing another volume, and read-only volumes are refused. Identity (`dev`, `ino`, file type) and the mount table are re-checked right before the move. The work runs on a blocking thread through an injectable `Trasher`: the `trash` crate with `DeleteMethod::NsFileManager` on macOS, the `trash` crate on Windows, and a small rename-only freedesktop implementation on Linux that can never fall back to copy-and-delete (D14).
7. **Out of the v1 core:** the treemap is an optional last milestone (M12). Vector icons, multi-select, bulk trash, the `getattrlistbulk` backend, clone and hard-link badges, and "Put Back" are all deferred.

---

## 1. Scope

### 1.1 In v1

Every **[MUST]** in UX §15 is in v1, plus the cheap **[SHOULD]** items marked ✓ below.

| Area | v1 contents | Cheap SHOULDs included |
|---|---|---|
| Start screen | Home card, system disk card, mounted volume cards with capacity meters, Choose Folder… (rfd), `dir-stat <path>` CLI, `--help`/`--version`, invalid-path banner, keyboard on cards (UX §4) | ✓ drag and drop, ✓ Recent (last 5), ✓ macOS privacy footnote (with an FDA heuristic, FS §5.3) |
| Scanning | Live tree from the first listing. Activity field: "Opening…", items, bytes, rate, elapsed, current path, Stop. Progress is determinate for volume roots and indeterminate otherwise. Stop keeps results and is final at once. Per-row scan states. Calm sorting. Trash only for completed subtrees (UX §5) | ✓ "4 of 23 folders done", ✓ ETA, ✓ Unaccounted space row |
| Tree | Columns Name / Size / % of parent / Items; size-descending default; header sort; single selection; full keyboard map; zoom, breadcrumbs, history; listed files plus an aggregate row with on-demand listing of small files; selection bar; toasts; responsive columns (UX §6) | ✓ cleanup pills, ✓ package badge, ✓ mouse back/forward, ✓ Scan This Volume |
| Cleanup pane | Scoped to the zoom root. Stat block. Safe and Review sections, groups ranked by size with a reason, click to reveal in the tree, empty and scanning states, disclaimer (UX §7) | ✓ age column, ✓ "how to clean" hints, ✓ fold for smaller suggestions, ✓ "Re-downloadable" tag on Low groups |
| Delete | Move to Trash with confirmation, protected-path refusal, struck-through row kept in place, error handling (UX §9) | ✓ warning lines (packages, Review-first items, protected zones, cloud-only content, unreadable content, shared bytes, very large items) |
| Errors | Badges for unreadable folders (the folder itself vs. something inside), count in the activity field, macOS Full Disk Access info bar, state for an unreadable scan root (UX §10) | ✓ error list popover with Copy |
| Design | Light/dark that follows the OS, bundled Inter (400/500/600), tokens, type scale, 28 px rows, decimal units, formatting rules (UX §11–12) | ✓ persistence of recents and side-pane state |
| Rescan | Rescan of the selected folder, with an atomic swap on completion; Rescan everything (UX §6.6) | |

### 1.2 Deferred

- **Optional milestone M12 (after v1 is otherwise complete):** the treemap "Map" tab (UX §8). Until then the side pane shows only the Cleanup pane, with no tab bar: the Safe total sits in the pane header, and ⌘2 (`PaneMap`) does nothing.
- **v1.1:**
  - `getattrlistbulk` backend for macOS (SCAN §4, 1.1–1.7× faster);
  - multi-select, plus cleanup checkboxes with bulk trash (and with it, trashing a whole multi-match group);
  - context menu;
  - type-to-select;
  - ⌥→ to expand the largest path;
  - cleanup hover buttons;
  - vector icons (until then, the Unicode fallback from UX §11.7), and with them icon-only selection-bar buttons (D16);
  - draggable pane divider;
  - shortcut overlay.
- **LATER** (as in UX §15 and FS "Later" items):
  - search;
  - apparent-size toggle;
  - Quick Look;
  - permanent delete;
  - Put Back / undo;
  - clone, sparse and hard-link badges;
  - deterministic lowest-common-ancestor hard-link attribution (SCAN §7.9);
  - Time Machine snapshot hint after deletes;
  - "skip privacy-protected folders" mode;
  - tool-path enrichment (`brew --cache`, …; CAT §2.4);
  - a "volume didn't respond" (timeout) error state (D15);
  - localization;
  - native menu bar;
  - accessibility.

### 1.3 Deviations from the research

| # | Research says | v1 does | Why |
|---|---|---|---|
| D1 | CAT §1: Safe and Low map to "Safe to delete" | **Low maps to "Review first".** Only Safe goes under "Safe to delete" | Product decision by the user (2026-11-13). It also changes: the stat block ("likely reclaimable" = Safe only), the pill dot color, the dialog's default button, the Review-first section subtitle, the per-risk dialog wording and a "Re-downloadable" tag on Low groups (§5.3) |
| D2 | FS §2.10 / §10: when scanning `/`, enter `/System/Volumes/Data` in "data remainder" mode and skip firmlink targets | Skip `/System/Volumes/Data` like any mount point (SCAN §5.3). For roots under `/System/Volumes/Data`, every mount `m` is also registered as `/System/Volumes/Data` + `m`, so firmlinked mounts are still skipped before stat (§4.7). Scan This Volume is not offered on the Data-volume row | No firmlink bookkeeping is needed. The remainder (`.Spotlight-V100`, `.fseventsd`, …) is almost entirely unreadable for a normal app, and the Unaccounted row explains the gap. Registering every alias is over-inclusive but harmless, and avoids parsing `/usr/share/firmlinks` |
| D3 | FS §8.1: volume used bytes via `getattrlist(ATTR_VOL_SPACEUSED)`; UX §5.2: byte-based progress | Progress = **items scanned / inodes used** from `statfs`/`statvfs` (`f_files − f_ffree`, which is per volume even on APFS, FS §8.1), falling back to bytes. Unaccounted = container "used" (`f_blocks − f_bavail`) − scanned | No `getattrlist` FFI in v1. Item progress is smoother (FS §8.2). Container "used" matches the start screen and Finder, as UX §5.6 asks |
| D4 | FS §7.4: ancestor `(dev, ino)` loop guard per job | No general loop guard. On HFS+ roots (`fs_type == "hfs"`) a `(dev, ino)` visited set for directories turns repeat directory hard links into `Skip::LinkedDir`. `Backups.backupdb` at a volume root and `.zfs` at a ZFS dataset root are `Skip::Excluded` (FS §2.6, §11) | Symlinks are never followed and every mount point is a boundary. The remaining loop sources (buggy network filesystems) end at PATH_MAX with `ENAMETOOLONG`, because the std backend walks by path (FS §6.4). HFS+ directory links are not loops but multiply counts, so they need the visited set |
| D5 | FS §1: badges for hard links, clones, sparse files, compressed files | No badges in v1. Dataless cloud folders get a row state. The scanner reports per-listing counters (hard-linked bytes, dataless files) that feed the trash dialog's warnings and the toast wording | Not in the UX MUST list. Counters are cheap; per-file flags in the protocol are not |
| D6 | UX §4/§10: legacy deep link `com.apple.preference.security?Privacy_AllFiles` | `x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles` | Apple documents it for macOS 13+ (FS §5.3). The minimum macOS for v1 is 13 |
| D7 | UX §8: treemap tab [SHOULD] | Optional milestone M12 | Keeps v1's critical path short. The architecture leaves a `SidePane::Map` slot |
| D8 | CAT §1: `tool` removal = never offer Move to Trash | Cleanup pane: no trash path, only the command (with Copy); ⌘⌫ there shows a toast pointing to the command. Tree: Move to Trash stays possible, but the dialog shows a warning with the recommended command and Cancel is the default button | The tree is the general-purpose tool and shouldn't hide actions. The cleanup pane must not invite them |
| D9 | SCAN §7.9: lowest-common-ancestor attribution of hard links (LATER) | The first sighting owns a hard-linked inode's bytes. The full scan's owner map is kept, and a subtree rescan counts 0 bytes for inodes owned outside the target, so rescans don't double-count (pnpm stores, §4.6) | Exact across rescans at ~15 MB for 479k links; LCA attribution stays LATER |
| D10 | CAT §5.6 recommends git verification "before labelling" | Candidates inside a git work tree have **no section until verified**: not in the pane, no pill, neutral dialog reason. Not ignored → Review first. Tracked files → hidden everywhere. Git unavailable (or the work tree is dataless) → Review first, with the reason "Couldn't check with git" | Items never jump between sections: they appear once, already verified |
| D11 | UX §4: Linux network volumes are shown with a "Network" tag | Shown, but without a capacity meter. Network mounts are never `statvfs`'ed | A dead NFS server blocks `statvfs` in uninterruptible sleep (FS §3.7) |
| D12 | UX §4: Windows shows "Local Disk (C:)" with a meter | Windows (nice-to-have): Home card plus a `%SystemDrive%` card without a meter. Size = logical size, and the column is labelled "Size". The trash identity re-check is weaker (file type, plus length and mtime for files; std has no stable file index) | No `windows-sys` dependency in v1 (FS §4.1 option a) |
| D13 | CAT `contents` removal | Implemented as "trash every direct child that is unchanged since the scan, keep the folder" | Same effect, and it goes through the same trash primitive and guards |
| D14 | API §g: `trash` crate on every OS | Linux uses an own ~150-line freedesktop trash (rename only, never copy; §7.5). macOS and Windows use the crate | `trash` 5.2.9 falls back to the home Trash with a recursive cross-device copy + `remove_dir_all` (`freedesktop.rs` 36-66, 583-602), which can fill the home disk and recurse into nested mounts |
| D15 | UX §10.1/§13: "timeout: the volume didn't respond" state | Deferred. `ErrorClass` has no `Timeout` | Mount points are never entered, so a hang can only happen when the root itself is on a dead network volume; "Opening…" plus Stop is the remedy (§4.5) |
| D16 | UX §6.1 [MUST]: selection-bar buttons collapse to icon-only buttons when narrow | They collapse to short text labels ("Reveal", "Copy", "Rescan", "Trash…") with tooltips | v1 has no vector icons, and the Unicode fallback set has no glyphs for these actions |

---

## 2. Workspace, crates and dependencies

### 2.1 Layout

```
dir-stat/
  Cargo.toml                    # [workspace], shared [workspace.package] + profiles
  crates/
    core/                       # package dirstat-core (lib), no iced
      src/
        lib.rs
        scan/mod.rs             # start(), ScanRequest, Options, ScanHandle, events (Update, Batch, DirListing…)
        scan/queue.rs           # work-stealing queue (copy of scan-bench queue.rs, stealing variant only)
        scan/listing.rs         # list_dir(): std backend only (copy, drop At/Bulk)
        scan/worker.rs          # list_one(): stats, aggregation, mount/skip checks, catalog classification
        scan/batcher.rs         # opening the root, coalescing batcher, UpdateSink trait
        tree/mod.rs             # Tree, Node, NodeId, apply(), completion, queries
        tree/arena.rs           # chunked arena
        tree/splice.rs          # rescan splice, compaction, trash accounting
        tree/testing.rs         # TreeBuilder (feature "test-util")
        catalog/mod.rs          # pub use; Section, display_section; RootContext
        catalog/model.rs        # copy of proto model.rs (Rule, Matcher, Cond, Risk, markers m::*)
        catalog/table.rs        # copy of proto catalog.rs (178 rules), unchanged content
        catalog/engine.rs       # copy of proto engine.rs (+ small API additions, §5.5)
        platform/mod.rs         # cfg dispatch, shared types
        platform/{macos,linux,windows,other}.rs
        volumes.rs              # start-screen volume list + scan estimates
        safety.rs               # delete guards (path rules + live checks; per-OS lists)
        trash/mod.rs            # plan items, execute(), Trasher trait
        trash/freedesktop.rs    # Linux rename-only trash (D14)
        git.rs                  # git verification of cleanup candidates
        smallfiles.rs           # on-demand listing of one directory's files
      tests/                    # fixture-based integration tests (scanner, du parity, catalog, git, trash)
      examples/scan.rs          # CLI: scan a dir, print totals + top children (for du comparison)
    app/                        # package dir-stat (bin "dir-stat")
      assets/fonts/             # Inter-Regular.ttf, Inter-Medium.ttf, Inter-SemiBold.ttf, OFL.txt
      src/
        main.rs                 # CLI parsing, iced::application(..) setup
        app.rs                  # App, Env, Message, update() dispatch, subscription(), view() dispatch
        start.rs                # start screen state + view
        results/mod.rs          # Results state, scan wiring, update handlers for scan/tree
        results/scan_task.rs    # UiSink newtype, start/stop/rescan, draining scans
        results/migrate.rs      # SavedView: capture/restore NodeId-keyed state by path (splice, replace)
        results/rows.rs         # flattening, RowKey, virtualized view, anchoring
        results/order.rs        # calm sorting + ReorderClock (pure, unit-tested)
        results/columns.rs      # row anatomy, header, responsive column rules
        results/activity.rs     # activity field model + view (rate EMA, ETA, progress, sweep)
        results/nav.rs          # zoom, breadcrumbs, history
        results/selection_bar.rs
        results/candidates.rs   # git job scheduling, status map, display sections for the app
        results/cleanup.rs      # cleanup pane model (pure build_cleanup) + view
        results/trash.rs        # trash plan, dialog, outcomes
        results/errors.rs       # FDA bar, errors popover, root-unreadable state
        widgets.rs              # modal, toast, pill, share bar, tooltip helpers, truncation helpers
        theme.rs                # Tokens (light/dark), style fns, application style
        fonts.rs                # include_bytes!, Font constants
        keys.rs                 # keyboard::Event -> KeyAction (pure, unit-tested)
        format.rs               # sizes, counts, percent, time (pure, unit-tested)
        prefs.rs                # PrefsStore trait + JSON implementation
        script.rs               # DIRSTAT_SCRIPT input hook (feature "screenshot")
  scripts/bundle-macos.sh       # builds dir-stat.app with Info.plist, codesigns (M11)
  docs/research/…, docs/design/…
```

There are no other crates. The work-stealing queue, mount parsing and catalog are small enough to stay modules.

### 2.2 Dependencies

`[workspace.package]`: `edition = "2024"`, `rust-version = "1.88"` (iced 0.14's MSRV; API §j). `DirEntry::metadata()` is `fstatat`-relative only from Rust 1.9x onward (SCAN §4), so build with a current stable (1.93.1 verified).

`dirstat-core`:

| crate | version | why |
|---|---|---|
| `libc` | `0.2` (0.2.191 verified) | `getmntinfo`, `statfs`/`statvfs`, `setrlimit`, `setiopolicy_np` (declared locally), errno constants. `[target.'cfg(unix)'.dependencies]` |
| `trash` | `5` (5.2.9) | Move to Trash on macOS (`trash::macos::{DeleteMethod, TrashContextExtMacos}`) and Windows. `[target.'cfg(any(target_os = "macos", windows))'.dependencies]` (D14) |
| `dirs` | `6` | home, config and data directories |
| `tempfile` | `3` (dev) | fixture trees in tests |

Feature of `dirstat-core`: `test-util` exposes `tree::testing::TreeBuilder` and `ScanHandle::inert()` to the app crate's tests (§10.3). It is enabled only through the app's `[dev-dependencies]`.

`dir-stat` (app):

| crate | version | features | why |
|---|---|---|---|
| `iced` | `0.14` | `["tokio"]` (+ `"canvas"` only in M12) | `time::every` and the tokio executor need `tokio` (API §0). Default features stay on |
| `tokio` | `1` | `["rt"]` | `spawn_blocking` inside `Task::perform` (API §g) |
| `rfd` | `0.17` | default | folder picker (API §g) |
| `opener` | `0.9` | `["reveal"]` | Reveal in Finder / file manager; opening the settings URL |
| `serde` | `1` | `["derive"]` | prefs |
| `serde_json` | `1` | | prefs |
| `png` | `0.18` | optional | only with the `screenshot` feature |
| `dirstat-core` | path | (`test-util` in dev-dependencies) | |

Feature flags of the `dir-stat` crate:
- `debug`: turns on `iced/debug` (devtools). It requires `Message: Debug`, which we derive anyway.
- `screenshot`: the visual-verification harness. This is the only way an implementation pass can visually verify UI work (the iced probe did the same; API §1, `src/main.rs`).
  - `DIRSTAT_SCREENSHOT=<file.png>` (with optional `DIRSTAT_SCREENSHOT_AFTER_MS`, default 1500): take a `window::screenshot`, write a PNG and exit. A pending screenshot keeps the tick subscription alive (§6.5).
  - `ICED_THEME=Light|Dark` forces the theme for light and dark shots (API §c); without it the app follows the OS.
  - `DIRSTAT_SCRIPT="wait:1500;key:Down;key:Trash;wait:300;shot:dialog.png;…"`: a scripted sequence of `Message`s (keys by `KeyAction` name, `path:<p>` for `ScanPath`, `click:<RowKey-ish>` for a few named targets), with `shot:` steps writing screenshots. It lets an implementation pass capture dialogs, popovers, the FDA bar and the reveal flash.

Profiles in the workspace root:
- `[profile.dev] opt-level = 1` and `[profile.dev.package."*"] opt-level = 2`. This keeps debug builds of the GUI and scanner usable (API §j).
- `[profile.release] lto = "thin"`.

---

## 3. Core data model

### 3.1 Scanner events (the protocol)

This is scan-bench `scanner.rs` (SCAN §7.2) with the additions marked `// NEW`:

```rust
// dirstat-core::scan
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NodeId(pub u32);
impl NodeId { pub const ROOT: NodeId = NodeId(0); pub const NONE: NodeId = NodeId(u32::MAX); }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage { pub alloc: u64, pub apparent: u64, pub count: u64 }   // non-directory entries

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMatch { Target(catalog::RuleIdx), Protected(catalog::RuleIdx) }   // NEW

#[derive(Clone, Debug)]
pub struct FileEntry {                     // scanner wire type: no UI state
    pub name: Box<OsStr>,
    pub alloc: u64,
    pub apparent: u64,
    pub mtime: i64,                        // NEW: seconds since epoch (selection bar "modified")
    pub ino: u64,                          // NEW: identity re-check before trashing
    pub nlink: u32,                        // NEW: > 1 → "may free less" (dialog)
    pub matched: Option<FileMatch>,        // NEW: catalog file rule (installers in ~/Downloads, …)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    OtherFilesystem,   // st_dev differs from the root's
    MountPoint,        // mount-table entry (or its /System/Volumes/Data alias) below the root
    Excluded,          // NEW: macOS /.vol, /.nofollow, /.resolve when the root is "/"; Backups.backupdb, .zfs (D4)
    CloudOnly,         // NEW: macOS SF_DATALESS directory (never read, FS §1.11)
    LinkedDir,         // NEW: HFS+ directory hard link already counted elsewhere (D4)
}

#[derive(Clone, Debug)]
pub struct Subdir { pub name: Box<OsStr>, pub ino: u64 /* NEW */, pub skip: Option<Skip> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {              // NEW: classified once, in core (FS §0 rule 7)
    PrivacyBlocked,                // EPERM on macOS: TCC / SIP / sandbox -> "needs Full Disk Access"
    PermissionDenied,              // EACCES (and EPERM off macOS)
    CloudNotDownloaded,            // EDEADLK (materialization prevented)
    PathTooLong,                   // ENAMETOOLONG
    Internal,                      // a panic inside the worker was caught (§4.2)
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanError { pub class: ErrorClass, pub errno: Option<i32> }

#[derive(Clone, Debug)]
pub struct DirListing {
    pub id: NodeId,
    pub dir_alloc: u64,
    pub files: Usage,
    pub linked_alloc: u64,                 // NEW: alloc of counted files with nlink > 1
    pub cloud_files: u32,                  // NEW: dataless (cloud-only) files among the direct entries
    pub large_files: Vec<FileEntry>,       // files with max(alloc, apparent) >= 1 MiB
    pub first_child: NodeId,               // subdirs[i] has id first_child + i
    pub subdirs: Vec<Subdir>,
    pub error: Option<ScanError>,          // the directory itself could not be (fully) read
    pub vanished: bool,                    // NEW: the directory disappeared before it was read (no error)
    pub entry_errors: u32,
    pub entry_error_samples: Vec<(Box<OsStr>, ScanError)>,   // NEW: first 4 unstat-able entries
    pub newest_mtime: i64,                 // NEW: max mtime of direct entries (i64::MIN if empty)
    pub markers: u64,                      // NEW: OR of catalog marker bits of the entries
    pub verdict: catalog::Verdict,         // NEW: classification of this directory
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub dirs_listed: u64, pub files: u64, pub alloc: u64,
    pub errors: u64,                       // unreadable dirs + unstat-able entries (vanished excluded)
    pub vanished: u64,                     // NEW: ENOENT/ENOTDIR/ELOOP, "changed during scan"
    pub outstanding: u64, pub elapsed: Duration,
    pub current: Option<(NodeId, Duration)>, // NEW: in-flight job shown in the activity field (§4.4)
}
#[derive(Clone, Debug)]
pub struct RootInfo {                      // NEW
    pub root: PathBuf,                     // canonical
    pub dev: u64, pub ino: u64,
    pub fs_type: String,
    pub context: catalog::RootContext,
}
#[derive(Clone, Debug)]
pub struct StartError { pub kind: io::ErrorKind, pub message: String }   // NEW
#[derive(Clone, Debug)]
pub struct Batch { pub listings: Vec<DirListing>, pub progress: Progress }
#[derive(Clone, Debug)]
pub struct Summary {
    pub progress: Progress, pub cancelled: bool, pub hardlinks_deduped: u64,
    pub internal_errors: u32,                       // NEW: caught panics
    pub links: Arc<LinkOwners>,                     // NEW: (dev, ino) -> owning NodeId, nlink > 1 only (D9)
}
#[derive(Clone, Debug)]
pub enum Update {
    Started(RootInfo),       // NEW: always first on success
    Failed(StartError),      // NEW: the root could not be opened; nothing follows
    Batch(Batch),
    Finished(Summary),       // always last after Started
}
```

All event types derive `Clone` because iced messages must be `Clone` (API §1 gotcha 15). They are never actually cloned on the hot path; `Summary.links` is behind an `Arc` for that reason.

### 3.2 The tree

The UI keeps one `Tree` per scan. It is the scan-bench `model.rs` extended with the fields the UI needs. Only directories are nodes. Large files live inside their parent node; small files exist only as `files` totals.

The tree is a deep module: `Node` fields are private to `dirstat-core` and read through accessors; nothing outside the tree can change sizes, counts or structure. The app changes only UI flags, through `set_flag`.

```rust
// dirstat-core::tree
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState { Pending, Scanning, Done, Skipped(Skip), Removed /* tombstone after a splice */ }

#[derive(Clone, Copy, Debug, Default)]
pub struct Totals {
    pub alloc: u64,        // dir_alloc + files.alloc of every listed dir in the subtree (hard links once)
    pub apparent: u64,
    pub linked_alloc: u64, // part of alloc in files with nlink > 1 (may not be freed by trashing)
    pub items: u64,        // files + directories strictly inside (the "Items" column)
    pub error_dirs: u32,   // unreadable directories strictly inside (the outline-triangle badge)
    pub boundaries: u32,   // MountPoint / OtherFilesystem skips strictly inside (G1 refuses)
    pub cloud_items: u32,  // CloudOnly dirs + dataless files strictly inside (dialog warning)
}

pub struct LargeFile {     // tree-side copy of FileEntry, with UI flags
    pub name: Box<OsStr>, pub alloc: u64, pub apparent: u64, pub mtime: i64, pub ino: u64,
    pub nlink: u32, pub matched: Option<FileMatch>, pub flags: FileFlags,   // TRASHED, TRASH_PENDING
}

pub struct Node {          // all fields pub(crate); accessors below
    parent: NodeId,            // NodeId::NONE for the root
    first_child: NodeId,
    child_count: u32,
    unfinished: u32,           // children not Done yet (completion tracking, SCAN §7.8)
    name: Box<OsStr>,          // exact bytes from readdir, never normalized (FS §6.1–6.2)
    ino: u64,                  // identity for the trash re-check (dev = the scan root's dev)
    markers: u64,              // catalog markers: rescan context + package detection + git root
    dir_alloc: u64,
    files: Usage,              // direct non-directory entries
    total: Totals,             // includes own; final once state == Done
    newest_mtime: i64,         // max over the subtree, propagated on apply
    large_files: Box<[LargeFile]>, // empty slice = no allocation
    entry_errors: u32,
    error: Option<ScanError>,
    verdict: catalog::Verdict, // 4 bytes
    state: NodeState,
    flags: NodeFlags,          // EXPANDED, TRASHED, TRASH_PENDING, RESCANNING, OUTDATED, VANISHED
}

pub struct ErrorRecord { pub dir: NodeId, pub entry: Option<Box<OsStr>>, pub error: ScanError }

pub struct Tree {
    root_path: PathBuf,        // canonical once started; the requested path while opening
    root_dev: u64,
    root_fs_type: String,
    root_ctx: catalog::RootContext,  // classification context of root_path's parent (§5.5)
    nodes: Arena,
    removed: u32,              // tombstones (compaction trigger, §4.6)
    candidates: Vec<CandidateRef>,   // appended in apply(); cleanup pane filters this (no tree walk)
    errors: Vec<ErrorRecord>,  // appended in apply(), capped at 10,000 (popover, §9)
    links: Arc<LinkOwners>,    // from Summary (D9)
    listed: u64,
    stopped: bool,             // Stop was pressed: non-Done sizes render as "≥"
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CandidateRef { Dir(NodeId), File(NodeId, u32) }

impl Tree {
    pub fn opening(requested: PathBuf) -> Tree;               // no nodes yet; phase Opening
    pub fn start(&mut self, info: RootInfo);                  // canonical root, dev, ctx; creates ROOT
    pub fn apply(&mut self, listing: DirListing) -> ApplyEffect;
    pub fn mark_stopped(&mut self);                           // Stop pressed (§4.5)
    pub fn finish(&mut self, summary: &Summary);              // stores link owners
    // reads
    pub fn node(&self, id: NodeId) -> &Node;                  // panics on a bad id
    pub fn get(&self, id: NodeId) -> Option<&Node>;           // None for out-of-range or Removed (view uses this)
    pub fn children(&self, id: NodeId) -> impl ExactSizeIterator<Item = NodeId>;
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId>;  // parent .. root
    pub fn is_within(&self, id: NodeId, ancestor: NodeId) -> bool;        // O(depth)
    pub fn path_of(&self, id: NodeId) -> PathBuf;
    pub fn relative_path(&self, id: NodeId, base: NodeId) -> Option<Vec<Box<OsStr>>>;
    pub fn find_relative(&self, base: NodeId, rel: &[Box<OsStr>]) -> Option<NodeId>;
    pub fn find_path(&self, path: &Path) -> Option<NodeId>;   // component walk by name
    pub fn candidates(&self) -> &[CandidateRef];
    pub fn errors(&self) -> &[ErrorRecord];
    pub fn context_for(&self, engine: &catalog::Engine, id: NodeId) -> catalog::RootContext; // §5.5
    pub fn counted_elsewhere(&self, target: NodeId) -> HashSet<(u64, u64)>; // D9, §4.6
    // UI flags only
    pub fn set_flag(&mut self, id: NodeId, flag: NodeFlags, on: bool);
    pub fn set_file_flag(&mut self, dir: NodeId, index: u32, flag: FileFlags, on: bool);
    // trash accounting (§7.4); no-ops on Removed nodes or below a TRASHED ancestor; saturating
    pub fn mark_trashed(&mut self, id: NodeId);
    pub fn mark_file_trashed(&mut self, dir: NodeId, index: u32);
    pub fn subtract_small_file(&mut self, dir: NodeId, alloc: u64, apparent: u64, linked: bool);
    pub fn subtract_contents(&mut self, id: NodeId, outcome: &trash::ContentsOutcome);
    // rescan (§4.6)
    pub fn splice(&mut self, at: NodeId, sub: Tree) -> SpliceEffect;
    pub fn needs_compaction(&self) -> bool;                   // removed > 25% of the arena and > 100k
    pub fn compacted(&self) -> Tree;                          // live nodes copied in DFS order
}

#[derive(Default)]
pub struct ApplyEffect {
    pub touched_expanded: bool,    // the listed node, or a node whose child list grew, is EXPANDED
    pub new_candidate: bool,
    pub root_listed: bool,
    pub first_privacy_error: bool, // the first PrivacyBlocked error of this scan (FDA bar trigger)
}
pub struct SpliceEffect { pub removed: u32, pub became_done: bool }
```

How `apply` works. The algorithm is SCAN §7.8 / scan-bench `model.rs::apply`; the extra steps are marked with *(new)*.

1. Grow the arena to `first_child + n`, then announce the children: set name, ino, parent, and the state `Pending` or `Skipped`.
2. Fill in the node itself: `own = files + dir_alloc`, error, markers, verdict, large files (converted to `LargeFile`), state `Scanning`. A `vanished` listing sets the `VANISHED` flag and has no entries.
3. Walk from the node to the root (O(depth)) and add to each node on the way:
   - `own` sizes and `linked_alloc` *(new)*;
   - `items += files.count + subdirs.len()`;
   - on the strict ancestors *(new)*: `error_dirs += 1` if this listing has an error; `boundaries += ` the number of `MountPoint`/`OtherFilesystem` children; `cloud_items += cloud_files + ` the number of `CloudOnly` children — the counts for the node's own children are added to the node itself too, since they are strictly inside it;
   - `newest_mtime = max(..)` *(new)*.
4. If the node has no unfinished children, complete it and propagate completion upward (amortized O(1)).
5. *(new)* If the verdict is `Target`, or a large file has `matched: Some(FileMatch::Target(_))`, push to `candidates`. Protect matches are never candidates.
6. *(new)* Append `ErrorRecord`s for `error` and `entry_error_samples`.

`Skipped` children have no size. They are never added to totals, and they count as finished for completion. `Items` counts them as directories. A skipped child does *not* make its parent eligible for trashing just because the parent is `Done`: G1 also checks `boundaries` (§7.2).

**Ids.** Ids are dense `u32`s allocated in blocks by the scanner, so blocks may arrive out of order (SCAN §7.4). The arena handles this by growing to the highest announced id. Vacant slots are filled by the parent's listing, which always arrives before the children's own listings.

### 3.3 Arena and memory budget

```rust
const CHUNK: usize = 1 << 16;                  // 65,536 nodes per chunk
struct Arena { chunks: Vec<Box<[Node]>>, len: u32 }  // index = id >> 16, id & 0xFFFF
impl Arena { fn ensure_len(&mut self, len: u32); fn get(&self, id: NodeId) -> &Node; fn get_mut(..) }
```

- **Why chunked.** A single `Vec<Node>` doubles when it grows, and the transient copy was half of the measured peak (~140 MiB vs ~70 MiB of nodes on `~/dev`; SCAN §2.6). With chunks, growing never moves existing nodes. The extra cost is a shift and a mask per access.
- **Size.** `Node` is about 168 bytes (checked by `assert!(size_of::<Node>() <= 176)` in a test), plus one heap allocation per name (~12 bytes on average + allocator overhead, SCAN §2.6).
- **Budget.** Directories cost ≤ 210 B each including the name. Large files cost ~90 B each; there are about 1% of them. The hard-link owner map costs ~30 B per linked inode.

| Tree | Dirs | Expected tree memory | Whole process (incl. ~21 MiB engine and ~60 MiB iced/wgpu) |
|---|---:|---:|---:|
| `~/dev` (measured shape, 479k linked inodes) | 533k | ~110 MB + ~15 MB links | ~210 MB |
| Full Data volume (FS §8.1: 1.2M dirs) | 1.2M | ~250 MB | ~340 MB |

- **During a rescan** the sub-tree exists next to the main tree. A rescan of the scan root replaces the tree instead of splicing (§4.6), so repeated ⌘R on the root does not grow memory; the old tree is dropped on a blocking thread.
- **Hard ceiling.** `u32` ids allow 4G directories, which is not a practical limit.
- **If memory becomes an issue later** (not v1): store sizes in u32 KiB units, intern names in one byte arena, or drop `apparent`.

Per-node UI state that is *not* in `Node` (kept sparse in `HashMap`s on the app side, only for nodes that need it):
- the cached child order of expanded nodes;
- the on-demand small-file listings;
- git verification state.

All of it is keyed by `NodeId` and is carried across splices, replacements and compaction by `results/migrate.rs` (§4.6).

---

## 4. Scanner

### 4.1 API

```rust
// dirstat-core::scan
pub struct Options {
    pub threads: usize,                // available_parallelism().min(8) (SCAN §2.2)
    pub large_file_threshold: u64,     // 1 MiB, a constant, not a setting (SCAN §6)
    pub batch_interval: Duration,      // 50 ms
    pub engine: Arc<catalog::Engine>,  // shared, immutable
}
pub enum Boundaries {
    System,                                            // platform::mounts() at scan start
    Fixed { mounts: Vec<PathBuf>, excluded: Vec<PathBuf> },  // tests: declare fixture dirs as mounts
}
pub struct ScanRequest {
    pub root: PathBuf,                         // as given; canonicalized on the scan thread
    pub context: Option<catalog::RootContext>, // None: root_context() on the scan thread. Some: rescans, tests
    pub boundaries: Boundaries,
    pub counted_elsewhere: Option<Arc<HashSet<(u64, u64)>>>,  // subtree rescans (D9)
}

/// Never blocks and never fails: spawns the `scan-main` thread and returns.
pub fn start(req: ScanRequest, opts: Options, sink: impl UpdateSink) -> ScanHandle;

impl ScanHandle {
    pub fn cancel(&self);             // Drop = cancel; threads are never joined on the UI thread
    #[cfg(feature = "test-util")] pub fn inert() -> ScanHandle;
}

pub trait UpdateSink: Send + 'static {
    fn try_deliver(&mut self, update: Update) -> Delivery;   // Accepted | Busy(Update) | Closed
    fn deliver_final(&mut self, update: Update);             // Started/Failed, final batch, Finished; retries
}
```

The `scan-main` thread opens the root and then becomes the batcher:
1. `canonicalize(root)` and `metadata(&root)`. If the root does not exist, is not a directory or cannot be stat'ed → `Update::Failed(StartError)`, and nothing else follows.
2. Take the mount snapshot (`Boundaries`) and `statfs` the root for `fs_type`. Compute `root_context(engine, &root)` unless one was given (§5.5).
3. Send `Update::Started(RootInfo)`, start the workers, run the batcher loop.

A root that exists but cannot be *listed* is not a `Failed`: the root's listing then arrives with `error: Some(..)` and the app shows the root-unreadable state (UX §10.4). Because all of this runs off the UI thread, a TCC prompt or a dead network root can only delay `Started`; the window stays responsive and Stop works (§4.5).

### 4.2 Threads and work distribution

Copy SCAN §7.3–7.7 and scan-bench `queue.rs` (keep only `StealingQueue`):
- `threads` workers, each with a 256 KiB stack and the name `scan-worker-N`, plus the `scan-main` thread (opener + batcher);
- per-worker `Mutex<VecDeque<Job>>`: a worker pops from the back of its own deque and steals from the front of others;
- an `outstanding` counter detects termination;
- an epoch condvar lets idle workers sleep.

`Job` gains the catalog context:

```rust
struct Job { id: NodeId, path: PathBuf, dir_alloc: u64, parent_name: Box<str>, ctx: catalog::Ctx /* 24 B, Copy */ }
```

Why work stealing and not a simpler shared queue: it is 0–12% faster, and above all the top-level folders complete early, which the live UI depends on (SCAN §2.3). The cost is ~120 lines that already exist and are tested.

**Panic safety.** The termination protocol needs every job to call `complete()` exactly once; a job that unwinds would leave `outstanding > 0` forever and hang the scan. So:
- each job runs as `catch_unwind(AssertUnwindSafe(|| list_one(..)))`. On a panic the worker sends a listing for the job's id with `error: Some(ScanError { class: Internal, errno: None })` and no entries, counts `internal_errors += 1`, and calls `complete()` with no children as usual;
- every shared mutex (deques, the hard-link map, the directory-link set) is locked with `lock().unwrap_or_else(PoisonError::into_inner)`;
- the batcher loop is also wrapped; if it panics it calls `deliver_final(Finished { cancelled: true, .. })` and cancels the workers.

### 4.3 Per-directory work (`worker.rs::list_one`)

At the start of the job the worker publishes `(job.id, now)` into its slot of the in-flight table (one `AtomicU64` per worker: id + start time in ms) and clears it at the end.

For job `D`:

1. `std::fs::read_dir(D.path)`.
   - `ENOENT`/`ENOTDIR`: the directory vanished after its parent listed it. Produce a listing with `vanished: true`, no entries and no error; `vanished += 1`. It is not an error (SCAN §7.10, FS §7.1).
   - Any other error: classify it and produce a listing with `error` set and no entries.
2. For each entry, stopping early if the cancel flag is set (checked every 256 entries; publish the file counter there too):
   1. `file_type()` is free (d_type). If it is a directory, or unknown, and the joined path is in the **mount set** → `Subdir { skip: MountPoint }` **without stat'ing it**. If it is in the **excluded set** → `Skip::Excluded`. Never touching a mount point is what keeps the scan safe on dead network mounts (FS §10 notes). Build the joined path only when `D.path` is in `boundary_parents` (the parent paths of all mount and excluded paths), so normal directories pay nothing.
   2. `entry.metadata()` (fstatat/statx, no symlink following) → `platform::EntryMeta`.
      - Error `ENOENT`/`ENOTDIR`/`ELOOP` → `vanished += 1`.
      - Any other error → `entry_errors += 1`, and the first 4 go into `entry_error_samples`.
   3. Directory:
      - `meta.dev != root_dev` → `Skip::OtherFilesystem`;
      - macOS `meta.dataless` (`st_flags & SF_DATALESS`) → `Skip::CloudOnly` (FS §1.11 step 2);
      - HFS+ root only: `(dev, ino)` already in the directory-link set → `Skip::LinkedDir` (D4);
      - otherwise a child job.
      - Directories also feed `newest_mtime`.
   4. Non-directory (file, symlink, socket, …):
      - `files.count += 1`; if `meta.dataless`, `cloud_files += 1`;
      - if `nlink > 1`: an inode in `counted_elsewhere` counts 0 bytes; otherwise check the global `Mutex<HashMap<(dev, ino), NodeId>>`: the first sighting records `D.id` as owner and counts its size (also into `linked_alloc`), a later sighting counts 0 bytes and `hardlinks_deduped += 1`;
      - if `max(alloc, apparent) >= threshold` → push a `FileEntry`;
      - `newest_mtime = max(..)`.
   5. `markers |= catalog::marker_of(name_str, is_dir)`. If the `CACHEDIR_TAG` bit came from this entry, run `catalog::is_valid_cachedir_tag(path)` (reads 43 bytes, rare) and clear the bit if it fails (CAT §3).
3. **Classify.** `(verdict, ctx) = engine.classify_dir(&job.ctx, &job.parent_name, name_str, markers)`. For each large file: `matched = engine.classify_file(&ctx, file_name)` (returns `FileMatch::Target` or `FileMatch::Protected`). Names that are not UTF-8 are classified via `to_string_lossy()`. They can't match any rule, which is all ASCII.
4. Reserve the child ids with one `fetch_add`. Push the child jobs, each carrying `ctx` and the parent name, to the local `children` vector. Send the listing to the batcher channel. *Then* call `queue.complete(me, children)`: the listing goes out before the children become visible, which is the parent-before-child guarantee (SCAN §7.4).

The worker never opens files except `CACHEDIR.TAG`, never follows symlinks, and never stats a skipped mount point. Metadata decoding is split into a pure `platform::meta_from_raw(RawStat) -> EntryMeta` (blocks, flags, dev, ino, nlink, mode, mtime) and a thin `entry_meta(&Metadata)` wrapper, so the dataless and flag logic is unit-tested without real iCloud files.

### 4.4 Batches to the UI

The batcher works as in SCAN §7.5:
- `recv_timeout` + `try_iter`;
- every 50 ms, `try_deliver`; on `Busy`, merge the new data into the next batch (coalescing backpressure);
- progress-only batches are sent while the file counter moves;
- `Progress.current`: if any in-flight job has been running for ≥ 1 s, the oldest one (the scan is waiting on it); otherwise the most recently started one;
- `deliver_final` delivers the last batch, then `Finished`.

The app-side sink is a newtype. The orphan rule forbids implementing the core trait for the foreign `futures` sender type, so the app wraps it:

```rust
// dir-stat::results::scan_task
pub struct UiSink(iced::futures::channel::mpsc::Sender<scan::Update>);
impl scan::UpdateSink for UiSink { /* try_send; is_full() -> Busy(into_inner); else Closed. SCAN §7.12 */ }
```

The channel is `mpsc::channel(1)`. Behind it, `Task::run` forwards into iced's winit proxy, which has its own bounded channel of 100 plus up to 100 in-flight actions (`iced_winit-0.14.1/src/proxy.rs`). So up to ~200 batches can queue before the batcher sees `Busy`; coalescing only engages after long UI stalls (window drag, the modal folder picker). That is acceptable, because iced runs `update` for every queued message and then `view` once. It also means queued batches of an aborted scan keep arriving, so the scan-id filter below is mandatory, not defensive.

Wiring in `update()`:

```rust
fn start_scan(&mut self, kind: ScanKind, req: ScanRequest) -> Task<Message> {
    let id = self.scan_ids.next();                                   // ScanId(u64)
    let (tx, rx) = iced::futures::channel::mpsc::channel(1);
    let handle = scan::start(req, self.scan_options(), UiSink(tx));  // returns at once, no I/O here
    let (task, abort) = Task::run(rx, move |u| Message::Scan(id, u)).abortable();
    self.active = Some(ActiveScan { id, kind, handle, _abort: abort.abort_on_drop(), started_at: Instant::now() });
    task
}
```

For a full scan the app switches to the Results screen at once, with `Tree::opening(path)` and `phase = Opening` (activity field "Opening…", Stop enabled). On `Started(info)`: `tree.start(info)`, `phase = Scanning`. On `Failed(e)` for a full scan: back to the start screen with the banner "Can't scan “/foo”: <message>." (§9). For a subtree rescan see §4.6.

Every message that carries results from a background job is tagged and filtered: `Message::Scan(id, _)`, `SmallFilesListed(id, ..)` and `GitChecked(id, ..)` with an id that is not the current one are dropped, and `TrashFinished` carries the tree generation (§7.4). This guarantees that a late result from a replaced scan never touches a new tree.

### 4.5 Cancellation (Stop keeps partial results)

Stop is final on the UI side immediately, because a worker blocked in a syscall (a TCC prompt left open, a dead NFS/SMB server) may delay `Finished` indefinitely (SCAN §7.6, FS §5.2).

When the user presses Stop during a **full scan**:
- call `handle.cancel()`;
- if the phase is `Opening`: drop the `ActiveScan` and return to the start screen (the opener thread is abandoned);
- otherwise: `tree.mark_stopped()` and `phase = Stopped` right away, so non-`Done` nodes render as "≥ size" (or "—" if never listed) with the "stopped" tooltip (UX §5.3), and move the `ActiveScan` into `draining: Option<ActiveScan>`.

A draining scan does **not** count as running: Rescan, Trash, New Scan (no "Stop scanning?" dialog) and the tick condition all look only at `active`. Late batches from the draining scan are still applied (they only add data), and its `Finished` fills in `summary`. Starting any new scan or rescan drops the draining scan: its task is aborted and stuck threads are abandoned. The errors popover does not wait for `Finished`: it reads `tree.errors()`, which grows with every applied listing.

When the user presses Stop during a **subtree rescan**: cancel it, drop the sub-tree and the `ActiveScan`, clear `RESCANNING`. `phase` is unchanged, so a complete main tree keeps reading "complete".

### 4.6 Rescan of a subtree

Rescan is MUST (UX §6.6). It is allowed when:
- no scan is running (`active.is_none()`; a draining scan doesn't count);
- the target and its ancestors are not `TRASHED`, `Removed` or `RESCANNING`;
- no trash job is pending inside the target (checked against `trash_jobs` with `is_within`).

Steps:
1. The target is the selected folder, or the zoom root if a file or nothing is selected. Set `RESCANNING` on it. The UI keeps showing the old numbers in secondary color with a scanning dot.
2. Build the request from the main tree: `context = Some(tree.context_for(&engine, target))` (§5.5), `counted_elsewhere = Some(tree.counted_elsewhere(target))` (the linked inodes whose owner is outside the target; O(links × depth), ~10–20 ms for 479k links), `boundaries = System`. Start a scan with `root = tree.path_of(target)` into a **separate** `Tree` (`ScanKind::Subtree { target }`).
3. `Failed(e)` for a subtree: clear `RESCANNING`. If `e.kind == NotFound`, show the toast "Folder no longer exists — rescan the parent"; otherwise an error toast with the message.
4. When `Finished { cancelled: false }` arrives, capture the view state (`SavedView::capture(&tree, target, &results)`, below) and then:
   - **If `target == ROOT`:** replace the tree (`mem::replace(&mut self.tree, sub)`) and send the old tree to a `spawn_blocking` drop.
   - **Otherwise:** `tree.splice(target, sub)`:
     1. Subtract `target.total` from the strict ancestors (saturating).
     2. Tombstone the old subtree: walk it once, set `state = Removed`, free each node's `name` and `large_files`; `removed += n`. Arena slots are not reused.
     3. Append the sub-tree's non-root nodes, remapping ids with `id + (arena_len − 1)`; sub-root id 0 maps to `target`. Nodes are moved, not cloned (~30–50 ms per million nodes on the UI thread; only large subtrees get close).
     4. Copy the sub-root's fields into `target`, keeping `name`, `parent` and `flags`.
     5. Add the new totals to the ancestors. Recompute `newest_mtime` along the ancestor chain as the max over each ancestor's own entries and children (O(depth × siblings)).
     6. **Completion.** If `target` was not `Done` before (the main scan was stopped) and the sub-root is `Done`, run the same upward propagation as `apply`: decrement `parent.unfinished` and mark ancestors `Done` while it reaches 0 (`SpliceEffect.became_done`).
     7. Re-derive candidates and error records: drop those inside `target` and add the sub-tree's, remapped.
     8. Hard-link owners: drop entries owned by removed nodes, insert the sub-scan's entries with remapped owners.
   - Then, if `tree.needs_compaction()`, replace the tree with `tree.compacted()` (same as the root case).
5. Restore the view state: `saved.restore(&tree, target, &mut results)`.
6. Afterwards, run one exact, anchored re-sort (UX §6.6: "everything swaps in one step with one re-order").

If the rescan is cancelled, the sub-tree is discarded and nothing changes.

**`results/migrate.rs` (`SavedView`).** One function pair carries every `NodeId`-keyed structure across a splice, a replacement or a compaction:
- *captured by path relative to `base`, then re-resolved with `find_relative`:* zoom, selection (a `File` key is re-resolved by file name), every history entry, the set of `EXPANDED` nodes, git statuses of candidates *outside* the target (only needed when all ids change);
- *dropped and recomputed:* `orders` (exact sort on restore), `small_files` for nodes inside `base`, flash, git statuses inside the target (the candidates are verified again, since their files changed), cleanup-pane expansion state;
- *never affected:* trash jobs, because a rescan never starts while one is pending inside the target, and a full replacement bumps the tree generation (§7.4).

Keys that no longer resolve fall back to the nearest surviving ancestor.

**Rescan everything** (⇧⌘R) is simply a new full scan of the same root. **Scan This Volume** is a new full scan with the mount point as root. It is not offered on the `/System/Volumes/Data` row (tooltip "Scanning / already covers this volume", D2).

### 4.7 Mount boundaries

The mount set is taken once, at scan start, on the scan thread: `platform::mounts()`, filtered to paths strictly below the root (SCAN §5.3, FS §11). Tests pass `Boundaries::Fixed`.

| OS | Source | What becomes a boundary |
|---|---|---|
| macOS | `getmntinfo(MNT_NOWAIT)` (never blocks) | Every mount below the root: `/System/Volumes/*` including Data (D2), `/dev`, autofs, `/Volumes/*`. Firmlinks are not mounts and are scanned normally, so user data is counted once, under `/Users` and so on. When the root is under `/System/Volumes/Data`, each mount `m` is also registered as the alias `/System/Volumes/Data` + `m` (FS §2.10) |
| Linux | `/proc/self/mountinfo` (field 5, octal-unescaped; scan-bench `mounts.rs`) | Every mount below the root: proc, sys, tmpfs, bind mounts, squashfs, overlay, network, other disks. btrfs subvolumes are caught by the `st_dev` check |
| Windows | none | Junctions and symlinks are never followed (std `is_symlink`) |

The excluded set (D4): on macOS with root `/`, `/.vol`, `/.nofollow`, `/.resolve` (FS §2.9); for every mount with `fs_type == "hfs"` (including the root itself), `<mount>/Backups.backupdb`; for every `zfs` mount, `<mount>/.zfs`. Their rows read "Time Machine backups – not scanned" / "System folder – not scanned".

The `st_dev` comparison stays as a backstop for mounts that appear during the scan and for btrfs subvolumes. An explicitly chosen root is always scanned, even if it is itself a mount point, because the rules apply strictly below the root.

Roots under `/System/Volumes/*` can be scanned for exploration, but Move to Trash is refused there (G5), and the cleanup pane shows one info line instead of suggestions: "Suggestions are off under /System/Volumes. Scan / or your home folder instead."

### 4.8 Process and thread setup, permissions, TCC

Process and thread setup:
- `platform::init_process()` runs once in `main` before iced starts:
  - macOS: `setiopolicy_np(IOPOL_TYPE_VFS_TRIGGER_RESOLVE, PROCESS, OFF)` (no autofs mounting);
  - Unix: raise `RLIMIT_NOFILE` to `min(hard, 10240)` (launchd's soft limit is 256; FS §7.2).
- `platform::init_io_thread()` sets, on macOS, dataless materialization OFF and atime updates OFF at thread scope (SCAN §7.11); elsewhere it does nothing. It runs at the start of every thread or blocking closure that touches the filesystem: `scan-main`, every scan worker, and the `spawn_blocking` closures for the small-file listing, trash and git. Thread scope is not inherited (FS §1.11), so each closure calls it.
- `smallfiles.rs` and contents-mode trash check `SF_DATALESS` on the directory before `read_dir`. Git verification is skipped (status `Unavailable`, reason "In iCloud – not checked") when the work-tree root or its `.git` is dataless, because the `git` child process runs with the process default (materialization on).

Error handling:
- Errors are classified in core: `EPERM` → `PrivacyBlocked` on macOS (`PermissionDenied` elsewhere), `EACCES` → `PermissionDenied`, `EDEADLK` → `CloudNotDownloaded`, `ENAMETOOLONG` → `PathTooLong`, a caught panic → `Internal`, anything else → `Other`. `ENOENT`/`ENOTDIR` on a directory is "vanished", not an error (§4.3).
- A `PrivacyBlocked` listing is how the app detects that Full Disk Access is missing (§9).
- dir-stat never triggers TCC prompts implicitly: a scan only starts after the user picks a root (SCAN §7.11). Parallel prompts for Desktop, Documents and Downloads may stack. This is accepted in v1, and the start-screen footnote and Home-card primer warn about it (UX §4). The prompts block only scan threads; the UI thread never touches the scanned paths.

---

## 5. Cleanup catalog engine

### 5.1 Representation

Port the prototype's `model.rs`, `catalog.rs` (as `table.rs`) and `engine.rs` unchanged, apart from the additions listed in §5.5. The representation (CAT §3):
- **Rules** are a `const` table (`Rule { id, name, category, os, kind, zone, matchers, removal, reason, caution, cleanup, shares_space, evidence }`).
- **Matchers** are `At` (anchored), `FileAt`, `Dir`, `Under` or `Contains`.
- **Marker bits** are a `u64` (51 used).
- **Kinds** are `Target(Risk)`, `Container { default }` and `Protect(Scope)`.
- **`Engine::new(&Resolver::from_env().canonicalized())`** builds a component trie for the anchored rules and hash maps for the heuristics. It is created once at app start and shared as `Arc<Engine>`. `canonicalized()` (new) runs `canonicalize` on home and on every env-var base (`XDG_*`, `CARGO_HOME`, `RUSTUP_HOME`, `GOPATH`, …) that exists, because tree paths are built from the canonical scan root: with `HOME=/home/u` and `/home/u -> /data/u`, uncanonicalized `~` rules would never match.
- **`RuleIdx = u16`.**

### 5.2 Matching algorithm

Matching runs in the scan worker, once per directory, right after its listing (§4.3 step 3). It follows CAT §2.2:
1. **Already inside a target?** The result is `Inside`, with zero work below candidates.
2. **One trie step.** Anchored rules at this exact path, plus `Protect` zone flags.
3. **Protect heuristics:** app bundles (only with `Contents/`, `Versions/` or `Info.plist`), `.git`/`.hg`/`.svn`, test fixtures, media libraries.
4. **An anchored Target or Container wins.** Otherwise a matched Protect returns `Protected`.
5. **Heuristic targets**, unless in a NoHeuristics zone, on an anchored path, or (for project rules) in a NoProjectRules zone. Candidates are taken by name, then by pattern, then by `Contains`, and the first rule in table order wins.
6. **Container default** for an unmatched direct child of a container.

**Outermost match wins.** A Target sets `F_INSIDE` in the context that its children inherit, so nothing below it can become a candidate and the "reclaimable" totals are plain sums over candidates.

**Containers** (`~/Library/Caches`, `~/.cache`, `/var/cache`, `$CARGO_HOME/registry` and `git`, JetBrains caches) are not candidates themselves. Each direct child is: either through a specific rule, or through the container's default rule (`by_default: true`).

**Project markers.** Sibling markers come from the parent's `Ctx.markers`, and own markers from this listing. That is how `target` next to `Cargo.toml` becomes `cargo-target`.

**Files.** `classify_file` returns `Option<FileMatch>`; only `FileMatch::Target` becomes a candidate (the Protect file rule `win-system-files` must never reach `section_for`).

The cost is about 0.2 µs per directory (CAT §2.2), which is negligible.

### 5.3 Risk → UI section (user decision D1) and git

All section decisions go through one pure function in core. The cleanup pane, the tree pills, the trash dialog's reason and icon, and `default_is_trash` call only this function; none of them looks at `Risk` directly.

```rust
// dirstat-core::catalog
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section { SafeToDelete, ReviewFirst }

pub const fn section_for(risk: Risk) -> Section {
    match risk {
        Risk::Safe => Section::SafeToDelete,
        Risk::Low | Risk::Review => Section::ReviewFirst,   // Low: restorable only by re-download/rebuild
    }
}
// Kind::Protect has no Risk: never a candidate, never shown (no "System" pill in v1).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitState { NotInRepo, Pending, Known(git::GitStatus) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    Pending,                       // waiting for git: not in the pane, no pill, neutral dialog reason
    Hidden,                        // tracked files: never suggested anywhere
    Shown { section: Section, why: Why },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why { Catalog, GitNotIgnored, GitUnavailable }

pub const fn display_section(risk: Risk, git: GitState) -> Display;
```

| Catalog risk | `GitState` | `Display` |
|---|---|---|
| Safe | `NotInRepo`, or `Known(Clean)` (ignored, no tracked files) | `Shown { SafeToDelete, Catalog }` |
| Low, Review | `NotInRepo`, or `Known(Clean)` | `Shown { ReviewFirst, Catalog }` |
| any | `Known(NotIgnored)` | `Shown { ReviewFirst, GitNotIgnored }` |
| any | `Known(Unavailable)` (no git, timeout, dataless work tree) | `Shown { ReviewFirst, GitUnavailable }` |
| any | `Known(Tracked)` | `Hidden` |
| any | `Pending` | `Pending` |

Consequences of D1 and of `display_section` elsewhere:
- **Stat block:** the big number is Σ `Shown { SafeToDelete, .. }` ("likely reclaimable"), and "+ X to review" is Σ `Shown { ReviewFirst, .. }`. The pane header total (the "Cleanup 38 GB" label of UX §7, shown in the header until the tab bar arrives in M12) is the same Σ Safe.
- **Pill dot in tree rows:** green (`status_good`) for `SafeToDelete`, amber (`status_warning`) for `ReviewFirst`, no pill for `Pending` and `Hidden`.
- **Section headers:** "✓ SAFE TO DELETE · rebuilt automatically" and "! REVIEW FIRST · costs a re-download, or may hold your data". The UX §7 subtitle "may contain your data" would be wrong for most of the section now, since nearly every `~/Library/Caches/*` child (container default Low), `node_modules`, `.venv`, Gradle and rustup are Low.
- **"Re-downloadable" tag:** Low groups in Review first carry a muted 11 px text tag "Re-downloadable" after the group name (open question 1 in §12 has this as its default). There is no separate sub-section.
- **Dialog reason line:** `Shown` → the section icon and the catalog reason; `Pending` → "Checking with git…" with no icon; `Hidden` → no catalog reason.
- **Dialog warning by cause (§7.3):** Low → "Restored only by re-downloading or rebuilding: <reason>"; Review → "May contain data you want: <reason>"; `GitNotIgnored` → "Not ignored by git in <repo>: it may contain your work."; `GitUnavailable` → "Couldn't check with git."; `Hidden` (tracked) → "Contains files tracked by git in <repo>."
- **Confirmation dialog default:** Return defaults to *Move to Trash* only for `Shown { SafeToDelete, Catalog }` with removal mode `Item` or `Contents` and no warning lines (§7.3).

A unit test (`catalog::display`) pins the whole table, the warning text per risk and cause, and `section_for`. A second test (`results::cleanup`) pins the stat block (Σ Safe only), Low → Review first, and that `Pending`/`Hidden` items are absent from the pane and have no pill.

### 5.4 Protection guards inside the catalog

The catalog's Protect rules and zones (CAT §2.3, §5.1) keep the catalog from *suggesting* things inside app bundles, package-manager prefixes, OS folders, VCS directories, test fixtures, iCloud Drive and Mail. They do **not** decide what may be deleted. That is the job of `safety.rs` (§7.2), so a catalog bug can't open a path to deleting system folders, and the delete guards can't hide suggestions.

The catalog can, however, *add warnings*: `plan` walks the target and its ancestors (O(depth)) for a `Verdict::Protected(rule)`, and a hit adds the warning "<rule name>: <reason>" (e.g. "iCloud Drive: deleting here deletes on all your devices.") and makes Cancel the default (§7.3).

### 5.5 Engine API additions and the root context

The engine only sees what the scan walks: a scan root's parent and siblings are outside the scan. So the root needs an explicitly computed context:

```rust
// dirstat-core::catalog
pub struct RootContext {
    pub parent_ctx: Ctx,               // context of the root's parent directory
    pub parent_name: String,
    pub git_root_above: Option<PathBuf>, // nearest ancestor of the root that contains `.git`
}

/// For a fresh, canonical scan root. Runs on the scan thread (§4.1), never on the UI thread.
/// The root "/" (or a Windows drive root) has no parent: it uses engine.root().
/// 1. (_, gp) = engine.enter(grandparent path)       — anchored rules + zones, no I/O
/// 2. parent markers = OR marker_of over a names-only read_dir(parent)   (d_type, no stat)
/// 3. parent_ctx = engine.classify_dir(&gp, gp_name, parent_name, parent_markers).1
/// 4. git_root_above = first ancestor A of root with A/.git existing (one lstat per level)
pub fn root_context(engine: &Engine, root: &Path) -> RootContext;

/// A neutral context, as if the root's parent were an ordinary folder such as /Users/x/dev.
/// For tests: fixtures under $TMPDIR = /private/var/folders/… sit in the macOS NoHeuristics
/// zone (mac-system protects /private), so root_context() would suppress every heuristic rule.
impl RootContext { pub fn detached(engine: &Engine) -> RootContext; }
```

For a rescan, `Tree::context_for(engine, id)` rebuilds the exact context of `id`'s parent. It starts from `tree.root_ctx`, folds `classify_dir` down the ancestor chain using each node's stored `markers`, and inherits `git_root_above`. `classify_dir` is a pure function of (parent context, parent name, name, markers), so the rescanned subtree is classified exactly as in the first scan. This is why `Node` stores `markers` (8 bytes): it is cheaper than storing the 24-byte `Ctx`, and it also drives package detection and git-root lookup.

Other additions:
- `Engine::rule(idx).risk()` already exists.
- New: `Engine::cleanup_hint(idx, name, path) -> Option<String>`, which renders the first `cleanup` entry as "Or run `…`." / "Prefer …" (CAT §1 mapping table).
- New: `classify_file` returns `Option<FileMatch>` (§5.2).
- New: `Resolver::canonicalized()` (§5.1).

If the scan root's own verdict is `Target` or `Inside`, nothing below it can be a candidate. The pane then shows one info line: "This whole folder is <rule name>: <reason>" (it has no trash action, because the scan root can't be trashed).

### 5.6 Git verification (`dirstat-core::git`, app `results/candidates.rs`)

A candidate needs verification if it has a work-tree root. The app finds that root without the scan-time `Ctx`: it is the nearest ancestor node whose `markers & m::GIT != 0` (this matches the engine's `in_git_worktree` flag), or else `tree.root_ctx.git_root_above`. If neither exists, the candidate is `GitState::NotInRepo` and is shown without a git call.

**Checking:**

```rust
pub struct GitJob { pub worktree: PathBuf, pub items: Vec<(CandidateRef, PathBuf /* relative */)> }
pub enum GitStatus { Clean, NotIgnored, Tracked, Unavailable }
pub fn verify(git: &GitBinary, job: &GitJob) -> Vec<(CandidateRef, GitStatus)>;
// per work tree: `git -C wt check-ignore -z --stdin` (ignored set) and
//                `git -C wt ls-files -z -- <rels…>` (tracked files), 10 s timeout, kill on timeout
// a dataless work tree or .git → Unavailable without running git (§4.8)
pub fn find_git() -> Option<GitBinary>;
// macOS: /opt/homebrew/bin/git, /usr/local/bin/git, else /usr/bin/git only if `xcode-select -p`
// succeeds (otherwise /usr/bin/git opens the CLT install dialog, CAT §5.6). Linux: `git` in PATH.
```

`results/candidates.rs` keeps `HashMap<CandidateRef, GitStatus>` and hands `GitState` to `display_section`. On every Tick, when no git job is running and some candidates are `Pending`, it starts one job: the work tree with the most pending candidates, all of them in one batch (`Task::perform` + `spawn_blocking`, tagged with the scan id). At most one git job runs at a time, which batches naturally. Pending candidates are not shown in the pane and get no pill (D10). While scanning, this matches "items appear as matched folders finish".

---

## 6. iced application architecture

### 6.1 Entry point

```rust
// dir-stat::main
fn main() -> iced::Result {
    let cli = Cli::parse_env();                       // hand-rolled: [<path>] | --help | --version
    dirstat_core::platform::init_process();
    iced::application(move || App::boot(cli.clone()), App::update, App::view)
        .settings(iced::Settings {                    // FIRST: settings() replaces fonts and default_font
            default_text_size: iced::Pixels(13.0),    // UX §11.3; there is no separate builder method
            ..Default::default()
        })
        .window(window::Settings {                    // before the other window options (API gotcha 14)
            size: Size::new(1200.0, 800.0), min_size: Some(Size::new(800.0, 500.0)),
            ..window::Settings::default()
        })
        .title(App::title)                            // "dir-stat" | "<zoom root name> — dir-stat"
        .theme(App::theme)                            // always None: follow the OS (API gotcha 10)
        .style(theme::app_style)                      // window background + default text from the tokens
        .subscription(App::subscription)
        .font(fonts::INTER_REGULAR).font(fonts::INTER_MEDIUM).font(fonts::INTER_SEMIBOLD)
        .default_font(fonts::INTER)                   // Font::with_name("Inter")
        .run()
}
```

Fonts:
- The three fonts are Inter 4.x **static** TTFs (`Inter-Regular`, `-Medium`, `-SemiBold`) plus `OFL.txt` in `crates/app/assets/fonts/`. Variable fonts don't work for this (API §i).
- Weights are written as `Font { weight: Weight::Medium, ..fonts::INTER }`. Bold (700) is never used.
- **Fallback glyphs** (`▸ ▾ › ✓ ⌂ ≥` and any other UX §11.7 symbol Inter lacks) are always separate `text` elements at Regular weight, never inside Medium/SemiBold text: cosmic-text's fallback only accepts faces with the exact weight, so a ✓ inside 600-weight text can render in Apple SD Gothic Neo or Menlo Bold (API gotcha 1, §i).

`App::boot(cli)` does the I/O (prefs from `dirs::config_dir()`, volume list, FDA check, `Engine::new`, `Guards::new`) and then calls `App::new(env, cli)`, which does none. Tests construct `App::new` with a fake `Env` (§10.3):

```rust
pub struct Env {
    pub engine: Arc<catalog::Engine>,
    pub guards: Arc<safety::Guards>,
    pub prefs_store: Box<dyn PrefsStore>,      // JSON file in production, in-memory in tests
    pub volumes: Vec<volumes::VolumeCard>,
    pub fda: Option<bool>,
    pub trasher: Arc<dyn trash::Trasher>,
    pub own_bundle_id: Option<String>,
}
```

### 6.2 State

```rust
pub struct App {
    env: Env,
    prefs: Prefs,                    // recents, side pane shown, sort
    screen: Screen,
    modal: Option<Modal>,            // + focused_button
    toast: Option<Toast>,
    window: WindowInfo,              // size (from resize events), id
    drag_hover: bool,
    left_button_held: bool,          // from event::listen_with (§6.5)
    scan_ids: ScanIdGen,
    #[cfg(feature = "screenshot")] script: Option<script::Script>,
}
enum Screen { Start(StartState), Results(Box<Results>) }

struct StartState { volumes: Vec<VolumeCard>, banner: Option<String>, focused_card: usize, fda: Option<bool> }

struct Results {
    tree: Tree,
    tree_gen: u32,                   // bumped on every full scan and on every tree replacement
    phase: Phase,                    // Opening | Scanning | Complete | Stopped | RootUnreadable(ScanError)
    active: Option<ActiveScan>,      // full scan or subtree rescan in flight
    draining: Option<ActiveScan>,    // a stopped full scan whose Finished hasn't arrived (§4.5)
    sub_tree: Option<Tree>,          // the rescan accumulator
    progress: ActivityModel,         // last Progress, rate EMA, ETA, current path (≤ 5 Hz), estimate
    estimate: Option<volumes::ScanEstimate>,
    summary: Option<Summary>,
    view: TreeView,
    candidates: CandidateStatus,     // git status map + job queue (§5.6)
    cleanup: CleanupPane,            // cached CleanupModel + cursor + expansion
    focus: Pane,                     // Tree | Cleanup
    pointer_in: Option<Pane>,        // from mouse_area on_enter/on_exit (calm sorting)
    side_pane: bool,
    fda_bar: FdaBar,                 // Hidden | Shown | Dismissed
    errors_popover: bool,
    small_files: HashMap<NodeId, SmallFiles>,   // Loading | Loaded(Vec<SmallFile>) | Failed(String)
    trash_jobs: HashMap<TrashJobId, TrashJob>,
}

struct TreeView {
    zoom: NodeId,
    sort: Sort,                      // column + direction
    orders: HashMap<NodeId, ChildOrder>,   // cached order of expanded nodes (+ zoom root)
    rows: Vec<Row>,                  // flattened visible rows (keys only; values read live from tree)
    scan_rows_dirty: bool, last_rows_build: Instant,   // throttling for scan-driven changes only
    reorder: ReorderClock,           // last tick, last activity, deferred since, post-trash pending
    viewport: Viewport,              // offset_y (app-side, clamped), last known height
    selection: Option<RowKey>,
    history: History,                // back/forward stacks of {zoom, selection, offset_y}
    flash: Option<(RowKey, Instant)>,
}

pub struct SmallFile { pub name: Box<OsStr>, pub alloc: u64, pub apparent: u64, pub mtime: i64,
                       pub ino: u64, pub kind: EntryKind, pub nlink: u32, pub flags: FileFlags }
```

### 6.3 Messages

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // start screen
    ScanPath(PathBuf),                         // card, recent entry, CLI, drop, Scan This Volume
    ChooseFolder, FolderChosen(Option<PathBuf>),
    FileHovered, FilesHoveredLeft, FileDropped(PathBuf),
    RemoveRecent(PathBuf),
    // scanning
    Scan(ScanId, scan::Update),                // Started / Failed / Batch / Finished
    Stop,
    NewScan, ConfirmNewScan,                   // asks first while scanning (UX §5.5)
    Rescan, RescanAll,
    Tick(Instant),                             // 50 ms, only while something is live (§6.5)
    // tree
    TreeScrolled(scrollable::Viewport),
    RowPressed(RowKey), RowDoubleClicked(RowKey), ToggleExpand(RowKey),
    SortBy(Column),
    ZoomTo(NodeId), ZoomOut, Back, Forward,
    SmallFilesListed(ScanId, NodeId, Result<Vec<SmallFile>, String>),
    // pointer activity (calm sorting, §6.7)
    PointerIn(Option<Pane>),                   // mouse_area on_enter/on_exit around each pane
    PointerMoved,                              // event::listen_with CursorMoved, only while a re-order can happen
    LeftButton(bool),                          // pressed / released, any event status
    // actions
    Reveal(PathBuf), CopyPath(PathBuf), OpenPrivacySettings,
    PlatformResult(Result<(), String>),        // reveal/open failures -> toast
    RequestTrash(TrashTarget), ConfirmTrash, CancelModal,
    TrashFinished(TrashJobId, u32 /* tree_gen */, Vec<TrashOutcome>),
    // cleanup pane
    CleanupToggleGroup(RuleIdx), CleanupShowMore(RuleIdx), CleanupShowSmaller,
    CleanupActivate(CandidateRef),             // click: reveal in tree (UX §7)
    CopyCommand(String),
    GitChecked(ScanId, Vec<(CandidateRef, GitStatus)>),
    // chrome
    Key(KeyAction),
    FocusPane(Pane), ToggleSidePane,
    ToggleErrorsPopover, CopyErrorList, DismissFdaBar, DismissToast,
    WindowOpened(window::Id), WindowResized(Size),
    MouseBack, MouseForward,
    #[cfg(feature = "screenshot")] Screenshot(window::Screenshot),
    #[cfg(feature = "screenshot")] ScriptStep,
}
```

`RowKey` identifies what a row shows. It stays stable across reorders:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RowKey {
    Dir(NodeId),
    File(NodeId, u32),        // dir + index into the node's large_files (owned by the Tree)
    SmallFile(NodeId, u32),   // dir + index into Results.small_files[dir] (owned by the app)
    Aggregate(NodeId),        // "≡ N files"
    MoreFiles(NodeId),        // "Show 50 more" / "and 1,190 smaller files" / "Listing files…"
    Placeholder(NodeId),      // "Waiting to be scanned…" / "This folder is empty."
    Unaccounted,              // root-level synthetic row (UX §5.6)
}
pub struct Row { pub key: RowKey, pub depth: u16 }
```

`view` resolves every key with bounds-checked lookups (`tree.get`, `large_files().get`, `small_files.get`) and skips a row that no longer resolves, so a stale key can never index out of bounds.

### 6.4 Update and view split

- `App::update` dispatches by message to small handler functions on `Results` / `StartState`. Each handler returns a `Task<Message>`, or `Task::none()` when nothing changes. Handlers mutate state only; they never build widgets. Side effects that matter for safety are created as plain values first (a `TrashJob`, a `ScanRequest`) and only then wrapped in a `Task` by a one-line helper, so app tests can assert on the values.
- `App::view` is pure. It reads `&App` and builds the elements; sizes, states and labels are read live from the `Tree` for the visible rows only.
- Derived data that is expensive to compute is cached in the state: the `rows` list, the `ChildOrder`s, and the cleanup model (`build_cleanup(tree, engine, candidates, scope) -> CleanupModel`, a pure function).
  - **User-initiated or structural changes rebuild `rows` synchronously in the same handler:** expand, collapse, zoom, sort, reveal, Back/Forward, splice or replacement, trash success. Any `scroll_to` is computed from the fresh rows in the same handler. iced applies widget operations returned from `update` before the next redraw, so there is no flicker.
  - **Scan-driven changes** (`ApplyEffect.touched_expanded`) only set `scan_rows_dirty`; the Tick rebuilds at most every 100 ms.
  - The cleanup model is rebuilt on the reorder tick, on rescope (zoom), on git results and after trash.
- **Testability:** every `results/*.rs` model (order, rows, activity, cleanup, candidates, trash plan, nav, migrate) is a plain struct with pure functions and unit tests. `view` functions are thin.

Layout of the Results screen (UX §6.1):

```rust
column![toolbar(activity_field), path_bar, row![column![tree_pane, fda_bar?], side_pane?], selection_bar]
// wrapped as stack![base, toast_layer?, modal_layer?]  (modal = API §d with_modal pattern)
```

The FDA bar sits at the bottom of the tree pane (UX §10.3), so it never shrinks the cleanup pane.

Widths are adapted with `responsive(|size| ..)` around the content row:
- the side pane is `clamp(320, 34% of the window, 480)`, is hidden by default below 1000 px, and is hidden whenever showing it would leave the tree pane narrower than 440 px (UX §6.1);
- the Items column is hidden below a tree width of 600 px;
- the % text is hidden below 480 px;
- the selection bar uses short labels with tooltips ("Reveal", "Copy", "Rescan", "Trash…") when the full labels don't fit (D16).

### 6.5 Tasks and subscriptions

| What | Mechanism |
|---|---|
| Scan updates | `Task::run(rx, ..).abortable()` as in §4.4 |
| Trash, reveal, open URL, small-file listing, git checks | `Task::perform(async move { tokio::task::spawn_blocking(move || { platform::init_io_thread(); work() }).await }, Message::X)` (API §g) |
| Dropping a replaced tree | `spawn_blocking(move || drop(old))`, result ignored |
| Folder picker | Build the `rfd::AsyncFileDialog` future in `update` (main thread), then `Task::perform` (API gotcha 16) |
| Clipboard | `iced::clipboard::write(text)` |
| Keyboard | `keyboard::listen().filter_map(keys::map)`. `keys::map` is a fn item (API gotchas 2–3) |
| Drop, hover, resize, mouse buttons | `event::listen_with(fn)` matching `Window(FileHovered/FileDropped/FilesHoveredLeft/Resized)`, `Mouse(ButtonPressed(Back/Forward))`, and `Mouse(ButtonPressed/ButtonReleased(Left))` with either event status (rows and the scrollbar capture presses, so `event::listen()` would miss them) |
| Pointer movement | `event::listen_with` matching `Mouse(CursorMoved)` → `PointerMoved`, included **only while** a re-order can happen (a scan is active or a post-trash re-order is pending) **and** `pointer_in` is `Some` |
| Window id | `window::open_events()` (for screenshots) |
| Tick | `time::every(50 ms).map(Message::Tick)`, returned **only while** any of these hold: a scan is active (not draining); a git job or small-file listing is pending; `scan_rows_dirty`; a post-trash re-order is pending; a toast with an expiry is visible; a row flash is running; a trash job is pending; the "Opening…" or indeterminate sweep is visible; a screenshot or script step is pending. When idle, nothing wakes the app (API §h) |

Rows carry no `on_move` handler: a message per mouse move would rebuild the whole view at pointer rate.

What `Tick` does:
1. Rebuild `rows` if `scan_rows_dirty` and the last rebuild was ≥ 100 ms ago (anchored, §6.6).
2. Run the reorder clock (§6.7).
3. Update the activity model's current path (≤ 5 Hz).
4. Expire the toast and flash.
5. Start a pending git job (§5.6).
6. With the `screenshot` feature: take a due screenshot or run a due script step.

The pulsing scan dots and the indeterminate sweep compute their phase from `Instant::now()` in `view`, so the tick alone animates them.

### 6.6 Row virtualization

This follows API §h and probe `rows.rs`, with one change: the viewport height comes from layout, not from `on_scroll`. iced 0.14's scrollable publishes no `Viewport` while the content does not overflow (`scrollable.rs` `notify_viewport`), so an `on_scroll`-only viewport goes stale when the content shrinks or the window grows.

```rust
pub const ROW_H: f32 = 28.0;  const OVERSCAN: usize = 6;  const TREE_ID: &str = "tree";
responsive(move |size| {
    let height = size.height - HEADER_H;
    let offset = view.viewport.offset_y.min((rows.len() as f32 * ROW_H - height).max(0.0));
    let (first, last) = range(offset, height, rows.len());
    column![ header(..),
             scrollable(column![ space().height(first as f32 * ROW_H),
                                 rows[first..last].map(row_view),          // each exactly ROW_H tall
                                 space().height((rows.len() - last) as f32 * ROW_H) ])
                 .id(TREE_ID).on_scroll(Message::TreeScrolled).height(Fill) ]
})
```

- **Viewport state.** `TreeScrolled` stores `absolute_offset().y` and the bounds height. Every rows rebuild clamps `offset_y` to `max(0, rows.len()·ROW_H − height)`. Zoom, Back and Forward set `offset_y` explicitly (0, or the history entry) and return `operation::scroll_to` in the same update. A unit test covers `range()` after the rows shrink below the viewport height.
- **Spacers:** never use a `Fixed(0)` width on a spacer (API gotcha 8).
- **Rows** are `mouse_area(hover(container(row), overlay))`, never wrapping a `button` (gotcha 9):
  - `on_press` → select;
  - `on_double_click` → zoom (folders) or reveal (files).
  - The disclosure chevron is a `mouse_area` in the indent zone that sends `ToggleExpand`.
- **The header** is a separate `row` above the scrollable that uses the same width constants.
- **Building `rows`.** Do a depth-first walk from the zoom root over `EXPANDED` nodes, using each node's `ChildOrder`. Per directory, emit in this order:
  - its subdirectories;
  - its displayed files: files ≥ 1% of the directory or ≥ 100 MB, at most 10 (UX §6.3);
  - the aggregate row, placed by size when sorting by size, and last when sorting by name;
  - when the aggregate is expanded: the remaining kept files in batches of 50, then the loaded small files (`SmallFile` keys), then a `MoreFiles` row.
  - Trashed rows stay until the next re-order, zoom or sort (UX §9); a successful trash schedules one (§7.4).
- **Cost:** O(visible structure). It is never rebuilt per batch.
- **Anchoring on every rebuild.** Before any rebuild, record the anchor (the selected row if it is visible, else the first fully visible row) and its y within the viewport. After the rebuild, find the anchor's new index; if its y would change, set `offset_y = index × ROW_H − anchor_y` and return `operation::scroll_to(TREE_ID, AbsoluteOffset { x: None, y: Some(offset_y) })` from the same update. This keeps rows still when a `Placeholder` above the viewport is replaced by N children. Unit test: an expanded pending node above the viewport receives its listing, and the anchor's y stays the same.
- **Reveal** centers the row: `index × ROW_H − viewport.height / 2`.

### 6.7 Calm sorting (`results/order.rs`)

This implements UX §5.4:

```rust
pub enum ChildKey { Dir(NodeId), File(u32), Aggregate }
pub struct ChildOrder { pub keys: Vec<ChildKey> }

pub fn bucket(size: u64) -> i32 { if size == 0 { -1 } else { ((size as f64).ln() / 1.1f64.ln()).floor() as i32 } }

/// Initial order when a node is expanded or its listing first arrives: exact sort.
pub fn exact(tree: &Tree, id: NodeId, sort: Sort) -> ChildOrder;
/// Tick re-order: stable sort by quantized key, previous order kept within a bucket.
pub fn calm(tree: &Tree, id: NodeId, prev: &ChildOrder, sort: Sort) -> ChildOrder;

pub struct ReorderClock { last_tick: Instant, last_activity: Instant, pending_since: Option<Instant>, post_trash: bool }
impl ReorderClock {
    pub fn activity(&mut self, now: Instant);
    /// true when a re-order should run now: ≥ 1.5 s since the last one, and either no activity
    /// in the last 700 ms, no button held and no overlay open, or deferred ≥ 5 s.
    pub fn due(&mut self, now: Instant, overlay_open: bool, button_held: bool) -> bool;
}
```

- **Activity inputs.** `activity()` is called for: `PointerMoved` (only subscribed while a re-order can happen and the pointer is over the tree or cleanup pane), `TreeScrolled` when `absolute_offset().y` actually changed (a content-height change alone also publishes a viewport and must not count), every key handled by the tree or the cleanup pane, row presses, and cleanup clicks. `button_held` is `App.left_button_held`. `overlay_open` is true when a modal or the errors popover is open.
- **The scanner makes the "new rows" rule simple.** A directory's subdirectories and large files all arrive at once, in its own listing, so a parent never gains siblings later. When an expanded node's listing arrives, its children are placed with `exact` immediately: nothing was there yet. Existing siblings only move on ticks. This covers UX §5.4 rule 4 without the append-then-place logic; the anchoring in §6.6 keeps the visible rows still.
- **Reorder tick, while scanning or with a post-trash re-order pending:**
  1. For every entry in `orders` (expanded nodes + the zoom root), call `calm()`, then drop trashed keys.
  2. Rebuild `rows` (anchored).
  3. Rebuild the cleanup model: re-rank groups and place groups that appeared since the last tick. The cleanup pane uses the same clock, so its rows don't jump under the cursor either.
  4. Clear `post_trash`.
- **Exact sorts** happen on completion, on a header click, on zoom, and after a rescan splice, each anchored to the selection (rule 6).
- **Name sort** uses a case-insensitive natural order on `to_string_lossy()`.
- **Items sort** uses `total.items`.

### 6.8 Keyboard handling (`keys.rs`)

```rust
pub enum Platform { Mac, Other }
pub fn map_for(platform: Platform, event: &keyboard::Event) -> Option<KeyAction>;   // pure, table-tested
pub fn map(event: keyboard::Event) -> Option<Message>;  // thin wrapper: map_for(cfg platform).map(Message::Key)

pub enum KeyAction { Up, Down, PageUp, PageDown, Home, End, Right, Left, Enter, ZoomOut, Back, Forward,
                     Reveal, CopyPath, Trash, Rescan, RescanAll, Escape, NewScan, ChooseFolder,
                     PaneCleanup, PaneMap, TogglePane, NextPane, PrevPane, Space, Tab, ShiftTab }
```

The bindings are exactly UX §6.4, with `Modifiers::command()` for ⌘/Ctrl. Platform differences:
- ⌫ / ⌘↑ vs Backspace / Alt+↑;
- ⌥⌘R vs Shift+Alt+R;
- ⌘⌫ vs Delete;
- ⌘. vs (none) for Stop.

Letter keys are compared case-insensitively, and ⌘[ / ⌘] match the physical keys (`Code::BracketLeft`/`BracketRight`). With ⌘ held, winit on macOS derives the logical key from `charactersIgnoringModifiers`, which keeps Shift and the keyboard layout, so ⇧⌘R arrives as `Character("R")`. The table test includes uppercase characters for every shifted shortcut.

Routing in `update` (`Message::Key`):
1. If a modal is open: Esc → cancel; Return → the default button; Space → the focused button; Tab / Shift-Tab → move `focused_button` (tracked by us). The default button starts focused.
2. If the errors popover is open: Esc closes it.
3. Otherwise:
   - `Escape` → Stop if a scan is active;
   - pane keys switch the pane (`PaneMap` is a no-op until M12);
   - `NextPane`/`PrevPane` toggle `focus`;
   - navigation keys go to the focused pane (tree: move selection / expand / zoom; cleanup: move the cursor / expand group / activate);
   - `Trash` in the cleanup pane: see §7.1;
   - every key handled by either pane calls `ReorderClock::activity`.

There are no `text_input`s in v1, so `keyboard::listen()` sees every key press.

### 6.9 Theme, tokens and fonts

- `App::theme` always returns `None`. iced then uses the built-in Light or Dark theme and switches live with the OS (API gotcha 10, UX §11.1). Returning a custom theme would stop live OS-theme following.
- Because the built-in palettes differ from the UX tokens (built-in Dark background is `#2B2D31`, tokens say `#141413`), `theme::app_style(&App, &Theme) -> iced::theme::Style { background_color: tokens(theme).bg_window, text_color: tokens(theme).text_primary }` is set on the application, and the root container also sets `text_color`.
- `theme.rs` defines `pub struct Tokens { bg_window, bg_surface, bg_raised, bg_hover, bg_selected, bg_selected_unfocused, border_hairline, border_strong, text_primary, text_secondary, text_muted, accent, accent_text, accent_fill, bar_track, meter_track, status_good, status_warning, status_critical, chip_good, chip_warning, chip_critical, scrim, shadow }` with `const LIGHT` and `const DARK`. The values are from UX §11.2.
- `pub fn tokens(theme: &Theme) -> &'static Tokens { if theme.extended_palette().is_dark { &DARK } else { &LIGHT } }`.
- Every widget gets an explicit style fn built from the tokens: buttons, containers, the scrollable, tooltips, `progress_bar` for share bars and meters. Every struct literal ends with `..Default::default()` (API gotcha 6).
- **Indeterminate progress** (folder scans and "Opening…"; UX §5.2): iced's `progress_bar` has no indeterminate mode, so the 2 px sweep is `row![space().width(FillPortion(a)), container(space()).width(FillPortion(25)).style(accent), space().width(FillPortion(75 − a))]` with `a = phase × 75` and a 1.4 s period, driven by the tick (FillPortion values ≥ 1).
- **Strikethrough** for trashed rows: `rich_text([span(name).strikethrough(true)]).on_link_click(iced::never).wrapping(text::Wrapping::None)` inside the clipping container (`on_link_click(never)` fixes the otherwise uninferable `Link` type).
- Text truncation uses `widgets::truncate_end(s, max_px, size)` and `truncate_middle_path(..)`, estimating 0.56 em per glyph. They are combined with `Wrapping::None` and a clipping container (API gotcha 13, UX §12.6).
- Icons are the Unicode fallback set of UX §11.7 (`▸ ▾ › · — ≥ … ✓ ⌂`), drawn with `Shaping::Auto` and never `Basic`, always at Regular weight in their own `text` (§6.1). The ⚠ badge is drawn as a small amber `container` with a dark "!" (avoiding the emoji ⚠).

### 6.10 Treemap (optional milestone M12)

If it is built, it follows UX §8 and the probe's `treemap.rs`:
- a `canvas` `Program` with two `canvas::Cache`s (blocks and overlay), from the squarify routine;
- 3 levels deep, 3 hues + gray;
- relayout at most once per second while scanning, deferred while the pointer is over the map;
- caches cleared when the data or the theme changes (API gotcha 11);
- selection shared with the tree.

It needs the `canvas` iced feature, `SidePane::Map` and the tab bar (which then also shows the "Cleanup 38 GB" label). Nothing else in the design depends on it.

---

## 7. Delete / Trash flow

### 7.1 Flow

```
trigger: selection bar button, or ⌘⌫/Delete on the tree selection,
         or ⌘⌫/Delete on a focused cleanup child row or single-match group
  → plan = trash::plan(target)                     (UI thread, pure: tree, catalog, git status, cached guards)
      ├─ Refused(reason)  → Modal::Refuse  (only [OK]; UX §9 wording)
      └─ Plan             → Modal::ConfirmTrash(plan)   (always; UX §9 dialog)
  → ConfirmTrash → job = TrashJob { id, tree_gen, items }  (a value; stored in trash_jobs)
                 → flags TRASH_PENDING on the rows ("Moving to Trash…")
                 → Task::perform(spawn_blocking(core::trash::execute(&job, &guards, &*trasher)))
  → TrashFinished(id, gen, outcomes) → stale gen: toast only, no tree change
                                     → per item: success → tree accounting + toast + post-trash re-order
                                                 failure → clear pending, error toast (sticky)
```

The cleanup pane has no trash buttons in v1 (hover buttons and bulk trash are v1.1). Its only trash path is ⌘⌫/Delete on a focused child row or a single-match group, which plans the same `TrashTarget` the tree would. On a multi-match group it shows the toast "Select one folder in the group to move it to the Trash."; on a ToolOnly rule it shows "Use the command instead: <command>" with [Copy] (D8).

```rust
// dir-stat::results::trash
pub enum TrashTarget { Dir(NodeId), File(NodeId, u32), SmallFile(NodeId, u32) }
pub struct TrashPlan {
    pub target: TrashTarget,
    pub items: Vec<core::trash::Item>,                  // 1 for Item mode; Contents expands here (§7.2 G9)
    pub title: String, pub facts: String,               // "24.1 GB · 312,004 items · modified 2 days ago"
    pub reason: Option<(Option<Section>, String)>,      // from display_section (§5.3)
    pub warnings: Vec<String>,                          // §7.3
    pub default_is_trash: bool,
}
// Candidate-ness comes from the node's verdict / the file's match, never from the trigger.

// dirstat-core::trash
pub enum EntryKind { Dir, File, Symlink, Other }
pub struct Identity { pub dev: u64, pub ino: u64, pub kind: EntryKind,
                      pub len_mtime: Option<(u64, i64)> }   // Windows only: weaker check (D12)
pub enum Mode { Item, Contents { entries: Vec<ContentsEntry>, scan_started: SystemTime } }
pub struct ContentsEntry { pub name: Box<OsStr>, pub expect: Identity }   // scanned subdirs + large files
pub struct Item { pub path: PathBuf, pub expect: Identity, pub mode: Mode }
pub struct TrashJob { pub id: TrashJobId, pub tree_gen: u32, pub items: Vec<Item> }
pub enum TrashOutcome {
    Trashed(PathBuf),
    Contents { path: PathBuf, outcome: ContentsOutcome },
    Failed(PathBuf, TrashError),
}
pub struct ContentsOutcome { pub trashed_dirs: Vec<Box<OsStr>>, pub files_alloc: u64, pub files_apparent: u64,
                             pub files_count: u64, pub left_in_place: u32, pub failures: Vec<(PathBuf, TrashError)> }
pub enum TrashError { Refused(Refusal), Changed, NoTrash, Os(String) }

pub trait Trasher: Send + Sync { fn trash(&self, path: &Path) -> Result<(), TrashError>; }
pub struct SystemTrasher;   // macOS: trash crate + NsFileManager; Windows: trash crate; Linux: §7.5
pub fn execute(job: &TrashJob, guards: &safety::Guards, trasher: &dyn Trasher) -> Vec<TrashOutcome>;
```

Tests use a `RecordingTrasher` (records paths, or renames into a temp "trash" dir), so every guard case runs in plain `cargo test` (§10.2).

### 7.2 Safety guards (all must pass; checked in `plan` and again in `execute`)

The guards are defined in `dirstat-core::safety`. `Guards::new()` runs once at startup and canonicalizes everything it stores: home, the XDG user dirs, every G3/G4 path that exists. It also records the `(dev, ino)` of every G4 folder-itself path that exists, so a symlinked or differently cased path to the same folder is still caught in `check_live`.

```rust
pub struct Guards { home: PathBuf, protected_tree: Vec<PathBuf>, protected_self: Vec<PathBuf>,
                    protected_ids: HashSet<(u64, u64)> }
pub enum Refusal { Protected(&'static str), ScanRootOrAncestor, MountPoint, ContainsOtherVolume,
                   ReadOnlyVolume, ImmutableFlag, SystemVolumes }
pub fn check(guards: &Guards, scan_root: &Path, path: &Path) -> Result<(), Refusal>;   // pure path rules
pub fn check_live(guards: &Guards, path: &Path, expect: &Identity) -> Result<(), TrashError>; // syscalls
```

| # | Guard | Where |
|---|---|---|
| G1 | **Eligibility.** The target node is `Done` (not `Removed`); neither it nor any ancestor is `TRASH_PENDING`, `TRASHED` or `RESCANNING`; it is not an aggregate, placeholder, Unaccounted, skipped (mount / other volume / cloud / excluded / linked) or unreadable row; `total.boundaries == 0`, i.e. nothing inside it is another volume (`ContainsOtherVolume`: "“X” contains another volume (…). dir-stat won't move it to the Trash."); no scan or rescan is running inside it | UI, `plan` (UX §5.5, §6.3) |
| G2 | **Scan root and its ancestors** are never trashed | `check` |
| G3 | **Protected paths, whole subtree:** macOS `/System`, `/usr` except `/usr/local/**`, `/bin`, `/sbin`, `/private/etc`, `/dev`. Linux `/bin`, `/boot`, `/dev`, `/etc`, `/lib*`, `/proc`, `/run`, `/sbin`, `/sys`, `/usr` except `/usr/local/**`. Windows `C:\Windows` | `check` (UX §9) |
| G4 | **Protected paths, folder itself:** macOS `/`, `/Library`, `/Applications`, `/Users`, `/Volumes`, `/private`, `/private/var`, `/private/tmp`, `/opt`, `/cores`, `/etc`, `/var`, `/tmp`, `~`, `~/{Library, Desktop, Documents, Downloads, Movies, Music, Pictures, Public, Applications, .Trash}`, `~/Library/Mobile Documents` and each of its direct children (iCloud Drive and app containers), `~/Library/CloudStorage` and each of its direct children (File Provider roots). Linux `/`, `/home`, `/root`, `/srv`, `/var`, `/opt`, `~`, the XDG user dirs. Windows drive roots, `C:\Program Files*`, `C:\ProgramData`, `C:\Users`, `%USERPROFILE%`, `pagefile.sys`/`hiberfil.sys`/`swapfile.sys` | `check` by path; `check_live` by `(dev, ino)` (UX §9 ∪ FS §9.3) |
| G5 | **Volumes.** Path rules: the target is not a mount point in the cached table and not under `/System/Volumes/*` (`SystemVolumes`). Live: `statfs`/`statvfs(path)` has no `MNT_RDONLY`/`ST_RDONLY` (`ReadOnlyVolume`; a path rule can't work on macOS, where `/` is the read-only System snapshot but firmlinked `/Users` lives on the writable Data volume); a fresh `platform::mounts()` has no mount equal to the target or strictly below it (`MountPoint` / `ContainsOtherVolume`; this catches mounts created after the scan) | `check` + `check_live` |
| G6 | **TOCTOU re-check** immediately before moving: `symlink_metadata(path)` must still have the same `dev`, `ino` and `EntryKind` (a row that was a symlink at scan time may still be trashed as a link; anything else that has become a symlink is `Changed`). On Windows: kind, plus length and mtime for files (D12). Otherwise `Changed` ("changed since the scan; rescan") | `check_live` in `execute` |
| G7 | **macOS flags:** refuse `SF_RESTRICTED`, `SF_NOUNLINK`, `SF_IMMUTABLE`, `UF_IMMUTABLE` (`st_flags`) | `check_live` |
| G8 | **No permanent delete, ever.** No code path removes data. macOS `trashItemAtURL` never deletes; its failures are reported as `Os(description)` (the crate only gives text, and we don't string-match it). Linux uses the rename-only trash (§7.5), which returns `NoTrash` ("This volume has no Trash. dir-stat doesn't delete files permanently.") when no trash directory on the target's filesystem is usable | `execute` |
| G9 | **Contents mode.** `plan` expands the entries from the tree: the scanned subdirectories (name + ino) and large files. At execution, after G6 on the folder (and a `SF_DATALESS` check), `read_dir` it and, per entry: a directory is trashed only if it matches a planned entry by name and identity; a file is trashed only if it matches a planned large file, or (small files, which the tree doesn't list) its mtime is before `scan_started`. Each trashed entry passes the full `check` and `check_live` (mount, flags, identity). Symlinks are trashed as links (the trash crate canonicalizes only the parent; FS §9.1). Entries that are new or changed are left in place and counted in `left_in_place`. The folder itself stays | `plan` + `execute` |

G2–G5's path rules compare **exact components** of canonical paths: no case folding on macOS (FS §6.2), and case-insensitive on Windows. Tree paths are canonical because they are built from the canonical scan root plus raw names; the guard paths are canonicalized at startup. Case and symlink variants that slip past the path rules are caught by the `(dev, ino)` check in `check_live`.

### 7.3 The confirmation dialog

It follows UX §9: title, path, facts, the reason line (§5.3), the line "The space is freed when you empty the Trash." and [Cancel] [Move to Trash].

Warning lines, shown when they apply:
- the target is inside a package (an ancestor is a bundle by extension, and for bundle types `markers & (BUNDLE_CONTENTS | INFO_PLIST | BUNDLE_VERSIONS)`; FS §6.6);
- the target or an ancestor has a `Protected(rule)` verdict: "<rule name>: <reason>" (§5.4);
- the candidate is in Review first, worded by cause (§5.3): Low → "Restored only by re-downloading or rebuilding: <reason>"; Review → "May contain data you want: <reason>"; not ignored by git → "Not ignored by git in <repo>: it may contain your work."; git unavailable → "Couldn't check with git."; git-tracked (`Hidden`) → "Contains files tracked by git in <repo>.";
- `total.cloud_items > 0` (or the target is itself dataless): "N items inside are stored only in iCloud. Moving it to the Trash removes them from iCloud on all your devices and frees almost no space here.";
- `total.error_dirs > 0`: "N folders inside couldn't be read; their contents are unknown.";
- the catalog has a `caution`;
- the catalog flags `shares_space`, or `linked_alloc ≥ 10%` of the target's size (or a file target has `nlink > 1`): "Some files are shared with other folders (hard links or clones); this may free less than shown.";
- the rule's removal is `ToolOnly` (D8): "Recommended instead: `<command>`";
- the target is more than 25% of the scan root.

`default_is_trash` is true only when all of these hold:
- the target's display is `Shown { SafeToDelete, Catalog }` (so git is `NotInRepo` or `Clean`; `Pending` never qualifies);
- its removal mode is `Item` or `Contents`;
- there are no warning lines.

Otherwise Return activates Cancel; Tab moves the focus to the destructive button and Space activates it (UX §9).

### 7.4 After the move

**Stale results.** If `TrashFinished.tree_gen` differs from `Results.tree_gen` (a new full scan or a tree replacement happened meanwhile), the outcome only produces the toast; the tree is not touched.

**Success:**
- Accounting, all at once and saturating, never below a `TRASHED` ancestor or on a `Removed` node:
  - folder: `tree.mark_trashed(id)` subtracts the target's totals from every ancestor;
  - large file: `tree.mark_file_trashed(dir, index)`;
  - small file: `tree.subtract_small_file(dir, alloc, apparent, nlink > 1)` and `TRASHED` on the `SmallFile` entry;
  - contents: `tree.subtract_contents(id, &outcome)` with the measured amounts (trashed subdirs by their tree totals, files by their measured sizes), so a partial result is exact. If `left_in_place > 0`, the toast says so.
- The row stays, struck through and muted, with the Size cell reading "In Trash".
- Set `post_trash` on the reorder clock: one anchored calm re-order of the affected ancestors' child orders and the cleanup model runs when the clock allows (§6.7), even after the scan has completed. That re-order removes struck-through rows and restores "biggest first".
- Toast: "Moved “X” to the Trash. Empty the Trash to free 24.1 GB." If a shared-space warning applied, "to free up to 24.1 GB".
- If `~/.Trash` is a node in the tree, set its `OUTDATED` flag (tooltip "Rescan to update").
- If the zoom root was trashed, zoom out to the nearest surviving ancestor and show a toast.

**Failure:** clear the pending flag and show a sticky error toast with the OS message (UX §9).

### 7.5 Linux trash (`trash/freedesktop.rs`, D14)

A rename-only implementation of the freedesktop Trash spec, so no path can copy across devices or call `remove_dir_all`:
1. Find the target's mount from `/proc/self/mountinfo` (longest mount-point prefix of the canonical path, confirmed by `st_dev`).
2. If it is the mount of `$XDG_DATA_HOME/Trash` (created if missing): use the home trash.
3. Otherwise, per the spec: `$topdir/.Trash/$uid` if `$topdir/.Trash` exists, is a directory, is not a symlink and has the sticky bit; else `$topdir/.Trash-$uid` (created with mode 0700 if possible). If neither is usable → `NoTrash`.
4. The chosen `files/` directory must have the target's `st_dev`; otherwise `NoTrash`.
5. Write `info/<name>.trashinfo` with `O_CREAT | O_EXCL` (unique name with a numeric suffix on collision; `Path=` percent-encoded, absolute for the home trash, relative to `$topdir` for a volume trash; `DeletionDate=`), then `rename(target, files/<name>)`. On any rename error, including `EXDEV` (bind mounts of the same filesystem), remove the info file and return `Os(..)`. Never copy.

The trash-directory choice is a pure function of (mount table, target dev, home, uid, directory probes) and gets a table test. A Linux-container test trashes a file on `/dev/shm` (tmpfs, a different filesystem from the container's overlay root): it must land in `/dev/shm/.Trash-<uid>`, never in the home trash.

---

## 8. Platform abstraction

All OS-specific code lives in `dirstat-core::platform`, behind one set of signatures (FS §4.6). The app's only OS-specific bits are:
- labels: "Reveal in Finder" / "Show in File Manager" / "Show in Explorer", "Move to Recycle Bin…";
- key bindings (`keys.rs`).

```rust
// dirstat-core::platform (mod.rs re-exports the cfg-selected implementation)
pub fn init_process();
pub fn init_io_thread();
pub struct RawStat { pub mode: u32, pub dev: u64, pub ino: u64, pub nlink: u64, pub blocks: u64,
                     pub size: u64, pub mtime: i64, pub flags: u32 }
pub struct EntryMeta { pub kind: EntryKind, pub dev: u64, pub ino: u64, pub nlink: u64,
                       pub alloc: u64, pub apparent: u64, pub mtime: i64, pub dataless: bool, pub flags: u32 }
pub fn meta_from_raw(raw: &RawStat) -> EntryMeta;   // pure, unit-tested (dataless, flags, sizes)
pub fn entry_meta(m: &std::fs::Metadata) -> EntryMeta;
pub fn classify_error(e: &io::Error) -> ErrorClass;
pub struct Mount { pub path: PathBuf, pub fs_type: String, pub local: bool, pub read_only: bool,
                   pub browsable: bool, pub automounted: bool, pub dev: Option<u64>, pub space: Option<Space> }
pub struct Space { pub total: u64, pub used: u64, pub free: u64, pub inodes_used: Option<u64> }
pub fn mounts() -> Vec<Mount>;                 // never blocks; space only where safe (see below)
pub fn space_of(path: &Path) -> Option<Space>; // statfs/statvfs of a LOCAL path (scan roots only)
pub fn is_read_only(path: &Path) -> io::Result<bool>;   // statfs f_flags / statvfs f_flag (G5)
pub fn home_dir() -> Option<PathBuf>;
pub fn system_disk_name() -> String;           // "Macintosh HD" (via /Volumes/* -> / symlink), "Computer", "Local Disk (C:)"
pub fn full_disk_access() -> Option<bool>;     // macOS: opendir("/Library/Application Support/com.apple.TCC") ok?
pub const PRIVACY_SETTINGS_URL: Option<&str>;  // macOS 13+ deep link (D6); None elsewhere
pub fn launched_from_terminal(own_bundle_id: Option<&str>) -> Option<String>;
    // macOS: TERM_PROGRAM first; else __CFBundleIdentifier unless it equals our own bundle id
    // (LaunchServices sets it for every app, including dir-stat.app started from Finder)
pub const SIZE_IS_ALLOCATED: bool;             // false on Windows -> column label "Size"
```

| | macOS (`macos.rs`) | Linux (`linux.rs`) | Windows (`windows.rs`) |
|---|---|---|---|
| `entry_meta` | `blocks()*512`, `st_flags` (`MetadataExt::st_flags`), `dataless = flags & SF_DATALESS` | `blocks()*512` | `len()` for both sizes, `dev = 0`, `ino = 0`, `nlink = 1` (no hard-link dedupe) |
| process/thread init | trigger-resolve off (process), RLIMIT_NOFILE; per I/O thread: materialize off, atime off | RLIMIT_NOFILE | none |
| `mounts()` | `getmntinfo(MNT_NOWAIT)`. Space comes from the same cached statfs (never blocks): `used = (f_blocks − f_bavail)·bsize` (container view; UX §4), `inodes_used = f_files − f_ffree` (per volume) | mountinfo text. Space: `statvfs` only for local block-device fs types (ext2/3/4, xfs, btrfs, f2fs, vfat, exfat, ntfs3, zfs) (D11); `used = (f_blocks − f_bfree)·frsize` | empty |
| start-screen volumes | `/` + `/Volumes/*` that are browsable and not the symlink to `/` | `/` and local filesystems mounted under `/home`, `/media`, `/mnt`, `/run/media` + network mounts (no meter); deduplicated by dev | Home + `%SystemDrive%` (no meter) (D12) |
| reveal / open | `opener::reveal` (`open -R`) / `opener::open(url)` | `opener::reveal` (D-Bus FileManager1, falls back to the parent) | `opener::reveal` |
| trash | `trash` crate, NsFileManager | own rename-only freedesktop trash (§7.5) | Recycle Bin (`trash` crate) |

The `setiopolicy_np`/`getiopolicy_np` declarations and the missing flag constants (`SF_DATALESS`, `SF_RESTRICTED`, `SF_NOUNLINK`, …) are copied from FS §2.11.

`volumes.rs` builds the start-screen cards and the scan estimate:

```rust
pub struct VolumeCard { pub path: PathBuf, pub name: String, pub kind: CardKind /* Home|System|External|Network */, pub space: Option<Space> }
pub fn start_cards() -> Vec<VolumeCard>;
pub struct ScanEstimate { pub items: Option<u64>, pub bytes: Option<u64>, pub container_used: Option<u64> }
/// Some only if `root` is a volume root. macOS "/" sums "/" and "/System/Volumes/Data" (D3).
pub fn estimate(root: &Path, mounts: &[Mount]) -> Option<ScanEstimate>;
```

`estimate` uses only the cached mount table, so the app can call it on the UI thread. Progress for a volume-root scan is `min(0.99, (dirs_listed + files) / items)`, falling back to `alloc / bytes` when there are no inode counts (btrfs). Folder scans get the indeterminate sweep (UX §5.2, §6.9).

---

## 9. Error handling and the Full Disk Access UX

- **The scanner never fails a scan because of an unreadable entry.**
  - An unreadable directory becomes a node with `error`: size "—" (or what was read), a filled warning badge, not expandable, and the tooltip text from UX §10.1 by `ErrorClass` (`Internal`: "dir-stat hit an internal error here.").
  - Ancestors show the outline badge when `total.error_dirs > 0`.
  - Vanished entries and directories are counted only in the activity-field tooltip ("N items changed during the scan"); a vanished directory row is muted with the tooltip "Removed during the scan".
- **The activity field** shows "Opening…" until `Started`, "· N unreadable" while scanning, and the link "N folders couldn't be read" after the scan ends or is stopped. The current path comes from `Progress.current` via `path_of`; if the id is not in the tree yet, the previous path stays. When the in-flight job has run ≥ 1 s, the field reads "Waiting on <path>…". The link opens the errors popover (UX §10.2): the first 200 of `tree.errors()` with reasons, a [Copy] button that copies all recorded "path\treason" lines, "and N more" past 200, and rows that reveal in the tree.
- **The FDA info bar** (macOS only) appears on the first `PrivacyBlocked` error of a scan (`ApplyEffect.first_privacy_error`). It sits at the bottom of the tree pane (§6.4), with [Open Privacy Settings] and [Not Now]. If `launched_from_terminal(own_bundle_id)` returns an app name, the text names that app (UX §10.3). "Not Now" sets `Dismissed` for the session.
- **Root unreadable:** if the root listing has an error, the tree area shows the full-pane state (UX §10.4) with [Open Privacy Settings] (EPERM only), [Choose Another Folder] and [Try Again].
- **Start-screen footnote:** shown on macOS unless `full_disk_access() == Some(true)`. The Home card shows the primer line from UX §4.
- **Invalid CLI or dropped path:** the Results screen shows "Opening…" briefly, then `Update::Failed` returns to the start screen with the banner "Can't scan “/foo”: No such folder." (from `StartError`).
- **Internal errors.** Platform actions (reveal, open URL, clipboard) fail with an error toast and never panic. A panic inside a scan job is caught (§4.2): the directory shows the `Internal` error badge and the scan finishes normally.
- **TCC identity.** For TCC to remember grants, the app must run as a signed `.app` bundle with a stable identity and the purpose strings from FS §5.2. `scripts/bundle-macos.sh` (M11) writes `Info.plist` with:
  - `NSDesktopFolderUsageDescription`, `NSDocumentsFolderUsageDescription`, `NSDownloadsFolderUsageDescription`;
  - `NSNetworkVolumesUsageDescription`, `NSRemovableVolumesUsageDescription`;
  - `NSFileProviderDomainUsageDescription`, `NSAppDataUsageDescription`.

  The script then signs the bundle with the identity in `$DIRSTAT_SIGN_IDENTITY`, falling back to an ad-hoc signature with a warning that grants won't survive rebuilds.

---

## 10. Testing strategy

### 10.1 Unit tests (`cargo test --workspace`, fast, no GUI)

| Module | What is tested |
|---|---|
| `tree` | Built from synthetic `DirListing`s via `TreeBuilder`. Out-of-order id blocks. Completion propagation; root `Done` iff all listed. `Skipped` children never block completion. Totals equal the sum of listings. `error_dirs`, `boundaries`, `cloud_items`, `linked_alloc` and `newest_mtime` propagation. `mark_stopped`. `mark_trashed`, `mark_file_trashed`, `subtract_small_file`, `subtract_contents` arithmetic, including no-ops on `Removed` nodes and below trashed ancestors, and saturation. `splice` (totals before/after, id remap, tombstones are `Removed`, candidates and errors re-derived, link owners). **Stop, then splice a complete sub-tree into the only unfinished child: the ancestors become `Done`.** `compacted` preserves totals and paths. `relative_path`/`find_relative`/`path_of`/`find_path` round trip with non-UTF-8 names (Linux). `size_of::<Node>() <= 176` |
| `catalog` | The prototype's 9 tests (catalog invariants: reason ≤ 70 chars and ends with "."; projects and exclusions; anchored containers and files; more heuristics; fixtures and git; enter mid-tree; Linux and Windows paths; env overrides and markers), ported. **New:** `section_for` and the full `display_section` table with the per-risk and per-cause warning text (the D1 test); no Protect rule, for directories or files, ever produces a candidate; `context_for` reproduces the scan-time verdicts for a fixture tree (classify at scan time == classify via stored markers); `root_context` on synthetic paths, including `/`; `Resolver::canonicalized` with a symlinked home |
| `safety` | Table-driven: every G3/G4 path per OS, with an injected home and scan root; `/usr/local/x` allowed; `/Applications/Foo.app` allowed but `/Applications` refused; scan-root ancestors refused; `~/Library/Mobile Documents/com~apple~CloudDocs` refused; `/System/Volumes/Data/x` refused |
| `trash` (core) | With `RecordingTrasher` and a temp dir: file and directory trashed; `Changed` after replacing a dir with a symlink between plan and execute; a symlink row trashed as a link; G7 flags via `meta_from_raw`; contents mode with a new entry (left in place), a changed subdir (left in place) and a partial failure (exact `ContentsOutcome`); a fixture dir declared as a mount below the target is refused (`ContainsOtherVolume`); the Linux trash-dir choice table (§7.5) |
| `platform` | `meta_from_raw`: dataless, immutable flags, sizes. Linux mountinfo octal unescaping (string fixture). `launched_from_terminal` ignores its own bundle id |
| `format` (app) | Sizes (3 significant digits, rollover at 999.6 MB, "0 B", "≥ "), counts (8,912 / 12.3k / 1.21M), percent (<0.1%), elapsed and relative time (UX §12) |
| `order` (app) | `bucket` hysteresis (two sizes within 10% never swap on `calm`), stability, exact sort on completion, name natural order. `ReorderClock`: no re-order within 700 ms of activity, none while a button is held or an overlay is open, at most one forced re-order per 5 s, `post_trash` runs after completion |
| `keys` (app) | Every binding in UX §6.4 via `map_for(Platform::Mac / Other, ..)`, including uppercase characters for shifted shortcuts and physical-key brackets |
| `rows` (app) | The flattening display rule for files (1% / 100 MB / top 10), aggregate placement by sort, `SmallFile` rows, the trashed-row lifetime, `range()` after the rows shrink below the viewport, anchoring when a placeholder above the viewport expands |
| `cleanup` (app) | `build_cleanup`: stat block is Σ Safe only; Low → Review first with the "Re-downloadable" tag; `Pending` and `Hidden` absent; pills follow `display_section` |
| `trash plan` (app) | `default_is_trash` truth table (Safe + `Catalog` + Item/Contents + no warnings only; `Pending` never); every warning line's trigger; `ContainsOtherVolume` refusal; G1 with trashed/pending ancestors |
| `migrate` (app) | Expand, select and zoom inside a target, splice: expansion, selection and history survive; keys that vanish fall back to the nearest ancestor |

### 10.2 Integration tests with fixture filesystems (`crates/core/tests/`)

Fixtures are built in `tempfile::TempDir`. Port scan-bench `tests.rs`. Every scan in these tests passes `context: Some(RootContext::detached(&engine))`: on macOS `$TMPDIR` canonicalizes to `/private/var/folders/…`, which the catalog's `mac-system` rule makes a NoHeuristics zone, so `root_context()` would yield zero candidates. Fixtures must also not live under the project's `target/` dir, which is itself a `cargo-target` candidate. The fixture contains:
- hard links within and across directories;
- a directory symlink (must not be followed) and a dangling symlink;
- an empty directory;
- a `chmod 000` directory (expect `PermissionDenied` with errno EACCES, counted as one error; skipped when running as root);
- a 2 MiB file (the only large file);
- a 3,000-file directory;
- `mnt/` declared as a mount through `Boundaries::Fixed`, containing a `chmod 000` directory (a stat would show up as an error), and `skipme/` declared excluded.

Tests on this fixture:
- **Streaming invariants:** `Started` first, parent before child, exactly once, nothing after `Finished`.
- **Accounting:** model total == an independent lstat walk == `Summary`.
- **du parity:** on Unix, compare against `du -skx` when `du` exists (KiB-exact; `mnt/` excluded from the comparison).
- **Boundaries:** `mnt` is `Skip::MountPoint`, nothing below it is listed, the root's `total.boundaries == 1`; `skipme` is `Skip::Excluded`.
- **Vanished:** a sink that deletes a directory when its parent's listing arrives: that directory's listing has `vanished: true`, no error, and `Progress.vanished > 0`.
- **Start failures:** a missing root yields `Failed(NotFound)` and nothing else.
- **Panic safety:** a test-only hook (`#[cfg(test)]` rule or name trigger) that panics on one directory: `Finished` arrives, that node has an `Internal` error, all others are complete.
- **Cancellation:** `Finished { cancelled: true }` within 1 s, and dropping the receiver stops the scan.
- **Hard links across rescans:** scan, rescan a subtree that shares links with a sibling via `counted_elsewhere`: the spliced totals equal a fresh full scan.
- **Catalog fixture:** `proj/Cargo.toml` + `proj/target/CACHEDIR.TAG` (valid signature) + `.rustc_info.json` → `target` is `Target(cargo-target)`, nothing inside it is a candidate, `proj/node_modules` next to `package.json` → Low → Review first.
- **Git fixture** (skipped if git is unavailable): `.git` + an untracked ignored `target` → `GitStatus::Clean`; a tracked `target/x` → `Tracked`; a non-ignored candidate → `NotIgnored`.
- **Real trash smoke test:** one `#[ignore]`d test that sends a temp file through `SystemTrasher` (it writes to the user's real Trash; run manually in M8 with `cargo test -- --ignored`).
- **Rescan splice end-to-end:** scan, modify a subtree, rescan the subtree, compare with a fresh full scan.

Linux: run `cargo test -p dirstat-core` in a Linux container (`docker run --rm -v $PWD:/w -w /w rust:latest cargo test -p dirstat-core`) when Docker is available; the `/dev/shm` trash test (§7.5) runs there. Also add a GitHub Actions matrix (macos-latest, ubuntu-latest, windows-latest: `cargo test -p dirstat-core`, `cargo build -p dir-stat`, `cargo clippy --all-targets -- -D warnings`), to be used once the project is in a git repository.

### 10.3 App-level tests without rendering

`App::new(Env { .. })` is constructed with an in-memory `PrefsStore`, a fixed volume list, a `RecordingTrasher`, and trees from `dirstat_core::tree::testing::TreeBuilder` (feature `test-util`). Scans are simulated by registering an `ActiveScan` with `ScanHandle::inert()` and feeding `Message::Scan(id, Update::..)` directly; the returned `Task`s are ignored. These tests check:
- Stop keeps partial nodes, marks the tree stopped at once, and enables Rescan **even if `Finished` never arrives**;
- Stop during a subtree rescan leaves `phase == Complete` and clears `RESCANNING`;
- `Failed` for a full scan returns to the start screen with the banner; for a subtree, the "no longer exists" toast;
- the trash button is disabled for non-`Done` rows and for folders containing another volume;
- no `TrashJob` exists before `ConfirmTrash`; Return in the dialog does nothing destructive for non-Safe items; Tab + Space does trash them;
- a stale `Message::Scan(old_id, ..)` is ignored, and a `TrashFinished` with an old `tree_gen` changes nothing in the tree;
- Rescan is refused while a trash job is pending inside the target;
- trash pending on a child, rescan of a sibling, then `TrashFinished`: totals stay consistent;
- zooming pushes history and Back restores selection and scroll; after a splice, history entries still resolve.

### 10.4 Visual checks (with the `screenshot` feature) and manual tests

Each UI milestone's acceptance includes screenshots in light and dark (`ICED_THEME=Light|Dark DIRSTAT_SCREENSHOT=…`), using `DIRSTAT_SCRIPT` to reach dialogs, popovers and other states. Manual test checklist before release (macOS unless noted):
1. Scan `/` without FDA: there is no double count of `/System/Volumes/Data`, the total ≤ the container used, Unaccounted is shown, and the FDA bar appears.
2. Grant FDA to the signed `.app` and rescan: the EPERM errors are gone.
3. A fresh Home scan triggers the Desktop, Documents and Downloads prompts; the app stays responsive while they are up, and Stop works.
4. An external disk and a DMG show as cards; inside a `/` scan they appear as "other volume" rows; a folder on the DMG is refused as read-only.
5. An iCloud Drive folder with "Optimize Storage": nothing downloads, and dataless folders show as "Cloud folder – not scanned"; trashing a parent of such content shows the iCloud warning with Cancel as default.
6. Trash a Safe candidate (Return confirms), a Low candidate and a Review candidate (Return cancels; Tab + Space confirms), a protected folder (refused), and a folder replaced by a symlink after the scan (Changed).
7. Stop mid-scan on `~`: "≥" sizes at once, then Rescan of a stopped folder: its ancestors become complete when it was the last unfinished child.
8. Linux VM (GNOME): theme switching, reveal, trash (home and a second disk), mounts, `/proc` skipped, a folder with a live bind mount below it is refused.
9. Windows (best effort): builds, scans `C:\Users\<me>`, trash to the Recycle Bin.
10. 2M-item scan: the UI stays ≥ 30 fps (scrolling while scanning feels smooth); memory stays within §3.3, also after three ⌘R on the root.

---

## 11. Implementation plan

Each milestone is one implementation pass. "Done" means: `cargo build`, `cargo test --workspace` and `cargo clippy --all-targets -- -D warnings` are clean, and the acceptance criteria hold.

**Cross-target checks** (used in M1–M3): only `aarch64-apple-darwin` is installed on the dev machine, and an offline pass can't run `rustup target add`. Use the engine-check variant: `RUSTC_BOOTSTRAP=1 cargo check -p dirstat-core -Zbuild-std=std,panic_abort --target x86_64-unknown-linux-gnu` (SCAN §8, `probes/engine-check`), or `rustup target add x86_64-unknown-linux-gnu` when the network is available, or the Docker run from §10.2. The Windows check (`x86_64-pc-windows-msvc`) is optional.

**M0 — Workspace skeleton, fonts, theme, formatting, screenshot harness**
- Workspace with both crates, profiles, deps (§2).
- `fonts.rs` with the three Inter TTFs (download the official Inter 4.x release; include OFL.txt).
- `theme.rs` tokens and `app_style`, `format.rs` with tests.
- An empty iced window (1200×800, min 800×500, default text size 13 via `.settings`) that shows a sample of the type scale and every fallback glyph at weights 400/500/600.
- `screenshot` feature: `DIRSTAT_SCREENSHOT`, `ICED_THEME`, `DIRSTAT_SCRIPT`; the tick runs while a screenshot is pending.
- *Accept:* screenshots (light and dark via `ICED_THEME`) show the 600-weight title in Inter (not Menlo or Apple SD Gothic Neo; compare with probe shot `03-fonts.png`), the token window background (not `#2B2D31`), and fallback glyphs rendered at Regular weight; an idle window still takes its screenshot and exits; `format` tests cover every UX §12 example.

**M1 — Event types and tree model**
- `scan` event types (§3.1) without the scanner.
- `tree/*`: `apply`, completion, totals and counters, `newest_mtime`, `candidates`, `errors`, path functions, the chunked arena, `mark_stopped`, trash accounting, `splice` with completion propagation, `compacted`, `counted_elsewhere`.
- `tree::testing::TreeBuilder` (feature `test-util`): nested literals → `DirListing`s with correct id blocks, verdicts and markers.
- *Accept:* the §10.1 tree tests, all built from synthetic listings; the cross-target check for Linux.
- *Done 2026-11-13.* Deviations:
  - The catalog prototype (`model`, `table`, `engine`) was copied into `catalog/` already, unchanged apart from two clippy fixes and `impl Default for Ctx`, so the tree uses the real `Verdict`/`RuleIdx`/`Ctx` types. The §5.5 additions stay in M3.
  - `Node` stores `own_mtime` next to `newest_mtime`, so `splice` can recompute `newest_mtime` exactly. To stay within 176 bytes, both are kept in whole minutes (`i32`); `newest_mtime()` still returns seconds.
  - Added `Tree::is_lower_bound(id)` (stopped and not `Done`) for the "≥" rendering.
  - `trash::ContentsOutcome` and `TrashError` exist with what accounting needs; `TrashError::Refused` arrives with `safety` in M8. Open point for M8: `subtract_contents` doesn't flag the target's large files as `TRASHED`, because `ContentsOutcome` doesn't name trashed files.

**M2 — Scanner**
- `scan/*` ported from scan-bench (std backend, work stealing, batcher, `UpdateSink`), with the `scan-main` opener, `Started`/`Failed`, `catch_unwind` per job and poison-tolerant locks; catalog fields stubbed (`verdict = None`, `markers = 0`).
- `platform` for macOS/Linux, with a compiling Windows stub: `meta_from_raw`/`entry_meta`, `classify_error`, `mounts()`, `init_*`.
- `Boundaries` (system and fixed), mount-before-stat skip, mount aliases under `/System/Volumes/Data`, the excluded set, HFS+ directory-link set, `Skip::{Excluded, CloudOnly, LinkedDir}`, vanished handling, the hard-link owner map and `counted_elsewhere`, the in-flight table and `Progress.current`.
- `examples/scan.rs` feeding a `Tree`.
- Integration tests from §10.2 (without catalog, git and trash).
- *Accept:* tests pass, including boundaries, vanished, start failures and panic safety; `cargo run -p dirstat-core --release --example scan -- ~/.cargo` prints a total equal to `du -skx ~/.cargo` (KiB) with `root == Done` and `total == Summary`; cancel latency < 1 s; RSS of the example on `~/dev` (or the largest available tree) ≤ the §3.3 budget; the cross-target check compiles.
- *Done 2026-11-13.* Results: `~/.cargo` 2,088,668 KiB and `~/dev` 223,252,460 KiB, both equal to `du -skx`; `~/dev` (531k dirs, 3.8M items) in 24 s with 131 MB peak RSS; cancel latency 4–6 ms on `~/dev`. Notes:
  - No `scan/listing.rs`: the worker calls `std::fs::read_dir` directly (the backend abstraction only served the benchmark).
  - The panic-safety test is a unit test (`scan/tests.rs`) using a `#[cfg(test)]` directory-name trigger; the vanished test deletes the root right after `Started`, which is deterministic.
  - Below `/System/Volumes/Data`, mounts already under that path get no second alias.
  - `Mount.dev` is `None` on macOS, and Linux `Mount.space` is `None` until the start screen needs it (M4).
  - A Stop on `~` took 229 s to produce `Finished`: a worker sat in a blocked syscall (likely a privacy prompt). That is the §4.5 case: the UI must treat Stop as final at once.
  - The Linux tests were only compile-checked (no container runtime was running).

**M3 — Catalog and git verification (core)**
- Port the prototype into `catalog/*`; add `section_for`, `display_section`, `FileMatch`, `RootContext`/`root_context`/`detached`, `Resolver::canonicalized`, `cleanup_hint`.
- Wire classification into the worker (`Job.ctx`, markers, CACHEDIR.TAG validation, `classify_file` for large files) and `root_context` into the opener.
- `Tree::context_for`.
- `git.rs` (`find_git`, batched `check-ignore`/`ls-files`, timeout, dataless skip).
- *Accept:* the ported 9 tests plus the new ones (§10.1); the catalog and git fixture integration tests; `examples/scan.rs --candidates ~/dev` lists Cargo targets as Safe and node_modules as Review first (Low); a scan of `/Applications` gives 0 candidates (CAT §6 smoke results).
- *Done 2026-11-13.* `~/dev`: 618 candidates; Safe to delete 108 items / 110 GB (Cargo targets, Gradle builds), Review first 330 items / 4.5 GB (node_modules, .venv as Low), 180 hidden (tracked by git); 27 work trees verified in 6 s. `/Applications`: 0 candidates. Notes:
  - `classify_file` keeps the prototype's signature (the ported tests use it); the new `Engine::match_file` returns `Option<FileMatch>`.
  - Every listing is classified, including unreadable and vanished folders, so `context_for` reproduces every node's verdict. The UI decides what to do with an unreadable candidate.
  - `catalog::warnings(risk, display, reason, repo)` holds the dialog warning text of §5.3, pinned by tests.
  - `git::plan_jobs` groups candidates by work tree and `git::worktree_of` finds the work tree from stored markers; the scheduling (one job per tick) stays in the app.
  - **Open catalog issue:** npm's bundled `node_modules` inside Node.js distributions (`…/lib/node_modules/npm/node_modules`, next to npm's own `package.json`) match `node-modules` and are suggested under Review first. Deleting them breaks that npm. Needs a rule fix (e.g. a Protect zone for global `lib/node_modules` prefixes); not changed in M3.

**M4 — App shell, start screen, live scan wiring**
- `App`/`Env`/`Screen`/`Message` skeleton; `App::boot` vs `App::new`; `PrefsStore`.
- Start screen: cards from `volumes::start_cards()` with meters, Choose Folder… (rfd), CLI path, drag & drop overlay, invalid-path banner, keyboard on cards.
- `results/scan_task.rs` (UiSink, start, "Opening…", Stop with `mark_stopped` and draining, ScanId filtering).
- The activity field (counts, bytes, rate EMA, elapsed, current path and "Waiting on …", determinate bar or indeterminate sweep, Stop, complete and stopped states).
- A *temporary* minimal tree list (top-level children only, unsorted) so that progress can be seen.
- *Accept:* `dir-stat ~/.cargo` shows a live activity field and completes with the same total as `du -skx`; Stop on `~` shows "Scan stopped" and "≥" within ~0.1 s; the app-level tests for Stop (including "`Finished` never arrives"), `Failed` and stale ids pass; a screenshot of the start screen matches UX §4; a nonexistent CLI path returns to the start screen with the banner.

**M5 — Tree view**
- `rows.rs` virtualization with the layout-driven viewport and anchoring on every rebuild, `columns.rs` row anatomy (disclosure, name, size, share bar, %, items, per-row states and pulsing dot, "≥"/"—", tooltips), header sort.
- File rows + aggregate row (kept files only).
- `order.rs` calm sorting + `ReorderClock` with its activity inputs (`PointerIn`, `PointerMoved`, `LeftButton`, real scroll changes, keys).
- Synchronous rebuilds for user actions; throttled rebuilds for scan batches.
- Responsive columns, the 440 px tree minimum; placeholder rows before the root listing.
- *Accept:* the `order` and `rows` unit tests (no re-order within 700 ms of pointer, key or scroll activity or while a button is held; at most one forced re-order per 5 s; siblings within 10% never flip; the anchor stays put when listings arrive above the viewport; no blank tree after collapsing a long list); `view()` stays under 2 ms with 160k rows expanded (log timing in debug); completion triggers one exact re-sort with the selection anchored; expanding and collapsing respond within one frame.

**M6 — Navigation, selection, actions**
- Selection (mouse + keys), focus panes, the full `keys.rs` map.
- Zoom, breadcrumbs (collapsing to "…"), history (⌘[ ⌘], mouse back/forward), zoom-root total in the path bar.
- Selection bar per row type, with the narrow-width short labels (D16).
- Reveal, Copy Path, toasts.
- On-demand small-file listing (`smallfiles.rs`, `SmallFile` and `MoreFiles` rows, "Listing files…").
- Window title.
- *Accept:* every UX §6.4 MUST shortcut works on macOS (manual checklist, including ⇧⌘R and ⌥⌘R on a real keyboard) and is covered by the `keys` tests; zoom in, out and back restores selection and scroll (app-level test); expanding "≡ N files" in a 3,000-file directory lists them in batches of 50.

**M7 — Errors and permissions**
- Badges (self vs. inside), tooltips by `ErrorClass`.
- Error count in the activity field, errors popover with Copy (from `tree.errors()`).
- FDA bar inside the tree pane (first EPERM, terminal app name, own bundle id ignored).
- Root-unreadable state; start-screen footnote with `full_disk_access()`.
- Cloud-folder, excluded, linked-folder and vanished rows.
- *Accept:* the `chmod 000` fixture shows the filled badge and the parent the outline badge; scanning `~/Library` without FDA shows the FDA bar once; the popover lists paths, also after Stop, and [Copy] produces TSV; screenshots of the popover and the FDA bar via `DIRSTAT_SCRIPT`.

**M8 — Candidate status and trash flow**
- `results/candidates.rs`: git job scheduling on the tick, status map, `GitState` → `display_section`.
- `safety.rs` (canonical guards, `protected_ids`, live checks), `trash/*` (`execute` with G6–G9, `Trasher`, `SystemTrasher`, Linux rename-only trash), `results/trash.rs` (plan, dialog with warnings and default-button logic, modal keys, pending rows, `TrashJob` values with `tree_gen`, struck-through "In Trash" rows, post-trash re-order, toasts, zoom-out-if-trashed, `~/.Trash` outdated flag).
- *Accept:* the `safety`, `trash` and `trash plan` tests; the app-level trash tests (§10.3); the manual checklist §10.4 item 6; ancestors' sizes drop immediately after success and the struck-through row leaves at the next re-order even after the scan has completed; a failure restores the row with a sticky toast; on `~/dev`, the dialog for a git-tracked build folder (CAT §5.1) shows no catalog reason and defaults to Cancel, and a candidate whose git check is still pending defaults to Cancel; screenshots of the dialog (Safe, Low, Review) via `DIRSTAT_SCRIPT`.

**M9 — Cleanup pane**
- `results/cleanup.rs`: `build_cleanup` (scope = zoom root, `display_section` only); stat block; Safe and Review sections with the D1 subtitles; the "Re-downloadable" tag; groups ranked by size and re-ranked on the reorder clock; single-match vs. multi-match groups; child rows with age ("8 mo" from `newest_mtime`); batches of 50; fold for groups under 10 MB; reason, caution and "how to clean" lines; Copy Command for ToolOnly rules; disclaimer; empty, scanning, error and `/System/Volumes` states; the Safe total in the pane header.
- Click → reveal in the tree (zoom to the scan root if needed, expand ancestors, select, center, flash).
- ⌘⌫ on a focused child or single-match group → §7 flow; multi-match group and ToolOnly → toasts (§7.1).
- Cleanup pills in tree rows (from `display_section`).
- *Accept:* the `cleanup` unit tests; on `~/dev`, Cargo targets appear under "Safe to delete" and node_modules under "Review first" with the "Re-downloadable" tag; the two tracked folders from CAT §5.1 appear nowhere and have no pill; with no git found, in-repo candidates appear under Review first with "Couldn't check with git"; the stat block and header count only Safe; clicking a child reveals it centered in the tree; ⌘⌫ on a ToolOnly item shows the command toast.

**M10 — Rescan**
- Rescan of the selected folder / zoom root (⌘R) with the old numbers kept visible and the `RESCANNING` state; the eligibility rules of §4.6.
- `counted_elsewhere`, splice, root replacement, compaction, `results/migrate.rs`, one exact re-sort.
- Rescan everything (⇧⌘R); Scan This Volume (not on the Data-volume row); New Scan confirmation while scanning.
- *Accept:* the end-to-end splice and hard-link rescan tests (§10.2); the `migrate` tests; the UI shows old numbers until the swap, with no dip-then-grow; expansion, selection and history survive a rescan; a cancelled rescan leaves the tree unchanged; three ⌘R on the root of a `~/dev` scan keep RSS within §3.3.

**M11 — Polish and packaging**
- Unaccounted space row (volume-root scans, ≥ 1 GB and ≥ 1%).
- ETA; "N of M folders done".
- Recents and prefs persistence.
- Package badge.
- `scripts/bundle-macos.sh` (Info.plist with purpose strings, icon placeholder, codesign).
- A README section on running and bundling.
- *Accept:* the manual checklist §10.4 items 1–5, 7, 10 pass with the bundled app; recents survive a restart.

**M12 — Treemap (optional, SHOULD)**
- UX §8 as summarized in §6.10, plus the tab bar with the "Cleanup 38 GB" label and ⌘2.
- *Accept:* the layout of 200 × 3-level blocks takes under 8 ms; no relayout while the pointer is over the map; selection syncs both ways.

The dependency order is M0 → M1 → M2 → M3 → M4 → M5 → M6. After M6: M7 any time; M8 before M9 (the pane needs the candidate status); M10 after M8 (rescan eligibility depends on pending trash). M11 comes last, then the optional M12. M1–M3 have no GUI and can be checked purely by tests.

---

## 12. Open questions for the user

**Decided 2026-11-13:** the user accepted the default for all six questions. The body of this document already uses them.

1. **Low vs. Review inside "Review first."** Following your decision, Low items (re-downloadable: `node_modules`, `.venv`, `~/.m2`, Gradle caches, toolchains, most of `~/Library/Caches`) and Review items (may hold your data: Trash, Archives, VMs, backups) share one section, with wording that differs per kind in the dialog. Should Low groups also carry a small muted tag "Re-downloadable" in the pane? *Default: yes, a muted text tag on Low groups. There is no separate sub-section.*
2. **Treemap in v1?** Should the Map tab (M12) ship with v1, or wait for v1.1? *Default: build it only after M11, if you still want it then.*
3. **"Tool-only" items in the tree.** For folders the catalog says should be cleaned with a tool command (Go module cache, iOS backups, Docker disk, simulator runtimes), the cleanup pane shows only the command. In the tree, should Move to Trash still be possible (with a warning, and Cancel as the default button), or disabled? *Default: possible, with the warning (D8).*
4. **Folders with iCloud-only content.** Trashing a folder that contains not-downloaded iCloud items removes them from iCloud on all devices and frees almost nothing locally. Should that be allowed with a warning (Cancel as default), or refused like folders that contain another volume? *Default: allowed with the warning.*
5. **Running `git` in your project folders.** To avoid suggesting tracked build folders, dir-stat runs `git check-ignore` and `git ls-files` (read-only) in repositories it finds. Is that acceptable? *Default: yes. If git isn't installed, those items go to Review first.*
6. **App identity for macOS.** Which bundle identifier, and which signing identity (Developer ID, or a self-signed certificate), should the `.app` use? Privacy grants such as Full Disk Access are tied to it. *Default: `com.example.dir-stat` and ad-hoc signing until you choose.*
