# Contributing to omadroid 🤝

Thanks for helping! omadroid is small on purpose: one window, one job.


## Getting started 🚀

```sh
git clone https://github.com/abraham-ukachi/omadroid.git && cd omadroid
cargo test --workspace     # catalog, config, theme, versions (no Waydroid needed)
cargo run -p omadroid      # the window, on your desktop
```


## Project layout 🗂️

| No. | Path | What lives there |
|:----|:-----|:-----------------|
| 1 | `crates/omadroid-core` | Waydroid control, catalog, downloads, theme, storage. No UI, fully tested |
| 2 | `app/ui/*.slint` | The window, in Slint |
| 3 | `app/src` | The glue: UI callbacks and the worker thread |
| 4 | `pkgbuild` | PKGBUILD, `.desktop`, icon |


## Design rules 🎨

1. Follow **Omacalc**: page color + ink color, everything else is a mix of the two (5% rows, 13% borders, 16% panels, 50% muted). The accent is a whisper: the live dot and the selected version, nothing else.
2. iA Writer Mono everywhere.
3. Sharp corners unless Hyprland rounds windows: `border-radius` always comes from `Theme.radius`.
4. Say what is wrong in the window (`notice`), never fail silently.


## Git commit messages ✍️

We use [gitmoji](https://gitmoji.dev) + a short sentence in the imperative mood:

| Emoji | Code | When |
|:------|:-----|:-----|
| ✨ | `:sparkles:` | a new feature |
| 🐛 | `:bug:` | a bug fix |
| 💄 | `:lipstick:` | UI & style |
| 📝 | `:memo:` | docs |
| 🔧 | `:wrench:` | config & tooling |
| ✅ | `:white_check_mark:` | tests |
| 🔖 | `:bookmark:` | a release (`Release 0.1.0`) |

Example: `:sparkles: Swap Waydroid's images_path instead of copying images`
