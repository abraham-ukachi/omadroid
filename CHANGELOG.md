# Changelog

All notable changes to this project will be documented in this file. Versions follow [SemVer](https://semver.org), see [CONTRIBUTING.md](./CONTRIBUTING.md#git-commit-messages) for the commit guidelines.

## [0.3.1] - 2026-11-15

### Fixed
- :bug: The phone keeps the keyboard after it is moved: on Hyprland 0.56 `hl.dsp.window.focus` does not exist, and the failed call looked like success. So the phone counted as the focused window and never parked itself.
- :bug: A parked phone is unpinned, so it cannot float over the workspace the user moved to, and the keyboard is handed back to the window that had it.
- :bug: The phone is placed as soon as its window exists, instead of floating at its own 1280x720 while Waydroid's status catches up.

## [0.3.0] - 2026-11-15

### Added
- :sparkles: Fullscreen button beside settings, and `f`: the phone takes the monitor, `esc` or the button puts it back.
- :eye: The phone parks itself on `special:omadroid` when another window takes the keyboard, and is placed again when omadroid does.

### Known limits
- :iphone: The panel still scales Waydroid's 1920x1080 framebuffer. `wm size` moves Android's display but leaves Waydroid's window black at every size tried, so it is not used.
- :keyboard: Waydroid's window asks for focus itself, which can keep the park-on-blur from triggering.

## [0.2.0] - 2026-11-15

### Added
- :sparkles: Waydroid installs itself from the window on first run: `pacman`, the container service, then `waydroid init` for the images.
- :iphone: Android's window is placed on the phone panel by Hyprland and follows it, pinned above the tiling layout.
- :gear: Settings pane in the right column: versions, keys, about. The app bar carries the version.

### Fixed
- :bug: The window was sized from physical pixels, so on a scaled display it hung off the screen.

## [0.1.0] - 2026-11-15

### Added
- :sparkles: One window listing every Android version: the ones installed on the machine, then the ones that can be downloaded, newest first.
- :sparkles: A live catalog built from `WayDroid-ATV/waydroid-builds` releases (Android 11 to 17, x86_64), so a new release appears without a rebuild. GAPPS wins over VANILLA when a release ships both.
- :sparkles: Resumable downloads through `curl -C -`, with a progress bar and the size in the window; the zips are removed once their `.img` is unpacked.
- :sparkles: A four-second version switch: Waydroid's `images_path` is rewritten, the `data` symlink follows the version, the previous overlay is cleared, the container restarts. Nothing is copied.
- :sparkles: The machine's existing Waydroid images are adopted as a version of their own, and their data directory is moved (not deleted) the first time another version starts.
- :sparkles: Omarchy theme read live from `colors.toml`, corners from Hyprland's `decoration:rounding`, iA Writer Mono, keyboard-first with `j`/`k`, Space, `d`, `x`, `o`, `r`, `R`, `?`, `q`.
- :sparkles: Installed as an Arch package with a launcher entry and icon (`bin/install`).

### Notes
- One Android version is live at a time: Waydroid has a single container.
- Android 14 to 17 come from unofficial community builds.
