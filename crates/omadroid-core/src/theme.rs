//! The Omarchy theme, painted the Omacalc way.
//!
//! Omarchy writes the whole palette to `colors.toml` on every theme switch.
//! Three colors matter: the background, the foreground and the accent. Every
//! other color of the window is a mix of the first two, exactly like Omacalc:
//!
//! | No. | Role | Mix of the foreground into the background |
//! |:----|:-----|:------------------------------------------|
//! | 1 | key / row | 5% |
//! | 2 | border | 13% |
//! | 3 | panel / selection | 16% |
//! | 4 | muted text | 50% |

use std::path::Path;
use std::process::Command;

/// An RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parses `#rrggbb` (case-insensitive, quotes and spaces tolerated).
    pub fn hex(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_matches('"').trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// Mixes `tint` into `self` by `amount` (0.0 to 1.0), like Omacalc's `mixColors`.
    pub fn mix(self, tint: Rgb, amount: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
        Rgb(m(self.0, tint.0), m(self.1, tint.1), m(self.2, tint.2))
    }

    /// `0xAARRGGBB`, the format Slint's `Color::from_argb_encoded` takes.
    pub fn argb(self) -> u32 {
        0xff00_0000 | (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }
}

/// The three colors omadroid needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub accent: Rgb,
}

impl Theme {
    /// Tokyo Night, used when Omarchy is not there (or the file is unreadable).
    pub fn omarchy_default() -> Theme {
        Theme {
            background: Rgb(0x1a, 0x1b, 0x26),
            foreground: Rgb(0xa9, 0xb1, 0xd6),
            accent: Rgb(0x7a, 0xa2, 0xf7),
        }
    }

    /// Reads `background`, `foreground` and `accent` out of a `colors.toml`.
    /// A missing key falls back to the Tokyo Night value for that key.
    pub fn parse(text: &str) -> Theme {
        let mut theme = Theme::omarchy_default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let Some(color) = Rgb::hex(value) else {
                continue;
            };
            match key.trim() {
                "background" => theme.background = color,
                "foreground" => theme.foreground = color,
                "accent" => theme.accent = color,
                _ => {}
            }
        }
        theme
    }

    /// The live theme, or the default when the file is missing.
    pub fn load(path: &Path) -> Theme {
        match std::fs::read_to_string(path) {
            Ok(text) => Theme::parse(&text),
            Err(_) => Theme::omarchy_default(),
        }
    }

    /// Rows and keys: 5% of the foreground.
    pub fn key(&self) -> Rgb {
        self.background.mix(self.foreground, 0.05)
    }

    /// Borders and separators: 13%.
    pub fn border(&self) -> Rgb {
        self.background.mix(self.foreground, 0.13)
    }

    /// Panels and the selected row: 16%.
    pub fn panel(&self) -> Rgb {
        self.background.mix(self.foreground, 0.16)
    }

    /// Muted text: 50%.
    pub fn muted(&self) -> Rgb {
        self.background.mix(self.foreground, 0.5)
    }
}

/// Hyprland's `decoration:rounding`, so panels match the windows. 0 on Omarchy
/// until the user rounds them, and 0 again when `hyprctl` is not there.
pub fn hypr_rounding() -> f32 {
    let Ok(out) = Command::new("hyprctl")
        .args(["getoption", "decoration:rounding"])
        .output()
    else {
        return 0.0;
    };
    if !out.status.success() {
        return 0.0;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("int:")
                .and_then(|value| value.trim().parse::<f32>().ok())
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_omarchy_palette() {
        let theme = Theme::parse(
            "mode = \"dark\"\n\naccent = \"#f38d70\"\nbackground = \"#2c2525\"\nforeground = \"#e6d9db\"\n",
        );
        assert_eq!(theme.accent, Rgb(0xf3, 0x8d, 0x70));
        assert_eq!(theme.background, Rgb(0x2c, 0x25, 0x25));
        assert_eq!(theme.foreground, Rgb(0xe6, 0xd9, 0xdb));
    }

    #[test]
    fn a_missing_key_keeps_the_default() {
        let theme = Theme::parse("background = \"#000000\"\n");
        assert_eq!(theme.background, Rgb(0, 0, 0));
        assert_eq!(theme.accent, Theme::omarchy_default().accent);
    }

    #[test]
    fn mixing_matches_omacalc() {
        let black = Rgb(0, 0, 0);
        let white = Rgb(255, 255, 255);
        assert_eq!(black.mix(white, 0.5), Rgb(128, 128, 128));
        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(white.argb(), 0xffff_ffff);
    }
}
