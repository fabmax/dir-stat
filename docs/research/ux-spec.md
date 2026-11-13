# dir-stat: UI/UX specification (v1)

Status: proposal for implementation · Target: iced 0.14, macOS first (Linux must work, Windows nice-to-have)
Window: 1200 × 800 default, resizable, minimum 800 × 500.

Priority tags used throughout:
**[MUST]** ships in v1 · **[SHOULD]** v1 if cheap · **[LATER]** after v1.
A consolidated matrix is in §15.

---

## 0. TL;DR

- **One window, three states:** *Start* (pick a location) → *Scanning* → *Results*. Scanning and Results are the same screen. The tree shows up straight away and fills in live.
- **Layout:** a toolbar with a single **activity field** for scan progress and summary · a **path bar** with breadcrumbs · a **size-sorted tree** on the left · a **side pane** on the right with two tabs, **Cleanup** (default) and **Map** (treemap) · a **selection bar** at the bottom that holds the actions.
- **Live but calm:** sizes update in place as scanner batches arrive (up to 20× per second). Rows are re-ordered at most every 1.5 s, with hysteresis and scroll anchoring, and the re-order waits (up to 5 s) while the pointer or keyboard is active. Design rule: *nothing moves under the pointer.*
- **Safe by default:** Move to Trash only (no permanent delete in v1). It always asks for confirmation, refuses system and user-root folders, and explains that space is freed only when the Trash is emptied.
- **Numbers like Finder:** decimal units (1 GB = 10⁹ bytes), size on disk, 3 significant digits.
- **Quiet chrome, loud data:** warm-neutral surfaces, one blue accent, and status colors only where they carry meaning. The palette was checked with a contrast/CVD validator (§11).

---

## 1. Research: what the best tools get right and wrong

| Tool | What makes it easy | What confuses or annoys | What we take |
|---|---|---|---|
| **DaisyDisk** (macOS) | Launch view lists all volumes, startup disk first, with a fill gauge (green = more than half free, red = call to action). One "Scan" per disk, "Scan Folder…" or drag & drop. The sunburst shows folders in color and files in gray. Items too thin to see are merged into a "smaller objects" item. Point at a segment to see its contents, click to drill in, click the center to go up. Deleting goes through the **Collector**: you review first, then Delete with a 5 s cancel window. It refuses protected locations (/System, /Library, home). A **"hidden space"** pseudo item explains used space the scan can't see (purgeable, other APFS volumes, "still hidden"). Shortcuts: ⌘[ ⌘] history, ⌘↑ up, Return opens, ⌘R rescans the folder, ⇧⌘R rescans the disk. | Gives no guidance on what is safe. Its official advice is "only delete files you can recognize". Deleting is permanent (no Trash). Reviewers disagree on that point, which shows the confusion. Advanced scanning needs Full Disk Access and an admin scan. | Volume list with gauges, "hidden space" → our *Unaccounted space* row, review-before-delete, history shortcuts, rescan shortcuts, protected-path refusal. |
| **GrandPerspective** (macOS) | A treemap of everything. Color by folder or file type. Hovering shows the full path. | "Powerful but not friendly". The wall of colored tiles is confusing at first. Users call the gradients "harsh" and "dated". The treemap doesn't show hierarchy as well as a list. | Flat (not cushioned) treemap, path on hover. The treemap is a companion, not the primary view. |
| **OmniDiskSweeper** (macOS) | A column browser sorted **largest first**, with Trash and Reveal buttons. Columns appear almost immediately and re-sort "as you watch". | No visualization. Looks dated. Partial VoiceOver support (sizes are not read). Re-sorting while you watch makes rows jump. | Size-descending default everywhere. Reveal + Trash are always one click away. Show results immediately. |
| **Disk Inventory X** (macOS) | Treemap + file hierarchy + **kind statistics**, with selection synced across all views. | Aging build (needs Rosetta). Three panes is a lot to read. | **One selection shared by every view.** |
| **WinDirStat** (Win) | Directory list + extension list + cushion treemap. "Subtree percentage" bars. A per-row **Pac-Man animation while a subtree is being scanned**. | Three panes are busy. You can't drill down in the treemap. Slow on big drives. Version 2.6 reportedly switched the % column to "% of whole scan" by default (the bar stays % of parent), which suggests that "% of parent" alone is often not what users want. | A per-row "still scanning" indicator. Bar = % of parent, tooltip = % of the scan root too. |
| **WizTree** (Win) | Extremely fast (reads the MFT). Tree with % of parent, size and item counts. A separate "File View" of the largest files (top 1000). Treemap. | The power-user filter syntax is not for beginners. | Item counts. A global "largest files" list as a later candidate. |
| **SpaceSniffer** (Win) | A treemap only, which **animates live while scanning**. Large items show up within seconds. Click to expand in place, double-click to zoom. | The live animation is "too much" for some users, which is why there is a Pause button. The interface is cluttered. | Live filling is delightful, but motion must be throttled and pausable by interaction. |
| **TreeSize** (Win) | An Explorer-like tree sorted by size. Opens from Explorer's context menu. Start screen lists the drives. | Ribbon-heavy UI. Many features are behind the Pro edition. | Plain tree conventions. Start screen with drives. |
| **QDirStat** (Linux) | Tree + treemap + details panel. Percent bars use a **different color per tree level**. Files are grouped into a **`<Files>` pseudo-directory** so they can be compared with subdirectories. Does **not cross filesystem boundaries** by default. Breadcrumbs. Configurable cleanup actions. | Lots of columns and options. Uses 1024-based units (differs from Finder/GNOME). | Aggregated "N files" row, staying on one filesystem with mount points as visible stubs, breadcrumbs. |
| **Baobab** (GNOME) | "Devices & Locations" start view, each entry with "X Total / Y Available" and a level bar. Tree columns Folder, Size, Contents, Modified, **sorted by size, descending**. Switchable ring chart / treemap. Path bar. Toasts for errors. A desktop notification "Scan completed" when the window is unfocused. Decimal units (GLib). | Only one chart at a time, and the chart doesn't explain itself. | Start screen pattern, minimal columns, decimal units, toasts. |
| **Filelight** (KDE) | Concentric rings, very pretty, quick Home/Root/Folder entry points. | Rings are hard to compare and labels are sparse. | Pretty is allowed, but it must not cost readability. That is one reason we pick the treemap over the sunburst. |
| **ncdu** | Keyboard-first (arrows/hjkl, Enter, s/n/C to sort, d to delete with confirmation, i for info). Clear one-character flags: `!` read error, `.` error below, `>` other filesystem, `@` special file, `H` hard link, `e` empty. Graph relative to the largest item, percent relative to the parent. | Deletion is permanent. The flags are cryptic for newcomers. | The **error semantics** (error here vs. error below), the "other filesystem" stubs and the keyboard model, but with words and icons instead of cryptic letters. |
| **gdu** | ncdu keys plus `D` = move to Trash. **Esc stops a scan and keeps partial results.** The web UI's **cleanup projection** colors *safe* candidates strongly and *review-first* ones muted, with everything else gray. **Deleted items stay in place, cross-hatched, so the layout doesn't jump.** Build directories count only next to a project marker. | Two UIs (TUI and web), and the projections need explaining. | Stop keeps partial results. Trashed rows stay in place until the next re-order. Cleanup coloring idea (LATER). |
| **dua** | Browse **while** scanning (since 2.4). A multi-stage, mark-then-delete flow. `dua clean` lists disposable directories (node_modules, Python caches/venvs, Cargo `target`, Zig caches) largest first, as sizing finishes. In Git repos, candidates must be ignored and contain no tracked files. | Terminal only. Marking is not obvious without the help screen. | Live browsing. A ranked cleanup list that fills in as sizes settle. Safety heuristics are passed on to the catalog agent. |
| **npkill** | A list of `node_modules` with **size + "last modified" of the project**. ⚠ on folders apps depend on (Spotify, Discord…). | Deletes immediately (`rm -rf`). | **Staleness ("8 mo") is a strong "safe to delete" signal.** Show it in cleanup items. |

**Takeaways that drive this spec**

1. **The sorted list is the hero, and visualizations are companions.** Every tool users praise for being "easy" leads with "largest first" (OmniDiskSweeper, Baobab, TreeSize). Treemap-only tools have the steepest learning curve (GrandPerspective, SpaceSniffer).
2. **Live results are loved, jumping rows are not.** SpaceSniffer needs a Pause button. OmniDiskSweeper re-sorts as you watch. We solve this with calm sorting (§5.4).
3. **"What is safe to delete?" is the unmet need.** DaisyDisk's top complaint is that it gives no guidance. gdu, dua clean and npkill show that a curated candidate list with a reason and a risk level works.
4. **Explain missing space.** "Why doesn't it add up to what Finder says?" is the classic confusion. DaisyDisk's *hidden space* answers it, and so does our *Unaccounted space* (§5.6).
5. **Deletion must be reviewable and recoverable.** Trash instead of permanent delete. Confirmation with a summary. Refuse system paths.

---

## 2. Principles

1. **Biggest first.** Everything defaults to size-descending, so the answer to "what is big?" is always at the top.
2. **Live but calm.** Data streams in, but *nothing moves under the pointer*.
3. **One current folder, one selection.** The *zoom root* scopes the tree, the treemap and the cleanup list. The *selection* is the same in the tree, the treemap, the cleanup list and the selection bar.
4. **Safe by default.** Trash only. Always confirm. State the consequences. Refuse system paths.
5. **Explain the numbers.** Use Finder-style units, size on disk and an explicit "unaccounted" figure.
6. **Quiet chrome, loud data.** Neutral surfaces, one accent, and status colors only as status.
7. **Keyboard-complete.** Every action has a shortcut, and every important action also has a visible button.

---

## 3. Screens and navigation model

```
            dir-stat <path>  ───────────────┐
                                            ▼
 ┌────────────┐  click location /    ┌────────────────────────────────────┐
 │   Start    │  Choose Folder… /    │  Results (scanning ⇢ complete)     │
 │ (locations)│  drop folder ───────▶│  zoom root · selection · side pane │
 └────────────┘                      └────────────────────────────────────┘
       ▲            New Scan (⌘N)                 │
       └──────────────────────────────────────────┘  (asks first if a scan is running)
```

- **Scan root**: the folder the user picked. It never changes within a session, except through a new scan.
- **Zoom root**: the folder currently shown as the top of the tree, treemap and cleanup list. It starts at the scan root. Breadcrumbs show its path from the scan root.
- **Selection**: one row [MUST]. Multi-select [LATER].
- **Window title**: `dir-stat` on the start screen, `<zoom root name> — dir-stat` in results.
- Dialogs are in-window modals (iced has no native sheets). Toasts appear bottom-center above the selection bar.

---

## 4. Start screen

