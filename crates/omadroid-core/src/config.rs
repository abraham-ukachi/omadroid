//! The little that has to be remembered between runs.
//!
//! Where the machine's own Waydroid images live, which Android version they
//! are, and whether their data directory has already been adopted. Everything
//! else is read back from disk, so a deleted store costs at most one probe.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// `~/.config/omadroid/omadroid.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    /// The images Waydroid was already using before omadroid existed.
    #[serde(default)]
    pub system_images: Option<String>,
    /// That image's Android release, or 0 when it could not be read.
    #[serde(default)]
    pub system_android: u32,
    /// Their `data` has been moved into `versions/` once already.
    #[serde(default)]
    pub adopted: bool,
}

impl Store {
    /// Reads the store, or starts empty when it is missing or corrupt.
    pub fn load(path: &Path) -> Store {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Store::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Writes the store atomically: a temp file, then a rename. The previous
    /// file is kept as `omadroid.json.bak`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text).map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
        if path.exists() {
            let _ = std::fs::copy(path, path.with_extension("json.bak"));
        }
        std::fs::rename(&temp, path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_store_is_empty() {
        assert_eq!(Store::load(Path::new("/nonexistent/omadroid.json")), Store::default());
    }

    #[test]
    fn a_corrupt_store_is_empty() {
        let dir = std::env::temp_dir().join(format!("omadroid-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("omadroid.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Store::load(&path), Store::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_round_trips() {
        let dir = std::env::temp_dir().join(format!("omadroid-save-{}", std::process::id()));
        let path = dir.join("omadroid.json");
        let store = Store {
            system_images: Some("/usr/share/waydroid-extra/images".into()),
            system_android: 13,
            adopted: true,
        };
        store.save(&path).unwrap();
        assert_eq!(Store::load(&path), store);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
