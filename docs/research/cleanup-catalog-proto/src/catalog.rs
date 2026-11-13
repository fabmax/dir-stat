//! The cleanup-suggestion catalog: one static table, grouped for documentation.
//!
//! Order matters only between *heuristic* rules (Dir/Under/Contains): when several match the
//! same folder, the first one in this table wins, so specific rules come before generic ones
//! (the `GENERIC` group is last). Anchored rules (At/FileAt) always beat heuristic ones.
//!
//! Evidence (`ev`): DOCS = vendor docs, SOURCE = tool source code, LOCAL = observed on the
//! reference Mac, SECONDARY = forums/guides only, 0 = from knowledge (double-check).

use crate::model::Base::*;
use crate::model::Category::*;
use crate::model::Matcher::*;
use crate::model::Pat::*;
use crate::model::Risk::*;
use crate::model::{Cond, Os, Removal, Rule, Scope, When, ev, m};

pub struct Group {
    pub title: &'static str,
    pub rules: &'static [Rule],
}

pub static CATALOG: &[Group] = &[
    Group { title: "macOS: system and user folders", rules: MACOS },
    Group { title: "Apple development (Xcode, simulators, Swift, CocoaPods, Carthage)", rules: APPLE_DEV },
    Group { title: "JavaScript / TypeScript", rules: JS },
    Group { title: "Rust", rules: RUST },
    Group { title: "Python and ML", rules: PYTHON },
    Group { title: "JVM (Gradle, Maven, Kotlin, Scala) and JetBrains IDEs", rules: JVM },
    Group { title: "Go", rules: GO },
    Group { title: "Mobile (Android, Flutter/Dart)", rules: MOBILE },
    Group { title: "Native and other ecosystems", rules: OTHER },
    Group { title: "Containers and virtual machines", rules: VMS },
    Group { title: "Linux system", rules: LINUX },
    Group { title: "Windows system", rules: WINDOWS },
    Group { title: "Generic heuristics (evaluated after all specific rules)", rules: GENERIC },
    Group { title: "Protected locations (never suggested)", rules: PROTECTED },
];

