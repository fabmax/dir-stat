//! Data model of dir-stat's cleanup-suggestion catalog.
//!
//! Everything here is `const`-constructible so the whole catalog is one static table
//! (`super::table::CATALOG`). The matcher (`super::engine`) indexes it once at startup.

/// How risky it is to delete a matched folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Risk {
    /// Pure cache / derived data. The owning tool recreates it on demand; no user data.
    Safe,
    /// Restorable, but only by re-downloading / rebuilding / re-running a tool
    /// (costs time or bandwidth, may break offline work until then).
    Low,
    /// May contain user data or state that cannot be recreated automatically. Inspect first.
    Review,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Cache,
    BuildArtifacts,
    Dependencies,
    PackageCache,
    DeveloperTools,
    Logs,
    Trash,
    Installers,
    VirtualMachines,
    Backups,
    Temporary,
    SystemUpdates,
    Models,
    Protected,
}

impl Category {
    /// Short label for the tree-row pill (ux-spec §6 / §16.2).
    pub const fn pill(self) -> &'static str {
        match self {
            Category::Cache => "Cache",
            Category::BuildArtifacts => "Build",
            Category::Dependencies => "Deps",
            Category::PackageCache => "Cache",
            Category::DeveloperTools => "Tools",
            Category::Logs => "Logs",
            Category::Trash => "Trash",
            Category::Installers => "Installer",
            Category::VirtualMachines => "VM",
            Category::Backups => "Backup",
            Category::Temporary => "Temp",
            Category::SystemUpdates => "Update",
            Category::Models => "Model",
            Category::Protected => "System",
        }
    }
    pub const fn title(self) -> &'static str {
        match self {
            Category::Cache => "Cache",
            Category::BuildArtifacts => "Build artifacts",
            Category::Dependencies => "Dependencies",
            Category::PackageCache => "Package-manager cache",
            Category::DeveloperTools => "Developer tools",
            Category::Logs => "Logs",
            Category::Trash => "Trash",
            Category::Installers => "Installers",
            Category::VirtualMachines => "VMs/containers",
            Category::Backups => "Backups",
            Category::Temporary => "Temporary files",
            Category::SystemUpdates => "System updates",
            Category::Models => "ML models",
            Category::Protected => "Protected",
        }
    }
}

/// Platform bit set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Os(pub u8);

impl Os {
    pub const MAC: Os = Os(1);
    pub const LINUX: Os = Os(2);
    pub const WIN: Os = Os(4);
    pub const UNIX: Os = Os(1 | 2);
    pub const MAC_WIN: Os = Os(1 | 4);
    pub const ALL: Os = Os(1 | 2 | 4);
    pub const fn has(self, other: Os) -> bool {
        self.0 & other.0 != 0
    }
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MAC
        } else if cfg!(windows) {
            Os::WIN
        } else {
            Os::LINUX
        }
    }
    pub fn label(self) -> &'static str {
        match self.0 {
            1 => "mac",
            2 => "linux",
            4 => "win",
            3 => "mac, linux",
            5 => "mac, win",
            6 => "linux, win",
            7 => "all",
            _ => "-",
        }
    }
}

/// Where an anchored path starts. Resolved once at startup (`engine::Resolver`);
/// a base that does not exist on the current platform (or an unset env var) resolves to
/// nothing and the matcher is simply not indexed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// `/` (Unix only).
    Root,
    /// `$HOME` / `%USERPROFILE%`.
    Home,
    /// `~/Library/Caches` (macOS only).
    MacCaches,
    /// `$XDG_CACHE_HOME` or `~/.cache` (Linux *and* macOS: many CLI tools use it there too).
    XdgCache,
    /// `$XDG_DATA_HOME` or `~/.local/share` (Unix).
    XdgData,
    /// `%LOCALAPPDATA%` (Windows).
    LocalAppData,
    /// `%APPDATA%` (Windows, roaming).
    AppData,
    /// `%TEMP%` (Windows).
    Temp,
    /// `%SystemRoot%`, usually `C:\Windows`.
    WinDir,
    /// `%SystemDrive%\`, usually `C:\`.
    SystemDrive,
    /// `%ProgramData%`.
    ProgramData,
    /// Absolute path taken from an environment variable (first entry if it is a path list).
    Env(&'static str),
}

