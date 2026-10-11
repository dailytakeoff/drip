//! TOML config at $XDG_CONFIG_HOME/drip/config.toml (all keys optional).

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub logo: Option<String>,
    pub logo_colors: Vec<String>,
    pub logo_gap: Option<usize>,
    pub style: Option<String>,
    pub key_color: Option<String>,
    pub separator: Option<String>,
    pub truncate: Option<bool>,
    pub modules: Option<Vec<String>>,
    pub keys: HashMap<String, String>,
    pub rain: Rain,
}

/// Settings for `drip --drip`.
#[derive(Deserialize, Default, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Rain {
    pub intensity: Option<f32>,
    pub chars: Vec<String>,
    pub colors: Vec<String>,
    pub speed: Option<[f32; 2]>,
    pub length: Option<[u16; 2]>,
    pub collision: Option<String>,
    pub fps: Option<u32>,
    pub refresh: Option<f32>,
}

pub fn default_path() -> Option<PathBuf> {
    crate::sys::config_home().map(|c| c.join("drip").join("config.toml"))
}

/// Missing file -> defaults. Broken file -> warning + defaults, so a typo
/// never leaves you without a fetch.
pub fn load(path: Option<&Path>) -> Config {
    let explicit = path.is_some();
    let Some(path) = path.map(Path::to_path_buf).or_else(default_path) else {
        return Config::default();
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            if explicit {
                eprintln!("drip: can't read {}: {e}", path.display());
            }
            return Config::default();
        }
    };
    match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("drip: {} is invalid, using defaults:\n{e}", path.display());
            Config::default()
        }
    }
}

pub const CLASSIC_MODULES: &[&str] = &[
    "title", "separator", "os", "host", "kernel", "init", "uptime", "packages", "shell", "display",
    "de", "wm", "theme", "icons", "font", "cursor", "terminal", "cpu", "gpu", "memory", "swap",
    "disk", "localip", "battery", "locale", "break", "colors",
];

pub const TREE_MODULES: &[&str] = &[
    "title", "break",
    "section:SYSTEM", "os", "host", "kernel", "init", "uptime", "packages",
    "section:SESSION", "shell", "terminal", "de", "wm", "display", "theme", "icons", "font",
    "section:HARDWARE", "cpu", "gpu", "memory", "swap", "disk",
    "section:CONNECTIVITY", "localip", "battery",
    "break", "colors",
];

pub const DEFAULT_CONFIG: &str = r##"# drip configuration — every key is optional.
# Location: $XDG_CONFIG_HOME/drip/config.toml (usually ~/.config/drip/config.toml)

# Logo: "auto" (from /etc/os-release), a built-in name (`drip --list-logos`),
# a path to your own file, or "none". Logo files use fastfetch syntax:
# $1..$9 switch color, $$ is a literal dollar sign.
logo = "auto"

# Colors for $1, $2, ... in the logo. Names (cyan, bright_blue), palette
# numbers (0-255) or hex ("#8747ff"). Empty = the distro's usual colors.
logo_colors = []

# Spaces between the logo and the info column.
logo_gap = 3

# "classic" = fastfetch-style `Key: value`
# "tree"    = dripfetch-style sections drawn with box characters
style = "classic"

# Color for keys, the title and tree lines. "auto" = the logo's first color.
key_color = "auto"

# Between key and value in classic style.
separator = ": "

# Cut lines that would wrap past the terminal edge.
truncate = true

# Which modules to show, in order. Leave commented out to use the default
# for the chosen style. Besides the info modules (`drip --list-modules`):
#   "title"          user@hostname
#   "separator"      a line under the title
#   "break"          an empty line
#   "colors"         the 16 terminal colors
#   "section:NAME"   a section header (tree style groups modules under it)
# modules = [
#     "title", "separator",
#     "os", "host", "kernel", "init", "uptime", "packages",
#     "shell", "display", "de", "wm", "terminal",
#     "cpu", "gpu", "memory", "swap", "disk", "localip", "battery",
#     "break", "colors",
# ]

# Rename keys.
[keys]
# os = "Distro"
# localip = "IP"

# The animated rain shown by `drip --drip`.
[rain]
# Drops on screen, scaled to an 80x24 terminal. 0 = no rain.
intensity = 60

# Characters to fall; one is picked per drop.
chars = ["│"]

# Drop colors. Empty = the logo's colors.
colors = []

# [min, max] fall speed in rows per second, and trail length in rows.
speed = [10.0, 35.0]
length = [4, 16]

# What drops do when they reach the fetch:
#   "box"   stop at a box around it (cleanest)
#   "shape" land on the logo and text themselves
#   "none"  fall behind it
collision = "box"

fps = 30

# Seconds between refreshes of uptime, memory, swap, disk, battery and IP.
refresh = 2.0
"##;