// ---------------------------------------------------------------------------------------------
static MACOS: &[Rule] = &[
    Rule::container("mac-caches", "App caches (~/Library/Caches)", Cache, Os::MAC, Some("mac-app-cache"), &[At(MacCaches, &[])])
        .why("Per-app caches; apps must be able to rebuild them.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("mac-app-cache", "App cache", Cache, Low, Os::MAC, &[])
        .why("Cache of one app; it is rebuilt, possibly by re-downloading.")
        .caution("Quit the app first.")
        .ev(ev::DOCS),
    Rule::target("mac-sandbox-cache", "Sandboxed app cache", Cache, Low, Os::MAC, &[At(Home, &["Library", "Containers", "*", "Data", "Library", "Caches"])])
        .why("Cache of a sandboxed (App Store) app; rebuilt on demand.")
        .caution("Quit the app first. macOS 14+ asks before reading other apps' containers.")
        .removal(Removal::Contents),
    Rule::target("mac-logs", "Logs (~/Library/Logs)", Logs, Safe, Os::MAC, &[At(Home, &["Library", "Logs"])])
        .why("Log files and crash reports; only useful for troubleshooting.")
        .removal(Removal::Contents)
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("mac-system-logs", "System logs (/Library/Logs)", Logs, Safe, Os::MAC, &[At(Root, &["Library", "Logs"])])
        .why("System-wide logs and crash reports; deleting needs admin rights.")
        .removal(Removal::Contents)
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("mac-trash", "Trash", Trash, Review, Os::MAC, &[At(Home, &[".Trash"]), At(Root, &["Volumes", "*", ".Trashes"])])
        .why("Files you moved to the Trash; emptying deletes them for good.")
        .caution("Measuring ~/.Trash may require Full Disk Access.")
        .cleanup(&["Finder ▸ Empty Trash"])
        .removal(Removal::ToolOnly)
        .ev(ev::LOCAL),
    Rule::target(
        "mac-mail-downloads",
        "Mail attachment copies",
        Cache,
        Review,
        Os::MAC,
        &[At(Home, &["Library", "Containers", "com.apple.mail", "Data", "Library", "Mail Downloads"]), At(Home, &["Library", "Mail Downloads"])],
    )
    .why("Copies of attachments opened in Mail; edited ones may hold changes.")
    .cleanup(&["Mail ▸ Settings ▸ General ▸ Remove unedited downloads"])
    .removal(Removal::Contents)
    .ev(ev::DOCS | ev::SECONDARY),
    Rule::target(
        "ios-backups",
        "iPhone/iPad backups",
        Backups,
        Review,
        Os::MAC_WIN,
        &[
            At(Home, &["Library", "Application Support", "MobileSync", "Backup"]),
            At(Home, &["Apple", "MobileSync", "Backup"]),
            At(AppData, &["Apple Computer", "MobileSync", "Backup"]),
        ],
    )
    .why("Local device backups; a deleted backup cannot be recovered.")
    .caution("Needs Full Disk Access to measure on macOS.")
    .cleanup(&["Finder ▸ (device) ▸ General ▸ Manage Backups…"])
    .removal(Removal::ToolOnly)
    .ev(ev::DOCS),
    Rule::target(
        "ios-firmware",
        "iOS firmware downloads (.ipsw)",
        Installers,
        Low,
        Os::MAC_WIN,
        &[
            At(Home, &["Library", "iTunes", "iPhone Software Updates"]),
            At(Home, &["Library", "iTunes", "iPad Software Updates"]),
            At(Home, &["Library", "iTunes", "iPod Software Updates"]),
            At(AppData, &["Apple Computer", "iTunes", "iPhone Software Updates"]),
            At(AppData, &["Apple Computer", "iTunes", "iPad Software Updates"]),
        ],
    )
    .why("Device firmware for updates/restores; downloaded again if needed.")
    .removal(Removal::Contents)
    .ev(ev::SECONDARY),
    Rule::target("mac-os-installer", "macOS installer app", Installers, Low, Os::MAC, &[At(Root, &["Applications", "Install macOS *.app"])])
        .why("macOS installer app (12+ GB); it can be downloaded again.")
        .cleanup(&["softwareupdate --fetch-full-installer"]),
    Rule::target(
        "downloads-installers",
        "Installers in Downloads",
        Installers,
        Review,
        Os::ALL,
        &[FileAt(Home, &["Downloads"], Ext(&["dmg", "pkg", "mpkg", "iso", "xip", "msi", "msix", "exe", "deb", "rpm", "appimage"]))],
    )
    .why("Installer or disk image you downloaded; rarely needed after install."),
    Rule::target("homebrew-cache", "Homebrew downloads", PackageCache, Safe, Os::UNIX, &[At(Env("HOMEBREW_CACHE"), &[]), At(MacCaches, &["Homebrew"]), At(XdgCache, &["Homebrew"])])
        .why("Downloaded bottles and casks; installed software doesn't need them.")
        .cleanup(&["brew cleanup --prune=all"])
        .removal(Removal::Contents)
        .ev(ev::DOCS | ev::LOCAL),
];

// ---------------------------------------------------------------------------------------------
static APPLE_DEV: &[Rule] = &[
    Rule::target("xcode-derived-data", "Xcode DerivedData", BuildArtifacts, Safe, Os::MAC, &[At(Home, &["Library", "Developer", "Xcode", "DerivedData"])])
        .why("Xcode build products and indexes; rebuilt on next build.")
        .caution("Quit Xcode first.")
        .removal(Removal::Contents)
        .ev(ev::SECONDARY),
    Rule::target("xcode-archives", "Xcode Archives", Backups, Review, Os::MAC, &[At(Home, &["Library", "Developer", "Xcode", "Archives"])])
        .why("Archived app builds with dSYMs, needed to symbolicate crashes.")
        .caution("Apple: \"You must retain the Xcode archive for each build of your app you distribute.\"")
        .cleanup(&["Xcode ▸ Window ▸ Organizer ▸ Archives"])
        .ev(ev::DOCS),
    Rule::target(
        "xcode-device-support",
        "Xcode device support",
        DeveloperTools,
        Low,
        Os::MAC,
        &[
            At(Home, &["Library", "Developer", "Xcode", "iOS DeviceSupport"]),
            At(Home, &["Library", "Developer", "Xcode", "watchOS DeviceSupport"]),
            At(Home, &["Library", "Developer", "Xcode", "tvOS DeviceSupport"]),
            At(Home, &["Library", "Developer", "Xcode", "visionOS DeviceSupport"]),
        ],
    )
    .why("Debug symbols per device OS version; recopied when a device connects.")
    .caution("Quit Xcode first. Re-copying needs the device and takes minutes.")
    .removal(Removal::Contents)
    .ev(ev::SECONDARY),
    Rule::target("xcode-previews", "SwiftUI preview simulators", Cache, Safe, Os::MAC, &[At(Home, &["Library", "Developer", "Xcode", "UserData", "Previews"])])
        .why("Simulators and build data used for SwiftUI previews.")
        .cleanup(&["xcrun simctl --set previews delete all"])
        .removal(Removal::ToolOnly)
        .ev(ev::SECONDARY),
    Rule::target("xcode-cache", "Xcode cache", Cache, Safe, Os::MAC, &[At(MacCaches, &["com.apple.dt.Xcode"])])
        .why("Xcode's own cache; rebuilt automatically.")
        .caution("Quit Xcode first."),
    Rule::target("coresim-devices", "Simulator devices", DeveloperTools, Review, Os::MAC, &[At(Home, &["Library", "Developer", "CoreSimulator", "Devices"])])
        .why("Simulator devices with the apps and data installed in them.")
        .caution("`delete unavailable` only removes simulators whose runtime is gone.")
        .cleanup(&["xcrun simctl delete unavailable", "xcrun simctl erase all"])
        .removal(Removal::ToolOnly)
        .ev(ev::SECONDARY),
    Rule::target("xctest-devices", "Test simulator clones", DeveloperTools, Safe, Os::MAC, &[At(Home, &["Library", "Developer", "XCTestDevices"])])
        .why("Simulator clones for parallel testing; recreated by test runs.")
        .caution("Quit Xcode and stop running tests first. APFS clones: deleting may free far less than shown.")
        .cleanup(&["xcrun simctl --set ~/Library/Developer/XCTestDevices delete all"])
        .shares_space()
        .removal(Removal::ToolOnly)
        .ev(ev::SECONDARY),
    Rule::target("coresim-caches", "Simulator caches", Cache, Safe, Os::MAC, &[At(Home, &["Library", "Developer", "CoreSimulator", "Caches"])])
        .why("Simulator dyld caches; regenerated when a simulator boots.")
        .removal(Removal::Contents)
        .ev(ev::SECONDARY),
    Rule::target(
        "coresim-runtimes",
        "Simulator runtimes",
        DeveloperTools,
        Low,
        Os::MAC,
        &[At(Root, &["Library", "Developer", "CoreSimulator", "Images"]), At(Root, &["Library", "Developer", "CoreSimulator", "Profiles", "Runtimes"])],
    )
    .why("Simulator runtimes (several GB each); re-downloadable in Xcode.")
    .caution("Remove only via Xcode or simctl. Mounted runtime volumes (…/CoreSimulator/Volumes) are separate file systems.")
    .cleanup(&["Xcode ▸ Settings ▸ Components", "xcrun simctl runtime list", "xcrun simctl runtime delete <id>"])
    .removal(Removal::ToolOnly)
    .ev(ev::DOCS | ev::SECONDARY),
    Rule::target("xcode-extra", "Additional Xcode versions", DeveloperTools, Review, Os::MAC, &[At(Root, &["Applications", "Xcode-*.app"]), At(Root, &["Applications", "Xcode_*.app"])])
        .why("Another Xcode installation (10+ GB); keep it if a project needs it."),
    Rule::target("swiftpm-build", "SwiftPM .build", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".build"), &[Cond::sib(m::PACKAGE_SWIFT)])])
        .why("Swift package build output; rebuilt by swift build.")
        .cleanup(&["swift package clean", "swift package reset"])
        .projects()
        .ev(ev::LOCAL),
    Rule::target("swiftpm-cache", "SwiftPM cache", PackageCache, Safe, Os::UNIX, &[At(MacCaches, &["org.swift.swiftpm"]), At(XdgCache, &["org.swift.swiftpm"])])
        .why("Cached Swift package repositories; fetched again when needed.")
        .cleanup(&["swift package purge-cache"])
        .ev(ev::LOCAL),
    Rule::target("cocoapods-cache", "CocoaPods cache", PackageCache, Safe, Os::MAC, &[At(MacCaches, &["CocoaPods"])])
        .why("Downloaded pods; CocoaPods fetches them again when needed.")
        .cleanup(&["pod cache clean --all"])
        .ev(ev::DOCS),
    Rule::target("cocoapods-pods", "CocoaPods Pods", Dependencies, Low, Os::MAC, &[Dir(Is("Pods"), &[Cond::sib(m::PODFILE)])])
        .why("Pods installed into the project; pod install restores them.")
        .caution("Some teams commit Pods/ to git.")
        .cleanup(&["pod install"])
        .projects(),
    Rule::target("cocoapods-repos", "CocoaPods spec repos", PackageCache, Low, Os::MAC, &[At(Home, &[".cocoapods", "repos"])])
        .why("Local clones of CocoaPods spec repos (the CDN is the default).")
        .cleanup(&["pod repo list", "pod repo remove <name>"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("carthage", "Carthage checkouts and builds", Dependencies, Low, Os::MAC, &[Dir(Is("Carthage"), &[Cond::sib(m::CARTFILE)])])
        .why("Carthage dependencies; carthage bootstrap restores them.")
        .caution("With --use-submodules, Carthage/Checkouts are git submodules.")
        .cleanup(&["carthage bootstrap"])
        .projects(),
    Rule::target("carthage-cache", "Carthage cache", PackageCache, Safe, Os::MAC, &[At(MacCaches, &["org.carthage.CarthageKit"])])
        .why("Carthage download cache; fetched again when needed."),
];

// ---------------------------------------------------------------------------------------------
static JS: &[Rule] = &[
    Rule::target("node-modules-pnpm", "node_modules (pnpm)", Dependencies, Low, Os::ALL, &[Dir(Is("node_modules"), &[Cond::sib(m::PACKAGE_JSON).and_has_any(m::PNPM_MODULES)])])
        .why("Packages linked by pnpm; pnpm install restores them.")
        .caution("Files are hard links/clones of the pnpm store: little space is freed until `pnpm store prune`.")
        .cleanup(&["pnpm install", "pnpm store prune"])
        .shares_space()
        .projects()
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("node-modules", "node_modules", Dependencies, Low, Os::ALL, &[Dir(Is("node_modules"), &[Cond::sib(m::PACKAGE_JSON)])])
        .why("Installed npm/Yarn/Bun packages; reinstalled by npm ci or yarn.")
        .cleanup(&["npm ci", "yarn install", "bun install"])
        .projects()
        .ev(ev::LOCAL),
    Rule::target("npm-cache", "npm cache", PackageCache, Safe, Os::ALL, &[At(Env("npm_config_cache"), &[]), At(Home, &[".npm"]), At(LocalAppData, &["npm-cache"])])
        .why("npm download cache and npx packages; refetched on demand.")
        .cleanup(&["npm cache clean --force", "npm cache npx rm"])
        .removal(Removal::Contents)
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target(
        "yarn-cache",
        "Yarn 1 cache",
        PackageCache,
        Safe,
        Os::ALL,
        &[At(Env("YARN_CACHE_FOLDER"), &[]), At(MacCaches, &["Yarn"]), At(XdgCache, &["yarn"]), At(LocalAppData, &["Yarn", "Cache"])],
    )
    .why("Yarn classic's global package cache; refetched on demand.")
    .cleanup(&["yarn cache clean"])
    .ev(ev::SECONDARY | ev::LOCAL),
    Rule::target("yarn-berry-cache", "Yarn 2+ global cache", PackageCache, Safe, Os::ALL, &[At(Home, &[".yarn", "berry", "cache"])])
        .why("Yarn 2+ shared package archives; refetched on demand.")
        .cleanup(&["yarn cache clean --mirror"])
        .ev(ev::DOCS),
    Rule::target("yarn-project-cache", "Project .yarn/cache", Dependencies, Review, Os::ALL, &[Under(".yarn", Is("cache"), &[])])
        .why("Project-local Yarn archives; Zero-Install repos commit them to git.")
        .projects()
        .ev(ev::DOCS),
    Rule::target(
        "pnpm-store",
        "pnpm store",
        PackageCache,
        Low,
        Os::ALL,
        &[
            At(Env("PNPM_HOME"), &["store"]),
            At(XdgData, &["pnpm", "store"]),
            At(Home, &["Library", "pnpm", "store"]),
            At(LocalAppData, &["pnpm", "store"]),
            Dir(Is(".pnpm-store"), &[]),
        ],
    )
    .why("pnpm's package store shared by all projects; refetched on demand.")
    .caution("Projects using the global virtual store need `pnpm install` afterwards.")
    .cleanup(&["pnpm store prune"])
    .shares_space()
    .ev(ev::DOCS | ev::LOCAL),
    Rule::target("pnpm-cache", "pnpm metadata cache", PackageCache, Safe, Os::ALL, &[At(MacCaches, &["pnpm"]), At(XdgCache, &["pnpm"]), At(LocalAppData, &["pnpm-cache"])])
        .why("pnpm package metadata and dlx cache; refetched on demand.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("bun-cache", "Bun cache", PackageCache, Low, Os::ALL, &[At(Env("BUN_INSTALL_CACHE_DIR"), &[]), At(Home, &[".bun", "install", "cache"])])
        .why("Bun's global package cache; refetched on demand.")
        .caution("Projects using Bun's global store need `bun install` afterwards.")
        .cleanup(&["bun pm cache rm"])
        .shares_space()
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("next-build", "Next.js .next", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".next"), &[Cond::sib(m::NEXT_CONFIG | m::PACKAGE_JSON)])])
        .why("Next.js build output and cache; recreated by next dev/build.")
        .caution("Not on a server that is running this app.")
        .projects()
        .ev(ev::DOCS),
    Rule::target("nuxt-build", "Nuxt .nuxt / .output", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".nuxt"), &[Cond::sib(m::NUXT_CONFIG)]), Dir(Is(".output"), &[Cond::sib(m::NUXT_CONFIG)])])
        .why("Nuxt generated files and build output; recreated on build.")
        .cleanup(&["npx nuxt cleanup"])
        .projects()
        .ev(ev::DOCS),
    Rule::target("sveltekit", "SvelteKit .svelte-kit", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".svelte-kit"), &[Cond::sib(m::SVELTE_CONFIG | m::PACKAGE_JSON)])])
        .why("SvelteKit generated files; recreated by vite dev / build.")
        .projects(),
    Rule::target("angular-cache", "Angular .angular", Cache, Safe, Os::ALL, &[Dir(Is(".angular"), &[Cond::sib(m::ANGULAR_JSON)])])
        .why("Angular CLI build cache (.angular/cache).")
        .cleanup(&["ng cache clean"])
        .projects()
        .ev(ev::DOCS),
    Rule::target("turbo-cache", "Turborepo .turbo", Cache, Safe, Os::ALL, &[Dir(Is(".turbo"), &[Cond::sib(m::TURBO_JSON | m::PACKAGE_JSON)])])
        .why("Turborepo local task cache and run logs.")
        .projects()
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("parcel-cache", "Parcel cache", Cache, Safe, Os::ALL, &[Dir(Is(".parcel-cache"), &[Cond::sib(m::PACKAGE_JSON)])])
        .why("Parcel bundler cache; rebuilt on next build.")
        .projects(),
    Rule::target("electron-cache", "Electron downloads", PackageCache, Safe, Os::ALL, &[At(MacCaches, &["electron"]), At(XdgCache, &["electron"]), At(LocalAppData, &["electron", "Cache"])])
        .why("Electron binaries downloaded by @electron/get.")
        .ev(ev::DOCS),
    Rule::target(
        "node-gyp-cache",
        "node-gyp headers",
        DeveloperTools,
        Safe,
        Os::ALL,
        &[At(MacCaches, &["node-gyp"]), At(XdgCache, &["node-gyp"]), At(LocalAppData, &["node-gyp", "Cache"]), At(Home, &[".node-gyp"])],
    )
    .why("Node.js headers for building native add-ons; downloaded on demand.")
    .ev(ev::LOCAL),
    Rule::target(
        "playwright-browsers",
        "Playwright browsers",
        DeveloperTools,
        Low,
        Os::ALL,
        &[At(Env("PLAYWRIGHT_BROWSERS_PATH"), &[]), At(MacCaches, &["ms-playwright"]), At(XdgCache, &["ms-playwright"]), At(LocalAppData, &["ms-playwright"])],
    )
    .why("Browser builds downloaded by Playwright for tests.")
    .cleanup(&["npx playwright uninstall --all"])
    .ev(ev::DOCS),
    Rule::target(
        "cypress-cache",
        "Cypress binaries",
        DeveloperTools,
        Low,
        Os::ALL,
        &[At(Env("CYPRESS_CACHE_FOLDER"), &[]), At(MacCaches, &["Cypress"]), At(XdgCache, &["Cypress"]), At(LocalAppData, &["Cypress", "Cache"])],
    )
    .why("Cypress app binaries, one per version.")
    .cleanup(&["npx cypress cache prune", "npx cypress cache clear"])
    .ev(ev::DOCS),
    Rule::target("puppeteer-cache", "Puppeteer browsers", DeveloperTools, Low, Os::ALL, &[At(Home, &[".cache", "puppeteer"])])
        .why("Browsers downloaded by Puppeteer for automation/tests.")
        .cleanup(&["npx puppeteer browsers install"])
        .ev(ev::DOCS),
    Rule::target("deno-cache", "Deno cache", PackageCache, Safe, Os::ALL, &[At(Env("DENO_DIR"), &[]), At(MacCaches, &["deno"]), At(XdgCache, &["deno"]), At(LocalAppData, &["deno"])])
        .why("Deno's module cache; refetched on demand.")
        .cleanup(&["deno clean"])
        .ev(ev::DOCS),
];