/// Name pattern. `Glob` supports `*` only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pat {
    Is(&'static str),
    Prefix(&'static str),
    Suffix(&'static str),
    Glob(&'static str),
    /// File extension (case-insensitive), files only.
    Ext(&'static [&'static str]),
}

/// One conjunction over marker bit sets (see [`m`]):
/// `sib_*` test the parent's children (= siblings incl. the dir itself),
/// `own_*` test the directory's own children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cond {
    pub sib_all: u64,
    pub sib_any: u64,
    pub sib_none: u64,
    pub own_all: u64,
    pub own_any: u64,
    pub own_none: u64,
}

impl Cond {
    pub const NONE: Cond = Cond {
        sib_all: 0,
        sib_any: 0,
        sib_none: 0,
        own_all: 0,
        own_any: 0,
        own_none: 0,
    };
    /// Parent contains at least one of `m`.
    pub const fn sib(m: u64) -> Cond {
        Cond {
            sib_any: m,
            ..Cond::NONE
        }
    }
    /// Parent contains all of `m`.
    pub const fn sibs(m: u64) -> Cond {
        Cond {
            sib_all: m,
            ..Cond::NONE
        }
    }
    /// Directory itself contains all of `m`.
    pub const fn has(m: u64) -> Cond {
        Cond {
            own_all: m,
            ..Cond::NONE
        }
    }
    /// Directory itself contains at least one of `m`.
    pub const fn has_any(m: u64) -> Cond {
        Cond {
            own_any: m,
            ..Cond::NONE
        }
    }
    pub const fn and_has(self, m: u64) -> Cond {
        Cond {
            own_all: self.own_all | m,
            ..self
        }
    }
    pub const fn and_has_any(self, m: u64) -> Cond {
        Cond {
            own_any: self.own_any | m,
            ..self
        }
    }
    pub const fn and_sibs(self, m: u64) -> Cond {
        Cond {
            sib_all: self.sib_all | m,
            ..self
        }
    }
    /// ... and the directory itself contains none of `m`.
    pub const fn lacks(self, m: u64) -> Cond {
        Cond {
            own_none: self.own_none | m,
            ..self
        }
    }
    /// ... and the parent contains none of `m`.
    pub const fn no_sib(self, m: u64) -> Cond {
        Cond {
            sib_none: self.sib_none | m,
            ..self
        }
    }
    pub const fn ok(&self, sib: u64, own: u64) -> bool {
        (sib & self.sib_all) == self.sib_all
            && (self.sib_any == 0 || sib & self.sib_any != 0)
            && sib & self.sib_none == 0
            && (own & self.own_all) == self.own_all
            && (self.own_any == 0 || own & self.own_any != 0)
            && own & self.own_none == 0
    }
}

/// Disjunction of conjunctions; empty = unconditional.
pub type When = &'static [Cond];

#[derive(Clone, Copy, Debug)]
pub enum Matcher {
    /// Anchored directory: base + relative components (components may be `*`-globs).
    At(Base, &'static [&'static str]),
    /// Regular file directly inside an anchored directory.
    FileAt(Base, &'static [&'static str], Pat),
    /// Heuristic: directory anywhere whose own name matches, gated by markers.
    Dir(Pat, When),
    /// Heuristic: like `Dir`, but the parent directory must be named exactly so.
    Under(&'static str, Pat, When),
    /// Heuristic: directory anywhere identified only by its own children (`own_*` conditions).
    Contains(When),
}

/// What a `Protect` rule does below the protected directory (the directory itself is
/// never suggested in every case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Only this directory is protected; rules keep working inside (e.g. `~/Documents`).
    SelfOnly,
    /// Inside, project rules (node_modules, target, build, .venv, ...) are disabled
    /// (app-data and tool folders: `~/Library`, `~/.*`, `%APPDATA%`).
    NoProjectRules,
    /// Inside, no heuristic rule applies at all; anchored rules still do
    /// (app bundles, OS folders, package-manager prefixes).
    NoHeuristics,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The matched folder (or file) is one cleanup candidate. Nothing below it is evaluated
    /// again ("outermost match wins").
    Target(Risk),
    /// A well-known root of independent caches (`~/Library/Caches`, `~/.cache`, ...): not a
    /// candidate itself; each direct child is matched against specific rules and otherwise
    /// gets `default` (if any).
    Container { default: Option<&'static str> },
    /// Never suggested.
    Protect(Scope),
}

