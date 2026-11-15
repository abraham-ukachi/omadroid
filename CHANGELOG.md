# Changelog

All notable changes to this project will be documented in this file. Versions follow [SemVer](https://semver.org), see [CONTRIBUTING.md](./CONTRIBUTING.md#git-commit-messages) for the commit guidelines.

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
