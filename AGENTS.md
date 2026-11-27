# AGENTS.md

Orientation notes for agents working on dir-stat. User-facing docs are in `README.md`; the
full v1 blueprint is `docs/design/design.md`.

## What it is

A desktop disk-usage analyzer: scans a folder or volume, shows a live tree sorted by size,
a treemap, a space-by-category bar, and cleanup suggestions (caches, build output,
`node_modules`, …) that can be moved to the Trash. Rust 2024, [iced](https://iced.rs) 0.14.
macOS first, Linux works, Windows best effort.

## Layout

```
crates/core/   dirstat-core: no iced dependency, everything testable without a GUI
  scan/          parallel scanner; emits Update::{Started, Batch, Finished, Failed}
  tree/          arena tree the UI reads (Node, Totals, verdicts, trash accounting, splice for rescans)
                 tree/testing.rs: TreeBuilder + dir()/large() fixtures (feature `test-util`)
  catalog/       cleanup rule table (table.rs, ~2k lines of static rules), model.rs (Rule,
                 Category, Risk, Kind::{Target,Container,Protect}), engine.rs (streaming matcher,
                 Resolver for $HOME/XDG/Windows bases), display.rs (risk → UI section), context.rs
  breakdown.rs   space-by-kind classification for the bar under the path bar (see below)
  safety.rs      trash guards (G1–G9 in design §7.2) — never weaken without asking
  trash/         macOS/Windows via `trash` crate; Linux has its own rename-only freedesktop trash
  platform/      per-OS bits (mounts, FDA, reveal, read-only checks)
  volumes.rs, git.rs, smallfiles.rs
  examples/scan.rs  CLI scan for comparing against `du` (`cargo run -p dirstat-core --release --example scan -- <dir>`)
  tests/         integration tests on fixture filesystems
crates/app/    dir-stat binary (iced)
  app.rs         App, Env (engine, places, prefs store, trasher… — injected, so tests use fakes),
                 Message, update/view, results_view layout, tree_height()
  results/       the Results screen: mod.rs (Results state, on_scan, tick), tree_view/rows/columns/order
                 (virtualized tree + calm sorting), cleanup*/candidates (Cleanup pane), treemap*
                 (Map tab), breakdown_view.rs (space bar), trash.rs (dialog/flow), nav.rs (path bar),
                 activity.rs (progress field), selection_bar.rs, errors.rs, migrate.rs (rescan swap)
  start.rs       start screen (volume cards, recents)
  theme.rs       design tokens (LIGHT/DARK), text roles, all widget styles, map_fill palette
  script.rs      screenshot/script harness (feature `screenshot`)
  tests.rs       app-level tests without rendering (feed Message::Scan by hand)
docs/research/ background research + probe crates (not workspace members)
docs/design/   design.md (decisions D1–D16, milestones, open questions), review-log.md
scripts/       bundle-macos.sh, make-icon.py
.air/cloud/startup.sh   environment setup for fresh cloud sessions (Rust + Xvfb + libs)
```

Doc comments cite the spec as `design §x.y`, `UX §x` (docs/research/ux-spec.md) and decisions
as `D<n>`. Keep doing that; when behavior deviates from the docs, update the docs or tell the user.

## Key concepts

- **Verdict** (per tree node, set by the catalog during the scan): `Target{rule}` (cleanup
  candidate), `Container(rule)` (cache root whose children are candidates), `Inside(rule)`
  (below a target), `Protected(rule)` (never suggested), `None`.
- **Sizes** are allocated bytes, hard links once (`Totals.alloc`). A node's own bytes =
  `total.alloc − Σ children total.alloc`. Trashed nodes keep their totals but are already
  subtracted from ancestors — skip `NodeFlags::TRASHED` children when summing.
- Only files ≥ 1 MiB (`scan::LARGE_FILE`) are kept individually (`Node::large_files`); smaller
  ones are aggregated per directory.
- `Engine::rule_by_id` indexes every rule regardless of OS (handy in tests); matching only
  uses rules for the current OS.
- **Throttled recompute pattern**: views derived from the whole tree (`MapState`,
  `BreakdownState`) get `touch()` on data changes and `tick(...)` from `Results::tick`, which
  recomputes at once on zoom change and at most every ~1 s while scanning.
- **Space bar** (`core/breakdown.rs` + `app/results/breakdown_view.rs`): kinds Documents,
  Applications, Games, Developer, Caches, Logs, Trash, System, Other (+ Unaccounted at a volume
  root). Precedence: catalog match category > well-known place (`Places`) > folder name
  (`steamapps`, `Epic Games`, …) > Protect rule id > parent. Colors are the 8 validated
  categorical slots in fixed order — don't add a 9th colored kind; fold into an existing one.
- **Fixed layout heights**: `tree_height()` in `app.rs` subtracts every strip above the tree
  (toolbar 44, path bar 32, `breakdown_view::HEIGHT`, rules, column header). Adding or resizing
  a strip means updating it, or tree virtualization/scrolling goes off.
- iced 0.14 text has no ellipsis: pre-truncate with `widgets::truncate_*` / estimate with
  `widgets::estimated_width`.
- Styles always read `theme::tokens(theme)`; text roles are `theme::BODY`, `SMALL`, `CAPTION`, ….

## Build, test, run

```sh
source ~/.cargo/env                       # if cargo isn't on PATH
cargo build --release -p dir-stat         # ~4 min cold
cargo test --workspace --no-fail-fast     # --no-fail-fast: cargo otherwise stops at the first failing binary
cargo clippy --workspace --all-targets --features dir-stat/screenshot
cargo fmt --all
```

Known failure in this Linux container (fails on clean master too, not caused by app changes):
`dirstat_core trash::tests::contents_mode_moves_only_what_is_unchanged`.

Tests never scan the real home folder; keep it that way (use `TreeBuilder` or `tempfile`).

## Running the GUI headless

A fresh session needs `.air/cloud/startup.sh` (installs Rust, Xvfb, X11/Mesa libs, starts
Xvfb on `:99`). Then:

```sh
DISPLAY=:99 ./target/release/dir-stat <dir> &            # interactive; drive with xdotool
DISPLAY=:99 import -window root out/shot.png             # screenshot (ImageMagick)
```

Better for reproducible checks — the built-in script harness (build with `--features screenshot`):

```sh
cargo build --release -p dir-stat --features screenshot
DISPLAY=:99 ICED_THEME=Light DIRSTAT_SCRIPT="path:/some/dir;idle:20000;wait:800;shot:$PWD/out/a.png;select:sub;key:Enter;wait:1500;shot:$PWD/out/b.png" \
  ./target/release/dir-stat
```

Steps: `path:`, `wait:<ms>`, `idle:<max ms>`, `shot:<png>`, `key:<KeyAction variant>`,
`expand:<rel>`, `select:<rel>`, `click:<name>`, `report:<label>` (see `script.rs`). Check
light **and** dark (`ICED_THEME=Dark`). Shot paths should be absolute.

Tips:
- Set `HOME=/some/fake/home` to test home-relative rules (catalog, `Places`) on synthetic data
  (`head -c N /dev/urandom > file` makes real allocated bytes). Don't put a fake home under a
  path that a rule anchors.
- The window is 1200×800 inside a 1400×900 Xvfb screen; crop with `convert in.png -crop WxH+X+Y out.png`.
- Ignore `libEGL … DRI3` and `XDG_RUNTIME_DIR` warnings (software rendering, harmless).
- Stop the app with `pkill -x dir-stat` — **not** `pkill -f target/release/dir-stat`, which
  matches and kills your own shell command.
- Always look at the screenshot; a blank frame is a failed launch.

## Screenshots go in `out/`

Put screenshots and other visual output in `out/` in the project dir (git-ignored), **not** in
the session artifacts directory (`$AIR_ARTIFACTS_DIR`) — artifacts aren't reachable from the
user's frontend. Refer to them by path (e.g. `out/breakdown-light.png`) in replies.

## Working conventions

- Design decisions the user approved are recorded in `docs/design/design.md` (e.g.
  "Catalog fix (user-approved, 2026-11-13)"). Ask before changing safety guards, trash
  behavior or catalog risk levels.
- Catalog rules need a `reason` (≤ 70 chars, sentence case, ends with a period) and evidence
  bits where verified (`model.rs` `ev::*`).
- Prefer adding tests alongside code: core logic in `crates/core` with `TreeBuilder`, view
  logic as unit tests in the view module, app flows in `crates/app/src/tests.rs`.
- For charts/colors, reuse the palette in `theme.rs` / `breakdown_view::fill` (already
  validated for CVD in light and dark).
