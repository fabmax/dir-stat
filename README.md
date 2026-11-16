# dir-stat

A desktop app that shows what uses your disk and which large folders are safe to delete.
It scans a folder or a whole volume, shows a live tree sorted by size, and suggests
cleanups (build outputs, caches, `node_modules`, …) that you can move to the Trash.

Rust, [iced](https://iced.rs) 0.14. macOS first; Linux works; Windows is best effort.

## Running

```sh
cargo run --release -p dir-stat            # start screen
cargo run --release -p dir-stat -- ~/dev   # scan a folder right away
```

Run from a terminal, macOS attributes file access to the **terminal app**: its privacy
grants apply, and its name appears in the "Full Disk Access" hint. For the app's own
identity, build the bundle below.

Tests: `cargo test --workspace`. They never scan your home folder.

## Bundling for macOS

```sh
scripts/bundle-macos.sh
open target/release/dir-stat.app
```

The script builds `target/release/dir-stat.app` with:

- `Info.plist`: bundle id `com.example.dir-stat`, and the purpose strings macOS shows when
  the app first reads Desktop, Documents, Downloads, removable and network volumes, File
  Provider (iCloud) folders and other apps' data. Without them, access fails silently;
- the icon (`crates/app/assets/icon/AppIcon.icns`, a placeholder drawn by
  `scripts/make-icon.py`);
- a code signature.

**Signing.** Privacy grants are tied to the signature. By default the script signs
*ad-hoc*, which changes with every build: macOS then forgets Full Disk Access and folder
access after each rebuild. To keep them, sign with a stable identity:

```sh
DIRSTAT_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" scripts/bundle-macos.sh
```

A self-signed code-signing certificate from Keychain Access works too.

**Full Disk Access.** For complete results (Mail, Safari, `~/.Trash`, other apps' data),
add `dir-stat.app` under System Settings ▸ Privacy & Security ▸ Full Disk Access. The app
links there from its start screen and from the bar that appears when a folder is blocked.

## Development

- `crates/core` (`dirstat-core`): scanner, tree, cleanup catalog, git checks, trash, safety
  guards. No GUI.
- `crates/app` (`dir-stat`): the iced app.
- `docs/design/design.md`: the design and the milestone log; `docs/research/`: the research
  it is based on.
- Screenshots for visual checks: build with `--features screenshot` and run with
  `DIRSTAT_SCRIPT="path:/some/folder;wait:2000;shot:/tmp/a.png"` (see
  `crates/app/src/script.rs`).