/// Heuristic rules tagged `Projects` only fire outside `Scope::NoProjectRules` zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Anywhere,
    Projects,
}

/// How the app may delete a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// Move the matched folder (or file) itself to the Trash.
    Item,
    /// Move its contents to the Trash but keep the folder (system/tool-owned folder).
    Contents,
    /// Do not delete files directly; only show `cleanup` (tool command or UI path).
    ToolOnly,
}

/// Evidence bits: how the entry was verified.
pub mod ev {
    /// Official documentation of the tool / OS vendor.
    pub const DOCS: u8 = 1;
    /// Read in the tool's source code.
    pub const SOURCE: u8 = 2;
    /// Observed on the reference machine (macOS 26, Apple silicon).
    pub const LOCAL: u8 = 4;
    /// Secondary sources only (forums, vendor-neutral guides).
    pub const SECONDARY: u8 = 8;
    // 0 = from knowledge, unverified -> double-check before relying on it.
}

#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub id: &'static str,
    /// Group title in the cleanup panel ("node_modules", "Xcode DerivedData").
    pub name: &'static str,
    pub category: Category,
    pub os: Os,
    pub kind: Kind,
    pub zone: Zone,
    pub matchers: &'static [Matcher],
    pub removal: Removal,
    /// One sentence for the user, <= 70 chars, sentence case, ends with a period.
    pub reason: &'static str,
    /// Extra warning shown next to the delete action.
    pub caution: Option<&'static str>,
    /// Proper cleanup: shell commands (backticked when rendered) or UI paths (contain '▸').
    /// `{name}` = matched folder name, `{path}` = full path.
    pub cleanup: &'static [&'static str],
    /// Files are usually hard links / APFS clones shared with another folder (pnpm store,
    /// simulator clones): the measured size overstates what deleting frees.
    pub shares_space: bool,
    pub evidence: u8,
}

impl Rule {
    const fn base(
        id: &'static str,
        name: &'static str,
        category: Category,
        os: Os,
        kind: Kind,
        matchers: &'static [Matcher],
    ) -> Rule {
        Rule {
            id,
            name,
            category,
            os,
            kind,
            zone: Zone::Anywhere,
            matchers,
            removal: Removal::Item,
            reason: "",
            caution: None,
            cleanup: &[],
            shares_space: false,
            evidence: 0,
        }
    }
    pub const fn target(
        id: &'static str,
        name: &'static str,
        category: Category,
        risk: Risk,
        os: Os,
        matchers: &'static [Matcher],
    ) -> Rule {
        Rule::base(id, name, category, os, Kind::Target(risk), matchers)
    }
    pub const fn container(
        id: &'static str,
        name: &'static str,
        category: Category,
        os: Os,
        default: Option<&'static str>,
        matchers: &'static [Matcher],
    ) -> Rule {
        Rule {
            removal: Removal::ToolOnly,
            ..Rule::base(
                id,
                name,
                category,
                os,
                Kind::Container { default },
                matchers,
            )
        }
    }
    pub const fn protect(
        id: &'static str,
        name: &'static str,
        os: Os,
        scope: Scope,
        matchers: &'static [Matcher],
    ) -> Rule {
        Rule {
            removal: Removal::ToolOnly,
            ..Rule::base(
                id,
                name,
                Category::Protected,
                os,
                Kind::Protect(scope),
                matchers,
            )
        }
    }
    pub const fn why(self, reason: &'static str) -> Rule {
        Rule { reason, ..self }
    }
    pub const fn caution(self, c: &'static str) -> Rule {
        Rule {
            caution: Some(c),
            ..self
        }
    }
    pub const fn cleanup(self, c: &'static [&'static str]) -> Rule {
        Rule { cleanup: c, ..self }
    }
    pub const fn removal(self, r: Removal) -> Rule {
        Rule { removal: r, ..self }
    }
    pub const fn projects(self) -> Rule {
        Rule {
            zone: Zone::Projects,
            ..self
        }
    }
    pub const fn shares_space(self) -> Rule {
        Rule {
            shares_space: true,
            ..self
        }
    }
    pub const fn ev(self, e: u8) -> Rule {
        Rule {
            evidence: e,
            ..self
        }
    }
    pub const fn risk(&self) -> Option<Risk> {
        match self.kind {
            Kind::Target(r) => Some(r),
            _ => None,
        }
    }
}

