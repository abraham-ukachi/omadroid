//! Talking to Waydroid.
//!
//! `waydroid` reads `images_path` out of `/var/lib/waydroid/waydroid.cfg` when
//! the container starts and loop-mounts `system.img` / `vendor.img` from it.
//! Android's `/data` is bound from `~/.local/share/waydroid/data`.
//!
//! So a version switch is: stop, point the `data` symlink at the version's own
//! data, clear the system/vendor overlay, write one line of the config, start.
//! No image is ever copied, which is why the swap takes seconds and not a
//! reboot.

use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use crate::paths;
use crate::versions::Version;

/// What `waydroid status` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub session: String,
    pub container: String,
    /// `MAINLINE` or `VANILLA`, as Waydroid reports it.
    pub vendor: String,
}

impl Status {
    /// The container is up (a frozen container is still a running one).
    pub fn running(&self) -> bool {
        self.container == "RUNNING" || self.container == "FROZEN"
    }

    /// The session manager is up, so `show-full-ui` will find a display.
    pub fn session_running(&self) -> bool {
        self.session == "RUNNING"
    }
}

impl Default for Status {
    fn default() -> Self {
        Status {
            session: "STOPPED".into(),
            container: "STOPPED".into(),
            vendor: String::new(),
        }
    }
}

/// Runs a command and returns its stdout, with stderr as the error message.
pub fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run {cmd}: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("{cmd} exited with {}", out.status)
        } else {
            stderr.to_string()
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Runs a command through `sudo`.
pub fn run_sudo(cmd: &str, args: &[&str]) -> Result<String, String> {
    let mut all = Vec::with_capacity(args.len() + 1);
    all.push(cmd);
    all.extend_from_slice(args);
    run("sudo", &all)
}

/// `waydroid status`, parsed. A Waydroid that will not answer reads as stopped.
pub fn status() -> Status {
    let Ok(out) = run("waydroid", &["status"]) else {
        return Status::default();
    };
    let mut status = Status::default();
    for line in out.lines() {
        if let Some(value) = line.strip_prefix("Session:") {
            status.session = value.trim().to_string();
        }
        if let Some(value) = line.strip_prefix("Container:") {
            status.container = value.trim().to_string();
        }
        if let Some(value) = line.strip_prefix("Vendor type:") {
            status.vendor = value.trim().to_string();
        }
    }
    status
}

/// `waydroid --version` (the release, not the image): `1.6.3`.
pub fn version() -> Option<String> {
    let out = run("waydroid", &["--version"]).ok()?;
    let version = out.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// The `images_path` in a `waydroid.cfg`.
pub fn images_path(cfg: &str) -> Option<std::path::PathBuf> {
    cfg.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("images_path")?;
        let value = rest.trim_start().strip_prefix('=')?;
        Some(std::path::PathBuf::from(value.trim()))
    })
}

/// A `waydroid.cfg` with `images_path` set, and Waydroid's own updater off (it
/// would try to fetch an OTA over images omadroid manages).
pub fn rewrite_images_path(cfg: &str, dir: &Path) -> String {
    let mut out = String::new();
    let mut written = false;
    for line in cfg.lines() {
        if line.trim_start().starts_with("images_path") {
            out.push_str(&format!("images_path = {}\n", dir.display()));
            written = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !written {
        out.push_str(&format!("images_path = {}\n", dir.display()));
    }
    if !out.contains("waydroid.updater.disabled") {
        if !out.contains("[properties]") {
            out.push_str("\n[properties]\n");
        }
        out.push_str("waydroid.updater.disabled = true\n");
    }
    out
}

/// The images directory Waydroid is pointed at right now.
pub fn active_images() -> Option<std::path::PathBuf> {
    let cfg = paths::waydroid_config();
    let text = run_sudo("cat", &[cfg.to_string_lossy().as_ref()]).ok()?;
    images_path(&text)
}

/// The Android release of the running container, if it answers.
pub fn android_release() -> Option<u32> {
    let out = run("waydroid", &["prop", "get", "ro.build.version.release"]).ok()?;
    out.trim().parse().ok()
}

/// Writes `images_path` into the config, through `sudo tee`.
pub fn write_images_path(dir: &Path) -> Result<(), String> {
    let path = paths::waydroid_config();
    let cfg = run_sudo("cat", &[path.to_string_lossy().as_ref()])?;
    let next = rewrite_images_path(&cfg, dir);
    if next == cfg {
        return Ok(());
    }

    let mut child = Command::new("sudo")
        .arg("tee")
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run sudo tee: {e}"))?;
    child
        .stdin
        .as_mut()
        .ok_or("sudo tee took no input")?
        .write_all(next.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

/// Points Waydroid's `/data` at a version's own data directory.
///
/// The first time this runs on a machine that already used Waydroid, the
/// existing data directory is *moved* (a rename, on the same disk) into
/// `versions/<id>/data` and linked back, so nothing the user installed is lost.
pub fn point_data(target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| format!("cannot create {}: {e}", target.display()))?;

    let link = paths::waydroid_data();
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }

    match fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if crate::same_path(&link, target) {
                return Ok(());
            }
            fs::remove_file(&link).map_err(|e| format!("cannot replace {}: {e}", link.display()))?;
        }
        Ok(meta) if meta.is_dir() => {
            let empty = fs::read_dir(target)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false);
            if !empty {
                return Err(format!(
                    "{} already holds data omadroid does not manage",
                    link.display()
                ));
            }
            fs::remove_dir(target).map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
            fs::rename(&link, target)
                .map_err(|e| format!("cannot move {} aside: {e}", link.display()))?;
        }
        Ok(_) => {
            fs::remove_file(&link).map_err(|e| format!("cannot replace {}: {e}", link.display()))?;
        }
        Err(_) => {}
    }

    symlink(target, &link).map_err(|e| format!("cannot link {}: {e}", link.display()))
}

