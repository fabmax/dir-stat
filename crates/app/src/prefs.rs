//! Persistent preferences. The store is a trait so app tests never touch disk.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// At most this many recent scan roots are kept.
const MAX_RECENTS: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Most recent first, as (path, unix seconds).
    pub recents: Vec<(PathBuf, i64)>,
}

impl Prefs {
    pub fn add_recent(&mut self, path: PathBuf, now: i64) {
        self.recents.retain(|(p, _)| *p != path);
        self.recents.insert(0, (path, now));
        self.recents.truncate(MAX_RECENTS);
    }
}

pub trait PrefsStore: Send {
    fn load(&self) -> Prefs;
    fn save(&self, prefs: &Prefs);
}

/// `<config dir>/dir-stat/prefs.json`. A missing or broken file gives the defaults.
pub struct JsonPrefs {
    path: Option<PathBuf>,
}

impl JsonPrefs {
    pub fn in_config_dir() -> Self {
        JsonPrefs {
            path: dirs::config_dir().map(|d| d.join("dir-stat").join("prefs.json")),
        }
    }
}

impl PrefsStore for JsonPrefs {
    fn load(&self) -> Prefs {
        self.path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, prefs: &Prefs) {
        let Some(path) = &self.path else { return };
        let Ok(json) = serde_json::to_vec_pretty(prefs) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Write then rename, so a crash never leaves half a file.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

#[cfg(test)]
pub struct MemoryPrefs(pub std::sync::Mutex<Prefs>);

#[cfg(test)]
impl PrefsStore for MemoryPrefs {
    fn load(&self) -> Prefs {
        self.0.lock().unwrap().clone()
    }

    fn save(&self, prefs: &Prefs) {
        *self.0.lock().unwrap() = prefs.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recents_are_unique_newest_first_and_capped() {
        let mut prefs = Prefs::default();
        for (i, p) in ["/a", "/b", "/c", "/a", "/d", "/e", "/f"]
            .into_iter()
            .enumerate()
        {
            prefs.add_recent(p.into(), i as i64);
        }
        let paths: Vec<_> = prefs
            .recents
            .iter()
            .map(|(p, _)| p.to_str().unwrap())
            .collect();
        assert_eq!(paths, ["/f", "/e", "/d", "/a", "/c"]);
    }

    #[test]
    fn json_round_trip_and_broken_files() {
        let dir = tempfile_dir();
        let store = JsonPrefs {
            path: Some(dir.join("sub/prefs.json")),
        };
        assert_eq!(store.load(), Prefs::default());
        let mut prefs = Prefs::default();
        prefs.add_recent("/x".into(), 42);
        store.save(&prefs);
        assert_eq!(store.load(), prefs);
        std::fs::write(dir.join("sub/prefs.json"), b"{not json").unwrap();
        assert_eq!(store.load(), Prefs::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dir-stat-prefs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
}
