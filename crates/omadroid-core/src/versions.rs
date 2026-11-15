//! The Android versions installed on this machine.
//!
//! A version is a directory with `system.img`, `vendor.img`, `data/` and a
//! `meta.json`. The images the machine had before omadroid are a version too:
//! they stay where they are (`/usr/share/waydroid-extra/images`), and only
//! their data directory moves into `versions/` the first time another version
//! is started.
//!
//! Nothing is copied: switching points Waydroid at another directory.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::Variant;
use crate::paths;
use crate::Store;

/// `meta.json`, written next to the images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub android: u32,
    #[serde(default)]
    pub lineage: String,
    #[serde(default)]
    pub variant: Option<Variant>,
}

/// One installable, startable Android.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: String,
    pub android: u32,
    pub lineage: String,
    pub variant: Option<Variant>,
    pub images: PathBuf,
    pub data: PathBuf,
    /// The images that were on the machine before omadroid (read-only to us).
    pub system: bool,
}

impl Version {
    /// `Android 17`, or `Android` when the system image's release is unknown.
    pub fn title(&self) -> String {
        if self.android == 0 {
            "Android".to_string()
        } else {
            format!("Android {}", self.android)
        }
    }

    /// `LineageOS 24.0 · GAPPS`, or `your own images` for the system one.
    pub fn subtitle(&self) -> String {
        match (&self.variant, self.lineage.is_empty()) {
            (Some(variant), false) => format!("LineageOS {} · {}", self.lineage, variant.label()),
            (Some(variant), true) => variant.label().to_string(),
            (None, _) => "your own images".to_string(),
        }
    }

    /// The id for a downloaded version. One row per Android release.
    pub fn id(android: u32) -> String {
        format!("android-{android}")
    }

    /// `~/.local/share/omadroid/versions/<id>/data`.
    pub fn data_dir(id: &str) -> PathBuf {
        paths::versions_dir().join(id).join("data")
    }
}

/// Every version that can be started right now, newest first.
pub fn installed(store: &Store) -> Vec<Version> {
    let mut out = Vec::new();

    // The images the machine already had.
    let system_images = store
        .system_images
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(paths::system_images);
    if system_images.join("system.img").is_file() {
        let id = if store.system_android == 0 {
            "system".to_string()
        } else {
            format!("system-{}", store.system_android)
        };
        out.push(Version {
            id: id.clone(),
            android: store.system_android,
            lineage: String::new(),
            variant: None,
            images: system_images,
            data: Version::data_dir(&id),
            system: true,
        });
    }

    // Everything omadroid downloaded.
    if let Ok(entries) = std::fs::read_dir(paths::versions_dir()) {
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() || !is_complete(&dir) {
                continue;
            }
            let Some(id) = dir.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            let meta = read_meta(&dir).unwrap_or(Meta {
                android: android_from_id(&id),
                lineage: String::new(),
                variant: None,
            });
            out.push(Version {
                android: meta.android,
                lineage: meta.lineage,
                variant: meta.variant,
                id: id.clone(),
                images: dir,
                data: Version::data_dir(&id),
                system: false,
            });
        }
    }

    out.sort_by(|a, b| b.android.cmp(&a.android).then(b.system.cmp(&a.system)));
    out
}

/// The images directory Waydroid is pointed at right now.
pub fn active_images() -> Option<PathBuf> {
    crate::engine::active_images()
}

/// The version that matches Waydroid's current `images_path`, if it is ours.
pub fn active(versions: &[Version]) -> Option<&Version> {
    let images = active_images()?;
    versions
        .iter()
        .find(|version| crate::same_path(&images, &version.images))
}

fn is_complete(dir: &Path) -> bool {
    dir.join("system.img").is_file() && dir.join("vendor.img").is_file()
}

/// `android-17` -> `17`.
fn android_from_id(id: &str) -> u32 {
    id.rsplit('-').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// Reads `meta.json`, if there is one.
pub fn read_meta(dir: &Path) -> Option<Meta> {
    let text = std::fs::read_to_string(dir.join("meta.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes `meta.json` atomically.
pub fn write_meta(dir: &Path, meta: &Meta) -> Result<(), String> {
    let text = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    let temp = dir.join("meta.json.tmp");
    std::fs::write(&temp, text).map_err(|e| format!("cannot write meta.json: {e}"))?;
    std::fs::rename(&temp, dir.join("meta.json")).map_err(|e| format!("cannot keep meta.json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        assert_eq!(Version::id(17), "android-17");
        assert_eq!(android_from_id("android-17"), 17);
        assert_eq!(android_from_id("system-13"), 13);
        assert_eq!(android_from_id("system"), 0);
    }

    #[test]
    fn titles_and_subtitles_read_well() {
        let version = Version {
            id: "android-17".into(),
            android: 17,
            lineage: "24.0".into(),
            variant: Some(Variant::Gapps),
            images: PathBuf::from("/tmp/17"),
            data: PathBuf::from("/tmp/17/data"),
            system: false,
        };
        assert_eq!(version.title(), "Android 17");
        assert_eq!(version.subtitle(), "LineageOS 24.0 · GAPPS");
    }

    #[test]
    fn an_unknown_system_image_has_no_number() {
        let version = Version {
            id: "system".into(),
            android: 0,
            lineage: String::new(),
            variant: None,
            images: PathBuf::from("/tmp/system"),
            data: PathBuf::from("/tmp/system/data"),
            system: true,
        };
        assert_eq!(version.title(), "Android");
        assert_eq!(version.subtitle(), "your own images");
    }

    #[test]
    fn meta_round_trips() {
        let dir = std::env::temp_dir().join(format!("omadroid-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let meta = Meta {
            android: 16,
            lineage: "23.2".into(),
            variant: Some(Variant::Vanilla),
        };
        write_meta(&dir, &meta).unwrap();
        assert_eq!(read_meta(&dir), Some(meta));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
