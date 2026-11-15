//! Every path omadroid reads or writes, in one place.
//!
//! Waydroid owns `/var/lib/waydroid` (root) and `~/.local/share/waydroid`
//! (yours). omadroid keeps its own state in the XDG dirs and never deletes
//! anything it did not create.

use std::path::PathBuf;

/// The user's home, or `/` when the environment is broken.
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}

/// `~/.config/omadroid`.
pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("omadroid")
}

/// `~/.local/share/omadroid`.
pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("omadroid")
}

/// `~/.local/share/omadroid/versions`: one directory per Android version.
pub fn versions_dir() -> PathBuf {
    data_dir().join("versions")
}

/// `~/.config/omadroid/omadroid.json`: the little that has to be remembered.
pub fn store_file() -> PathBuf {
    config_dir().join("omadroid.json")
}

/// `~/.local/share/waydroid`: Waydroid's user state.
pub fn waydroid_state() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join("waydroid")
}

/// `~/.local/share/waydroid/data`: the Android `/data` that Waydroid binds in.
/// omadroid points this symlink at the selected version's own data directory.
pub fn waydroid_data() -> PathBuf {
    waydroid_state().join("data")
}

/// `/var/lib/waydroid`: root-owned work directory.
pub fn waydroid_work() -> PathBuf {
    PathBuf::from("/var/lib/waydroid")
}

/// `/var/lib/waydroid/waydroid.cfg`: holds the `images_path` we swap.
pub fn waydroid_config() -> PathBuf {
    waydroid_work().join("waydroid.cfg")
}

/// `/usr/share/waydroid-extra/images`: the images the machine already had.
pub fn system_images() -> PathBuf {
    PathBuf::from("/usr/share/waydroid-extra/images")
}

/// `~/.local/state/omarchy/current/theme/colors.toml`: the live accent.
pub fn omarchy_colors() -> PathBuf {
    home().join(".local/state/omarchy/current/theme/colors.toml")
}
