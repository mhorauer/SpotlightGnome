use std::path::PathBuf;

use gtk::gdk;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// User settings, stored in ~/.config/spotlight/config.toml.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Spotlight diameter as a percentage of the screen height.
    pub size_percent: f64,
    /// Color of the dimmed area, as "#rrggbb".
    pub shade_color: String,
    /// Opacity of the dimmed area, 0.0 to 1.0.
    pub shade_opacity: f64,
    /// Width of the ring around the spotlight in pixels (0 = no ring).
    pub border_width: f64,
    /// Ring color, as "#rrggbbaa" or any CSS color.
    pub border_color: String,
    /// Draw a small dot where the pointer is.
    pub center_dot: bool,
    pub center_dot_size: f64,
    pub center_dot_color: String,
    /// How long after the device stops moving the spotlight disappears.
    pub hide_delay_ms: u64,
    /// Monitor connector name (e.g. "HDMI-1"), or None to let GNOME choose.
    pub monitor: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            size_percent: 30.0,
            shade_color: "#000000".into(),
            shade_opacity: 0.65,
            border_width: 3.0,
            border_color: "rgba(255,255,255,0.6)".into(),
            center_dot: false,
            center_dot_size: 6.0,
            center_dot_color: "#ff3030".into(),
            hide_delay_ms: 600,
            monitor: None,
        }
    }
}

impl Config {
    fn path() -> PathBuf {
        gtk::glib::user_config_dir()
            .join("spotlight")
            .join("config.toml")
    }

    /// Loads the settings, falling back to defaults if the file is missing or broken.
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(config) => config,
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "invalid config, using defaults");
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                warn!(path = %path.display(), error = %e, "cannot read config, using defaults");
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let path = Self::path();
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, toml::to_string_pretty(self)?)?;
            Ok(())
        })();
        match result {
            Ok(()) => tracing::debug!(path = %path.display(), "settings saved"),
            Err(e) => warn!(path = %path.display(), error = %e, "failed to save settings"),
        }
    }
}

/// Parses a CSS color string, falling back to `fallback` if it is invalid.
pub fn parse_color(s: &str, fallback: gdk::RGBA) -> gdk::RGBA {
    gdk::RGBA::parse(s).unwrap_or(fallback)
}