// ---------------------------------------------------------------------------------------------
const CARGO_REGISTRY: &[crate::model::Matcher] = &[At(Env("CARGO_HOME"), &["registry"]), At(Home, &[".cargo", "registry"])];
const CARGO_GIT: &[crate::model::Matcher] = &[At(Env("CARGO_HOME"), &["git"]), At(Home, &[".cargo", "git"])];

static RUST: &[Rule] = &[
    Rule::target(
        "cargo-target",
        "Cargo target",
        BuildArtifacts,
        Safe,
        Os::ALL,
        &[Dir(Is("target"), &[Cond::sib(m::CARGO_TOML), Cond::has(m::CACHEDIR_TAG | m::RUSTC_INFO)])],
    )
    .why("Rust build output; cargo build recreates it (full rebuild).")
    .cleanup(&["cargo clean"])
    .projects()
    .ev(ev::SOURCE | ev::LOCAL),
    Rule::target("cargo-target-dir", "Cargo target (custom dir)", BuildArtifacts, Safe, Os::ALL, &[Contains(&[Cond::has(m::CACHEDIR_TAG | m::RUSTC_INFO)])])
        .why("Rust build output (custom target dir); recreated by cargo build.")
        .ev(ev::SOURCE | ev::LOCAL),
    Rule::container("cargo-registry", "Cargo registry", PackageCache, Os::ALL, None, CARGO_REGISTRY)
        .why("Crates from registries; Cargo 1.88+ prunes unused ones itself.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("cargo-registry-src", "Cargo registry sources", PackageCache, Safe, Os::ALL, &[At(Env("CARGO_HOME"), &["registry", "src"]), At(Home, &[".cargo", "registry", "src"])])
        .why("Unpacked crate sources; re-extracted from registry/cache offline.")
        .caution("`cargo cache` is the third-party cargo-cache tool (cargo install cargo-cache).")
        .cleanup(&["cargo cache --autoclean"])
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("cargo-registry-cache", "Cargo .crate downloads", PackageCache, Low, Os::ALL, &[At(Env("CARGO_HOME"), &["registry", "cache"]), At(Home, &[".cargo", "registry", "cache"])])
        .why("Downloaded .crate archives; re-downloaded when a build needs them.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("cargo-registry-index", "Cargo registry index", PackageCache, Low, Os::ALL, &[At(Env("CARGO_HOME"), &["registry", "index"]), At(Home, &[".cargo", "registry", "index"])])
        .why("Registry index metadata; fetched again when needed.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::container("cargo-git", "Cargo git dependencies", PackageCache, Os::ALL, None, CARGO_GIT)
        .why("Git dependencies of Rust projects.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("cargo-git-checkouts", "Cargo git checkouts", PackageCache, Safe, Os::ALL, &[At(Env("CARGO_HOME"), &["git", "checkouts"]), At(Home, &[".cargo", "git", "checkouts"])])
        .why("Working copies checked out from git/db; recreated offline.")
        .caution("`cargo cache` is the third-party cargo-cache tool (cargo install cargo-cache).")
        .cleanup(&["cargo cache --autoclean"])
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("cargo-git-db", "Cargo git clones", PackageCache, Low, Os::ALL, &[At(Env("CARGO_HOME"), &["git", "db"]), At(Home, &[".cargo", "git", "db"])])
        .why("Bare clones of git dependencies; cloned again when needed.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("rustup-toolchain", "Rust toolchain", DeveloperTools, Low, Os::ALL, &[At(Env("RUSTUP_HOME"), &["toolchains", "*"]), At(Home, &[".rustup", "toolchains", "*"])])
        .why("One installed Rust toolchain (~1-1.5 GB); reinstallable with rustup.")
        .caution("Keep your default toolchain and those pinned by rust-toolchain.toml.")
        .cleanup(&["rustup toolchain list", "rustup toolchain uninstall {name}"])
        .removal(Removal::ToolOnly)
        .ev(ev::LOCAL),
    Rule::target(
        "rustup-tmp",
        "rustup leftovers",
        Temporary,
        Safe,
        Os::ALL,
        &[At(Env("RUSTUP_HOME"), &["tmp"]), At(Env("RUSTUP_HOME"), &["downloads"]), At(Home, &[".rustup", "tmp"]), At(Home, &[".rustup", "downloads"])],
    )
    .why("Leftover rustup downloads and temp files.")
    .removal(Removal::Contents)
    .ev(ev::LOCAL),
];

// ---------------------------------------------------------------------------------------------
static PYTHON: &[Rule] = &[
    Rule::target("python-venv", "Python virtualenv", Dependencies, Low, Os::ALL, &[Contains(&[Cond::has(m::PYVENV_CFG)])])
        .why("Python virtual environment; recreate it from your requirements.")
        .caution("If its packages aren't recorded anywhere, run `pip freeze > requirements.txt` first.")
        .projects()
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("pycache", "__pycache__", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("__pycache__"), &[])])
        .why("Compiled Python bytecode; regenerated on import.")
        .projects(),
    Rule::target("mypy-cache", "mypy cache", Cache, Safe, Os::ALL, &[Dir(Is(".mypy_cache"), &[])])
        .why("mypy's incremental type-check cache.")
        .projects()
        .ev(ev::SOURCE),
    Rule::target("pytest-cache", "pytest cache", Cache, Safe, Os::ALL, &[Dir(Is(".pytest_cache"), &[])])
        .why("pytest cache (last-failed tests and similar).")
        .projects()
        .ev(ev::SOURCE),
    Rule::target("ruff-cache", "Ruff cache", Cache, Safe, Os::ALL, &[Dir(Is(".ruff_cache"), &[])])
        .why("Ruff linter cache.")
        .projects()
        .ev(ev::SOURCE),
    Rule::target("tox-envs", "tox environments", Dependencies, Safe, Os::ALL, &[Dir(Is(".tox"), &[Cond::sib(m::TOX_INI | m::PY_PROJECT)])])
        .why("Virtualenvs created by tox; recreated on the next tox run.")
        .projects()
        .ev(ev::SOURCE),
    Rule::target("nox-envs", "nox environments", Dependencies, Safe, Os::ALL, &[Dir(Is(".nox"), &[Cond::sib(m::NOXFILE)])])
        .why("Virtualenvs created by nox; recreated on the next nox run.")
        .projects(),
    Rule::target(
        "pip-cache",
        "pip cache",
        PackageCache,
        Safe,
        Os::ALL,
        &[At(Env("PIP_CACHE_DIR"), &[]), At(MacCaches, &["pip"]), At(XdgCache, &["pip"]), At(LocalAppData, &["pip", "Cache"])],
    )
    .why("pip's wheel and HTTP cache; refetched on demand.")
    .cleanup(&["pip cache purge"])
    .ev(ev::DOCS | ev::LOCAL),
    Rule::target("uv-cache", "uv cache", PackageCache, Safe, Os::ALL, &[At(Env("UV_CACHE_DIR"), &[]), At(XdgCache, &["uv"]), At(LocalAppData, &["uv", "cache"])])
        .why("uv's package cache; refetched on demand.")
        .caution("uv docs: never modify the cache directly; use `uv cache clean`.")
        .cleanup(&["uv cache clean", "uv cache prune"])
        .shares_space()
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("uv-python", "uv-managed Pythons", DeveloperTools, Low, Os::ALL, &[At(Env("UV_PYTHON_INSTALL_DIR"), &[]), At(XdgData, &["uv", "python"]), At(AppData, &["uv", "data", "python"])])
        .why("Python interpreters installed by uv; reinstallable.")
        .cleanup(&["uv python list --only-installed", "uv python uninstall <version>"])
        .removal(Removal::ToolOnly),
    Rule::target(
        "poetry-cache",
        "Poetry cache",
        PackageCache,
        Low,
        Os::ALL,
        &[At(Env("POETRY_CACHE_DIR"), &[]), At(MacCaches, &["pypoetry"]), At(XdgCache, &["pypoetry"]), At(LocalAppData, &["pypoetry", "Cache"])],
    )
    .why("Poetry's cache, incl. its virtualenvs; poetry install recreates them.")
    .cleanup(&["poetry cache clear pypi --all"])
    .ev(ev::DOCS),
    Rule::target("conda-pkgs", "conda package cache", PackageCache, Low, Os::ALL, &[Dir(Is("pkgs"), &[Cond::sibs(m::CONDA_META)]), At(Home, &[".conda", "pkgs"])])
        .why("Downloaded and extracted conda packages.")
        .caution("Deleting by hand can break environments that link into it; use conda clean.")
        .cleanup(&["conda clean --all"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target(
        "hf-hub",
        "Hugging Face models/datasets",
        Models,
        Low,
        Os::ALL,
        &[At(Env("HF_HUB_CACHE"), &[]), At(Env("HF_HOME"), &["hub"]), At(XdgCache, &["huggingface", "hub"]), At(Home, &[".cache", "huggingface", "hub"])],
    )
    .why("Downloaded Hugging Face models/datasets; re-downloaded on next use.")
    .cleanup(&["hf cache ls", "hf cache rm <repo>", "hf cache prune"])
    .ev(ev::DOCS),
    Rule::target("hf-xet", "Hugging Face upload cache", Cache, Safe, Os::ALL, &[At(Env("HF_XET_CACHE"), &[]), At(XdgCache, &["huggingface", "xet"]), At(Home, &[".cache", "huggingface", "xet"])])
        .why("Xet shard cache used to speed up uploads.")
        .ev(ev::DOCS),
    Rule::target(
        "ollama-models",
        "Ollama models",
        Models,
        Review,
        Os::ALL,
        &[At(Env("OLLAMA_MODELS"), &[]), At(Home, &[".ollama", "models"]), At(Root, &["usr", "share", "ollama", ".ollama", "models"])],
    )
    .why("Local LLM weights; pulled models can be downloaded again.")
    .caution("Models made with `ollama create` may not be reproducible.")
    .cleanup(&["ollama list", "ollama rm <model>"])
    .removal(Removal::ToolOnly)
    .ev(ev::SECONDARY),
];

// ---------------------------------------------------------------------------------------------
static JVM: &[Rule] = &[
    Rule::target("gradle-caches", "Gradle caches", PackageCache, Low, Os::ALL, &[At(Env("GRADLE_USER_HOME"), &["caches"]), At(Home, &[".gradle", "caches"])])
        .why("Gradle dependency and build caches; Gradle prunes unused entries.")
        .caution("Stop Gradle daemons first (`gradle --stop`).")
        .removal(Removal::Contents)
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("gradle-wrapper-dists", "Gradle distributions", DeveloperTools, Low, Os::ALL, &[At(Env("GRADLE_USER_HOME"), &["wrapper", "dists"]), At(Home, &[".gradle", "wrapper", "dists"])])
        .why("Gradle versions downloaded by gradlew; unused ones auto-deleted.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("gradle-jdks", "Gradle-provisioned JDKs", DeveloperTools, Low, Os::ALL, &[At(Env("GRADLE_USER_HOME"), &["jdks"]), At(Home, &[".gradle", "jdks"])])
        .why("JDKs auto-downloaded by Gradle toolchains; fetched again if needed.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("gradle-daemon", "Gradle daemon logs", Logs, Safe, Os::ALL, &[At(Env("GRADLE_USER_HOME"), &["daemon"]), At(Home, &[".gradle", "daemon"])])
        .why("Gradle daemon registry and logs.")
        .caution("Stop Gradle daemons first (`gradle --stop`).")
        .removal(Removal::Contents)
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("gradle-project-cache", "Gradle project .gradle", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".gradle"), &[Cond::sib(m::GRADLE_BUILD | m::GRADLE_SETTINGS | m::GRADLEW)])])
        .why("Per-project Gradle state (incremental build, config cache).")
        .projects()
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("gradle-build", "Gradle build", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("build"), &[Cond::sib(m::GRADLE_BUILD)])])
        .why("Gradle build output of this project or module.")
        .cleanup(&["./gradlew clean"])
        .projects()
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("kotlin-project-dir", "Kotlin .kotlin", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".kotlin"), &[Cond::sib(m::GRADLE_BUILD | m::GRADLE_SETTINGS)])])
        .why("Kotlin Gradle plugin's per-project data (sessions, errors).")
        .projects()
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("android-cxx", "Android .cxx", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".cxx"), &[Cond::sib(m::GRADLE_BUILD)])])
        .why("CMake/ndk-build intermediates of an Android module.")
        .projects(),
    Rule::target("maven-repo", "Maven local repository", Dependencies, Low, Os::ALL, &[At(Home, &[".m2", "repository"])])
        .why("Downloaded Maven artifacts plus ones you installed locally.")
        .caution("Artifacts you only built locally or installed by hand (mvn install:install-file) are lost.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("maven-wrapper", "Maven wrapper distributions", DeveloperTools, Low, Os::ALL, &[At(Home, &[".m2", "wrapper"])])
        .why("Maven versions downloaded by mvnw; fetched again if needed.")
        .ev(ev::LOCAL),
    Rule::target("maven-target", "Maven target", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("target"), &[Cond::sib(m::POM_XML)])])
        .why("Maven build output; mvn package recreates it.")
        .cleanup(&["mvn clean"])
        .projects(),
    Rule::target("sbt-target", "sbt target", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("target"), &[Cond::sib(m::BUILD_SBT)])])
        .why("sbt build output; recreated on the next build.")
        .cleanup(&["sbt clean"])
        .projects(),
    Rule::target(
        "coursier-cache",
        "Coursier cache",
        PackageCache,
        Low,
        Os::ALL,
        &[At(Env("COURSIER_CACHE"), &[]), At(MacCaches, &["Coursier"]), At(XdgCache, &["coursier"]), At(LocalAppData, &["Coursier", "Cache"])],
    )
    .why("Scala/sbt dependency cache; re-downloaded on demand.")
    .ev(ev::DOCS),
    Rule::target("konan", "Kotlin/Native toolchains", DeveloperTools, Low, Os::ALL, &[At(Env("KONAN_DATA_DIR"), &[]), At(Home, &[".konan"])])
        .why("Kotlin/Native compilers and toolchains; re-downloaded by builds.")
        .caution("Never cleaned automatically; the next Kotlin/Native build is slower.")
        .ev(ev::DOCS | ev::SECONDARY | ev::LOCAL),
    Rule::container("jetbrains-caches", "JetBrains IDE caches", Cache, Os::ALL, Some("jetbrains-system"), &[At(MacCaches, &["JetBrains"]), At(XdgCache, &["JetBrains"]), At(LocalAppData, &["JetBrains"])])
        .why("One system folder per JetBrains IDE version.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("jetbrains-system", "JetBrains IDE system folder", Cache, Low, Os::ALL, &[])
        .why("IDE caches, indexes and Local History of one IDE version.")
        .caution("Close the IDE first. Local History is lost; indexes are rebuilt.")
        .cleanup(&["IDE ▸ Help ▸ Delete Leftover IDE Directories…", "IDE ▸ File ▸ Invalidate Caches…"])
        .ev(ev::DOCS),
    Rule::target(
        "android-studio-system",
        "Android Studio system folder",
        Cache,
        Low,
        Os::ALL,
        &[At(MacCaches, &["Google", "AndroidStudio*"]), At(XdgCache, &["Google", "AndroidStudio*"]), At(LocalAppData, &["Google", "AndroidStudio*"])],
    )
    .why("Android Studio caches, indexes and Local History (one per version).")
    .caution("Close Android Studio first. Local History is lost.")
    .cleanup(&["Android Studio ▸ Help ▸ Delete Leftover IDE Directories…"]),
];