/// Marker bits: cheap facts about a directory's children, computed from the names (and the
/// file/dir type) that `read_dir` returns anyway. A directory's marker set is passed down to
/// its children, which see it as their *sibling* set.
pub mod m {
    pub const PACKAGE_JSON: u64 = 1 << 0;
    pub const CARGO_TOML: u64 = 1 << 1;
    /// `CACHEDIR.TAG` regular file. The scanner must clear the bit unless the first 43 bytes
    /// are the spec signature (see `engine::is_valid_cachedir_tag`).
    pub const CACHEDIR_TAG: u64 = 1 << 2;
    /// `.rustc_info.json` (written by Cargo into every target dir).
    pub const RUSTC_INFO: u64 = 1 << 3;
    pub const PYVENV_CFG: u64 = 1 << 4;
    pub const POM_XML: u64 = 1 << 5;
    /// `build.gradle` / `build.gradle.kts`
    pub const GRADLE_BUILD: u64 = 1 << 6;
    /// `settings.gradle` / `settings.gradle.kts`
    pub const GRADLE_SETTINGS: u64 = 1 << 7;
    pub const GRADLEW: u64 = 1 << 8;
    pub const BUILD_SBT: u64 = 1 << 9;
    pub const PACKAGE_SWIFT: u64 = 1 << 10;
    pub const PODFILE: u64 = 1 << 11;
    pub const CARTFILE: u64 = 1 << 12;
    pub const PUBSPEC: u64 = 1 << 13;
    /// `stack.yaml`
    pub const STACK_YAML: u64 = 1 << 14;
    /// `*.cabal` / `cabal.project`
    pub const CABAL: u64 = 1 << 15;
    pub const MIX_EXS: u64 = 1 << 16;
    pub const COMPOSER_JSON: u64 = 1 << 17;
    pub const COMPOSER_LOCK: u64 = 1 << 18;
    pub const GO_MOD: u64 = 1 << 19;
    pub const BUILD_ZIG: u64 = 1 << 20;
    pub const PROJECT_GODOT: u64 = 1 << 21;
    pub const CMAKE_CACHE: u64 = 1 << 22;
    pub const CMAKE_LISTS: u64 = 1 << 23;
    /// `next.config.{js,mjs,cjs,ts}`
    pub const NEXT_CONFIG: u64 = 1 << 24;
    /// `nuxt.config.*`
    pub const NUXT_CONFIG: u64 = 1 << 25;
    /// `svelte.config.*`
    pub const SVELTE_CONFIG: u64 = 1 << 26;
    pub const ANGULAR_JSON: u64 = 1 << 27;
    pub const TURBO_JSON: u64 = 1 << 28;
    /// `.modules.yaml` file or `.pnpm` dir (inside a pnpm-managed node_modules)
    pub const PNPM_MODULES: u64 = 1 << 29;
    /// dir `Assets` (Unity)
    pub const UNITY_ASSETS: u64 = 1 << 30;
    /// dir `ProjectSettings` (Unity)
    pub const UNITY_SETTINGS: u64 = 1 << 31;
    /// `*.csproj` / `*.fsproj` / `*.vbproj`
    pub const DOTNET_PROJ: u64 = 1 << 32;
    /// `*.tf` / `*.tf.json` / `.terraform.lock.hcl`
    pub const TF_FILES: u64 = 1 << 33;
    /// `*.uproject`
    pub const UPROJECT: u64 = 1 << 34;
    pub const ELM_JSON: u64 = 1 << 35;
    pub const CDK_JSON: u64 = 1 << 36;
    /// `*.vbox` (VirtualBox VM folder)
    pub const VBOX: u64 = 1 << 37;
    /// `.git` dir or file
    pub const GIT: u64 = 1 << 38;
    /// dir `conda-meta`
    pub const CONDA_META: u64 = 1 << 39;
    /// dir `execroot` (Bazel output base)
    pub const BAZEL_EXECROOT: u64 = 1 << 40;
    /// dir `external` (Bazel output base)
    pub const BAZEL_EXTERNAL: u64 = 1 << 41;
    /// dir `Local Storage` (Chromium/Electron profile)
    pub const LOCAL_STORAGE: u64 = 1 << 42;
    /// dir `GPUCache` (Chromium/Electron profile)
    pub const GPU_CACHE: u64 = 1 << 43;
    /// dir `ruby` (Bundler's vendor/bundle/ruby/<abi>)
    pub const RUBY_DIR: u64 = 1 << 44;
    pub const TOX_INI: u64 = 1 << 45;
    pub const NOXFILE: u64 = 1 << 46;
    /// `pyproject.toml` / `setup.py` / `setup.cfg`
    pub const PY_PROJECT: u64 = 1 << 47;
    /// dir `Contents` (macOS bundle layout)
    pub const BUNDLE_CONTENTS: u64 = 1 << 48;
    /// file `Info.plist` (flat iOS-style bundle)
    pub const INFO_PLIST: u64 = 1 << 49;
    /// dir `Versions` (macOS framework layout)
    pub const BUNDLE_VERSIONS: u64 = 1 << 50;

