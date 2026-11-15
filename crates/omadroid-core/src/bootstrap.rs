//! Installing Waydroid itself.
//!
//! omadroid is not an emulator, so "installing Android" means installing the
//! Waydroid container and an image, both of which Arch already packages. On a
//! machine without them the window shows the setup panel and the button runs
//! exactly these steps, in this order:
//!
//! 1. `pacman -S --needed waydroid` (it is in Arch's `extra`)
//! 2. `systemctl enable --now waydroid-container`
//! 3. `waydroid init -f -s GAPPS`, which downloads the system and vendor images
//!    itself (about 1.6 GB) unless a `waydroid-image` package already put them
//!    in `/usr/share/waydroid-extra/images`.
//!
//! The first step needs root. `sudo -n` is tried first (a passwordless or
//! already-cached sudo works), then `pkexec`, which is the one a desktop can
//! actually show a password prompt for. Neither is a secret: when both fail the
//! panel says which command to run by hand.

use crate::engine;
use crate::paths;

/// What the machine is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deps {
    /// The `waydroid` program is on the PATH.
    pub waydroid: bool,
    /// A system image is unpacked somewhere Waydroid looks.
    pub image: bool,
    /// `/dev/binderfs` (or `/dev/binder`) is mounted: the kernel can do this.
    pub binder: bool,
    /// `/var/lib/waydroid/waydroid.cfg` exists, so `waydroid init` has run.
    pub initialized: bool,
}

impl Deps {
    /// Enough to boot something.
    pub fn ready(&self) -> bool {
        self.waydroid && self.initialized
    }

    /// One line for the setup panel, naming every piece that is missing.
    pub fn describe(&self) -> String {
        let mut missing = Vec::new();
        if !self.waydroid {
            missing.push("waydroid is not installed");
        }
        if self.waydroid && !self.image {
            missing.push("no android image");
        }
        if self.waydroid && !self.initialized {
            missing.push("the container is not initialised");
        }
        if !self.binder {
            missing.push("the kernel has no binder");
        }
        if missing.is_empty() {
            "ready".to_string()
        } else {
            missing.join(" · ")
        }
    }
}

/// Looks at the machine, not at a config file.
pub fn detect() -> Deps {
    Deps {
        waydroid: which("waydroid"),
        image: paths::system_images().join("system.img").is_file()
            || paths::waydroid_work().join("system.img").is_file(),
        binder: std::path::Path::new("/dev/binderfs").exists()
            || std::path::Path::new("/dev/binder").exists(),
        initialized: paths::waydroid_config().is_file(),
    }
}

/// `command -v`, because a PATH lookup is what "installed" means here.
fn which(program: &str) -> bool {
    let Ok(out) = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program}"))
        .output()
    else {
        return false;
    };
    out.status.success()
}

/// The package to ask for: Waydroid is in `extra`, and it fetches its own
/// images. The image package, when the machine has one, is only a cache.
pub fn packages() -> Vec<&'static str> {
    vec!["waydroid"]
}

/// Runs a command as root: `sudo -n` first, then `pkexec`.
fn privileged(cmd: &str, args: &[&str]) -> Result<String, String> {
    let mut line = vec![cmd];
    line.extend_from_slice(args);

    let mut sudo = vec!["-n"];
    sudo.extend_from_slice(&line);
    if let Ok(out) = engine::run("sudo", &sudo) {
        return Ok(out);
    }

    match engine::run("pkexec", &line) {
        Ok(out) => Ok(out),
        Err(error) => Err(format!(
            "needs root, and neither sudo nor pkexec would take it ({error})"
        )),
    }
}

/// Installs Waydroid and an image, enables the container, initialises it.
/// Every step is reported through `progress` as the sentence the window shows.
pub fn install(progress: &mut dyn FnMut(&str)) -> Result<(), String> {
    let packages = packages();
    progress(&format!("Installing {}", packages.join(" and ")));

    let mut args = vec!["-S", "--needed", "--noconfirm"];
    args.extend_from_slice(&packages);
    privileged("pacman", &args).map_err(|error| format!("pacman failed: {error}"))?;

    progress("Enabling the waydroid container");
    privileged("systemctl", &["enable", "--now", "waydroid-container"])
        .map_err(|error| format!("systemctl failed: {error}"))?;

    // GAPPS is the image with Play Store; a build without that flag still
    // initialises, so a refusal is retried vanilla rather than reported.
    progress("Downloading Android (about 1.6 GB)");
    if let Err(error) = privileged("waydroid", &["init", "-f", "-s", "GAPPS"]) {
        progress("GAPPS was refused, initialising vanilla");
        privileged("waydroid", &["init", "-f"])
            .map_err(|_| format!("waydroid init failed: {error}"))?;
    }

    let deps = detect();
    if !deps.ready() {
        return Err(format!("still not ready: {}", deps.describe()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_without_anything_lists_everything() {
        let deps = Deps {
            waydroid: false,
            image: false,
            binder: false,
            initialized: false,
        };
        assert!(!deps.ready());
        let said = deps.describe();
        assert!(said.contains("waydroid is not installed"));
        assert!(said.contains("no binder"));
    }

    #[test]
    fn a_ready_machine_says_ready() {
        let deps = Deps {
            waydroid: true,
            image: true,
            binder: true,
            initialized: true,
        };
        assert!(deps.ready());
        assert_eq!(deps.describe(), "ready");
    }

    #[test]
    fn a_container_without_an_init_is_not_ready() {
        let deps = Deps {
            waydroid: true,
            image: true,
            binder: true,
            initialized: false,
        };
        assert!(!deps.ready());
        assert!(deps.describe().contains("not initialised"));
    }

    #[test]
    fn the_container_package_is_the_only_one_needed() {
        assert_eq!(packages(), vec!["waydroid"]);
    }
}