// ---------------------------------------------------------------------------------------------
static GO: &[Rule] = &[
    Rule::target("go-build-cache", "Go build cache", Cache, Safe, Os::ALL, &[At(Env("GOCACHE"), &[]), At(MacCaches, &["go-build"]), At(XdgCache, &["go-build"]), At(LocalAppData, &["go-build"])])
        .why("Go build and test cache; trimmed automatically, rebuilt as needed.")
        .cleanup(&["go clean -cache"])
        .ev(ev::DOCS),
    Rule::target("go-mod-cache", "Go module cache", Dependencies, Low, Os::ALL, &[At(Env("GOMODCACHE"), &[]), At(Env("GOPATH"), &["pkg", "mod"]), At(Home, &["go", "pkg", "mod"])])
        .why("Downloaded Go modules; re-downloaded on demand.")
        .caution("Files are read-only, so plain deletion fails; use `go clean -modcache`.")
        .cleanup(&["go clean -modcache"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
];

// ---------------------------------------------------------------------------------------------
static MOBILE: &[Rule] = &[
    Rule::target(
        "android-system-images",
        "Android emulator images",
        DeveloperTools,
        Low,
        Os::ALL,
        &[
            At(Env("ANDROID_HOME"), &["system-images"]),
            At(Env("ANDROID_SDK_ROOT"), &["system-images"]),
            At(Home, &["Library", "Android", "sdk", "system-images"]),
            At(Home, &["Android", "Sdk", "system-images"]),
            At(LocalAppData, &["Android", "Sdk", "system-images"]),
        ],
    )
    .why("Emulator system images (1-3 GB each); re-downloadable.")
    .caution("Emulators (AVDs) that use a removed image stop working.")
    .cleanup(&["sdkmanager --list_installed", "sdkmanager --uninstall \"system-images;android-<api>;<tag>;<abi>\""])
    .ev(ev::DOCS),
    Rule::target(
        "android-ndk",
        "Android NDKs",
        DeveloperTools,
        Low,
        Os::ALL,
        &[
            At(Env("ANDROID_HOME"), &["ndk"]),
            At(Env("ANDROID_SDK_ROOT"), &["ndk"]),
            At(Home, &["Library", "Android", "sdk", "ndk"]),
            At(Home, &["Android", "Sdk", "ndk"]),
            At(LocalAppData, &["Android", "Sdk", "ndk"]),
        ],
    )
    .why("Installed Android NDK versions (~1 GB each); re-downloadable.")
    .cleanup(&["sdkmanager --uninstall \"ndk;<version>\""]),
    Rule::target("android-avds", "Android emulators (AVDs)", VirtualMachines, Review, Os::ALL, &[At(Env("ANDROID_AVD_HOME"), &[]), At(Env("ANDROID_USER_HOME"), &["avd"]), At(Home, &[".android", "avd"])])
        .why("Emulator devices with their data and snapshots.")
        .cleanup(&["Android Studio ▸ Device Manager", "avdmanager delete avd -n <name>"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("flutter-build", "Flutter build", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("build"), &[Cond::sib(m::PUBSPEC)])])
        .why("Flutter/Dart build output; recreated on the next build.")
        .cleanup(&["flutter clean"])
        .projects(),
    Rule::target("dart-tool", "Dart .dart_tool", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".dart_tool"), &[Cond::sib(m::PUBSPEC)])])
        .why("Dart tool state; regenerated by dart pub get.")
        .cleanup(&["flutter clean"])
        .projects(),
    Rule::target("pub-cache", "Dart/Flutter pub cache", PackageCache, Low, Os::ALL, &[At(Env("PUB_CACHE"), &[]), At(Home, &[".pub-cache"]), At(LocalAppData, &["Pub", "Cache"])])
        .why("Packages shared by all Dart/Flutter projects; refetched on demand.")
        .caution("Also removes globally activated tools (dart pub global activate).")
        .cleanup(&["dart pub cache clean"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
];

// ---------------------------------------------------------------------------------------------
static OTHER: &[Rule] = &[
    Rule::target("cmake-build", "CMake build tree", BuildArtifacts, Low, Os::ALL, &[Contains(&[Cond::has(m::CMAKE_CACHE).lacks(m::CMAKE_LISTS | m::GIT)])])
        .why("Out-of-source CMake build tree; regenerate with cmake and rebuild.")
        .projects(),
    Rule::target(
        "ccache",
        "ccache",
        Cache,
        Safe,
        Os::ALL,
        &[At(Env("CCACHE_DIR"), &[]), At(Home, &[".ccache"]), At(MacCaches, &["ccache"]), At(XdgCache, &["ccache"]), At(LocalAppData, &["ccache"])],
    )
    .why("ccache compiler cache (default limit 5 GiB).")
    .caution("ccache.conf in this folder is configuration; `ccache -C` keeps it.")
    .cleanup(&["ccache -C", "ccache -c"])
    .removal(Removal::ToolOnly)
    .ev(ev::DOCS | ev::SOURCE),
    Rule::target(
        "sccache",
        "sccache",
        Cache,
        Safe,
        Os::ALL,
        &[At(Env("SCCACHE_DIR"), &[]), At(XdgCache, &["sccache"]), At(MacCaches, &["Mozilla.sccache"]), At(LocalAppData, &["Mozilla", "sccache"])],
    )
    .why("sccache compiler cache (default limit 10 GB).")
    .caution("Stop the server first (`sccache --stop-server`).")
    .removal(Removal::Contents)
    .ev(ev::DOCS),
    Rule::target(
        "bazel-output-root",
        "Bazel output root",
        BuildArtifacts,
        Low,
        Os::ALL,
        &[At(XdgCache, &["bazel"]), At(MacCaches, &["bazel"]), At(Root, &["private", "var", "tmp", "_bazel_*"]), At(Home, &["_bazel_*"])],
    )
    .why("Bazel build outputs and external repos of all workspaces.")
    .caution("Run `bazel shutdown` first; prefer `bazel clean --expunge` per workspace.")
    .cleanup(&["bazel shutdown", "bazel clean --expunge"])
    .ev(ev::DOCS | ev::LOCAL),
    Rule::target("bazel-output-base", "Bazel output base", BuildArtifacts, Low, Os::ALL, &[Contains(&[Cond::has(m::BAZEL_EXECROOT | m::BAZEL_EXTERNAL)])])
        .why("Build outputs and external repos of one Bazel workspace.")
        .caution("Run `bazel shutdown` in the workspace first.")
        .cleanup(&["bazel clean --expunge"])
        .ev(ev::DOCS),
    Rule::target("bazelisk-cache", "Bazelisk downloads", DeveloperTools, Safe, Os::ALL, &[At(Env("BAZELISK_HOME"), &[]), At(MacCaches, &["bazelisk"]), At(XdgCache, &["bazelisk"]), At(LocalAppData, &["bazelisk"])])
        .why("Bazel binaries downloaded by Bazelisk; fetched again on demand.")
        .ev(ev::LOCAL),
    Rule::target(
        "zig-cache",
        "Zig cache / zig-out",
        BuildArtifacts,
        Safe,
        Os::ALL,
        &[Dir(Is(".zig-cache"), &[Cond::sib(m::BUILD_ZIG)]), Dir(Is("zig-cache"), &[Cond::sib(m::BUILD_ZIG)]), Dir(Is("zig-out"), &[Cond::sib(m::BUILD_ZIG)])],
    )
    .why("Zig build cache and output; rebuilt by zig build.")
    .projects()
    .ev(ev::LOCAL),
    Rule::target("zig-global-cache", "Zig global cache", Cache, Safe, Os::ALL, &[At(XdgCache, &["zig"]), At(LocalAppData, &["zig"])])
        .why("Zig's global build cache; rebuilt as needed.")
        .ev(ev::LOCAL),
    Rule::target("dotnet-bin-obj", ".NET bin / obj", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("bin"), &[Cond::sib(m::DOTNET_PROJ)]), Dir(Is("obj"), &[Cond::sib(m::DOTNET_PROJ)])])
        .why(".NET project build output; recreated by dotnet build.")
        .cleanup(&["dotnet clean"])
        .projects(),
    Rule::target("nuget-packages", "NuGet global packages", PackageCache, Low, Os::ALL, &[At(Env("NUGET_PACKAGES"), &[]), At(Home, &[".nuget", "packages"])])
        .why("NuGet packages for all projects; restore downloads them again.")
        .caution("Close IDEs first, then restore your projects.")
        .cleanup(&["dotnet nuget locals global-packages --clear"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target(
        "nuget-http-cache",
        "NuGet HTTP cache",
        PackageCache,
        Safe,
        Os::ALL,
        &[At(Env("NUGET_HTTP_CACHE_PATH"), &[]), At(Home, &[".local", "share", "NuGet", "v3-cache"]), At(Home, &[".local", "share", "NuGet", "http-cache"]), At(LocalAppData, &["NuGet", "v3-cache"])],
    )
    .why("Cached NuGet feed responses and packages.")
    .cleanup(&["dotnet nuget locals http-cache --clear"])
    .ev(ev::DOCS),
    Rule::target("stack-work", "Haskell .stack-work", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".stack-work"), &[Cond::sib(m::STACK_YAML | m::CABAL)])])
        .why("Stack build output; recreated by stack build.")
        .cleanup(&["stack clean --full"])
        .projects()
        .ev(ev::DOCS),
    Rule::target("cabal-dist", "Cabal dist-newstyle", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("dist-newstyle"), &[Cond::sib(m::CABAL)])])
        .why("Cabal build output; recreated by cabal build.")
        .cleanup(&["cabal clean"])
        .projects(),
    Rule::target("elixir-build", "Elixir _build", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("_build"), &[Cond::sib(m::MIX_EXS)])])
        .why("Mix build output; recreated by mix compile.")
        .projects(),
    Rule::target("elixir-deps", "Elixir deps", Dependencies, Low, Os::ALL, &[Dir(Is("deps"), &[Cond::sib(m::MIX_EXS)])])
        .why("Mix dependencies; mix deps.get restores them.")
        .cleanup(&["mix deps.get"])
        .projects(),
    Rule::target("bundler-vendor", "Bundler vendor/bundle", Dependencies, Low, Os::ALL, &[Under("vendor", Is("bundle"), &[Cond::has(m::RUBY_DIR)])])
        .why("Gems installed into the project; bundle install restores them.")
        .cleanup(&["bundle install"])
        .projects(),
    Rule::target("php-vendor", "Composer vendor", Dependencies, Low, Os::ALL, &[Dir(Is("vendor"), &[Cond::sibs(m::COMPOSER_JSON | m::COMPOSER_LOCK).no_sib(m::GO_MOD)])])
        .why("Composer packages; composer install restores them from the lock.")
        .caution("Some deployments commit vendor/ instead of running composer.")
        .cleanup(&["composer install"])
        .projects(),
    Rule::target(
        "composer-cache",
        "Composer cache",
        PackageCache,
        Safe,
        Os::ALL,
        &[At(Env("COMPOSER_CACHE_DIR"), &[]), At(MacCaches, &["composer"]), At(XdgCache, &["composer"]), At(Home, &[".composer", "cache"]), At(LocalAppData, &["Composer"])],
    )
    .why("Composer download cache; refetched on demand.")
    .cleanup(&["composer clear-cache"])
    .ev(ev::DOCS),
    Rule::target("terraform", "Terraform .terraform", Dependencies, Low, Os::ALL, &[Dir(Is(".terraform"), &[Cond::sib(m::TF_FILES)])])
        .why("Provider/module cache and workspace pointer; terraform init redoes it.")
        .caution("Not during a backend migration. State files sit next to it; never delete those.")
        .cleanup(&["terraform init"])
        .projects()
        .ev(ev::DOCS),
    Rule::target("terragrunt-cache", "Terragrunt cache", Cache, Safe, Os::ALL, &[Dir(Is(".terragrunt-cache"), &[])])
        .why("Terragrunt's copy of modules; recreated on the next run.")
        .projects(),
    Rule::target("unity-library", "Unity Library", BuildArtifacts, Low, Os::ALL, &[Dir(Is("Library"), &[Cond::sibs(m::UNITY_ASSETS | m::UNITY_SETTINGS)])])
        .why("Unity's import cache; rebuilt when the project is next opened.")
        .caution("Close the project in Unity first. Re-import and recompile can take long.")
        .projects()
        .ev(ev::DOCS),
    Rule::target("unity-temp", "Unity Temp / obj", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("Temp"), &[Cond::sibs(m::UNITY_ASSETS | m::UNITY_SETTINGS)]), Dir(Is("obj"), &[Cond::sibs(m::UNITY_ASSETS | m::UNITY_SETTINGS)])])
        .why("Unity temp and C# intermediate files.")
        .caution("Close the project in Unity first.")
        .projects()
        .ev(ev::DOCS),
    Rule::target("unreal-intermediate", "Unreal Intermediate", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("Intermediate"), &[Cond::sib(m::UPROJECT)])])
        .why("Unreal Engine intermediate build files.")
        .projects(),
    Rule::target("unreal-ddc", "Unreal DerivedDataCache", Cache, Low, Os::ALL, &[Dir(Is("DerivedDataCache"), &[Cond::sib(m::UPROJECT)])])
        .why("Unreal's project derived-data cache; rebuilt on demand (slow).")
        .projects(),
    Rule::target("godot-cache", "Godot import cache", BuildArtifacts, Safe, Os::ALL, &[Dir(Is(".godot"), &[Cond::sib(m::PROJECT_GODOT)]), Dir(Is(".import"), &[Cond::sib(m::PROJECT_GODOT)])])
        .why("Godot import cache; regenerated when the project opens.")
        .projects(),
    Rule::target("cdk-out", "AWS CDK cdk.out", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("cdk.out"), &[Cond::sib(m::CDK_JSON)])])
        .why("Synthesized CloudFormation templates and assets.")
        .projects(),
    Rule::target("elm-stuff", "Elm elm-stuff", BuildArtifacts, Safe, Os::ALL, &[Dir(Is("elm-stuff"), &[Cond::sib(m::ELM_JSON)])])
        .why("Elm build artifacts; recreated by elm make.")
        .projects(),
];

