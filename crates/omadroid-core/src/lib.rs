//! # omadroid-core
//!
//! Everything omadroid does that is not drawing: talking to Waydroid, reading
//! the Android version catalog, downloading image sets and reading the Omarchy
//! theme. The app crate only wires these to the UI.
//!
//! The one idea worth knowing: **omadroid does not emulate Android.** Waydroid
//! boots the real Android userspace in the kernel you already run, so the phone
//! is as fast as the machine. What omadroid adds is the part Waydroid leaves to
//! the terminal: one Android version per directory, and a four-second swap
//! between them.

pub mod bootstrap;
pub mod catalog;
pub mod config;
pub mod desktop;
pub mod engine;
pub mod install;
pub mod paths;
pub mod theme;
pub mod versions;

pub use bootstrap::Deps;
pub use catalog::{Release, Variant};
pub use config::Store;
pub use desktop::Rect;
pub use engine::Status;
pub use install::Progress;
pub use theme::{Rgb, Theme};
pub use versions::Version;

/// Two paths point at the same place (symlinks and `..` resolved).
pub fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
