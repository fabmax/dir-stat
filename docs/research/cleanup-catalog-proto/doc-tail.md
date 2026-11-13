
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