// ---------------------------------------------------------------------------------------------
static VMS: &[Rule] = &[
    Rule::target("docker-desktop-vm", "Docker Desktop disk image", VirtualMachines, Review, Os::MAC_WIN, &[At(Home, &["Library", "Containers", "com.docker.docker", "Data", "vms"]), At(LocalAppData, &["Docker", "wsl"]), At(ProgramData, &["DockerDesktop", "vm-data"])])
        .why("All Docker images, containers, build cache and volumes.")
        .caution("Deleting it deletes every container and volume (databases!). Finder shows the maximum size of a sparse file.")
        .cleanup(&["docker system df -v", "docker system prune", "docker builder prune"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("docker-engine", "Docker data (Linux)", VirtualMachines, Review, Os::LINUX, &[At(Root, &["var", "lib", "docker"])])
        .why("All Docker images, containers, build cache and volumes.")
        .caution("`docker volume prune` deletes data; volumes are never pruned by default.")
        .cleanup(&["docker system df -v", "docker system prune", "docker builder prune"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("podman-machine", "Podman machines", VirtualMachines, Review, Os::ALL, &[At(XdgData, &["containers", "podman", "machine"])])
        .why("Podman VM disks with their images, containers and volumes.")
        .cleanup(&["podman system df", "podman system prune", "podman machine rm <name>"])
        .removal(Removal::ToolOnly)
        .ev(ev::LOCAL),
    Rule::target("podman-storage", "Podman storage (rootless)", VirtualMachines, Review, Os::LINUX, &[At(XdgData, &["containers", "storage"])])
        .why("Rootless Podman images, containers and volumes.")
        .cleanup(&["podman system prune"])
        .removal(Removal::ToolOnly),
    Rule::target("colima-lima", "Colima / Lima VMs", VirtualMachines, Review, Os::UNIX, &[At(Home, &[".colima"]), At(Home, &[".lima"])])
        .why("Linux VMs (and their disks) managed by Colima or Lima.")
        .cleanup(&["colima delete", "limactl delete <name>"])
        .removal(Removal::ToolOnly),
    Rule::target("vm-bundles", "Virtual machine", VirtualMachines, Review, Os::ALL, &[Dir(Suffix(".pvm"), &[]), Dir(Suffix(".utm"), &[]), Dir(Suffix(".vmwarevm"), &[])])
        .why("A virtual machine (Parallels/UTM/VMware) with its own OS and files.")
        .cleanup(&["Delete the VM from inside Parallels, UTM or VMware"]),
    Rule::target("virtualbox-vm", "VirtualBox VM", VirtualMachines, Review, Os::ALL, &[Contains(&[Cond::has_any(m::VBOX)])])
        .why("A VirtualBox VM folder with its virtual disks.")
        .cleanup(&["VBoxManage unregistervm <name> --delete"]),
    Rule::target("vagrant-boxes", "Vagrant boxes", VirtualMachines, Low, Os::ALL, &[At(Env("VAGRANT_HOME"), &["boxes"]), At(Home, &[".vagrant.d", "boxes"])])
        .why("Downloaded Vagrant base boxes; re-downloadable.")
        .cleanup(&["vagrant box list", "vagrant box prune"])
        .removal(Removal::ToolOnly),
    Rule::target("wsl-distro", "WSL distribution disk", VirtualMachines, Review, Os::WIN, &[FileAt(LocalAppData, &["Packages", "*", "LocalState"], Is("ext4.vhdx")), At(LocalAppData, &["wsl", "*"])])
        .why("The whole disk of a WSL Linux distribution.")
        .cleanup(&["wsl --list --verbose", "wsl --unregister <distro>"])
        .removal(Removal::ToolOnly),
];

// ---------------------------------------------------------------------------------------------
static LINUX: &[Rule] = &[
    Rule::container("xdg-cache", "User cache (~/.cache)", Cache, Os::UNIX, Some("xdg-cache-entry"), &[At(XdgCache, &[])])
        .why("Per-user non-essential (cached) data, per the XDG spec.")
        .ev(ev::DOCS | ev::LOCAL),
    Rule::target("xdg-cache-entry", "Program cache", Cache, Low, Os::UNIX, &[])
        .why("Cache of one program; rebuilt or re-downloaded on demand.")
        .caution("Close the program first. A few tools keep login tokens here.")
        .ev(ev::DOCS),
    Rule::target("thumbnails", "Thumbnails", Cache, Safe, Os::LINUX, &[At(XdgCache, &["thumbnails"])])
        .why("Image thumbnails; regenerated by the file manager."),
    Rule::container("var-cache", "System cache (/var/cache)", Cache, Os::LINUX, Some("var-cache-entry"), &[At(Root, &["var", "cache"])])
        .why("System-wide application caches (FHS: must be regenerable).")
        .ev(ev::DOCS),
    Rule::target("var-cache-entry", "System program cache", Cache, Review, Os::LINUX, &[])
        .why("System cache of one program; prefer its own cleanup command.")
        .caution("Needs root. Some packages keep state here despite the FHS.")
        .ev(ev::DOCS),
    Rule::protect("debconf", "debconf database", Os::LINUX, crate::model::Scope::NoHeuristics, &[At(Root, &["var", "cache", "debconf"])])
        .why("Debian's package configuration database lives here; never delete.")
        .ev(ev::DOCS),
    Rule::target("apt-cache", "APT package cache", PackageCache, Safe, Os::LINUX, &[At(Root, &["var", "cache", "apt", "archives"])])
        .why("Downloaded .deb packages; not needed after installation.")
        .cleanup(&["sudo apt clean"])
        .removal(Removal::ToolOnly),
    Rule::target("dnf-cache", "DNF/YUM cache", PackageCache, Safe, Os::LINUX, &[At(Root, &["var", "cache", "dnf"]), At(Root, &["var", "cache", "yum"])])
        .why("Downloaded RPM packages and repository metadata.")
        .cleanup(&["sudo dnf clean all"])
        .removal(Removal::ToolOnly),
    Rule::target("pacman-cache", "pacman package cache", PackageCache, Low, Os::LINUX, &[At(Root, &["var", "cache", "pacman", "pkg"])])
        .why("Downloaded packages; old versions allow downgrades.")
        .cleanup(&["sudo paccache -rk2", "sudo pacman -Sc"])
        .removal(Removal::ToolOnly),
    Rule::target("journal", "systemd journal", Logs, Safe, Os::LINUX, &[At(Root, &["var", "log", "journal"])])
        .why("Persistent system logs; vacuum old ones with journalctl.")
        .cleanup(&["journalctl --disk-usage", "sudo journalctl --vacuum-size=500M", "sudo journalctl --vacuum-time=2weeks"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("linux-trash", "Trash", Trash, Review, Os::LINUX, &[At(XdgData, &["Trash"]), Dir(Prefix(".Trash-"), &[]), Dir(Is(".Trash"), &[])])
        .why("Files moved to the trash; emptying deletes them for good.")
        .cleanup(&["gio trash --empty"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("snap-revisions", "Snap packages", SystemUpdates, Low, Os::LINUX, &[At(Root, &["var", "lib", "snapd", "snaps"])])
        .why("Installed snaps, incl. disabled old revisions kept for rollback.")
        .caution("Never delete .snap files by hand.")
        .cleanup(&["snap list --all", "sudo snap remove <name> --revision=<rev>", "sudo snap set system refresh.retain=2"])
        .removal(Removal::ToolOnly)
        .ev(ev::SECONDARY),
    Rule::target("flatpak", "Flatpak apps and runtimes", SystemUpdates, Low, Os::LINUX, &[At(Root, &["var", "lib", "flatpak"]), At(XdgData, &["flatpak"])])
        .why("Installed Flatpak apps and runtimes; unused runtimes can go.")
        .cleanup(&["flatpak uninstall --unused"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("flatpak-app-cache", "Flatpak app cache", Cache, Safe, Os::LINUX, &[At(Home, &[".var", "app", "*", "cache"])])
        .why("Cache of one Flatpak app (its XDG_CACHE_HOME).")
        .removal(Removal::Contents)
        .ev(ev::DOCS),
    Rule::target("coredumps", "Crash dumps", Logs, Safe, Os::LINUX, &[At(Root, &["var", "lib", "systemd", "coredump"]), At(Root, &["var", "crash"])])
        .why("Crash dumps; only needed to debug crashes.")
        .cleanup(&["coredumpctl list"])
        .removal(Removal::Contents),
];

// ---------------------------------------------------------------------------------------------
static WINDOWS: &[Rule] = &[
    Rule::target("win-temp", "Temporary files", Temporary, Low, Os::WIN, &[At(Temp, &[]), At(WinDir, &["Temp"])])
        .why("Temporary files of apps and installers.")
        .caution("Skip files in use; finish pending installs/restarts first.")
        .cleanup(&["Settings ▸ System ▸ Storage ▸ Temporary files"])
        .removal(Removal::Contents),
    Rule::target("win-old", "Previous Windows installation", SystemUpdates, Review, Os::WIN, &[At(SystemDrive, &["Windows.old"])])
        .why("Previous Windows install, needed to roll back; removed after 10 days.")
        .cleanup(&["Settings ▸ System ▸ Storage ▸ Temporary files ▸ Previous version of Windows", "Disk Cleanup ▸ Clean up system files"])
        .removal(Removal::ToolOnly)
        .ev(ev::DOCS),
    Rule::target("win-update-cache", "Windows Update downloads", SystemUpdates, Low, Os::WIN, &[At(WinDir, &["SoftwareDistribution", "Download"])])
        .why("Downloaded updates; not needed once they are installed.")
        .caution("Not while updates install or a restart is pending; stop wuauserv and bits first.")
        .cleanup(&["Disk Cleanup ▸ Clean up system files ▸ Windows Update Cleanup"])
        .removal(Removal::Contents)
        .ev(ev::SECONDARY),
    Rule::target("win-setup-temp", "Windows upgrade leftovers", SystemUpdates, Review, Os::WIN, &[At(SystemDrive, &["$WINDOWS.~BT"]), At(SystemDrive, &["$WINDOWS.~WS"])])
        .why("Temporary files of a Windows upgrade.")
        .cleanup(&["Disk Cleanup ▸ Clean up system files ▸ Temporary Windows installation files"])
        .removal(Removal::ToolOnly)
        .ev(ev::SECONDARY),
    Rule::target("win-recycle-bin", "Recycle Bin", Trash, Review, Os::WIN, &[At(SystemDrive, &["$Recycle.Bin"]), Dir(Is("$Recycle.Bin"), &[])])
        .why("Files you deleted; emptying removes them for good.")
        .cleanup(&["Recycle Bin ▸ Empty Recycle Bin"])
        .removal(Removal::ToolOnly),
    Rule::target("win-crash-dumps", "App crash dumps", Logs, Safe, Os::WIN, &[At(LocalAppData, &["CrashDumps"])])
        .why("Crash dumps of applications; only for debugging.")
        .removal(Removal::Contents),
    Rule::target("win-minidumps", "System crash dumps", Logs, Low, Os::WIN, &[At(WinDir, &["Minidump"]), FileAt(WinDir, &[], Is("MEMORY.DMP"))])
        .why("Blue-screen dumps; only needed to analyze system crashes."),
    Rule::protect("win-system", "Windows / Program Files", Os::WIN, crate::model::Scope::NoHeuristics, &[At(WinDir, &[]), At(SystemDrive, &["Program Files"]), At(SystemDrive, &["Program Files (x86)"]), At(ProgramData, &[])])
        .why("Windows and installed programs; uninstall via Settings ▸ Apps."),
    Rule::protect("win-winsxs", "Component store (WinSxS)", Os::WIN, crate::model::Scope::NoHeuristics, &[At(WinDir, &["WinSxS"])])
        .why("Never delete; shrink it with DISM component cleanup instead.")
        .cleanup(&["Dism.exe /online /Cleanup-Image /StartComponentCleanup"])
        .ev(ev::DOCS),
    Rule::protect("win-installer-cache", "Windows Installer cache", Os::WIN, crate::model::Scope::NoHeuristics, &[At(WinDir, &["Installer"]), At(ProgramData, &["Package Cache"])])
        .why("Needed to repair, update or uninstall programs; never delete."),
    Rule::protect(
        "win-system-files",
        "Page/hibernation file",
        Os::WIN,
        crate::model::Scope::SelfOnly,
        &[FileAt(SystemDrive, &[], Is("pagefile.sys")), FileAt(SystemDrive, &[], Is("hiberfil.sys")), FileAt(SystemDrive, &[], Is("swapfile.sys"))],
    )
    .why("Managed by Windows; `powercfg /h off` removes hiberfil.sys.")
    .cleanup(&["powercfg /h off"]),
];

// ---------------------------------------------------------------------------------------------
const CHROMIUM_PROFILE: When = &[Cond::sibs(m::LOCAL_STORAGE | m::GPU_CACHE)];

static GENERIC: &[Rule] = &[
    Rule::target(
        "chromium-cache",
        "Browser / Electron app cache",
        Cache,
        Safe,
        Os::ALL,
        &[
            Dir(Is("Cache"), CHROMIUM_PROFILE),
            Dir(Is("Code Cache"), CHROMIUM_PROFILE),
            Dir(Is("GPUCache"), CHROMIUM_PROFILE),
            Dir(Is("DawnCache"), CHROMIUM_PROFILE),
            Dir(Is("DawnGraphiteCache"), CHROMIUM_PROFILE),
            Dir(Is("DawnWebGPUCache"), CHROMIUM_PROFILE),
            Dir(Is("GrShaderCache"), CHROMIUM_PROFILE),
        ],
    )
    .why("Web cache of a Chromium browser or Electron app (Slack, VS Code…).")
    .caution("Quit the app first."),
    Rule::target("service-worker-cache", "Website offline data", Cache, Low, Os::ALL, &[Under("Service Worker", Is("CacheStorage"), &[]), Under("Service Worker", Is("ScriptCache"), &[])])
        .why("Offline data of websites and web apps; downloaded again online.")
        .caution("Quit the app first."),
    Rule::target("cachedir-tag", "Tagged cache (CACHEDIR.TAG)", Cache, Low, Os::ALL, &[Contains(&[Cond::has(m::CACHEDIR_TAG)])])
        .why("Its creator tagged it as a regenerable cache (CACHEDIR.TAG).")
        .caution("Tags are hints, not guarantees (the spec: \"never the final word\").")
        .ev(ev::DOCS | ev::LOCAL),
];

// ---------------------------------------------------------------------------------------------
/// A real bundle has `Contents/` (macOS), `Versions/` (framework) or a top-level `Info.plist`
/// (iOS-style); a cache folder that is merely named like a bundle id ("com.example.app") is not.
const BUNDLE: When = &[Cond::has_any(m::BUNDLE_CONTENTS | m::BUNDLE_VERSIONS | m::INFO_PLIST)];

static PROTECTED: &[Rule] = &[
    Rule::protect(
        "top-level",
        "Top-level folder",
        Os::ALL,
        Scope::SelfOnly,
        &[
            At(Root, &[]),
            At(Root, &["Users"]),
            At(Root, &["Applications"]),
            At(Root, &["home"]),
            At(Root, &["Volumes", "*"]),
            At(Home, &[]),
            At(Home, &["Desktop"]),
            At(Home, &["Documents"]),
            At(Home, &["Downloads"]),
            At(Home, &["Pictures"]),
            At(Home, &["Movies"]),
            At(Home, &["Music"]),
            At(Home, &["Videos"]),
            At(Home, &["Public"]),
            At(SystemDrive, &[]),
            At(SystemDrive, &["Users"]),
        ],
    )
    .why("Top-level folder; never suggested as a whole."),
    Rule::protect("mac-library", "~/Library", Os::MAC, Scope::NoProjectRules, &[At(Home, &["Library"])])
        .why("App data and settings; only known caches inside are suggested."),
    Rule::protect("home-dotdirs", "Hidden tool folder", Os::ALL, Scope::NoProjectRules, &[At(Home, &[".*"]), At(Home, &["AppData"])])
        .why("Tool settings and data; only known caches inside are suggested."),
    Rule::protect("icloud-drive", "iCloud Drive", Os::MAC, Scope::NoHeuristics, &[At(Home, &["Library", "Mobile Documents"])])
        .why("iCloud Drive: deleting here deletes on all your devices."),
    Rule::protect("mail-messages", "Mail and Messages", Os::MAC, Scope::NoHeuristics, &[At(Home, &["Library", "Mail"]), At(Home, &["Library", "Messages"])])
        .why("Your mail and message history."),
    Rule::protect("media-libraries", "Photos / Music library", Os::MAC, Scope::NoHeuristics, &[Dir(Suffix(".photoslibrary"), &[]), Dir(Suffix(".musiclibrary"), &[])])
        .why("Media library; delete items inside its app instead."),
    Rule::protect(
        "app-bundles",
        "Application bundle",
        Os::ALL,
        Scope::NoHeuristics,
        &[
            Dir(Suffix(".app"), BUNDLE),
            Dir(Suffix(".framework"), BUNDLE),
            Dir(Suffix(".bundle"), BUNDLE),
            Dir(Suffix(".plugin"), BUNDLE),
            Dir(Suffix(".appex"), BUNDLE),
            Dir(Suffix(".xpc"), BUNDLE),
            Dir(Suffix(".kext"), BUNDLE),
        ],
    )
    .why("Inside an app bundle; uninstall the app instead."),
    Rule::protect(
        "test-fixtures",
        "Test fixtures",
        Os::ALL,
        Scope::NoProjectRules,
        &[
            Dir(Is("testData"), &[]),
            Dir(Is("testdata"), &[]),
            Dir(Is("test-data"), &[]),
            Dir(Is("test_data"), &[]),
            Dir(Is("testResources"), &[]),
            Dir(Is("test-resources"), &[]),
            Dir(Is("fixtures"), &[]),
            Dir(Is("__fixtures__"), &[]),
        ],
    )
    .why("Test data checked into a repository; artifacts inside may be fixtures."),
    Rule::protect("vcs-data", "Version-control data", Os::ALL, Scope::NoHeuristics, &[Dir(Is(".git"), &[]), Dir(Is(".hg"), &[]), Dir(Is(".svn"), &[])])
        .why("Repository history; not a cache."),
    Rule::protect(
        "homebrew-prefix",
        "Homebrew prefix",
        Os::UNIX,
        Scope::NoHeuristics,
        &[
            At(Root, &["opt", "homebrew"]),
            At(Root, &["usr", "local", "Cellar"]),
            At(Root, &["usr", "local", "Caskroom"]),
            At(Root, &["usr", "local", "Homebrew"]),
            At(Root, &["home", "linuxbrew", ".linuxbrew"]),
        ],
    )
    .why("Homebrew-managed software; let brew remove old versions.")
    .cleanup(&["brew cleanup", "brew autoremove"])
    .ev(ev::DOCS | ev::LOCAL),
    Rule::protect("python-tools", "uv / pipx tools", Os::ALL, Scope::NoHeuristics, &[At(XdgData, &["uv", "tools"]), At(XdgData, &["pipx"]), At(Home, &[".local", "pipx"])])
        .why("Command-line tools installed with uv or pipx.")
        .cleanup(&["uv tool uninstall <name>", "pipx uninstall <name>"]),
    Rule::protect(
        "mac-system",
        "macOS system",
        Os::MAC,
        Scope::NoHeuristics,
        &[At(Root, &["System"]), At(Root, &["usr"]), At(Root, &["bin"]), At(Root, &["sbin"]), At(Root, &["Library"]), At(Root, &["private"]), At(Root, &["cores"])],
    )
    .why("Managed by macOS; only specific caches inside are suggested."),
    Rule::protect(
        "linux-system",
        "Linux system",
        Os::LINUX,
        Scope::NoHeuristics,
        &[
            At(Root, &["usr"]),
            At(Root, &["bin"]),
            At(Root, &["sbin"]),
            At(Root, &["lib"]),
            At(Root, &["lib64"]),
            At(Root, &["etc"]),
            At(Root, &["boot"]),
            At(Root, &["opt"]),
            At(Root, &["snap"]),
            At(Root, &["nix"]),
            At(Root, &["srv"]),
            At(Root, &["var", "lib"]),
            At(Root, &["proc"]),
            At(Root, &["sys"]),
            At(Root, &["dev"]),
            At(Root, &["run"]),
        ],
    )
    .why("Operating-system or package-managed files."),
];
