//! macOS packages (UX §6.9): bundles that Finder shows as one item.

use dirstat_core::catalog::m;
use dirstat_core::tree::{NodeId, Tree};

/// Code bundles: the extension and the bundle layout inside (FS §6.6).
const CODE_BUNDLES: &[&str] = &[
    "app",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "appex",
    "xpc",
];
/// Document packages, recognized by extension alone: they have no common layout.
const DOCUMENT_PACKAGES: &[&str] = &[
    "photoslibrary",
    "musiclibrary",
    "tvlibrary",
    "imovielibrary",
    "fcpbundle",
    "xcarchive",
    "logarchive",
    "rtfd",
];

pub fn is_package(tree: &Tree, id: NodeId) -> bool {
    let node = tree.node(id);
    let name = node.name().to_string_lossy();
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    if DOCUMENT_PACKAGES.contains(&extension.as_str()) {
        return true;
    }
    CODE_BUNDLES.contains(&extension.as_str())
        && node.markers() & (m::BUNDLE_CONTENTS | m::INFO_PLIST | m::BUNDLE_VERSIONS) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{TreeBuilder, dir};

    #[test]
    fn bundles_need_their_layout_document_packages_dont() {
        let tree = TreeBuilder::new(
            "/r",
            dir("r")
                .child(dir("Safari.app").markers(m::BUNDLE_CONTENTS))
                .child(dir("fake.app"))
                .child(dir("Photos Library.photoslibrary"))
                .child(dir("notes.txt.d")),
        )
        .build();
        let find = |p: &str| tree.find_path(std::path::Path::new(p)).unwrap();
        assert!(is_package(&tree, find("/r/Safari.app")));
        assert!(!is_package(&tree, find("/r/fake.app")));
        assert!(is_package(&tree, find("/r/Photos Library.photoslibrary")));
        assert!(!is_package(&tree, find("/r/notes.txt.d")));
    }
}
