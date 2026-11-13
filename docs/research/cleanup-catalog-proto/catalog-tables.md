
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
