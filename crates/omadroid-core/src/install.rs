//! Downloading an Android version.
//!
//! Each release is two zips (`system`, `vendor`) of well over a gigabyte, so
//! the work is done by `curl` — with `-C -` it resumes, and the progress is read
//! off the file size instead of parsing curl's output. The zips are removed
//! once their `.img` is extracted: two versions plus their data fit in ~8 GB.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use crate::catalog::{Release, Variant};
use crate::engine;
use crate::paths;
use crate::versions::{self, Meta, Version};

/// Which half of the release is being worked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    System,
    Vendor,
}

impl Part {
    pub fn label(self) -> &'static str {
        match self {
            Part::System => "system",
            Part::Vendor => "vendor",
        }
    }
}

/// Progress, as the window shows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    Downloading { part: Part, done: u64, total: u64 },
    Extracting { part: Part },
}

impl Progress {
    /// 0.0 to 1.0 over the whole install: the vendor image is smaller, so the
    /// big system download gets most of the bar.
    pub fn fraction(self) -> f32 {
        match self {
            Progress::Downloading { part, done, total } => {
                let ratio = if total == 0 {
                    0.0
                } else {
                    (done as f64 / total as f64).min(1.0) as f32
                };
                match part {
                    Part::System => ratio * 0.72,
                    Part::Vendor => 0.72 + ratio * 0.20,
                }
            }
            Progress::Extracting { part } => match part {
                Part::System => 0.94,
                Part::Vendor => 0.97,
            },
        }
    }

    /// `Downloading the system image · 812 MB of 1.4 GB`.
    pub fn describe(self) -> String {
        match self {
            Progress::Downloading { part, done, total } => format!(
                "Downloading the {} image · {} of {}",
                part.label(),
                bytes(done),
                bytes(total)
            ),
            Progress::Extracting { part } => format!("Extracting the {} image", part.label()),
        }
    }
}

/// `1.4 GB`, `812 MB`.
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit < UNITS.len() - 1 {
        size /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// Downloads and unpacks a release. Returns the version it installed.
pub fn install(
    release: &Release,
    progress: &mut dyn FnMut(Progress),
) -> Result<Version, String> {
    let id = release.id();
    let dir = paths::versions_dir().join(&id);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    match release.vendor_url.as_deref() {
        // The new releases: two zips, one image each.
        Some(vendor_url) => {
            fetch_image(&dir, &release.system_url, Part::System, progress)?;
            fetch_image(&dir, vendor_url, Part::Vendor, progress)?;
        }
        // The older releases: a single zip holding both images.
        None => {
            let zip = dir.join("images.zip");
            download(&release.system_url, &zip, &mut |done, total| {
                progress(Progress::Downloading {
                    part: Part::System,
                    done,
                    total,
                });
            })?;
            progress(Progress::Extracting {
                part: Part::System,
            });
            let result = extract(&zip, &dir, &["system.img", "vendor.img"]);
            let _ = fs::remove_file(&zip);
            result?;
        }
    }

    let meta = Meta {
        android: release.android,
        lineage: release.lineage.clone(),
        variant: Some(release.variant),
    };
    versions::write_meta(&dir, &meta)?;

    Ok(Version {
        id,
        android: release.android,
        lineage: release.lineage.clone(),
        variant: Some(release.variant),
        images: dir.clone(),
        data: Version::data_dir(&release.id()),
        system: false,
    })
}

/// Removes an installed version and its data. The active one is refused.
pub fn remove(version: &Version) -> Result<(), String> {
    if version.system {
        return Err("these are the images the machine already had".into());
    }
    if let Some(active) = versions::active(std::slice::from_ref(version)) {
        if crate::same_path(&active.images, &version.images) {
            return Err("stop this Android version first".into());
        }
    }
    fs::remove_dir_all(&version.images)
        .map_err(|e| format!("cannot remove {}: {e}", version.images.display()))
}

/// The download size of a release, from the server, so the bar has a total
/// before the first byte arrives.
fn remote_size(url: &str) -> u64 {
    let Ok(out) = Command::new("curl")
        .args(["-fsIL", "--max-time", "20", url])
        .output()
    else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.to_ascii_lowercase();
            line.strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .next_back()
        .unwrap_or(0)
}

/// Downloads `url` to `dest`, resuming a partial file. Progress is the file
/// size, polled four times a second.
pub fn download(
    url: &str,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let total = remote_size(url);
    let part = PathBuf::from(format!("{}.part", dest.display()));

    let mut child = Command::new("curl")
        .args(["-fL", "-C", "-", "--silent", "--show-error", "-o"])
        .arg(&part)
        .arg(url)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run curl: {e}"))?;

    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(format!(
                    "the download failed (curl {status}); run it again to resume"
                ));
            }
            break;
        }
        let done = fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
        progress(done, total);
        sleep(Duration::from_millis(250));
    }

    let done = fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
    progress(done, if total == 0 { done } else { total });
    fs::rename(&part, dest).map_err(|e| format!("cannot keep {}: {e}", dest.display()))
}

