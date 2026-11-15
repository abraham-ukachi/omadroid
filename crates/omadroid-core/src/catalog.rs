//! The Android version catalog.
//!
//! Waydroid's own OTA channel carries one version (LineageOS 20, Android 13).
//! The community builds at `WayDroid-ATV/waydroid-builds` carry Android 11
//! through 17 in x86_64, so the catalog is built from their releases at run
//! time instead of a hardcoded list that would rot: a new release appears in
//! omadroid the same day, without a rebuild.
//!
//! Each release ships two zips that are extracted side by side into one
//! directory: `...-system.zip` (holds `system.img`) and `...-vendor.zip`
//! (holds `vendor.img`).

use std::collections::BTreeMap;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Which image flavor a release ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    /// Google apps and the Play Store.
    Gapps,
    /// The minimal build: no Google services.
    Vanilla,
}

impl Variant {
    pub fn label(self) -> &'static str {
        match self {
            Variant::Gapps => "GAPPS",
            Variant::Vanilla => "VANILLA",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Variant::Gapps => "gapps",
            Variant::Vanilla => "vanilla",
        }
    }
}

/// One downloadable Android version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub android: u32,
    pub lineage: String,
    pub variant: Variant,
    pub tag: String,
    pub system_url: String,
    /// `None` when a single zip carries both images (the older releases).
    pub vendor_url: Option<String>,
    pub system_bytes: u64,
    pub vendor_bytes: u64,
}

impl Release {
    /// The id the UI and the store use: one row per Android version.
    pub fn id(&self) -> String {
        format!("android-{}", self.android)
    }

    /// `LineageOS 24.0 · GAPPS · 1.8 GB`.
    pub fn subtitle(&self) -> String {
        format!(
            "LineageOS {} · {} · {:.1} GB",
            self.lineage,
            self.variant.label(),
            (self.system_bytes + self.vendor_bytes) as f64 / 1e9
        )
    }
}

/// The GitHub API call, as a plain `curl`: no TLS stack to compile, and curl is
/// already required to download the images.
const API: &str =
    "https://api.github.com/repos/WayDroid-ATV/waydroid-builds/releases?per_page=20";