/// Clears the writable system/vendor overlay. It belongs to the previous
/// version: carrying it over is how a swap ends in a boot loop.
pub fn reset_overlay() -> Result<(), String> {
    let work = paths::waydroid_work();
    for part in ["system", "vendor"] {
        let dir = work.join("overlay_rw").join(part);
        let dir = dir.to_string_lossy().to_string();
        let _ = run_sudo("rm", &["-rf", &dir]);
        run_sudo("mkdir", &["-p", &dir])?;
    }
    Ok(())
}

/// Starts Waydroid's session manager in the background, and waits for it.
pub fn ensure_session() -> Result<(), String> {
    run_sudo("systemctl", &["start", "waydroid-container"])?;
    if status().session_running() {
        return Ok(());
    }

    spawn_detached("waydroid", &["session", "start"])?;
    for _ in 0..160 {
        if status().session_running() {
            return Ok(());
        }
        sleep(Duration::from_millis(250));
    }
    Err("the Waydroid session did not start (see `waydroid log`)".into())
}

/// Stops the session and the container, and waits for both.
pub fn stop() -> Result<(), String> {
    let _ = run("waydroid", &["session", "stop"]);
    for _ in 0..80 {
        if !status().session_running() {
            break;
        }
        sleep(Duration::from_millis(250));
    }

    if status().container != "STOPPED" {
        let _ = run_sudo("waydroid", &["container", "stop"]);
        for _ in 0..80 {
            if status().container == "STOPPED" {
                break;
            }
            sleep(Duration::from_millis(250));
        }
    }
    Ok(())
}

/// Switches Waydroid to a version and boots it. About four seconds.
pub fn activate(version: &Version) -> Result<(), String> {
    stop()?;
    point_data(&version.data)?;
    reset_overlay()?;
    write_images_path(&version.images)?;
    ensure_session()
}

/// Opens the phone window.
pub fn show() -> Result<(), String> {
    spawn_detached("waydroid", &["show-full-ui"])
}

/// Starts a program that outlives us: its own session, no shared stdio.
fn spawn_detached(cmd: &str, args: &[&str]) -> Result<(), String> {
    Command::new("setsid")
        .arg(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {cmd}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "[waydroid]\narch = x86_64\nmount_overlays = True\nimages_path = /usr/share/waydroid-extra/images\n\n[properties]\n";

    #[test]
    fn reads_the_images_path() {
        assert_eq!(
            images_path(CFG),
            Some(std::path::PathBuf::from("/usr/share/waydroid-extra/images"))
        );
        assert_eq!(images_path("[waydroid]\narch = x86_64\n"), None);
    }

    #[test]
    fn swaps_the_images_path_and_keeps_the_rest() {
        let next = rewrite_images_path(CFG, Path::new("/home/me/omadroid/versions/android-17"));
        assert_eq!(
            images_path(&next),
            Some(std::path::PathBuf::from(
                "/home/me/omadroid/versions/android-17"
            ))
        );
        assert!(next.contains("arch = x86_64"));
        assert!(next.contains("[properties]"));
        assert!(next.contains("waydroid.updater.disabled = true"));
    }

    #[test]
    fn disables_the_updater_only_once() {
        let once = rewrite_images_path(CFG, Path::new("/tmp/a"));
        let twice = rewrite_images_path(&once, Path::new("/tmp/b"));
        assert_eq!(twice.matches("waydroid.updater.disabled").count(), 1);
        assert_eq!(images_path(&twice), Some(std::path::PathBuf::from("/tmp/b")));
    }

    #[test]
    fn adds_the_key_when_there_is_no_properties_section() {
        let next = rewrite_images_path("[waydroid]\narch = x86_64\n", Path::new("/tmp/v"));
        assert!(next.contains("[properties]"));
        assert!(next.contains("waydroid.updater.disabled = true"));
    }

    #[test]
    fn a_stopped_waydroid_reads_as_stopped() {
        assert!(!Status::default().running());
        assert!(Status {
            session: "RUNNING".into(),
            container: "FROZEN".into(),
            ..Status::default()
        }
        .running());
    }
}