    pub const NAMES: &[(u64, &str)] = &[
        (PACKAGE_JSON, "package.json"),
        (CARGO_TOML, "Cargo.toml"),
        (CACHEDIR_TAG, "CACHEDIR.TAG"),
        (RUSTC_INFO, ".rustc_info.json"),
        (PYVENV_CFG, "pyvenv.cfg"),
        (POM_XML, "pom.xml"),
        (GRADLE_BUILD, "build.gradle[.kts]"),
        (GRADLE_SETTINGS, "settings.gradle[.kts]"),
        (GRADLEW, "gradlew"),
        (BUILD_SBT, "build.sbt"),
        (PACKAGE_SWIFT, "Package.swift"),
        (PODFILE, "Podfile"),
        (CARTFILE, "Cartfile"),
        (PUBSPEC, "pubspec.yaml"),
        (STACK_YAML, "stack.yaml"),
        (CABAL, "*.cabal|cabal.project"),
        (MIX_EXS, "mix.exs"),
        (COMPOSER_JSON, "composer.json"),
        (COMPOSER_LOCK, "composer.lock"),
        (GO_MOD, "go.mod"),
        (BUILD_ZIG, "build.zig"),
        (PROJECT_GODOT, "project.godot"),
        (CMAKE_CACHE, "CMakeCache.txt"),
        (CMAKE_LISTS, "CMakeLists.txt"),
        (NEXT_CONFIG, "next.config.*"),
        (NUXT_CONFIG, "nuxt.config.*"),
        (SVELTE_CONFIG, "svelte.config.*"),
        (ANGULAR_JSON, "angular.json"),
        (TURBO_JSON, "turbo.json"),
        (PNPM_MODULES, ".modules.yaml|.pnpm/"),
        (UNITY_ASSETS, "Assets/"),
        (UNITY_SETTINGS, "ProjectSettings/"),
        (DOTNET_PROJ, "*.csproj|*.fsproj|*.vbproj"),
        (TF_FILES, "*.tf|*.tf.json|.terraform.lock.hcl"),
        (UPROJECT, "*.uproject"),
        (ELM_JSON, "elm.json"),
        (CDK_JSON, "cdk.json"),
        (VBOX, "*.vbox"),
        (GIT, ".git"),
        (CONDA_META, "conda-meta/"),
        (BAZEL_EXECROOT, "execroot/"),
        (BAZEL_EXTERNAL, "external/"),
        (LOCAL_STORAGE, "Local Storage/"),
        (GPU_CACHE, "GPUCache/"),
        (RUBY_DIR, "ruby/"),
        (TOX_INI, "tox.ini"),
        (NOXFILE, "noxfile.py"),
        (PY_PROJECT, "pyproject.toml|setup.py|setup.cfg"),
        (BUNDLE_CONTENTS, "Contents/"),
        (INFO_PLIST, "Info.plist"),
        (BUNDLE_VERSIONS, "Versions/"),
    ];