```
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│                                                                                          │
│                                        dir-stat                                          │
│                            Find out what is using your disk.                             │
│                                                                                          │
│             ┌────────────────────────────────────────────────────────────────┐           │
│             │ ⌂  Home                                                     ›  │           │
│             │    /Users/max · on Macintosh HD                                │           │
│             └────────────────────────────────────────────────────────────────┘           │
│             ┌────────────────────────────────────────────────────────────────┐           │
│             │ ▭  Macintosh HD                                     995 GB  ›  │           │
│             │    ███████████████████████████████████████████████░░░░░░░░░    │           │
│             │    852 GB used · 143 GB free                                   │           │
│             └────────────────────────────────────────────────────────────────┘           │
│             ┌────────────────────────────────────────────────────────────────┐           │
│             │ ▭  Backup                                External · 2.00 TB ›  │           │
│             │    ███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    │           │
│             │    610 GB used · 1.39 TB free                                  │           │
│             └────────────────────────────────────────────────────────────────┘           │
│                                                                                          │
│                     [ Choose Folder… ]    or drop a folder onto this window              │
│                                                                                          │
│             Recent                                                                       │
│             ~/dev                                          yesterday         ›           │
│             /Applications                                  3 days ago        ›           │
│                                                                                          │
│             (i) macOS may ask for access to some folders. For complete results,          │
│                 give dir-stat Full Disk Access.   Open Privacy Settings                  │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

**Content**
- Title "dir-stat" (24 px semibold) and subtitle (13 px, secondary) [MUST].
- **Location cards** in a single centered column, 560 px wide [MUST]:
  - **Home**: home icon, "Home", subtitle "`/Users/max` · on Macintosh HD". No size, because it is unknown before scanning.
  - **System disk**: on macOS "Macintosh HD", the name taken from the `/Volumes/<name> → /` symlink, falling back to "Macintosh HD". On Linux "Computer" (`/`). On Windows "Local Disk (C:)".
  - **Other mounted volumes** that a user would recognize: macOS `/Volumes/*` minus the symlink to `/`, and never the `nobrowse` system volumes (VM, Preboot, Update, xarts, iSCPreboot, Hardware, Data). On Linux, real filesystems under `/`, `/home`, `/media`, `/mnt`, `/run/media`, skipping pseudo filesystems such as proc, sysfs, tmpfs, devtmpfs, overlay, squashfs. Deduplicate by device ID. Tag them "External" or "Network" when known. Network volumes get the subtitle "Network volumes can be slow to scan".
  - **Capacity meter** on volume cards: 6 px tall, radius 3, full card width. Fill = accent. When free space is below 10%, the fill turns status-critical and the text reads "Almost full · 12 GB free" with a warning icon (never color alone). The capacity goes top right ("995 GB"). Below the meter: "852 GB used · 143 GB free".
    - On APFS, *used* = (f_blocks − f_bavail) × f_bsize, which matches Finder's container view. *Free* = f_bavail × f_bsize. Tooltip: "Finder may show more available space because it counts purgeable data as free."
- **Choose Folder…**, a secondary button that opens the native folder picker (`rfd`) [MUST]. **Drag & drop** a folder anywhere on the window [SHOULD]. While something is dragged over the window, an overlay with a 2 px accent border and the text "Drop to scan" appears.
- **Recent**: the last 5 scan roots with relative time. Click to scan again; on hover a × removes the entry [SHOULD]. Results are not cached, so this simply starts a new scan.
- **macOS privacy footnote** (12 px, secondary, info icon). "Open Privacy Settings" opens `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles` [SHOULD]. The Home card also gets the line "macOS will ask for access to Desktop, Documents and Downloads" [SHOULD].

**Interaction**
- A card is a single click target that starts the scan immediately. There is no extra "Scan" button, because the card *is* the button. Hover: background `bg.hover` and border `border.strong`. Keyboard: ↑/↓ moves focus between cards (2 px focus ring), Return scans, ⌘O opens Choose Folder… [MUST].
- Volume info loads asynchronously. A card that hasn't answered within 300 ms (e.g., a stalled network mount) shows "Checking…" in its meter area. Its *contents* are never read until the user clicks it.

**CLI** [MUST]
- `dir-stat <path>` skips the start screen and starts scanning `<path>`, with relative paths resolved against the current directory. `dir-stat` with no arguments opens the start screen.
- If the path is invalid (doesn't exist, not a folder, unreadable), show the start screen with an error banner at the top: "Can't scan “/foo”: No such folder." (critical icon + text).
- `--help` / `--version` print and exit.

---

## 5. Scanning

### 5.1 Layout while scanning (same screen as results)

```
┌───────────────────────────────────────────────────────────────────────────────────────────────┐
│ [+ New Scan]       ╭──────────────────────────────────────────────────────────╮     [?] [◫]   │
│                    │ ◌ Scanning Macintosh HD · 37%                     [Stop] │               │
│                    │   412,031 items · 312 GB · 9.8k/s · 0:42 · …/Caches/fsC… │               │
│                    ╰━━━━━━━━━━━━━━━━━━━━━━────────────────────────────────────╯               │
├───────────────────────────────────────────────────────────────────────────────────────────────┤
│ [‹][›]  ▭ Macintosh HD                                                           ≥ 312 GB     │
├────────────────────────────────────────────────────────────────┬──────────────────────────────┤
│ Name                              Size ▾   % of parent   Items │  Cleanup    Map              │
│ ▸ ▭ Users                      •  201 GB   ███████▋ 64%  1.10M │ ──────────────────────────── │
│ ▸ ▭ Applications               • 52.0 GB   █▊       17%   230k │ In Macintosh HD              │
│ ▸ ▭ Library                    • 31.9 GB   █        10%   180k │ ≥ 9.80 GB likely reclaimable │
│ ▸ ▭ System                       14.2 GB   ▌       4.5%   402k │ Still scanning, more will    │
│ ▸ ▭ private                    • 12.4 GB   ▍       4.0%  41.2k │ appear.                      │
│ ▸ ▭ usr                          1.90 GB           0.6%  98.1k │                              │
│ ▸ ▭ opt                       ○      —                         │ ✓ Safe to delete             │
│   ▭ dev                              —     other volume        │ ▸ DerivedData     ≥ 6.10 GB  │
│   ≡ 12 files                     3.10 kB          <0.1%     12 │ ▸ Homebrew cache    2.10 GB  │
│                                                                │                              │
├────────────────────────────────────────────────────────────────┴──────────────────────────────┤
│ Nothing selected   ·   ↑↓ select   → expand   ⏎ zoom in   ⌫ back                              │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
  •  pulsing dot = still being scanned      ○ = waiting      — = no size yet / not counted
  ━  2 px progress line along the bottom edge of the activity field
```

- The tree appears **as soon as the scan root's own listing is read** [MUST]. Usually that is under 100 ms. Until then, show 6 placeholder rows (muted bars, no shimmer). If the root listing takes over 1 s, the tree area says "Reading /Users/max…".
- Rows are added as directories are discovered and sizes grow in place. The UI applies the scanner's coalesced batches as they arrive, **at most 20 per second** (see `scan-engine.md` §7.5). Heavy derived data, such as the flattened row list, is rebuilt at most every 100 ms and only when something changed [MUST].

### 5.2 Activity field (toolbar center)

The single home for scan status. It has the same width in every state, so the layout never shifts. It is 36 px tall with two lines, width clamp(360 px, 50% of window, 640 px), radius 8, background `bg.surface` with a hairline border.

| State | Line 1 (13 px) | Line 2 (11 px, secondary) | Button | Bottom-edge 2 px line |
|---|---|---|---|---|
| Scanning, root is a volume | spinner · "Scanning Macintosh HD · 37%" · [SHOULD] "· about 2 min left" | "412,031 items · 312 GB · 9.8k items/s · 0:42" · current path (`text.muted`) | Stop | accent, determinate |
| Scanning, root is a folder | spinner · "Scanning Library" · [SHOULD] "· 4 of 23 folders done" | same stats · current path | Stop | accent, indeterminate: a 25%-wide segment sweeping every 1.4 s |
| Complete | ✓ (status good) · "Macintosh HD · 832 GB · 2,113,402 items" | "Scanned in 1:12" + (if errors) "· **23 folders couldn't be read**" (link, opens §10.2) | Rescan (everything, ⇧⌘R) | hidden |
| Stopped | "Scan stopped · ≥ 312 GB · 412,031 items" | "Sizes marked ≥ are incomplete" | Rescan (everything) | hidden |

- **Current path** [MUST]: line 2 ends with the path being read right now, in `text.muted`. It takes the width left after the stats and is middle-truncated ("…/Caches/com.apple.Safari/fsCachedData"). The path is relative to the scan root and refreshes at most 5×/s so it stays legible. If less than 120 px remain, the path moves into the field's tooltip. Hovering the field always shows the full current path.
- **Determinate progress** [MUST]. Used when the scan root is a volume mount point (on macOS this includes "/" = Macintosh HD).
  - progress = scanned_bytes / denominator, capped at 99% until done.
  - Denominator = Σ volume-used of the volumes the scan traverses, where volume-used = (f_blocks − f_bfree) × f_bsize, i.e. df's "Used". For macOS "/" that is `/` + `/System/Volumes/Data` (on this machine 16 GB + 816 GB ≈ 832 GB). The firmlinked Data volume must not be double counted.
  - When the scan completes, the bar snaps to 100% and disappears. Any remaining gap is explained by *Unaccounted space* (§5.6).
- **Rate** = items/s as an exponential moving average over about 3 s, shown as "9.8k items/s" [MUST]. **ETA** [SHOULD], determinate scans only: from bytes/s averaged over 10 s, shown after 5 s and once past 5%, rounded ("about 2 min left", "less than a minute left"). It never grows by more than 30% between updates (smoothing).
- **Stop** [MUST]: Esc or ⌘. on macOS, Esc on Linux/Windows. Esc closes any open overlay first. Stopping keeps every partial result. Directories that were not finished show their size as "≥ 12.3 GB" (secondary color) with the tooltip "Scan was stopped before this folder was fully measured."
- **Completion while unfocused**: request user attention (dock bounce) [LATER].

### 5.3 Per-row scan state [MUST]

| State | Size cell | Other |
|---|---|---|
| Waiting (not read yet) | `○` hollow 6 px ring + "—" in `text.muted` | Items "—". Expanding shows one muted row "Waiting to be scanned…" |
| Scanning (the subtree is not complete) | `•` 6 px dot, pulsing (opacity 0.35 ↔ 1, 1.2 s) + size in `text.secondary` | The size is a growing lower bound |
| Done | size in `text.primary` (in `text.secondary` when < 1 MB, to quiet the noise) | none |
| Incomplete (scan stopped) | "≥ 12.3 GB" in `text.secondary` | tooltip as above |
| Unreadable | "—" | Filled warning badge after the name (§10) |
| Other volume (mount point) | "—" | Muted tag "other volume". Not counted. Not expandable. Selection bar offers "Scan This Volume" [SHOULD] |

Animate the dots only on rows that are actually visible. Keep requesting frames (`window::frames()` or `time::every(50 ms)`) only while a visible row is in the scanning state.

### 5.4 Calm sorting while sizes change [MUST]

Problem: sizes grow constantly. Re-sorting on every update makes rows swap places, moves click targets out from under the cursor and makes reading impossible.

**Policy**
1. **Values update in place; order updates on a slow tick.** Sizes, bars, % and counts refresh on every UI batch (≤ 20/s). Sibling order is recomputed only on a **re-order tick every 1.5 s**.
2. **No re-order during interaction.** A tick is deferred while any of these hold:
   - the pointer has moved over the tree within the last 700 ms;
   - a mouse button is held;
   - a navigation key was pressed within the last 700 ms;
   - a menu, popover or dialog is open.

   A deferred tick runs at most 5 s late, so ordering never starves.
3. **Hysteresis.** Sort by a *quantized* key, `bucket = floor(ln(size) / ln(1.1))`, so buckets are about 10% wide. Use a **stable** sort that keeps the previous order within a bucket. Two siblings of nearly equal size never flip back and forth.
4. **New rows** found between ticks are appended at the end of their parent's list and placed properly on the next tick. Inserting them mid-list would push the rows below them down.
5. **Scroll anchoring.** Before a re-order, record the anchor: the selected row if it is visible, otherwise the first fully visible row, plus its y offset in the viewport. After the re-order, scroll (`widget::operation::scroll_to`) so the anchor keeps the same y. If the anchor's parent was collapsed, use the nearest visible ancestor.
6. **Final settle.** On completion, run one exact sort with no quantization and with anchoring. An explicit header click or a zoom triggers an immediate exact sort, anchored to the selection.
7. **Only expanded parents are sorted.** The children of a collapsed directory are sorted when it is expanded, so a tick costs O(visible structure), not O(all nodes).
8. Sorting by name doesn't need ticks except to place new rows. The same append-then-place rule applies.

The treemap uses the same rule (§8.6): it re-lays out at most once per second while scanning and never while the pointer is over it.

### 5.5 Interacting during a scan
- Expand, zoom, select, Reveal and Copy path are all allowed [MUST].
- **Move to Trash** is enabled only for rows whose subtree is *Done*. Otherwise the button is disabled with the tooltip "Available when this folder is fully scanned." [MUST]
- **Rescan** is disabled until the scan ends [MUST].
- **New Scan** while scanning opens a confirmation: "Stop scanning Macintosh HD?" with [Keep Scanning] [Stop and Start Over] [MUST].
- Scanning expanded or visible directories first [LATER] (a scanner priority hint).

### 5.6 Unaccounted space [SHOULD]
Only for volume-root scans, after completion, and only if the gap is ≥ 1 GB and ≥ 1% of used space.
- Extra row at the end of the root's children: dotted-box icon, name "Unaccounted space" in `text.secondary`, size = volume used (container view, §4) − scanned total. Its "% of parent" is "—", because it is excluded from shares so the children still add up to 100%.
- Tooltip and selection bar text: "Used space the scan couldn't see: APFS snapshots, purgeable data, system volumes (VM, Preboot, Recovery), other users' files, and folders dir-stat isn't allowed to read." On macOS, when permission errors occurred, the selection bar also offers [Open Privacy Settings].
- Hidden if the gap is negative (e.g., APFS clones counted twice).
- Not drawn in the treemap.

---

## 6. Results: tree, navigation, actions

### 6.1 Full layout (1200 × 800, complete, zoomed into ~/Library, one row selected)

```
┌───────────────────────────────────────────────────────────────────────────────────────────────┐
│ [+ New Scan]       ╭──────────────────────────────────────────────────────────╮     [?] [◫]   │
│                    │ ✓ Macintosh HD · 832 GB · 2,113,402 items       [Rescan] │               │
│                    │   Scanned in 1:12 · 23 folders couldn't be read          │               │
│                    ╰──────────────────────────────────────────────────────────╯               │
├───────────────────────────────────────────────────────────────────────────────────────────────┤
│ [‹][›]  ▭ Macintosh HD › Users › ⌂ max › Library                    182 GB · 22% of disk      │
├────────────────────────────────────────────────────────────────┬──────────────────────────────┤
│ Name                              Size ▾   % of parent   Items │  Cleanup    Map              │
│ ▾ ▭ Developer                    96.1 GB   ████▉    53%   420k │ ──────────────────────────── │
│   ▾ ▭ Xcode                      60.2 GB   ██████▎  63%   301k │ In Library                   │
│     ▸ ▭ DerivedData  [● Build]   24.1 GB   ███▉     40%   312k │ 38.2 GB                      │
│     ▸ ▭ iOS DeviceSupport        21.0 GB   ███▌     35%  88.0k │ likely reclaimable           │
│     ▸ ▭ Archives                 14.9 GB   ██▌      25%  1.20k │ + 12.4 GB to review          │
│   ▸ ▭ CoreSimulator  [● Sim]     30.0 GB   ███▏     31%   109k │                              │
│   ▸ ▭ XCTestDevices               5.90 GB  ▋       6.1%  10.4k │ ✓ Safe to delete             │
│ ▸ ▭ Containers                   31.0 GB   █▋       17%   210k │ ▸ DerivedData       24.1 GB  │
│ ▸ ▭ Caches          [● Cache]    18.0 GB   ▉       9.9%   190k │   Xcode build data, rebuilt  │
│ ▸ ▭ Application Support          15.2 GB   ▊       8.4%  96.0k │ ▸ node_modules · 214 18.3 GB │
│   ▭ Mail  ⚠                           —                        │ ...                          │
│   ≡ 31 files                    412 MB            0.2%      31 │ Based on names & locations.  │
├────────────────────────────────────────────────────────────────┴──────────────────────────────┤
│ ▭ ~/Library/Developer/Xcode/DerivedData · 24.1 GB · 312,004 items · modified 2 days ago       │
│                         [Reveal in Finder] [Copy Path] [Rescan] [Move to Trash…]              │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
```
(The selection bar is a single 44 px line at 1200 px. It is drawn on two lines here only to fit the width.)

**Regions**

| Region | Height / width | Contents |
|---|---|---|
| Toolbar | 44 px | [+ New Scan] (left), activity field (center), [?] shortcuts, [◫] show/hide side pane (right) |
| Path bar | 32 px | [‹][›] history, breadcrumbs, zoom-root total (13 px semibold) + "· 22% of disk" (12 px secondary, only when zoomed) |
| Column header | 28 px | Name · Size · % of parent · Items. Clickable to sort |
| Tree | flex | Virtualized rows, 28 px each |
| Side pane | clamp(320, 34% of window, 480) px, default 400 | Tabs **Cleanup** · **Map**, 36 px tab bar. Divider: 1 px hairline with a 6 px drag handle [SHOULD] |
| Selection bar | 44 px | Selected item's icon, path, facts and action buttons |

**Responsive rules** [MUST]
- Window ≥ 1000 px: side pane visible by default; the user's toggle is remembered [SHOULD].
- Window < 1000 px: side pane hidden by default. When shown, the tree keeps at least 440 px.
- Tree width < 600 px: hide the **Items** column. Tree width < 480 px: hide the % text and shrink the bar to 48 px.
- Selection bar buttons collapse to icon-only buttons with tooltips when there isn't room for labels.

### 6.2 Columns and row anatomy [MUST]

```
12 │ depth×16 │ 16 │4│ 16 │6│ name …………… [pill] ⚠ │ 8 │   Size 88 →│12│ bar 72 │6│ % 34 →│12│ Items 64 →│16
   │  indent  │ ▸  │ │icon│ │                     │   │ • 24.1 GB  │  │███▎░░░░│ │  40%  │  │     312k  │
```

| Column | Width | Content | Sort |
|---|---|---|---|
| **Name** | flex, min 200 | Disclosure, icon, name, then optional pill and badges | A→Z first, then Z→A. Case-insensitive natural order ("file2" before "file10") |
| **Size** | 88, right-aligned | Recursive **size on disk** (allocated bytes; hard links counted once). Includes everything nested | **Default: descending**; second click ascending |
| **% of parent** | 112 = 72 bar + 6 + 34 text | Bar and number = share of the parent's size (for top-level rows, the zoom root) | Same as Size (clicking it sorts by size) |
| **Items** | 64, right-aligned | Recursive count of everything inside (files + folders + links). Blank for files | Descending first |

- **Header**: 12 px medium, `text.secondary`. The active sort column shows ▾ or ▴ and uses `text.primary`. Hover gets a light wash. Tooltip on "% of parent": "Share of the folder it is in."
- **Disclosure**: ▸ / ▾ chevron, 10 px glyph in a 24 px hit area that covers the whole indent zone. It is absent for leaves (files, empty folders, unreadable folders, mounts).
- **Name**: 13 px `text.primary`. Clipped, never wrapped. Truncated with an end ellipsis (see §12.6). Full path in the tooltip.
- **Cleanup pill** [SHOULD], on rows the catalog matches: 18 px tall, radius 9, 11 px medium text in `text.secondary`. It starts with a 6 px status dot (good = safe, warning = review) and gives a short category label ("Cache", "Build", "Deps", "Logs", "Sim", "Trash", "Installer"). Background = status color at 14% (e.g., light `#daf0da` / `#fcf2db`). Tooltip: reason + risk. At most one pill per row, and only on the matched folder itself.
- **Badges** after the name: ⚠ (§10), "package" (macOS bundles, §6.9), "other volume".
- **Bar** (§11.6): 6 px tall, radius 3; track `bar.track`, fill `bar.fill`; at least 2 px wide when the share is above 0.
- **Row states**: hover `bg.hover`; selected + tree focused `bg.selected`; selected + focus elsewhere `bg.selected.unfocused`. Text keeps its tokens in every state (no white-on-accent inversion, so bars stay visible).
- **Tooltips** (500 ms delay):
  - size → "24,113,402,880 bytes on disk", plus "(25.3 GB apparent)" if the scanner provides apparent size;
  - % → "40% of Xcode · 2.9% of Macintosh HD";
  - items → "312,004 items: 298,110 files, 13,894 folders" if split counts exist.

### 6.3 Files without bloating the tree [MUST]
- In every directory, files are **listed individually** only if they are ≥ 1% of that directory **or** ≥ 100 MB, up to 10 such files (the largest first). This is a display rule applied to the files the scanner keeps: per `scan-engine.md`, every file ≥ 1 MiB is kept individually, and smaller ones only as an aggregate.
- All remaining files are merged into **one aggregate row**, "≡ 1,234 files", or "≡ 1,234 other files" when some files are listed individually. It has a stack icon, the name in `text.secondary`, the sum of their sizes, a share bar, and Items = file count.
- With size sorting, the aggregate row is placed by its size like any other row. "These 3,000 small files are 4 GB" is useful to know. With name sorting it is always last.
- Expanding the aggregate row shows the remaining kept files (largest first, in batches of 50). Small files (< 1 MiB) are listed **on demand**: one `read_dir` of that folder, during which a muted "Listing files…" row is shown. Until they are loaded, a final muted row reads "and 1,190 smaller files" (see §16).
- Aggregate row actions: Reveal (opens the parent folder) and Copy Path (parent). **No Trash**, because bulk-deleting unnamed small files is unsafe.
- Symlinks are treated as files with their own (tiny) size and are not followed. Tooltip "→ target".
- A global "Largest files" list (WizTree's File View) [LATER].

### 6.4 Selection, focus and keyboard [MUST]
- Click selects. The tree has keyboard focus on entry. **Tab / ⇧Tab** moves focus between the tree and the side pane. The focused pane's selection uses `bg.selected` (accent tint), the unfocused one `bg.selected.unfocused`.
- Nothing is selected at first. The first ↓ selects row 1. The empty selection bar shows the key hints (wireframe §5.1).
- Double-click on a folder **zooms in**; on a file it **reveals in Finder**.

| Action | macOS | Linux / Windows |
|---|---|---|
| Move selection | ↑ ↓ | ↑ ↓ |
| Page / first / last | PgUp PgDn (fn↑ fn↓) · Home End (fn← fn→) | PgUp PgDn · Home End |
| Expand, or go to first child if expanded | → | → |
| Collapse, or go to parent if collapsed | ← | ← |
| Expand the largest path (up to 3 levels, children ≥ 1% only) / collapse the subtree | ⌥→ / ⌥← [SHOULD] | Shift+→ / Shift+← [SHOULD] |
| Zoom into folder (files: reveal; aggregate row: expand) | Return | Enter |
| Zoom out to parent | ⌫ or ⌘↑ | Backspace or Alt+↑ |
| History back / forward | ⌘[ / ⌘] · mouse back/forward [SHOULD] | Alt+← / Alt+→ · mouse back/forward |
| Type to select (prefix match among visible rows, resets after 1 s) | letters [SHOULD] | letters [SHOULD] |
| Reveal in Finder / file manager | ⌥⌘R | Shift+Alt+R |
| Copy path | ⌘C (also ⌥⌘C) | Ctrl+C (also Ctrl+Shift+C) |
| Move to Trash… | ⌘⌫ | Delete |
| Rescan selected folder (or zoom root) | ⌘R | Ctrl+R · F5 |
| Rescan everything | ⇧⌘R | Ctrl+Shift+R · Shift+F5 |
| Stop scan | Esc · ⌘. | Esc |
| New scan / Choose folder… | ⌘N / ⌘O | Ctrl+N / Ctrl+O |
| Side pane: Cleanup / Map / hide-show | ⌘1 / ⌘2 / ⌘0 | Ctrl+1 / Ctrl+2 / Ctrl+0 |
| Context menu | right-click · ⇧F10 [SHOULD] | right-click · Shift+F10 / Menu key [SHOULD] |
| Shortcut overlay | ? · ⌘/ [SHOULD] | ? · F1 [SHOULD] |
| Quick Look | Space [LATER] | n/a |

Map shortcuts with `keyboard::listen()` and `Modifiers::command()`, which is ⌘ on macOS and Ctrl elsewhere. iced has no native menu bar, so every shortcut must be handled in-app. A native menu bar (e.g., `muda`) is [LATER].

### 6.5 Zoom, breadcrumbs and history [MUST]
- **Zoom in** (Return or double-click) makes the folder the zoom root. Its children become the top-level rows, and the treemap and cleanup list rescope to it. Zooming never re-scans anything.
- **Breadcrumbs** start at the scan root. The first crumb gets a volume icon (volume root), a home icon (home folder) or a folder icon. Separator "›" in `text.muted`. Each crumb is clickable and zooms out to that folder. When there isn't room, the middle crumbs collapse into "…" (first crumb + "…" + last two). The "…" tooltip shows the full path.
- **History**: each zoom pushes {zoom root, selection, scroll offset} onto a back stack. [‹] and [›] are disabled when empty. Zooming out re-selects the folder you came from, as Finder does.
- Expansion state belongs to the node, not the view, so zooming out restores the tree as it was.
- The zoom root's total is shown right-aligned in the path bar: "182 GB" plus "· 22% of disk" (or "of <scan root>").

### 6.6 Actions on the selected row [MUST]

| Action | Button label (macOS / Linux / Windows) | Behavior |
|---|---|---|
| Reveal | "Reveal in Finder" / "Show in File Manager" / "Show in Explorer" | macOS `open -R <path>`. Linux D-Bus `org.freedesktop.FileManager1.ShowItems`, falling back to `xdg-open <parent>`. Windows `explorer /select,<path>` |
| Copy path | "Copy Path" | Absolute path (no `~`) to the clipboard. Toast "Copied path" |
| Rescan | "Rescan" | Rescans the selected folder, or the zoom root if a file or nothing is selected. Old numbers stay visible (secondary color + scanning dot) until the subtree's new scan **finishes**, then everything swaps in one step with one re-order. No dip-then-grow |
| Move to Trash… | "Move to Trash…" / "Move to Trash…" / "Move to Recycle Bin…" | §9 |
| Zoom in | (Return, double-click, context menu) | §6.5 |
| Scan This Volume | for mount-point rows [SHOULD] | Starts a new scan of that volume |

**Selection bar contents**

| Selected | Left side | Buttons |
|---|---|---|
| Folder | icon · `~/path` (middle-truncated, 13 px) · "24.1 GB · 312,004 items · modified 2 days ago" (12 px secondary) | Reveal, Copy Path, Rescan, Move to Trash… |
| File | icon · path · size · modified | Reveal, Copy Path, Move to Trash… |
| Aggregate "N files" | "1,234 files directly in ~/Downloads" · total | Reveal (parent), Copy Path (parent) |
| Unreadable folder | ⚠ · path · "Couldn't read: <reason>" (wording from §10.1) | Reveal, Copy Path, plus Open Privacy Settings on macOS when the reason is EPERM |
| Other volume | volume icon · path · "Separate volume, not included" | Scan This Volume, Reveal |
| Unaccounted space | explanation (§5.6) | (macOS) Open Privacy Settings |
| Nothing | key hints | none |

"modified" = the newest mtime anywhere inside the folder, if the scanner provides it; otherwise the folder's own mtime. The ~ abbreviation is for display only.

### 6.7 Context menu [SHOULD]
Right-click on a row selects it and opens a small menu (iced overlay: `stack` + `float`/`pin` at the cursor, 6 px radius, shadow):
```
 Zoom In                    ⏎
 Reveal in Finder         ⌥⌘R
 Copy Path                 ⌘C
 ───────────────────────────
 Rescan                    ⌘R
 ───────────────────────────
 Move to Trash…            ⌘⌫
```
Disabled items are shown dimmed, not hidden, so the menu stays consistent.

### 6.8 Toasts [MUST]
Bottom-center overlay, 8 px above the selection bar, radius 8, raised background with shadow, 13 px. Auto-dismiss after 4 s (errors stay until closed with ×). At most one visible; a new toast replaces the old one. Toasts never shift the layout.

### 6.9 macOS packages [SHOULD]
`.app`, `.framework`, `.bundle`, `.photoslibrary`, `.xcarchive` and similar bundles show a package icon and a small muted "package" tag. They can be expanded like any folder. Trashing an item **inside** a package adds a warning to the confirmation: "This is inside “Photos Library.photoslibrary”. Removing parts of a package can break it. Use the app that owns it instead."

---

## 7. Cleanup suggestions panel [MUST]

The catalog of patterns is built by another agent. This section covers presentation only. Expected fields per match are in §16.2.

```
│  Cleanup     Map                            │  ← tabs (36 px). Active tab: 2 px accent underline
│ ─────────────────────────────────────────── │
│  In Library                                 │  12 px secondary: scope = zoom root
│  38.2 GB                                    │  28 px semibold (stat value)
│  likely reclaimable · + 12.4 GB to review   │  12 px secondary
│                                             │
│  ✓ SAFE TO DELETE · rebuilt automatically   │  section header 11 px medium, secondary, status icon
│  ▸ DerivedData                     24.1 GB  │  group row 13 px; size right-aligned
│    Xcode build data. Rebuilt on next build. │  reason 12 px secondary, 1 line, ellipsis
│  ▾ node_modules · 214              18.3 GB  │  multi-match group (count)
│    Reinstalled by npm / yarn / pnpm.        │
│    □ ~/dev/shop/node_modules   1.20 GB  8 mo│  child row 12 px: checkbox, path, size, age
│    □ ~/dev/old/node_modules     980 MB   2 y│
│    Show 212 more                            │
│  ▸ Homebrew downloads               3.20 GB │
│    Old bottles and source archives.         │
│                                             │
│  ! REVIEW FIRST · may contain your data     │
│  ▸ iOS simulators                  14.0 GB  │
│    Simulator devices and runtimes.          │
│  ▸ Installers in Downloads          4.40 GB │
│    Disk images you may still need.          │
│                                             │
│  Show 9 smaller suggestions                 │
│ ─────────────────────────────────────────── │
│  2 selected · 2.18 GB     [Move to Trash…]  │  bulk bar, only when something is checked [SHOULD]
│  Based on names and locations. Check before │  11 px muted disclaimer, always visible
│  deleting.                                  │
```

**Structure**
- **Scope = zoom root.** The header names it ("In Library"). Zooming rescopes the list, just as filters scope everything below them.
- **Stat block**: total of *Safe* items (28 px semibold, proportional figures) with the label "likely reclaimable", plus "+ X to review". During a scan: "≥ 9.80 GB likely reclaimable" and the line "Still scanning, more will appear."
- **Sections by risk**, in this order: **Safe to delete** (status good, ✓ icon), **Review first** (status warning, ! icon). *Caution* (high-risk) matches are **not listed** in v1, because the panel must never invite risky deletions. The section icon + label carries the meaning, never color alone.
- **Groups** in each section are ranked by total size, descending. A group = all matches of one rule (e.g., every `node_modules`). A single-match group shows the match directly (title = folder name, e.g., "DerivedData"). A multi-match group shows "title · count" and expands to child rows.
- **Child rows** are ranked by size and show the path relative to home or the scan root (middle-truncated), the size and the **age** = time since the project or folder was last modified ("8 mo", "2 y") [SHOULD]. Large and stale means the easiest win. Children appear in batches of 50 ("Show 212 more").
- Groups whose total is under 10 MB go behind "Show N smaller suggestions".
- A group row's tooltip, or an optional second reason line, shows the catalog's "how to clean properly" hint, e.g., "Or run `cargo clean`" or "Prefer Xcode › Settings › Platforms" [SHOULD].

**Interaction**
- **Click** a single-match group or a child row to **reveal it in the tree** [MUST]:
  - if needed, zoom to the scan root (this pushes history);
  - expand its ancestors and select the row;
  - scroll it to the vertical center;
  - flash its background twice over 600 ms;
  - highlight it in the treemap.

  Focus stays in the cleanup pane, so ↑/↓ keep browsing suggestions while the tree follows.
- **Hover** shows two icon buttons to the left of the size: *Show in Finder* and *Move to Trash…* [SHOULD]. The trash button follows exactly the §9 flow; there is no shortcut path around the confirmation.
- **Checkboxes** on child rows and group rows (a group checkbox selects all its children). The bulk bar shows "N selected · size" and [Move to Trash…] → one confirmation listing the first 5 paths + "and N more" [SHOULD].
- Keys: ↑/↓ move, →/← expand or collapse a group, Return reveals in the tree and moves focus there, Space toggles the checkbox, ⌘⌫ / Delete trashes the checked items (or the focused one).
- After a trash, the item stays in place, struck through and muted, with the size replaced by "In Trash". The totals update. It is removed on the next re-order or rescope (same as the tree, §9).
- Special case: items **inside the Trash** (e.g., `~/.Trash` itself) show "Empty the Trash in Finder to free this space" and only a Show in Finder button. dir-stat never empties the Trash.

**States**
- Scanning: items appear as matched folders finish. In-progress sizes show as "≥".
- Empty (scan complete, no matches): the check icon in `text.muted` and "No caches or build folders found in Library." Second line: "The largest folders are at the top of the list. Review them yourself."
- Errors during the scan: under the stat block, the line "Some folders couldn't be read, so suggestions may be incomplete." (warning icon).

**Tab label**: "Cleanup", followed by the total in secondary text once it is ≥ 100 MB ("Cleanup 38 GB"). Default tab = Cleanup; the last choice is remembered [SHOULD].

---

## 8. Visualization: treemap ("Map" tab) [SHOULD]

### 8.1 Decision
**Keep it as a side-pane tab and drop the sunburst** (sunburst [LATER]).

Why keep it:
- It shows the space distribution of the zoomed folder at a glance.
- It surfaces big items buried 2–3 levels deep without expanding anything.
- It makes the live scan feel alive.
- It is cheap in iced: one `canvas` with cached geometry plus a squarify routine of about 100 lines.

Risks and their mitigations:
- *Clutter*: it lives in a tab, never competes with the tree, and uses 3 hues + gray.
- *Confusion* (the GrandPerspective problem): direct labels, a legend, flat fills (no cushions or gradients), and one selection shared with the tree.
- *Motion*: throttled relayout, and nothing moves under the pointer.

Why not the sunburst: worse use of space, labels are hard to place on arcs, and areas are harder to compare.

### 8.2 What it shows
- The **zoom root**, rendered up to **3 levels deep**. A block is subdivided only if it is at least 48 × 32 px, so detail adapts to the available space. Children smaller than 4 px in either dimension aren't drawn; their area is filled by the parent.
- At most 200 children per level are laid out. The rest are merged into one "N more" block.
- Files appear as blocks (the listed large files plus one "N files" aggregate block per directory, as in §6.3). *Unaccounted space* and free space are **not** drawn.
- **Squarified layout** (Bruls et al.), children sorted by size descending for stable placement.

### 8.3 Anatomy
```
┌─ Developer 96.1 GB ────────────────────┬─ Containers 31.0 GB ────────┐   ← level-1 header strip (18 px), full slot color + label
│┌ Xcode 60.2 GB ───────┐┌ CoreSim ─────┐│┌ com.apple.x ─┐┌ com.y ────┐│   ← level-2 blocks: slot at 72%, label if it fits
││                      ││ 30.0 GB      │││              ││           ││
││                      ││              ││└──────────────┘└───────────┘│
││                      ││              │├─ Caches 18.0 GB ────────────┤   ← each top-3 group keeps its own hue
││                      ││              ││┌ Safari ─────┐┌ Spotify ───┐│
│└──────────────────────┘└──────────────┘│└─────────────┘└────────────┘│
├─ Application Support 15.2 GB ┬─ Logs ──┴─────────┬─ 12 more ─────────┤   ← all other top-level folders: neutral gray
│                              │                   │                   │
└──────────────────────────────┴───────────────────┴───────────────────┘
 ■ Developer 96.1 GB  ■ Containers 31.0 GB  ■ Caches 18.0 GB  ■ Other (14) 37.1 GB
   ↑ legend row (28 px): swatch + name + size; click selects the folder
```
(The box lines in the sketch stand for the 2 px gaps. Real blocks are flat fills with no outlines.)

- **2 px surface-colored gaps** between sibling blocks and around nested blocks. No drawn borders.
- **Level-1 blocks** (children of the zoom root) that are at least 80 × 40 px get an **18 px header strip** with "Name  Size" (12 px medium name + 11 px size), padded 6 px, truncated to fit or dropped if it doesn't fit. Their children are laid out below the strip. The strip uses the full slot color.
- **Level-2 blocks** get a single inline label (11 px, name only) if it fits with 6 px padding on both sides. Otherwise no label: never clipped mid-glyph, and the tooltip carries it.
- **Level 3**: unlabeled.
- Corner radius: 4 px on level-1 blocks, 2 px on nested ones.

### 8.4 Color
Color has **one job: identifying the zoom root's top-level groups.** Area already encodes size, so color doesn't re-encode it.

- The **three largest top-level children** get categorical slots 1–3. **All other top-level children share a neutral "Other" gray.** In a treemap any block can touch any other, so the palette must pass *all-pairs* CVD checks, and only three hues clear that bar (§11.4). Three hues plus gray is also simply calmer.
- **Color follows the entity, not its rank.** Once a folder has a slot, it keeps it while it stays in the top 3. A newcomer takes the slot that was freed. Slots are reassigned only when the zoom root changes.
- **Nesting**: level 1 = base slot color, level 2 = slot blended 72% over the surface, level 3 = base again (alternating keeps each nested layer distinct from its container). "N files" aggregate blocks = slot blended 52%.
- **Labels inside fills** are `#ffffff` or `#0b0b0b`, whichever has the higher APCA contrast against that fill (table in §11.4). Labels never use the series color.
- **Hover**: the block lightens (12% toward white) and the tree row under the pointer gets `bg.hover` (cross-highlight [SHOULD]).
- **Selection** (synced with the tree): 2 px `text.primary` outline + 2 px surface ring outside it. If the selection is a folder, its subtree stays at full strength and everything else is dimmed to 55% opacity [SHOULD]. Focus + context.
- **Cleanup coloring mode** [LATER] (gdu's "cleanup projection"): everything neutral except safe (status good) and review (status warning) candidates, with a legend using icons. The validator results for it are in §11.4.

### 8.5 Interaction
- **Hover tooltip**, drawn inside the canvas, 12 px from the cursor, flips at the edges. Value first, then labels:
  ```
  60.2 GB
  Xcode
  ~/Library/Developer/Xcode
  33% of Library · 301,220 items
  ```
- **Click** selects the item in the tree: expands its ancestors, scrolls it into view, updates the selection bar.
- **Double-click** zooms into the folder (or into the file's parent). **Right-click** opens the same context menu as the tree [SHOULD].
- No wheel zoom, no panning. Navigation is zoom root + breadcrumbs only, as in the tree.
- The map has no keyboard focus in v1. While the Map tab is showing, Tab doesn't move focus into the side pane. The map mirrors the tree's selection instead.
- The **legend row** (28 px, 11 px text) shows the 3 colored groups and "Other (N)" with sizes. Clicking a legend entry selects that folder.

### 8.6 Live behavior and performance
- While scanning, relayout at most **once per second**, never while the pointer is over the map (deferred up to 5 s), anchored by sort order. After completion, relayout only when the zoom root, the pane size or the data changes (rescan, trash).
- Two cache layers: **blocks** (rebuilt on relayout) and **overlay** (hover, selection, tooltip). Hovering never re-tessellates the blocks.
- Empty states: zoom root of 0 bytes → "Nothing to show: this folder is empty." Pane narrower than 240 px → "Widen the pane to see the map."

---

## 9. Destructive actions: Move to Trash [MUST]

**Flow**: trigger (button, ⌘⌫/Delete, context menu, cleanup hover button) → **confirmation dialog (always)** → move to Trash in the background → update the tree → toast.

```
                 ┌──────────────────────────────────────────────────────┐
                 │  Move “DerivedData” to the Trash?                    │  15 px semibold
                 │                                                      │
                 │  ▭  ~/Library/Developer/Xcode/DerivedData            │  13 px, middle-truncated
                 │     24.1 GB · 312,004 items · modified 2 days ago    │  12 px secondary
                 │                                                      │
                 │  ✓  Xcode build data. Rebuilt on the next build.     │  catalog reason (if matched)
                 │                                                      │
                 │  The space is freed when you empty the Trash.        │  12 px secondary
                 │                                                      │
                 │                        [ Cancel ]  [ Move to Trash ] │  destructive = critical fill
                 └──────────────────────────────────────────────────────┘
```

- Modal card: 440 px wide, radius 12, raised background, shadow; the scrim dims the window (§11.2). Esc always cancels. **Return activates the default button**: *Move to Trash* for *Safe* catalog matches, *Cancel* for everything else. That way an unknown folder can't be trashed by pressing ⌘⌫ and then Return; the user has to click the button or Tab to it and press Space. The destructive button is `status.critical` with white text.
- **Extra warning lines** (warning icon + text) when they apply:
  - inside a macOS package (§6.9);
  - a *Review first* match ("May contain data you want: <reason>");
  - the item is a Git working tree ("Contains a Git repository") [LATER];
  - the item is very large (more than 25% of the scan root): "This is 31% of everything scanned."
- **Protected paths: refuse, don't confirm.** A dialog with only [OK]: "dir-stat won't move “Library” to the Trash because macOS or your apps need it."
  - Two kinds of rule. **Whole subtree refused** = the folder and everything inside it. **Folder itself refused** = the folder can't be trashed, but items inside it can.
  - macOS, whole subtree: `/System`, `/usr` (except `/usr/local/**`), `/bin`, `/sbin`, `/private/etc`, `/dev`, and anything on a read-only volume.
  - macOS, folder itself: `/`, `/Library`, `/Applications` (apps inside are allowed), `/Users`, `/Volumes`, `/private`, `/private/var`, `/opt`, `/cores`, the home folder, the home folders `~/Library`, `~/Desktop`, `~/Documents`, `~/Downloads`, `~/Movies`, `~/Music`, `~/Pictures`, `~/Public`, `~/Applications`, and the scan root.
  - Linux, whole subtree: `/bin`, `/boot`, `/dev`, `/etc`, `/lib*`, `/proc`, `/run`, `/sbin`, `/sys`, `/usr` (except `/usr/local/**`).
  - Linux, folder itself: `/`, `/home`, `/root`, `/srv`, `/var`, `/opt`, the home folder, the XDG user folders, and the scan root.
  - Windows: whole subtree `C:\Windows`; folder itself: drive roots, `C:\Program Files*`, the user profile root.
  - Root-owned system files mostly can't be trashed anyway (the OS refuses). These rules exist mainly to protect **user-owned** critical folders.
- **Implementation**: `trash` crate. On macOS use **`DeleteMethod::NsFileManager`**, which avoids the Finder-automation permission prompt; the trade-off is that "Put Back" may be missing. Freedesktop trash on Linux, the Recycle Bin on Windows. The trash runs off the UI thread. Meanwhile the row shows "Moving to Trash…" with a spinner.
- **After success**: the row stays **in place**, struck through (rich-text span `.strikethrough(true)`) and muted, with the Size cell reading "In Trash". Its size is subtracted from every ancestor right away. The row disappears at the next re-order, zoom or sort, so the layout never jumps (gdu's approach). Toast: "Moved “DerivedData” to the Trash. Empty the Trash to free 24.1 GB." If `~/.Trash` is part of the scan, mark that row "outdated" (tooltip: "Rescan to update").
- **Failure**: the row is restored and an error toast stays: "Couldn't move “X” to the Trash: Operation not permitted." Volumes without Trash support get the message "This volume has no Trash. dir-stat doesn't delete files permanently." **No permanent delete in v1** (Shift-delete [LATER], behind its own explicit dialog).
- **Undo** [LATER]: "Put Back" via the Finder method, or a recorded trash location.

---

## 10. Errors and permissions

### 10.1 In the tree [MUST]
- **Unreadable folder** (the folder itself couldn't be listed): filled warning badge after the name (12 px amber triangle with a dark "!"; it reads well in both themes), name in `text.secondary`, size "—", not expandable. Tooltip with the reason:
  - macOS EPERM: "Protected by macOS privacy settings. dir-stat needs Full Disk Access to read it."
  - EACCES: "Permission denied. The folder belongs to another user or the system."
  - timeout: "The volume didn't respond."
- **Folder containing unreadable folders**: a small *outline* triangle in `text.secondary` after the name. Tooltip: "3 folders inside couldn't be read, so the real size may be larger." Shown whether or not the row is expanded.
- Unreadable files count toward the folder's error count. They get no rows of their own.

### 10.2 Summary and list [MUST count, SHOULD list]
- The activity field, line 2, reads "23 folders couldn't be read" (the link part in accent text, §11.2) once the scan has finished or been stopped. During the scan it is appended to the stats as "· 23 unreadable".
- Clicking opens a **popover** (anchored to the activity field, 480 × 360 max):
  ```
  ┌────────────────────────────────────────────────────────────┐
  │ 23 folders couldn't be read                       [Copy]   │
  │ ────────────────────────────────────────────────────────── │
  │ ⚠ ~/Library/Mail                  Protected by macOS       │
  │ ⚠ ~/Library/Safari                Protected by macOS       │
  │ ⚠ /Users/guest                    Permission denied        │
  │ …                                                          │
  │ ────────────────────────────────────────────────────────── │
  │ Give dir-stat Full Disk Access to include protected        │
  │ folders.                       [Open Privacy Settings]     │
  └────────────────────────────────────────────────────────────┘
  ```
  Rows can be clicked to reveal them in the tree. [Copy] copies "path<TAB>reason" lines. The scanner keeps up to 200 samples, so the list ends with "and N more" when the count is higher.
- Entries that **vanished or changed during the scan** (ENOENT / ENOTDIR / ELOOP) are not errors. They are counted as "changed during scan" and mentioned only in the activity field tooltip.

### 10.3 macOS Full Disk Access hint [MUST]
- Triggered by the **first EPERM** during a scan on macOS. Under TCC, EPERM ("Operation not permitted") is distinct from EACCES ("Permission denied").
- Shown as an **info bar at the bottom of the tree pane**, above the selection bar. It shrinks the viewport from the bottom, so no row moves:
  ```
  (i) Some folders are protected by macOS, so their sizes are missing. Give dir-stat Full Disk
      Access in System Settings › Privacy & Security, then restart dir-stat and scan again.
                                                         [Open Privacy Settings]  [Not Now]
  ```
- If dir-stat was started from a terminal (`TERM_PROGRAM` or `__CFBundleIdentifier` is set), name that app: "…give **Terminal** Full Disk Access (dir-stat was started from it)…".
- "Not Now" hides the bar for the rest of the session. It never pops a modal on its own.
- Optional permission probe for the start-screen footnote [LATER]: check access silently, e.g., try to open `/Library/Application Support/com.apple.TCC/TCC.db` (fails with EPERM without access, no prompt).
- Linux/Windows: no bar. The count and the list are enough.

### 10.4 Scan root unreadable [MUST]
Full-pane state in the tree area:
```
                       ⚠
         Can't read “Documents”
         macOS needs your permission to show this folder.
         [Open Privacy Settings]   [Choose Another Folder]   [Try Again]
```

---

## 11. Visual design system

### 11.1 Theme [MUST]
- Light and dark themes **follow the OS live**.
  - The theme fn returns **`None`**, so iced picks the built-in `Theme::Light` / `Theme::Dark` and switches when the OS does.
  - Every dir-stat style function (and the canvas `draw`) selects the dir-stat light or dark token set from `theme.extended_palette().is_dark`. Every widget is styled explicitly (including scrollbars and tooltips), so iced's default colors never show.
  - **Don't return `Some(Theme::custom(..))`.** On macOS that pins the window appearance, and `system::theme_changes()` then stops firing (`iced-api.md` gotcha 10).
  - On Linux this needs the `linux-theme-detection` feature, which is a default feature in iced 0.14.
  - Manual override in a settings menu [LATER]. It would accept losing live switching while the override is set.
- Warm neutrals with a single blue accent, taken from the validated reference palette of the dataviz method (`palette.md`) and extended with UI states.

### 11.2 Color tokens

| Token | Light | Dark | Use |
|---|---|---|---|
| `bg.window` | `#f4f4f1` | `#141413` | Toolbar, path bar, selection bar, start screen background |
| `bg.surface` | `#fcfcfb` | `#1a1a19` | Tree, side pane, activity field |
| `bg.raised` | `#ffffff` | `#232321` | Cards, popovers, dialogs, toasts, menus |
| `bg.hover` | `#f2f2f1` | `#252525` | Row/card hover (ink 4% / white 5%) |
| `bg.selected` | `#dfeaf6` | `#233b56` | Selected row, pane focused (accent 14% / 30%) |
| `bg.selected.unfocused` | `#ebebea` | `#2f2f2e` | Selected row, focus elsewhere |
| `border.hairline` | `#e1e0d9` | `#2c2c2a` | Dividers, card borders, header bottom rule |
| `border.strong` | `#c3c2b7` | `#383835` | Hovered cards, input borders |
| `text.primary` | `#0b0b0b` | `#f0efec` | Names, sizes, titles |
| `text.secondary` | `#52514e` | `#c3c2b7` | Meta, headers, reasons, scanning sizes |
| `text.muted` | `#898781` | `#898781` | Placeholders "—", separators, disabled. **Never for essential info** |
| `accent` | `#2a78d6` | `#3987e5` | Bars, focus ring, active tab underline, progress |
| `accent.text` | `#256abf` | `#6da7ec` | Links / accent-colored text |
| `accent.fill` | `#256abf` | `#256abf` | Primary buttons (white text). Hover `#1c5cab`, pressed `#184f95` |
| `bar.track` | `#ebebea` | `#2c2c2b` | Share-bar track |
| `meter.track` | `#e6e5df` | `#2c2c2a` | Capacity-meter track |
| `status.good` | `#0ca30c` | `#0ca30c` | Safe, done ✓ |
| `status.warning` | `#fab219` | `#fab219` | Review, warning badges (with dark "!") |
| `status.critical` | `#d03b3b` | `#d03b3b` | Destructive buttons, almost-full meter |
| `chip.good.bg` / `chip.warning.bg` / `chip.critical.bg` | `#daf0da` / `#fcf2db` / `#f6e1e0` | `#182d17` / `#392f19` / `#331f1e` | Pill/chip backgrounds (status 14%) |
| scrim | `rgba(11,11,11,0.32)` | `rgba(0,0,0,0.50)` | Behind dialogs |
| shadow | `rgba(11,11,11,0.14)` y 8 blur 24 | `rgba(0,0,0,0.50)` y 8 blur 24 | Popovers, dialogs, toasts |

Status colors are fixed across themes, reserved for state, and **always paired with an icon + label**.

**Measured contrast** (WCAG 2 ratio; APCA where noted):

| Pair | Light | Dark |
|---|---|---|
| text.primary on surface | 19.2 : 1 | 15.2 : 1 |
| text.secondary on surface / window | 7.7 / 7.2 | 9.7 / 10.3 |
| text.muted on surface | 3.5 (non-essential only) | 4.9 |
| text.primary / secondary on `bg.selected` | 16.2 / 6.5 | 10.0 / 6.4 |
| accent.text on surface | 5.3 | 7.0 (APCA Lc 52) |
| white on accent.fill (buttons) | 5.4 (APCA Lc 82) | same (button vs dark surface 3.2) |
| white on status.critical (buttons) | 4.8 | 4.8 |
| bar fill vs track / vs `bg.selected` | 3.7 / 3.6 | 3.8 / 3.2 |
| meter fill vs meter.track (normal / critical) | 3.5 / 3.8 | 3.9 / 2.9 (critical always has a text label) |
| focus ring (accent) vs surface | 4.3 | 4.8 |

### 11.3 Typography [MUST]
- **Font: bundle Inter (OFL) as static TTFs for every weight used (Regular 400, Medium 500, SemiBold 600) and set it as `.default_font()` [MUST].**
  - Why: with `Font::DEFAULT`, cosmic-text 0.15 asks for "Open Sans" and falls back to SF on macOS. SF is a variable font registered only at weight 400, so **Medium/SemiBold fall back to Apple SD Gothic Neo and Bold to Menlo Bold**. This was verified on this machine (`iced-api.md` gotcha 1). The same fallback also draws "…" as a centered "⋯".
  - Bundling gives the same look on macOS, Linux and Windows for about 1 MB.
  - Fallback if bundling is rejected: use only weight 400 and build hierarchy from size and color.
- Scale, in logical px (size / line height / weight):

| Role | Spec | Where |
|---|---|---|
| Display | 24 / 30 / 600 | Start screen title |
| Title | 15 / 20 / 600 | Dialog titles |
| Stat | 28 / 34 / 600 | Cleanup total (proportional digits) |
| Body | 13 / 18 / 400 | Rows, breadcrumbs, cards, toasts |
| Body strong | 13 / 18 / 500 | Buttons, zoom-root total (600) |
| Small | 12 / 16 / 400 | Meta lines, reasons, header labels (500) |
| Caption | 11 / 14 / 500 | Pills, legend, activity line 2, section headers (uppercase strings) |

- Numbers are right-aligned in columns. iced can't enable `tnum`, which is acceptable with right alignment.

### 11.4 Data-viz palette (treemap, bars)
Checked with the dataviz skill's `validate_palette.js` (OKLab ΔE, Machado 2009 CVD simulation):

| Set | Mode / surface | Result |
|---|---|---|
| Treemap slots `#2a78d6, #eb6834, #1baf7a` | light / `#fcfcfb`, all pairs | PASS. Worst CVD ΔE 9.2 (aqua–orange, deutan), normal-vision 24.0. Aqua is 2.74:1 vs surface (WARN), relieved by direct labels, the legend and the tree as table view |
| Treemap slots `#3987e5, #d95926, #199e70` | dark / `#1a1a19`, all pairs | PASS. Worst CVD ΔE 9.4, normal-vision 20.9, all ≥ 3:1 |
| Cleanup mode `#0ca30c` vs `#fab219` | both | Good↔warning CVD ΔE 11.3 (protan). Neutral gray intentionally fails the chroma check because it is the de-emphasis color, not a series |

Treemap fills and label colors (label by APCA):

| Group | Light L1 (base) | Light L2 (72%) | Dark L1 (base) | Dark L2 (72%) |
|---|---|---|---|---|
| Slot 1 blue | `#2a78d6` · white | `#659de0` · white | `#3987e5` · white | `#3068ac` · white |
| Slot 2 orange | `#eb6834` · white | `#f0916c` · ink | `#d95926` · white | `#a44722` · white |
| Slot 3 aqua | `#1baf7a` · white | `#5ac59e` · ink | `#199e70` · white | `#197958` · white |
| Other (gray) | `#a8a69e` · ink | `#c0beb8` · ink | `#5f5e59` · white | `#4c4b47` · white |

"ink" = `#0b0b0b`. Aggregate "N files" blocks use a 52% blend, e.g., light blue `#8fb7e8` (ink) and dark blue `#2a5383` (white). Selection outline = `text.primary` + 2 px surface ring.

### 11.5 Spacing, radii, sizes [MUST]
- **Spacing scale** (4 px grid): 2 · 4 · 8 · 12 · 16 · 24 · 32 · 48. Pane padding 12, card padding 16, dialog padding 24, gap between buttons 8.
- **Radii**: 4 (bars, chips, small controls) · 6 (buttons, menus, inputs) · 8 (cards, activity field, popovers, toasts) · 12 (dialogs).
- **Fixed sizes**: **tree row 28 px (fixed, required for virtualization)** · column header 28 · toolbar 44 · path bar 32 · selection bar 44 · side-pane tab bar 36 · cleanup group row 44 (two lines) · cleanup child row 28 · buttons 28 tall (toolbar) / 32 (dialogs) · icon buttons 28 × 28 · indent 16 per level · icons 16 (rows) / 14 (buttons).

### 11.6 Size bars and meters [MUST]
- **Share bar** (tree): 72 × 6 px, radius 3, track `bar.track`, fill `accent`, fill width = share × 72 (min 2 px when > 0). Its rounded data end makes it read as a bar, not a block. One series, so no legend.
- **Capacity meter** (start screen): full card width × 6 px, radius 3, `meter.track`, fill accent or critical (< 10% free, with "Almost full" text + icon).
- **Progress line** (activity field): 2 px along the bottom edge, accent, determinate or sweeping (§5.2).
- No gradients, no cushion shading, no 3D anywhere.

### 11.7 Icons [SHOULD vector; MUST fallback]
- **Draw ~20 simple vector icons** on a 16 px grid with 1.5 px strokes, rounded caps and joins, colored with text tokens: either iced `canvas` paths cached per (icon, color, size), or iced's `svg` feature with ~20 inline SVG strings (e.g., a Lucide subset, ISC license). **No icon font.**
- Set: chevron-right, chevron-down, folder (fill = accent 18%, stroke = accent), file, package, stack (aggregate), drive, home, network, warning (filled amber triangle + dark "!"), warning-outline, check-circle, info, trash, reveal (arrow out of box), copy, refresh, stop (filled square), plus, back, forward, sidebar, question, dotted-box (unaccounted).
- **Unicode fallback** if vectors slip: `▸ ▾ › · — ≥ … ✓ ⌂`. These render on macOS through system font fallback with the default `Shaping::Auto`. Never force `Shaping::Basic` on symbol text, because it shows tofu. Avoid emoji, and avoid ⚠ without a text variation selector (it can render as a color emoji).

### 11.8 Motion [MUST be minimal]
- Scanning dot pulse 1.2 s · indeterminate sweep 1.4 s · reveal flash 2 × 300 ms · toast 4 s · tooltip delay 500 ms · re-order tick 1.5 s · treemap relayout ≤ 1 / s while scanning.
- No sliding rows or animated reorders in v1 (iced 0.14's `Animation` API makes row transitions possible [LATER]). Hover changes are instant.

---

## 12. Formatting rules [MUST]

### 12.1 Units: decimal (SI), everywhere
**1 kB = 1,000 B.** Symbols: `B kB MB GB TB PB`. This matches macOS Finder and Disk Utility (decimal since 10.6), GNOME (GLib), the iOS settings app and drive labels ("1 TB" disk = "1.00 TB", not "931 GiB"). The start screen meters and the tree then agree with Finder. Binary units (KiB…) as a setting [LATER]. Never mix the two in one build.

### 12.2 Sizes
- < 1,000 B: integer + " B" ("0 B", "512 B").
- Otherwise scale to the unit where 1 ≤ value < 1000 and keep **3 significant digits**: "1.23 GB", "12.3 GB", "123 GB". Keep trailing zeros ("4.50 GB") for column consistency. Rounding that reaches 1000 moves up a unit ("999.6 MB" → "1.00 GB").
- A lower bound gets the prefix "≥ " (narrow space). Pending: "—".
- Exact bytes in tooltips with thousands separators: "24,113,402,880 bytes".
- **Size = size on disk** (allocated blocks, like `du` and DaisyDisk): what deleting actually frees. Hard links are counted once. APFS clones may be counted twice (documented in the Unaccounted tooltip and help). iCloud files that are not downloaded count as 0 B, because they take no local space. The scanner never downloads them.

### 12.3 Counts
- Columns and pills: < 10,000 → "8,912"; < 1 M → 3 significant digits + "k" ("12.3k", "420k"); otherwise "1.21M".
- Prose (selection bar, tooltips, dialogs): full numbers, "312,004 items". Singular forms: "1 item", "1 folder couldn't be read".

### 12.4 Percentages
- ≥ 10 → "42%"; 0.1–10 → "4.2%"; > 0 and < 0.1 → "<0.1%"; 0 → "0%". Always share of the parent, unless the text says "of disk" or "of <name>".

### 12.5 Time
- Elapsed "0:42", "12:05", "1:02:33". ETA "about 2 min left", "less than a minute left".
- Relative dates: "just now", "5 min ago", "yesterday", "3 days ago", "8 months ago", "2 years ago". Compact form in cleanup rows: "5 d", "8 mo", "2 y". Absolute dates in tooltips, local format "2026-03-14 18:22".
- Rate "9.8k items/s".
- Locale: en-US separators in v1. Localization [LATER].

### 12.6 Truncation
- iced 0.14 text has **no ellipsis**. Use `Wrapping::None` + clipping as a safety net, plus a helper that pre-truncates by estimated width (average glyph width ≈ 0.56 em for 13 px system sans, refined per character class).
- **Names**: end ellipsis ("Application Supp…").
- **Paths**: middle ellipsis that keeps the first component and the last 1–2 components ("~/Library/…/Caches/com.apple.Safari").
- Show the full text in the tooltip whenever something is truncated.

---

## 13. Empty, loading and edge states

| Situation | Presentation |
|---|---|
| Start screen, volumes loading | Cards render immediately with the name. The meter area says "Checking…" until statfs returns |
| Scan starting, root not listed yet | 6 placeholder rows (muted 60% bars). After 1 s, the line "Reading /Users/max…" |
| Empty folder (zoom root or expanded) | Tree area / child row: "This folder is empty." (`text.secondary`, centered for the root) |
| Root unreadable | Full-pane state (§10.4) |
| Scan stopped | Activity field "Scan stopped" + Rescan. Partial rows show "≥" |
| Zoomed into a folder that later goes to Trash elsewhere (e.g., via cleanup) | Zoom out to the nearest surviving ancestor. Toast explains |
| Cleanup: nothing found / still scanning / errors | §7 states |
| Map: empty or too narrow | §8.6 |
| Network volume stops responding mid-scan | Those folders become unreadable (timeout) and the scan continues |
| Very many children (> 50,000) | Fine thanks to virtualization. Sorting happens off the UI thread if it takes more than 16 ms [SHOULD] |
| Window at minimum size | §6.1 responsive rules. Wireframe below |

**Minimum window (800 × 500), side pane hidden**
```
┌──────────────────────────────────────────────────────────────────────────────┐
│ [+]      ╭─────────────────────────────────────────────────╮        [?] [◫]  │
│          │ ✓ Library · 182 GB · 1.20M items      [Rescan]  │                 │
│          ╰─────────────────────────────────────────────────╯                 │
│ [‹][›] ⌂ max › Library                                           182 GB      │
│ Name                                       Size ▾  % of parent       Items   │
│ ▸ ▭ Developer                            96.1 GB   ████▉    53%      420k    │
│ ▸ ▭ Containers                           31.0 GB   █▋       17%      210k    │
│ ▸ ▭ Caches               [● Cache]       18.0 GB   ▉       9.9%      190k    │
│ ▸ ▭ Application Support                  15.2 GB   ▊       8.4%     96.0k    │
│ ...                                                                          │
│ ▭ ~/Library/Developer · 96.1 GB          [⌕] [⧉] [⟳] [Move to Trash…]        │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Shortcut overlay (?)** [SHOULD]: a centered raised card, 560 px wide, two columns (Navigate · Actions) listing §6.4. Esc closes it.

---

## 14. Platform and iced 0.14 implementation notes

**Platform**
- **macOS**:
  - Scanning "/" must not double count the firmlinked Data volume (`/usr/share/firmlinks`: /Users, /Applications, /Library, /private, /opt, /usr/local, …).
  - `/Volumes/Macintosh HD` is a symlink to `/`. Never follow it.
  - TCC: Desktop/Documents/Downloads and removable/network volumes trigger consent prompts. Other protected areas fail with EPERM and no prompt. So the Home scan primer and the EPERM-driven Full Disk Access bar (§10.3).
  - Trash via `NsFileManager` (no Automation prompt).
  - Reveal via `open -R`.
- **Linux**:
  - Stay on one filesystem. Mount points become "other volume" stubs.
  - Reveal via the D-Bus FileManager1 interface, or `xdg-open` on the parent.
  - Freedesktop trash.
  - Theme detection through `linux-theme-detection` (mundy).
- **Windows** (nice-to-have): drive letters on the start screen, `explorer /select,`, Recycle Bin wording, Ctrl-based shortcuts.

**iced 0.14 mapping**

| Need | Approach |
|---|---|
| Virtualized tree, 28 px rows | `scrollable` with `.id()` + `on_scroll(Viewport)`. Render only the visible slice in a `column` between two fixed-height spacers. Rebuild the flattened visible-row list only when expansion or order changes. Use `widget::operation::scroll_to` for anchoring and reveal. If per-row widgets prove slow, render the rows in one `canvas` with custom hit testing |
| Fixed header above the list | A separate `row` outside the scrollable, sharing column widths |
| Row mouse input | `mouse_area(container(row))`, never wrapping a `button` (a button swallows presses). Handlers: `on_press`, `on_double_click`, `on_right_press`, `on_enter` / `on_exit` / `on_move` (also feeds the "pointer active" timer for calm sorting) |
| Tooltips | `tooltip(...).delay(Duration::from_millis(500))` |
| Treemap | `canvas` with two `Cache`s (blocks, overlay). Custom tooltip drawn in the canvas |
| Popovers, menus, dialogs, toasts, info bar | `stack!` + `float` / `pin` + `opaque` / `mouse_area` scrim. Info bar = normal layout at the bottom of the tree pane |
| Keyboard | `keyboard::listen()` subscription, routed by focused region. `Modifiers::command()` for ⌘/Ctrl |
| Theme follows OS | Theme fn returns `None`. Style fns pick dir-stat tokens from `theme.extended_palette().is_dark` (§11.1) |
| Window | `window::Settings { size: 1200×800, min_size: 800×500 }`. `window::Event::FileDropped` / `FileHovered` for drag & drop |
| Clipboard | `iced::clipboard::write` |
| Reveal in Finder / file manager | `opener::reveal` (feature `reveal`). macOS `open -R`. Linux D-Bus FileManager1 / portal, falling back to the parent folder. Run it off the UI thread |
| Native folder picker | `rfd::AsyncFileDialog::pick_folder()`, with the future built in `update` (main thread) and run via `Task::perform`. The first panel takes about 0.6 s to open on macOS, so show the button as pressed right away |
| Animation ticks | `time::every` or `window::frames()`, subscribed **only** while something animates |
| No ellipsis, no native menus, no screen reader | §12.6. Shortcuts in-app. Keyboard completeness + contrast compensate. Accessibility (AccessKit) [LATER] |

**Performance budgets** [MUST]
- UI stays at 60 fps on scroll and at least 30 fps during a 2M-item scan.
- Scanner→UI batches ≤ 20/s (coalesced, the scanner never waits for the UI). A re-order tick costs O(expanded structure). Never re-flatten and re-sort the whole tree per batch: that was measured at up to 75 ms for 160k rows (`iced-api.md` §h).
- The treemap layout stays under 8 ms for 200 × 3-level blocks.

**Persistence** [SHOULD]: window size, side pane visibility/tab/width, sort column/direction and recents, stored as JSON in the platform config dir (`~/Library/Application Support/dir-stat/` on macOS).

---

## 15. Feature priority matrix

| Area | MUST (v1) | SHOULD (v1 if cheap) | LATER |
|---|---|---|---|
| Start | Home, system disk, mounted volumes with capacity meters; Choose Folder…; CLI `dir-stat <path>`; keyboard on cards | Drag & drop; Recent; macOS privacy footnote + Home primer | Favorites/pins, network discovery, permission probe |
| Scan | Live tree from the first listing; activity field (items, bytes, elapsed, rate, current path); determinate for volume roots, otherwise indeterminate; Stop keeps results; per-row states; calm sorting; trash only for done rows | ETA; "4 of 23 folders done"; Unaccounted space row | Scan expanded folders first; pause/resume; attention request on completion; live FS watching |
| Tree | Name / Size / % of parent / Items; size-descending default; header sort (size, name, items); single selection; full keyboard map; zoom + breadcrumbs + history; files: top-N + aggregate row; selection bar; toasts; responsive columns | Context menu; type-to-select; ⌥→ expand largest path; cleanup pills; package handling; mouse back/forward; "Scan This Volume" | Multi-select; filter/search; apparent-size toggle; Modified column; Quick Look; Open in Terminal; export; global largest-files list |
| Cleanup | Side-pane tab scoped to the zoom root; stat block; Safe / Review sections; groups ranked by size with reason; click → reveal in tree; empty/scanning states; disclaimer | Hover actions; checkboxes + bulk trash; age column; "how to clean" hints; smaller-items fold | Hide/ignore suggestions; custom rules; Caution section |
| Map | (none) | Treemap tab: zoom root, 3 levels, 3 hues + gray, labels, legend, hover tooltip, click select, double-click zoom, selection outline + dimming, throttled live relayout | Cleanup coloring mode; sunburst; file-type coloring; animated transitions |
| Delete | Move to Trash with confirmation; protected-path refusal; in-place struck-through row; error handling | Package / review-risk / huge-item warnings in the dialog | Undo (Put Back); permanent delete behind a separate dialog; Git-repo warning |
| Errors | Unreadable badges (self vs. inside); count in the activity field; macOS FDA info bar; root-unreadable state | Error list popover with copy | Scan as administrator |
| Design | Light/dark following the OS (theme fn `None` + token-picking style fns); bundled Inter (400/500/600); tokens, type scale, spacing, 28 px rows, decimal units, formatting rules | Vector icons (canvas/svg); persistence of UI state; draggable pane divider | Theme override, density setting, binary units, localization, native menu bar, accessibility |

---

## 16. Contracts this UX depends on (handoff notes)

### 16.1 From the scanner
1. **Batched update stream**, ≤ 20/s (as designed in `scan-engine.md`: one `DirListing` per directory, parents before children):
   - new nodes (id, parent, name, kind: dir / file / symlink / mount / package);
   - size and item-count deltas propagated to ancestors (or enough data for the UI to do it);
   - per-directory state transitions `waiting → scanning → done | error(kind)`, where a directory is *done* only when its whole subtree is.
2. **Progress snapshot**: items, bytes, elapsed, current path, and directories done / total among the root's children.
3. **Sizes**: allocated bytes (required), apparent bytes (optional), hard links counted once.
4. **Per directory**: every file ≥ 1 MiB (name + size), an aggregate (count, bytes) of the smaller ones, an on-demand listing of the small files when the aggregate row is expanded, and the newest mtime in the subtree (optional, for "modified" and staleness).
5. **Errors**: path + kind (EPERM / EACCES / timeout / other), counted per directory and rolled up to ancestors.
6. **Volume info**: mount point, display name, capacity, container-used, volume-used, flags (external, network, read-only). The traversed-volume set for the determinate denominator.
7. **Control**: stop (keep partial results), rescan subtree (atomic swap on completion), and a priority hint for folders that are expanded or visible [LATER].

### 16.2 From the cleanup catalog (per match)
`rule_id`, `group_title` (e.g., "node_modules"), `short_label` for the pill ("Deps"), `risk` (`safe` | `review` | `caution`), `reason` (≤ 70 chars, sentence case, ends with a period), optional `how_to_clean` (≤ 60 chars, e.g., "Or run `cargo clean`."), optional `project_last_modified`, `path`. Sizes come from the tree, so the panel always agrees with it.

---

## 17. Sources

DaisyDisk guide: [Disk overview](https://daisydiskapp.com/guide/4/en/DisksOverview/), [Understanding the disk map](https://daisydiskapp.com/guide/4/en/UnderstandingSunburst/), [Deleting files](https://daisydiskapp.com/guide/4/en/DeletingFiles/), [Hidden space](https://daisydiskapp.com/guide/4/en/HiddenSpace/), [Full Disk Access](https://daisydiskapp.com/guide/4/en/FullDiskAccess/), [Hotkeys](https://daisydiskapp.com/guide/4/en/Hotkeys/), [What is safe to delete](https://daisydiskapp.com/guide/4/en/WhatToDelete/), [Mismatches with Finder](https://daisydiskapp.com/guide/4/en/FinderMismatch/). Reviews: [Today on Mac](https://www.todayonmac.com/daisydisk-where-did-all-your-mac-storage-go/), [The High Tech Society](https://thehightechsociety.com/daisydisk-review/), [Crumb vs DaisyDisk](https://cleanwithcrumb.com/blog/crumb-vs-daisydisk-2026-why-a-disk-map-isn-t-enough-to-free-space/) (written by a competitor).
GrandPerspective: [App Store reviews](https://apps.apple.com/us/app/grandperspective/id1111570163?mt=12&see-all=reviews&platform=mac), [Nektony roundup](https://nektony.com/reviews/best-disk-space-analyzer-for-mac) (written by a competitor), [Cindori](https://cindori.com/reviews/best-mac-disk-analyzer-apps), [DEV guide](https://dev.to/kushkrg/how-to-fix-macbook-storage-using-grandperspective-step-by-step-d6l).
OmniDiskSweeper: [Omni Group](https://www.omnigroup.com/more), [NullDisk review](https://nulldisk.app/reviews/omnidisksweeper/), [Podfeet](https://www.podfeet.com/blog/2025/08/disk-sweeping-apps/), [InsanelyMac](https://www.insanelymac.com/blog/best-disk-space-analyzers-mac/).
Disk Inventory X: [screenshots page](https://www.derlien.com/screenshots/index.html), [Softpedia](https://mac.softpedia.com/get/Utilities/Disk-Inventory-X.shtml), [Disk Inventory Z fork](https://github.com/danifunker/disk-inventory-z/).
WinDirStat: [Directory list docs](https://documentation.help/WinDirStat/directorytree.htm), [Computer Hope](https://www.computerhope.com/tips/tip225.htm), [LinDirStat PR (2.6 absolute percentages)](https://github.com/danjboyd/LinDirStat/pull/52).
WizTree: [guide](https://diskanalyzer.com/guide). SpaceSniffer: [official](http://www.uderzo.it/main_products/space_sniffer/), [Softonic](https://spacesniffer.en.softonic.com/), [FileCroco](https://www.filecroco.com/download-spacesniffer/). TreeSize: [JAM Software](https://www.jam-software.com/treesize_free). Windows comparisons: [WindowsForum](https://windowsforum.com/news/top-10-windirstat-alternatives-for-faster-disk-cleanup-and-visualization.384876/), [Univik](https://univik.com/blog/best-disk-space-analyzer-windows.html).
QDirStat: [README](https://github.com/shundhammer/qdirstat). Baobab: [apps.gnome.org](https://apps.gnome.org/Baobab/), [help](https://help.gnome.org/users/baobab/stable/), [source](https://gitlab.gnome.org/GNOME/baobab) (location list, folder display, window strings). Filelight: [apps.kde.org](https://apps.kde.org/filelight/).
ncdu: [manual](https://dev.yorhel.nl/ncdu/man). gdu: [README](https://github.com/dundee/gdu). dua: [README](https://github.com/Byron/dua-cli), [releases](https://github.com/byron/dua-cli/releases). npkill: [README](https://github.com/voidcosmos/npkill).
iced: [CHANGELOG 0.14](https://github.com/iced-rs/iced/blob/master/CHANGELOG.md), [system theme PR #3051](https://github.com/iced-rs/iced/pull/3051), [table PR #3018](https://github.com/iced-rs/iced/pull/3018). I also read the published sources of iced 0.14 / iced_widget / iced_runtime / iced_graphics, cosmic-text 0.15 (font fallback) and trash 5.2.9 (macOS `DeleteMethod`).
Palette method: the dataviz skill's reference palette and `validate_palette.js`. Results are reproducible with `/tmp/dirstat-work/research/ux-spec-assets/scripts/*.mjs` (run as `node <script> <path-to-validate_palette.js>`).
Sibling research this spec was aligned with (read-only): `/tmp/dirstat-work/research/scan-engine.md` (batching ≤ 20/s, files ≥ 1 MiB kept individually, mount and firmlink policy, errno-preserving errors, 200 error samples) and `/tmp/dirstat-work/research/iced-api.md` (macOS font-weight fallback, theme-following gotcha, virtualization measurements, rfd/opener/trash usage).
