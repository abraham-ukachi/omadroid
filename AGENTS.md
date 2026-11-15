# AGENTS.md — omadroid 🤖

> Read this first if you are an AI agent (or a new human) picking up this repo.

**omadroid** is a Rust + Slint window that runs Android versions through Waydroid with Omarchy's look & feel.
The long-term goal: **pick any Android version, press space, be on the home screen.**

## Golden rules

1. 🦀 **Core first.** Waydroid control, the catalog, downloads and the theme live in `crates/omadroid-core` with unit tests. The app crate only wires UI to core.
2. 🧪 `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` at 0 before every push (`bin/test`).
3. 🚫 **Never copy an image to switch versions.** Waydroid reads `images_path` at container start: rewrite the config, move the symlink, restart. A `cp` here is a bug.
4. 🎨 **Omacalc is the design reference.** Two colors, a few mixes, iA Writer Mono, nothing else. No new colors, no icons with fills.
5. 💾 **Never delete what omadroid did not create.** The images at `/usr/share/waydroid-extra/images` are the user's; they are read-only to us.
6. 📴 Never block the UI thread: container starts and downloads go through the worker in `app/src/lib.rs`.
7. ✍️ Gitmoji commits, a CHANGELOG entry per version.

## Where things are

| Task | Files |
|:-----|:------|
| Talk to Waydroid (status, swap, start, stop) | `crates/omadroid-core/src/engine.rs` |
| Change which Android versions are offered | `crates/omadroid-core/src/catalog.rs` |
| Change the download or the unpack | `crates/omadroid-core/src/install.rs` |
| Where versions and data live | `crates/omadroid-core/src/versions.rs`, `paths.rs` |
| The live Omarchy colors / rounding | `crates/omadroid-core/src/theme.rs` |
| A screen, a key, a color | `app/ui/app.slint`, `app/ui/theme.slint` |
| Wiring, worker thread, timers | `app/src/lib.rs` |
| Package, icon, launcher entry | `pkgbuild/` |