    pub fn describe(mask: u64) -> String {
        NAMES
            .iter()
            .filter(|(b, _)| mask & b != 0)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// Marker bits for one directory entry. O(1): one `match` on the name plus one on the
/// extension; call it for every entry `read_dir` returns and OR the results.
pub fn marker_of(name: &str, is_dir: bool) -> u64 {
    use m::*;
    if is_dir {
        return match name {
            "Assets" => UNITY_ASSETS,
            "ProjectSettings" => UNITY_SETTINGS,
            "conda-meta" => CONDA_META,
            "execroot" => BAZEL_EXECROOT,
            "external" => BAZEL_EXTERNAL,
            "Local Storage" => LOCAL_STORAGE,
            "GPUCache" => GPU_CACHE,
            "ruby" => RUBY_DIR,
            ".git" => GIT,
            ".pnpm" => PNPM_MODULES,
            "Contents" => BUNDLE_CONTENTS,
            "Versions" => BUNDLE_VERSIONS,
            _ => 0,
        };
    }
    let exact = match name {
        "package.json" => PACKAGE_JSON,
        "Cargo.toml" => CARGO_TOML,
        "CACHEDIR.TAG" => CACHEDIR_TAG,
        ".rustc_info.json" => RUSTC_INFO,
        "pyvenv.cfg" => PYVENV_CFG,
        "pom.xml" => POM_XML,
        "build.gradle" | "build.gradle.kts" => GRADLE_BUILD,
        "settings.gradle" | "settings.gradle.kts" => GRADLE_SETTINGS,
        "gradlew" => GRADLEW,
        "build.sbt" => BUILD_SBT,
        "Package.swift" => PACKAGE_SWIFT,
        "Podfile" => PODFILE,
        "Cartfile" => CARTFILE,
        "pubspec.yaml" => PUBSPEC,
        "stack.yaml" => STACK_YAML,
        "cabal.project" => CABAL,
        "mix.exs" => MIX_EXS,
        "composer.json" => COMPOSER_JSON,
        "composer.lock" => COMPOSER_LOCK,
        "go.mod" => GO_MOD,
        "build.zig" => BUILD_ZIG,
        "project.godot" => PROJECT_GODOT,
        "CMakeCache.txt" => CMAKE_CACHE,
        "CMakeLists.txt" => CMAKE_LISTS,
        "angular.json" => ANGULAR_JSON,
        "turbo.json" => TURBO_JSON,
        ".modules.yaml" => PNPM_MODULES,
        ".terraform.lock.hcl" => TF_FILES,
        "cdk.json" => CDK_JSON,
        "elm.json" => ELM_JSON,
        "tox.ini" => TOX_INI,
        "noxfile.py" => NOXFILE,
        "pyproject.toml" | "setup.py" | "setup.cfg" => PY_PROJECT,
        ".git" => GIT, // worktrees / submodules use a .git *file*
        "Info.plist" => INFO_PLIST,
        _ => 0,
    };
    if exact != 0 {
        return exact;
    }
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return 0;
    };
    match ext {
        "csproj" | "fsproj" | "vbproj" => DOTNET_PROJ,
        "tf" => TF_FILES,
        "json" if stem.ends_with(".tf") => TF_FILES,
        "uproject" => UPROJECT,
        "cabal" => CABAL,
        "vbox" => VBOX,
        _ => match stem {
            "next.config" => NEXT_CONFIG,
            "nuxt.config" => NUXT_CONFIG,
            "svelte.config" => SVELTE_CONFIG,
            _ => 0,
        },
    }
}

/// Signature from the Cache Directory Tagging Specification (https://bford.info/cachedir/).
pub const CACHEDIR_SIGNATURE: &[u8; 43] = b"Signature: 8a477f597d28d172789f06886806bc55";