/// Downloads one zip and unpacks the image it carries.
fn fetch_image(
    dir: &Path,
    url: &str,
    part: Part,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    let image = match part {
        Part::System => "system.img",
        Part::Vendor => "vendor.img",
    };
    let zip = dir.join(format!("{}.zip", part.label()));

    download(url, &zip, &mut |done, total| {
        progress(Progress::Downloading { part, done, total });
    })?;

    progress(Progress::Extracting { part });
    let result = extract(&zip, dir, &[image]);
    let _ = fs::remove_file(&zip);
    result
}

/// Unpacks `.img` files out of a release zip, dropping the directory levels.
fn extract(zip: &Path, dir: &Path, images: &[&str]) -> Result<(), String> {
    let zip_path = zip.to_string_lossy().to_string();
    let dir_path = dir.to_string_lossy().to_string();

    let mut args: Vec<&str> = vec!["-o", "-j", &zip_path];
    args.extend_from_slice(images);
    args.push("-d");
    args.push(&dir_path);

    engine::run("unzip", &args).map_err(|e| {
        format!(
            "{} does not hold {} ({e})",
            zip.display(),
            images.join(" and ")
        )
    })?;

    for image in images {
        if !dir.join(image).is_file() {
            return Err(format!("{} has no {image} in it", zip.display()));
        }
    }
    Ok(())
}

/// Reads `meta.json` back out, or names a version from its id.
pub fn variant_of(version: &Version) -> Variant {
    version.variant.unwrap_or(Variant::Vanilla)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_a_download() {
        assert_eq!(bytes(812), "812 B");
        assert_eq!(bytes(812_000_000), "812.0 MB");
        assert_eq!(bytes(1_493_410_599), "1.5 GB");
    }

    #[test]
    fn the_bar_fills_once() {
        let system = Progress::Downloading {
            part: Part::System,
            done: 500,
            total: 1000,
        };
        let vendor = Progress::Downloading {
            part: Part::Vendor,
            done: 500,
            total: 1000,
        };
        assert!(system.fraction() > 0.0 && system.fraction() < 0.72);
        assert!(vendor.fraction() > 0.72);
        assert_eq!(
            Progress::Extracting {
                part: Part::Vendor
            }
            .fraction(),
            0.97
        );
    }

    #[test]
    fn progress_says_what_it_is_doing() {
        let text = Progress::Downloading {
            part: Part::System,
            done: 812_000_000,
            total: 1_493_410_599,
        }
        .describe();
        assert!(text.starts_with("Downloading the system image · 812.0 MB of 1.5 GB"));
    }

    #[test]
    fn a_third_of_a_file_is_not_a_whole_one() {
        let progress = Progress::Downloading {
            part: Part::System,
            done: 1,
            total: 0,
        };
        assert_eq!(progress.fraction(), 0.0);
    }
}
