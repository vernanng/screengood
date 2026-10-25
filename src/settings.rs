//! Scroll-feel settings: edited in the native window opened by
//! `screengood settings`, read once at launch by `mirror`/`listen`, so
//! changes apply on next launch. Stored as `key = value` text in
//! `~/Library/Application Support/ScreenGood/settings.txt` (the same
//! support dir as the signing material; `SCREENGOOD_SETTINGS` overrides
//! the path). A missing or unreadable file means the tuned defaults in
//! `crate::logic` — an unmodified install behaves exactly as before.

use crate::logic::{COOLDOWN, PIXEL_THRESHOLD, SWIPE_DUR_MS};
use std::path::PathBuf;

/// Valid ranges, shared by the settings-window sliders and validation.
pub const STEP_MIN_PX: f64 = 1.0;
pub const STEP_MAX_PX: f64 = 30.0;
pub const COOLDOWN_MIN_MS: u64 = 0;
pub const COOLDOWN_MAX_MS: u64 = 3000;
pub const SWIPE_MIN_MS: u32 = 50;
pub const SWIPE_MAX_MS: u32 = 1500;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollSettings {
    /// Accumulated wheel pixels per NEXT/PREV step (default 6).
    pub scroll_step_px: f64,
    /// Minimum time between two fired steps, ms (default 900).
    pub cooldown_ms: u64,
    /// NEXT/PREV swipe duration, ms (default 400).
    pub swipe_ms: u32,
}

impl Default for ScrollSettings {
    fn default() -> Self {
        Self {
            scroll_step_px: PIXEL_THRESHOLD,
            cooldown_ms: COOLDOWN.as_millis() as u64,
            swipe_ms: SWIPE_DUR_MS,
        }
    }
}

impl ScrollSettings {
    /// Clamp every value into its valid range; a non-finite step falls
    /// back to the default. Applied on load and before every save.
    pub fn validated(mut self) -> Self {
        if !self.scroll_step_px.is_finite() {
            self.scroll_step_px = PIXEL_THRESHOLD;
        }
        self.scroll_step_px = self.scroll_step_px.clamp(STEP_MIN_PX, STEP_MAX_PX);
        self.cooldown_ms = self.cooldown_ms.clamp(COOLDOWN_MIN_MS, COOLDOWN_MAX_MS);
        self.swipe_ms = self.swipe_ms.clamp(SWIPE_MIN_MS, SWIPE_MAX_MS);
        self
    }

    /// Parse `key = value` lines over the defaults: blank lines, `#`
    /// comments, unknown keys, and unparseable values are ignored, so a
    /// hand-edited or partial file can never break a launch.
    pub fn parse(contents: &str) -> Self {
        let mut out = Self::default();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "scroll_step_px" => {
                    if let Ok(v) = value.parse::<f64>() {
                        out.scroll_step_px = v;
                    }
                }
                "cooldown_ms" => {
                    if let Ok(v) = value.parse::<u64>() {
                        out.cooldown_ms = v;
                    }
                }
                "swipe_ms" => {
                    if let Ok(v) = value.parse::<u32>() {
                        out.swipe_ms = v;
                    }
                }
                _ => {}
            }
        }
        out.validated()
    }

    pub fn to_file_contents(self) -> String {
        let s = self.validated();
        format!(
            "# ScreenGood scroll settings - edited by `screengood settings`.\n\
             # Read once at launch; changes apply on next launch.\n\
             scroll_step_px = {}\n\
             cooldown_ms = {}\n\
             swipe_ms = {}\n",
            format_step(s.scroll_step_px),
            s.cooldown_ms,
            s.swipe_ms
        )
    }

    /// One-line effective values, printed at launch and after a save.
    pub fn summary(&self) -> String {
        let s = self.validated();
        format!(
            "step={}px cooldown={}ms swipe={}ms",
            format_step(s.scroll_step_px),
            s.cooldown_ms,
            s.swipe_ms
        )
    }
}

/// `6` when whole, else one decimal — the step as shown everywhere.
pub fn format_step(px: f64) -> String {
    if px.fract() == 0.0 {
        format!("{px:.0}")
    } else {
        format!("{px:.1}")
    }
}

/// Where settings live: `SCREENGOOD_SETTINGS` when set, else
/// `~/Library/Application Support/ScreenGood/settings.txt`.
pub fn settings_path() -> PathBuf {
    if let Ok(path) = std::env::var("SCREENGOOD_SETTINGS") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/ScreenGood/settings.txt")
}

/// Settings from `path`; a missing or unreadable file gives defaults.
pub fn load_from(path: &std::path::Path) -> ScrollSettings {
    match std::fs::read_to_string(path) {
        Ok(contents) => ScrollSettings::parse(&contents),
        Err(_) => ScrollSettings::default(),
    }
}

/// Settings from the standard path (see `settings_path`).
pub fn load() -> ScrollSettings {
    load_from(&settings_path())
}

/// Write validated settings to `path`, creating the support dir.
pub fn save_to(settings: &ScrollSettings, path: &std::path::Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, settings.to_file_contents())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_tuned_logic_values() {
        let d = ScrollSettings::default();
        assert_eq!(d.scroll_step_px, PIXEL_THRESHOLD);
        assert_eq!(d.cooldown_ms, 900);
        assert_eq!(d.swipe_ms, SWIPE_DUR_MS);
        assert_eq!(d.swipe_ms, 400);
    }

    #[test]
    fn parse_reads_all_keys_and_ignores_noise() {
        let s = ScrollSettings::parse(
            "# comment\nscroll_step_px = 10\nunknown = 1\n\ncooldown_ms=250\nswipe_ms = oops\n",
        );
        assert_eq!(s.scroll_step_px, 10.0);
        assert_eq!(s.cooldown_ms, 250);
        assert_eq!(s.swipe_ms, 400);
    }

    #[test]
    fn parse_clamps_out_of_range_and_nan() {
        let s = ScrollSettings::parse("scroll_step_px = 999\ncooldown_ms = 99999\nswipe_ms = 1\n");
        assert_eq!(s.scroll_step_px, STEP_MAX_PX);
        assert_eq!(s.cooldown_ms, COOLDOWN_MAX_MS);
        assert_eq!(s.swipe_ms, SWIPE_MIN_MS);
        assert_eq!(
            ScrollSettings::parse("scroll_step_px = NaN\n").scroll_step_px,
            PIXEL_THRESHOLD
        );
    }

    #[test]
    fn file_contents_round_trip() {
        let s = ScrollSettings {
            scroll_step_px: 8.5,
            cooldown_ms: 0,
            swipe_ms: 650,
        };
        assert_eq!(ScrollSettings::parse(&s.to_file_contents()), s);
        assert_eq!(s.summary(), "step=8.5px cooldown=0ms swipe=650ms");
    }

    #[test]
    fn save_then_load_same_values_missing_file_defaults() {
        let dir = std::env::temp_dir().join(format!("screengood-settings-{}", std::process::id()));
        let path = dir.join("settings.txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_from(&path), ScrollSettings::default());
        let s = ScrollSettings {
            scroll_step_px: 3.0,
            cooldown_ms: 1200,
            swipe_ms: 300,
        };
        save_to(&s, &path).unwrap();
        assert_eq!(load_from(&path), s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