/// Fetches and parses the catalog. Network failures stay failures: the window
/// says so instead of pretending the list is empty.
pub fn fetch() -> Result<Vec<Release>, String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "25",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: omadroid",
            API,
        ])
        .output()
        .map_err(|e| format!("cannot run curl: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "GitHub answered {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// The newest release known when this version was written, for when GitHub is
/// unreachable.
pub fn fallback() -> Vec<Release> {
    vec![Release {
        android: 17,
        lineage: "24.0".into(),
        variant: Variant::Gapps,
        tag: "20261017".into(),
        system_url: "https://github.com/WayDroid-ATV/waydroid-builds/releases/download/20261017/lineage-24.0-20261017-GAPPS-waydroid_x86_64-system.zip".into(),
        vendor_url: Some("https://github.com/WayDroid-ATV/waydroid-builds/releases/download/20261017/lineage-24.0-20261017-MAINLINE-waydroid_x86_64-vendor.zip".into()),
        system_bytes: 1_493_410_599,
        vendor_bytes: 260_970_514,
    }]
}

/// Turns the GitHub releases JSON into one entry per Android version, newest
/// first. A version with a GAPPS build wins over its VANILLA build.
pub fn parse(json: &str) -> Result<Vec<Release>, String> {
    let root: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("GitHub sent unreadable JSON: {e}"))?;
    let releases = root
        .as_array()
        .ok_or_else(|| "GitHub sent no release list".to_string())?;

    let mut by_android: BTreeMap<u32, Release> = BTreeMap::new();

    for release in releases {
        let name = release["name"].as_str().unwrap_or_default();
        let Some(android) = android_of(name) else {
            continue;
        };
        let empty = Vec::new();
        let assets = release["assets"].as_array().unwrap_or(&empty);

        // Two shapes ship in that repo: the new releases split the images into
        // a system zip and a vendor zip, the older ones pack both into a single
        // `-waydroid_x86_64.zip`. A release can also ship a VANILLA and a GAPPS
        // system image; the GAPPS one is the phone people expect, so it wins.
        let system = assets
            .iter()
            .find(|a| asset_name(a).ends_with("-system.zip") && asset_name(a).contains("-GAPPS-"))
            .or_else(|| {
                assets
                    .iter()
                    .find(|a| asset_name(a).ends_with("-system.zip"))
            })
            .or_else(|| {
                assets
                    .iter()
                    .find(|a| single_zip(asset_name(a)))
            });
        let Some(system) = system else {
            continue;
        };

        // Only LineageOS builds: /e/OS and vendor-only uploads are not phones
        // this app can offer as an Android version.
        let system_name = asset_name(system);
        if !system_name.starts_with("lineage-") {
            continue;
        }

        let vendor = assets
            .iter()
            .find(|a| asset_name(a).ends_with("-vendor.zip"));

        let variant = if system_name.contains("-GAPPS-") {
            Variant::Gapps
        } else {
            Variant::Vanilla
        };

        let candidate = Release {
            android,
            lineage: lineage_of(system_name),
            variant,
            tag: release["tag_name"].as_str().unwrap_or_default().to_string(),
            system_url: system["browser_download_url"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            vendor_url: vendor
                .and_then(|a| a["browser_download_url"].as_str())
                .map(str::to_string),
            system_bytes: system["size"].as_u64().unwrap_or(0),
            vendor_bytes: vendor
                .and_then(|a| a["size"].as_u64())
                .unwrap_or(0),
        };

        if candidate.system_url.is_empty() {
            continue;
        }

        // Releases arrive newest first, so the first entry for an Android
        // version wins, unless a GAPPS build turns up behind a VANILLA one.
        match by_android.get(&android) {
            None => {
                by_android.insert(android, candidate);
            }
            Some(existing)
                if existing.variant == Variant::Vanilla && candidate.variant == Variant::Gapps =>
            {
                by_android.insert(android, candidate);
            }
            Some(_) => {}
        }
    }

    let mut out: Vec<Release> = by_android.into_values().collect();
    out.sort_by_key(|release| std::cmp::Reverse(release.android));
    if out.is_empty() {
        return Err("no x86_64 Android versions in that release list".into());
    }
    Ok(out)
}

fn asset_name(asset: &serde_json::Value) -> &str {
    asset["name"].as_str().unwrap_or_default()
}

/// `lineage-22.2-20260224-UNOFFICIAL-waydroid_x86_64.zip`: both images inside.
fn single_zip(name: &str) -> bool {
    (name.ends_with("-waydroid_x86_64.zip") || name.ends_with("-waydroid.zip"))
        && !name.contains("-vendor")
}

/// `20261017 (LineageOS 24.0, Android 17 QPR0)` -> `17`.
fn android_of(name: &str) -> Option<u32> {
    let at = name.find("Android")?;
    let digits: String = name[at + "Android".len()..]
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// `lineage-24.0-20261017-GAPPS-waydroid_x86_64-system.zip` -> `24.0`.
fn lineage_of(asset: &str) -> String {
    asset
        .strip_prefix("lineage-")
        .and_then(|rest| rest.split('-').next())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {
        "tag_name": "20261017",
        "name": "20261017 (LineageOS 24.0, Android 17 QPR0)",
        "assets": [
          {"name": "lineage-24.0-20261017-VANILLA-waydroid_x86_64-system.zip", "size": 1167848338, "browser_download_url": "https://example.test/17-vanilla-system.zip"},
          {"name": "lineage-24.0-20261017-GAPPS-waydroid_x86_64-system.zip", "size": 1493410599, "browser_download_url": "https://example.test/17-gapps-system.zip"},
          {"name": "lineage-24.0-20261017-MAINLINE-waydroid_x86_64-vendor.zip", "size": 260970514, "browser_download_url": "https://example.test/17-vendor.zip"}
        ]
      },
      {
        "tag_name": "20260224",
        "name": "20260224 (LineageOS 22.2, Android 15)",
        "assets": [
          {"name": "lineage-22.2-20260224-UNOFFICIAL-waydroid_x86_64.zip", "size": 2900000000, "browser_download_url": "https://example.test/15.zip"}
        ]
      },
      {
        "tag_name": "20260125",
        "name": "20260125 (LineageOS 21.0, Android 14)",
        "assets": [
          {"name": "lineage-21.0-20260125-GAPPS-waydroid_x86_64-system.zip", "size": 1400000000, "browser_download_url": "https://example.test/14-gapps-system.zip"},
          {"name": "lineage-21.0-20260125-MAINLINE-waydroid_x86_64-vendor.zip", "size": 260000000, "browser_download_url": "https://example.test/14-vendor.zip"}
        ]
      },
      {
        "tag_name": "broken",
        "name": "a release without assets",
        "assets": []
      }
    ]"#;

    #[test]
    fn keeps_one_row_per_android_version_newest_first() {
        let releases = parse(SAMPLE).unwrap();
        assert_eq!(releases.len(), 3);
        assert_eq!(releases[0].android, 17);
        assert_eq!(releases[1].android, 15);
        assert_eq!(releases[2].android, 14);
    }

    #[test]
    fn an_old_release_ships_both_images_in_one_zip() {
        let releases = parse(SAMPLE).unwrap();
        let fifteen = &releases[1];
        assert_eq!(fifteen.vendor_url, None);
        assert_eq!(fifteen.vendor_bytes, 0);
        assert_eq!(fifteen.system_url, "https://example.test/15.zip");
        assert_eq!(fifteen.lineage, "22.2");
    }

    #[test]
    fn prefers_gapps_and_reads_lineage() {
        let releases = parse(SAMPLE).unwrap();
        assert_eq!(releases[0].variant, Variant::Gapps);
        assert_eq!(releases[0].vendor_url.as_deref(), Some("https://example.test/17-vendor.zip"));
        assert_eq!(releases[0].lineage, "24.0");
        assert_eq!(releases[0].tag, "20261017");
        assert_eq!(releases[0].system_bytes, 1_493_410_599);
    }

    #[test]
    fn reads_the_android_version_out_of_the_title() {
        assert_eq!(android_of("20260717 (LineageOS 23.2, Android 16 QPR2)"), Some(16));
        assert_eq!(android_of("20240801 (Android 11 with VA-API working)"), Some(11));
        assert_eq!(android_of("no version here"), None);
    }

    #[test]
    fn an_empty_list_is_an_error() {
        assert!(parse("[]").is_err());
    }

    #[test]
    fn ids_and_subtitles_are_single_rows() {
        let release = parse(SAMPLE).unwrap().remove(0);
        assert_eq!(release.id(), "android-17");
        assert_eq!(release.subtitle(), "LineageOS 24.0 · GAPPS · 1.8 GB");
    }
}
