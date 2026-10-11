//! Info modules. Each reads the system directly (/proc, /sys, files)
//! and only spawns a process when there is no other way (shell version).

use crate::color::{percent, Seg};
use crate::sys::{self, read, read_trim, Uname};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

pub struct Ctx {
    pub os: HashMap<String, String>,
    pub uname: Option<Uname>,
    pub config_home: Option<PathBuf>,
}

pub struct Entry {
    pub key: String,
    pub value: Vec<Seg>,
}

impl Entry {
    fn text(key: impl Into<String>, value: impl Into<String>) -> Entry {
        Entry { key: key.into(), value: vec![Seg::plain(value)] }
    }
}

/// (name, default key label, description)
pub const MODULES: &[(&str, &str, &str)] = &[
    ("os", "OS", "distribution name and architecture"),
    ("host", "Host", "machine model from DMI / device tree"),
    ("kernel", "Kernel", "kernel name and release"),
    ("init", "Init", "init system (systemd, dinit, runit, OpenRC, s6...)"),
    ("uptime", "Uptime", "time since boot"),
    ("packages", "Packages", "pacman, dpkg, rpm, portage, xbps, apk, flatpak, snap"),
    ("shell", "Shell", "parent shell and its version"),
    ("display", "Display", "connected outputs and resolution"),
    ("de", "DE", "desktop environment"),
    ("wm", "WM", "window manager / compositor"),
    ("theme", "Theme", "GTK / Qt theme"),
    ("icons", "Icons", "icon theme"),
    ("font", "Font", "GTK font"),
    ("cursor", "Cursor", "cursor theme and size"),
    ("terminal", "Terminal", "terminal emulator"),
    ("cpu", "CPU", "model, threads and max clock"),
    ("gpu", "GPU", "graphics adapters (via pci.ids)"),
    ("memory", "Memory", "used / total RAM"),
    ("swap", "Swap", "used / total swap"),
    ("disk", "Disk", "usage of each mounted filesystem"),
    ("localip", "Local IP", "address of the default-route interface"),
    ("battery", "Battery", "charge and status"),
    ("locale", "Locale", "LC_ALL / LANG"),
];

pub fn default_label(name: &str) -> Option<&'static str> {
    MODULES.iter().find(|m| m.0 == name).map(|m| m.1)
}

/// Modules worth running on their own thread.
pub fn is_slow(name: &str) -> bool {
    matches!(name, "shell" | "packages" | "gpu" | "wm" | "disk" | "terminal")
}

pub fn gather(name: &str, label: &str, ctx: &Ctx) -> Vec<Entry> {
    let one = |v: Option<String>| v.map(|v| vec![Entry::text(label, v)]).unwrap_or_default();
    match name {
        "os" => one(os(ctx)),
        "host" => one(host()),
        "kernel" => one(ctx.uname.as_ref().map(|u| format!("{} {}", u.sysname, u.release))),
        "init" => one(init()),
        "uptime" => one(uptime()),
        "packages" => one(packages()),
        "shell" => one(shell()),
        "display" => displays(label),
        "de" => one(de()),
        "wm" => one(wm()),
        "theme" => one(gtk_or_kde(ctx, "gtk-theme-name", Some(("General", "ColorScheme")))),
        "icons" => one(gtk_or_kde(ctx, "gtk-icon-theme-name", Some(("Icons", "Theme")))),
        "font" => one(font(ctx)),
        "cursor" => one(cursor(ctx)),
        "terminal" => one(terminal()),
        "cpu" => one(cpu()),
        "gpu" => gpus(label),
        "memory" => memory(label, false),
        "swap" => memory(label, true),
        "disk" => disks(label),
        "localip" => local_ip(label),
        "battery" => batteries(label),
        "locale" => one(sys::env("LC_ALL").or_else(|| sys::env("LANG"))),
        _ => Vec::new(),
    }
}

// ---- system ----------------------------------------------------------------

fn os(ctx: &Ctx) -> Option<String> {
    let name = ctx.os.get("PRETTY_NAME").or_else(|| ctx.os.get("NAME")).cloned().or_else(|| {
        ctx.uname.as_ref().map(|u| u.sysname.clone())
    })?;
    match &ctx.uname {
        Some(u) => Some(format!("{name} {}", u.machine)),
        None => Some(name),
    }
}

fn junk(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.is_empty()
        || l.contains("to be filled")
        || l.contains("o.e.m")
        || l.contains("default string")
        || l.contains("system product name")
        || l.contains("system version")
        || l.contains("not applicable")
        || l.contains("not specified")
        || l == "none"
        || l == "type1productconfigid"
        || l.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ' ')
}

fn host() -> Option<String> {
    let dmi = |f: &str| read_trim(format!("/sys/devices/virtual/dmi/id/{f}")).filter(|s| !junk(s));
    if let Some(name) = dmi("product_name") {
        return Some(match dmi("product_version") {
            // Lenovo keeps the marketing name here: "20L7S0CF00 (ThinkPad T480s)"
            Some(v) if v != name && !name.contains(&v) => format!("{name} ({v})"),
            _ => name,
        });
    }
    if let Some(f) = dmi("product_family") {
        return Some(f);
    }
    read("/sys/firmware/devicetree/base/model")
        .map(|m| m.trim_end_matches('\0').trim().to_owned())
        .filter(|m| !m.is_empty())
}

fn init() -> Option<String> {
    let comm = sys::comm_of(1)?;
    let name = match comm.as_str() {
        "systemd" => "systemd",
        "dinit" => "dinit",
        "runit" | "runit-init" => "runit",
        "s6-svscan" | "s6-linux-init" => "s6",
        "openrc-init" => "OpenRC",
        "shepherd" => "GNU Shepherd",
        "launchd" => "launchd",
        "init" if std::path::Path::new("/run/openrc").exists() => "OpenRC",
        "init" if std::path::Path::new("/run/runit").exists() => "runit",
        "init" => "SysVinit",
        // containers and sandboxes: PID 1 is whatever was launched
        other => return Some(other.to_owned()),
    };
    Some(name.to_owned())
}

pub fn uptime_secs() -> Option<u64> {
    let s = read("/proc/uptime")?;
    s.split_whitespace().next()?.parse::<f64>().ok().map(|f| f as u64)
}

pub fn fmt_duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    let unit = |n: u64, s: &str| format!("{n} {s}{}", if n == 1 { "" } else { "s" });
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(unit(d, "day"));
    }
    if h > 0 {
        parts.push(unit(h, "hour"));
    }
    if m > 0 || parts.is_empty() {
        parts.push(unit(m, "min"));
    }
    parts.join(", ")
}

fn uptime() -> Option<String> {
    uptime_secs().map(fmt_duration)
}

// ---- packages --------------------------------------------------------------

fn count_matches(path: &str, needle: &str) -> usize {
    read(path).map(|t| t.matches(needle).count()).unwrap_or(0)
}

fn packages() -> Option<String> {
    let mut counts: Vec<(usize, &str)> = Vec::new();

    // pacman: one directory per installed package
    counts.push((sys::count_dirs("/var/lib/pacman/local"), "pacman"));
    // portage: /var/db/pkg/<category>/<package>
    let portage = fs::read_dir("/var/db/pkg")
        .map(|d| d.filter_map(|e| e.ok()).map(|e| sys::count_dirs(e.path())).sum())
        .unwrap_or(0);
    counts.push((portage, "emerge"));
    counts.push((count_matches("/var/lib/dpkg/status", "Status: install ok installed"), "dpkg"));
    counts.push((count_matches("/lib/apk/db/installed", "\nP:") , "apk"));
    if let Ok(dir) = fs::read_dir("/var/db/xbps") {
        let n = dir
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("pkgdb-"))
            .map(|e| read(e.path()).map(|t| t.matches("<string>installed</string>").count()).unwrap_or(0))
            .sum();
        counts.push((n, "xbps"));
    }
    // rpm keeps a sqlite db; asking rpm is the only reliable way
    if std::path::Path::new("/var/lib/rpm").exists() && sys::which("rpm").is_some() {
        let n = sys::run("rpm", &["-qa", "--qf", "x"], Duration::from_millis(1500))
            .map(|s| s.len())
            .unwrap_or(0);
        counts.push((n, "rpm"));
    }
    let mut flatpak = sys::count_dirs("/var/lib/flatpak/app");
    if let Some(h) = sys::home() {
        flatpak += sys::count_dirs(h.join(".local/share/flatpak/app"));
    }
    counts.push((flatpak, "flatpak"));
    let snaps = fs::read_dir("/snap")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter(|e| e.file_name() != "bin")
                .count()
        })
        .unwrap_or(0);
    counts.push((snaps, "snap"));

    let parts: Vec<String> = counts
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, m)| format!("{n} ({m})"))
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

// ---- shell & terminal ------------------------------------------------------

/// Process names to look past when walking up from drip to the shell.
const WRAPPERS: &[&str] = &[
    "sudo", "doas", "su", "run0", "strace", "ltrace", "time", "script", "env", "cargo", "hyperfine",
    "perf", "valgrind", "watch", "timeout", "nice", "chrt", "taskset",
];
const SHELLS: &[&str] = &[
    "bash", "zsh", "fish", "nu", "dash", "sh", "ksh", "mksh", "oksh", "tcsh", "csh", "elvish",
    "xonsh", "osh", "ysh", "ion", "pwsh", "yash", "murex", "brush",
];

/// Returns (shell pid, shell name).
fn find_shell() -> Option<(u32, String)> {
    let mut pid = sys::parent_pid();
    for _ in 0..8 {
        let comm = sys::comm_of(pid)?;
        if !WRAPPERS.contains(&comm.as_str()) {
            return Some((pid, comm));
        }
        pid = sys::ppid_of(pid)?;
    }
    None
}

/// Pulls a version number out of `--version` output.
pub fn parse_version(out: &str) -> Option<String> {
    let line = out.lines().find(|l| !l.trim().is_empty())?;
    let tok = line
        .split(|c: char| c.is_whitespace() || c == ',')
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()) && t.contains('.'))?;
    let tok = tok.split('(').next().unwrap_or(tok);
    let tok = tok.trim_end_matches("-release").trim_end_matches(['.', ',', ')']);
    (!tok.is_empty()).then(|| tok.to_owned())
}

fn shell() -> Option<String> {
    let (pid, name) = find_shell()?;
    if !SHELLS.contains(&name.as_str()) {
        return Some(name);
    }
    // These have no --version flag.
    if matches!(name.as_str(), "dash" | "sh" | "ksh" | "mksh" | "oksh" | "yash") {
        return Some(name);
    }
    let exe = sys::exe_of(pid)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.clone());
    let version = sys::run(&exe, &["--version"], Duration::from_millis(400))
        .and_then(|o| parse_version(&o));
    Some(match version {
        Some(v) => format!("{name} {v}"),
        None => name,
    })
}

fn pretty_terminal(comm: &str) -> Option<&'static str> {
    Some(match comm {
        "kitty" => "kitty",
        "alacritty" => "Alacritty",
        "foot" | "footclient" => "foot",
        "wezterm-gui" | "wezterm" => "WezTerm",
        "ghostty" => "Ghostty",
        "konsole" => "Konsole",
        "gnome-terminal-" | "gnome-terminal" => "GNOME Terminal",
        "kgx" => "GNOME Console",
        "ptyxis-agent" | "ptyxis" => "Ptyxis",
        "xterm" => "xterm",
        "urxvt" | "urxvtd" | "rxvt" => "rxvt-unicode",
        "st" => "st",
        "terminator" => "Terminator",
        "tilix" => "Tilix",
        "xfce4-terminal" => "Xfce Terminal",
        "lxterminal" => "LXTerminal",
        "qterminal" => "QTerminal",
        "sakura" => "Sakura",
        "rio" => "Rio",
        "warp" => "Warp",
        "zed" | "zed-editor" => "Zed",
        "code" | "codium" => "VS Code",
        "contour" => "Contour",
        "cool-retro-term" => "cool-retro-term",
        "blackbox" => "Black Box",
        "tabby" => "Tabby",
        "wayst" => "wayst",
        "termite" => "Termite",
        "tmux: server" | "tmux" => "tmux",
        "screen" => "screen",
        "zellij" => "Zellij",
        "sshd" | "sshd-session" => "SSH",
        "login" | "agetty" => "TTY",
        _ => return None,
    })
}

fn terminal() -> Option<String> {
    if let Some(tp) = sys::env("TERM_PROGRAM") {
        let name = match tp.as_str() {
            "vscode" => "VS Code".to_owned(),
            "WezTerm" | "ghostty" | "iTerm.app" | "Apple_Terminal" | "zed" | "tmux" => {
                pretty_terminal(&tp.to_ascii_lowercase()).map(str::to_owned).unwrap_or(tp.clone())
            }
            _ => tp.clone(),
        };
        let ver = sys::env("TERM_PROGRAM_VERSION").unwrap_or_default();
        return Some(format!("{name} {ver}").trim().to_owned());
    }
    let (shell_pid, _) = find_shell()?;
    let mut pid = sys::ppid_of(shell_pid)?;
    for _ in 0..10 {
        if pid <= 1 {
            break;
        }
        let comm = sys::comm_of(pid)?;
        if let Some(p) = pretty_terminal(&comm) {
            return Some(p.to_owned());
        }
        let skip = WRAPPERS.contains(&comm.as_str()) || SHELLS.contains(&comm.as_str());
        if !skip {
            return Some(comm);
        }
        pid = sys::ppid_of(pid)?;
    }
    sys::env("TERM")
}

// ---- desktop ---------------------------------------------------------------

/// (process name, pretty name). Prefix match for names that comm truncates.
const WMS: &[(&str, &str)] = &[
    ("niri", "niri"),
    ("Hyprland", "Hyprland"),
    ("sway", "Sway"),
    ("river", "River"),
    ("mango", "MangoWC"),
    ("labwc", "labwc"),
    ("wayfire", "Wayfire"),
    ("kwin_wayland", "KWin"),
    ("kwin_x11", "KWin"),
    ("gnome-shell", "Mutter"),
    ("weston", "Weston"),
    ("cosmic-comp", "COSMIC"),
    ("dwl", "dwl"),
    ("hikari", "hikari"),
    ("qtile", "Qtile"),
    ("i3", "i3"),
    ("bspwm", "bspwm"),
    ("dwm", "dwm"),
    ("openbox", "Openbox"),
    ("xmonad", "xmonad"),
    ("awesome", "awesome"),
    ("herbstluftwm", "herbstluftwm"),
    ("fluxbox", "Fluxbox"),
    ("icewm", "IceWM"),
    ("xfwm4", "Xfwm4"),
    ("marco", "Marco"),
    ("muffin", "Muffin"),
    ("enlightenment", "Enlightenment"),
    ("spectrwm", "spectrwm"),
    ("leftwm", "LeftWM"),
    ("jwm", "JWM"),
    ("cwm", "cwm"),
    ("berry", "berry"),
    ("miracle-wm", "Miracle"),
    ("jay", "Jay"),
];

fn session_type() -> Option<&'static str> {
    match sys::env("XDG_SESSION_TYPE").as_deref() {
        Some("wayland") => Some("Wayland"),
        Some("x11") => Some("X11"),
        _ if sys::env("WAYLAND_DISPLAY").is_some() => Some("Wayland"),
        _ if sys::env("DISPLAY").is_some() => Some("X11"),
        _ => None,
    }
}

fn wm_name() -> Option<&'static str> {
    if sys::env("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Some("Hyprland");
    }
    if sys::env("NIRI_SOCKET").is_some() {
        return Some("niri");
    }
    if sys::env("SWAYSOCK").is_some() {
        return Some("Sway");
    }
    if sys::env("WAYLAND_DISPLAY").is_none() && sys::env("DISPLAY").is_none() {
        return None;
    }
    let comms = sys::all_comms();
    WMS.iter()
        .find(|(proc_name, _)| {
            comms.iter().any(|c| c == proc_name || (proc_name.len() >= 5 && c.starts_with(proc_name)))
        })
        .map(|(_, pretty)| *pretty)
}

fn wm() -> Option<String> {
    let name = wm_name()?;
    Some(match session_type() {
        Some(s) => format!("{name} ({s})"),
        None => name.to_owned(),
    })
}

fn de() -> Option<String> {
    let raw = sys::env("XDG_CURRENT_DESKTOP")
        .or_else(|| sys::env("XDG_SESSION_DESKTOP"))
        .or_else(|| sys::env("DESKTOP_SESSION"))?;
    let first = raw.split(':').next()?.to_owned();
    let lower = first.to_ascii_lowercase();
    // Standalone compositors set this to themselves; that's the WM line's job.
    if WMS.iter().any(|(p, n)| p.eq_ignore_ascii_case(&lower) || n.eq_ignore_ascii_case(&lower))
        || lower == "wlroots"
    {
        return None;
    }
    Some(match lower.as_str() {
        "kde" => match sys::env("KDE_SESSION_VERSION") {
            Some(v) => format!("KDE Plasma {v}"),
            None => "KDE Plasma".into(),
        },
        "gnome" => "GNOME".into(),
        "xfce" => "Xfce".into(),
        "x-cinnamon" | "cinnamon" => "Cinnamon".into(),
        "mate" => "MATE".into(),
        "lxqt" => "LXQt".into(),
        "cosmic" => "COSMIC".into(),
        "budgie" | "budgie:gnome" => "Budgie".into(),
        _ => first,
    })
}

fn displays(label: &str) -> Vec<Entry> {
    let Ok(dir) = fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut outs: Vec<(String, String)> = dir
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // "card1-eDP-1" -> connector "eDP-1"
            let (_, connector) = name.split_once('-')?;
            if read_trim(e.path().join("status"))? != "connected" {
                return None;
            }
            let mode = read(e.path().join("modes"))?.lines().next()?.trim().to_owned();
            let builtin = ["eDP", "LVDS", "DSI"].iter().any(|p| connector.starts_with(p));
            let value = if builtin { format!("{mode} [Built-in]") } else { mode };
            Some((connector.to_owned(), value))
        })
        .collect();
    outs.sort();
    let multi = outs.len() > 1;
    outs.into_iter()
        .map(|(c, v)| Entry::text(if multi { format!("{label} ({c})") } else { label.to_owned() }, v))
        .collect()
}

/// Reads a key from GTK 3/4 settings.ini.
fn gtk_setting(ctx: &Ctx, key: &str) -> Option<(String, &'static str)> {
    let cfg = ctx.config_home.as_ref()?;
    for (dir, tag) in [("gtk-3.0", "GTK3"), ("gtk-4.0", "GTK4")] {
        if let Some(text) = read(cfg.join(dir).join("settings.ini")) {
            if let Some(v) = sys::parse_kv(&text).get(key).filter(|v| !v.is_empty()) {
                return Some((v.clone(), tag));
            }
        }
    }
    None
}

fn kde_setting(ctx: &Ctx, group: &str, key: &str) -> Option<String> {
    let text = read(ctx.config_home.as_ref()?.join("kdeglobals"))?;
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == format!("[{group}]");
        } else if in_group {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().to_owned());
                }
            }
        }
    }
    None
}

fn gtk_or_kde(ctx: &Ctx, gtk_key: &str, kde: Option<(&str, &str)>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some((group, key)) = kde {
        if let Some(v) = kde_setting(ctx, group, key) {
            parts.push(format!("{v} [Qt]"));
        }
    }
    if let Some((v, tag)) = gtk_setting(ctx, gtk_key) {
        parts.push(format!("{v} [{tag}]"));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn font(ctx: &Ctx) -> Option<String> {
    let (raw, tag) = gtk_setting(ctx, "gtk-font-name")?;
    // "Noto Sans 10" -> "Noto Sans (10pt)"
    Some(match raw.rsplit_once(' ') {
        Some((name, size)) if size.parse::<f32>().is_ok() => format!("{name} ({size}pt) [{tag}]"),
        _ => format!("{raw} [{tag}]"),
    })
}

fn cursor(ctx: &Ctx) -> Option<String> {
    let name = sys::env("XCURSOR_THEME").or_else(|| gtk_setting(ctx, "gtk-cursor-theme-name").map(|v| v.0))?;
    let size = sys::env("XCURSOR_SIZE").or_else(|| gtk_setting(ctx, "gtk-cursor-theme-size").map(|v| v.0));
    Some(match size {
        Some(s) => format!("{name} ({s}px)"),
        None => name,
    })
}

// ---- hardware --------------------------------------------------------------

pub fn clean_cpu_name(raw: &str) -> String {
    let mut s = raw.to_owned();
    for junk in ["(R)", "(r)", "(TM)", "(tm)"] {
        s = s.replace(junk, "");
    }
    if let Some(i) = s.find(" @ ") {
        s.truncate(i);
    }
    for junk in [
        " with Radeon Graphics",
        " with Radeon Vega Mobile Gfx",
        " with Radeon Vega Graphics",
        " Processor",
        " CPU",
    ] {
        s = s.replace(junk, "");
    }
    s.split_whitespace()
        .filter(|t| !t.ends_with("-Core"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn cpu() -> Option<String> {
    let info = read("/proc/cpuinfo")?;
    let field = |k: &str| {
        info.lines()
            .find(|l| l.split(':').next().map(str::trim) == Some(k))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let name = field("model name")
        .or_else(|| field("Hardware"))
        .or_else(|| field("cpu model"))
        .or_else(|| field("Model"))
        .map(|n| clean_cpu_name(&n))
        .unwrap_or_else(|| "Unknown CPU".into());

    let threads = info.lines().filter(|l| l.starts_with("processor")).count();
    let threads = if threads > 0 {
        threads
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    };

    // Highest max clock across cores (hybrid CPUs differ per core).
    let max_khz = fs::read_dir("/sys/devices/system/cpu")
        .ok()
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| {
                    let n = e.file_name();
                    let n = n.to_string_lossy();
                    n.starts_with("cpu") && n[3..].chars().all(|c| c.is_ascii_digit()) && n.len() > 3
                })
                .filter_map(|e| read_trim(e.path().join("cpufreq/cpuinfo_max_freq")))
                .filter_map(|s| s.parse::<u64>().ok())
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let ghz = if max_khz > 0 {
        Some(max_khz as f64 / 1e6)
    } else {
        field("cpu MHz").and_then(|m| m.parse::<f64>().ok()).map(|m| m / 1000.0)
    };

    Some(match ghz {
        Some(g) => format!("{name} ({threads}) @ {g:.2} GHz"),
        None => format!("{name} ({threads})"),
    })
}

/// Looks up a vendor/device pair in pci.ids text.
pub fn pci_lookup(db: &str, vendor: &str, device: &str) -> (Option<String>, Option<String>) {
    let vpat = format!("\n{vendor}  ");
    let Some(vpos) = db.find(&vpat) else { return (None, None) };
    let after = &db[vpos + 1..];
    let mut lines = after.lines();
    let vendor_name = lines.next().map(|l| l[vendor.len()..].trim().to_owned());
    let dpat = format!("\t{device}  ");
    for line in lines {
        if line.starts_with('#') || line.starts_with("\t\t") || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            break; // next vendor
        }
        if let Some(name) = line.strip_prefix(&dpat) {
            return (vendor_name, Some(name.trim().to_owned()));
        }
    }
    (vendor_name, None)
}

fn short_vendor(id: &str, db_name: Option<&str>) -> String {
    match id {
        "8086" => "Intel".into(),
        "10de" => "NVIDIA".into(),
        "1002" | "1022" => "AMD".into(),
        "1af4" => "Virtio".into(),
        "15ad" => "VMware".into(),
        "1234" => "QEMU".into(),
        "80ee" => "VirtualBox".into(),
        "5143" => "Qualcomm".into(),
        _ => db_name.unwrap_or("Unknown").split_whitespace().next().unwrap_or("Unknown").to_owned(),
    }
}

/// "TU106 [GeForce RTX 2060]" -> "GeForce RTX 2060"
pub fn clean_gpu_name(name: &str) -> String {
    match (name.rfind('['), name.rfind(']')) {
        (Some(a), Some(b)) if b > a + 1 => name[a + 1..b].to_owned(),
        _ => name.to_owned(),
    }
}

fn gpus(label: &str) -> Vec<Entry> {
    let Ok(dir) = fs::read_dir("/sys/bus/pci/devices") else { return Vec::new() };
    let mut found: Vec<(String, String, Option<String>)> = Vec::new();
    for e in dir.filter_map(|e| e.ok()) {
        let p = e.path();
        let Some(class) = read_trim(p.join("class")) else { continue };
        if !class.starts_with("0x03") {
            continue;
        }
        let id = |f: &str| read_trim(p.join(f)).map(|s| s.trim_start_matches("0x").to_ascii_lowercase());
        let (Some(v), Some(d)) = (id("vendor"), id("device")) else { continue };
        let driver = fs::read_link(p.join("driver"))
            .ok()
            .and_then(|l| l.file_name().map(|f| f.to_string_lossy().into_owned()));
        found.push((v, d, driver));
    }
    if found.is_empty() {
        return Vec::new();
    }
    let db = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"]
        .iter()
        .find_map(|p| read(p))
        .unwrap_or_default();

    let multi = found.len() > 1;
    found
        .into_iter()
        .enumerate()
        .map(|(i, (v, d, driver))| {
            let (vname, dname) = pci_lookup(&db, &v, &d);
            let vendor = short_vendor(&v, vname.as_deref());
            let value = match dname {
                Some(n) => format!("{vendor} {}", clean_gpu_name(&n)),
                None => match driver {
                    Some(drv) => format!("{vendor} GPU {d} ({drv})"),
                    None => format!("{vendor} GPU {d}"),
                },
            };
            let key = if multi { format!("{label} {}", i + 1) } else { label.to_owned() };
            Entry::text(key, value)
        })
        .collect()
}

fn meminfo() -> HashMap<String, u64> {
    read("/proc/meminfo")
        .map(|t| {
            t.lines()
                .filter_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    let kb: u64 = v.split_whitespace().next()?.parse().ok()?;
                    Some((k.to_owned(), kb * 1024))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn usage_entry(key: String, used: u64, total: u64, suffix: Option<String>) -> Entry {
    let pct = if total > 0 { used as f64 / total as f64 * 100.0 } else { 0.0 };
    let mut value = vec![
        Seg::plain(format!("{} / {} (", sys::fmt_bytes(used), sys::fmt_bytes(total))),
        percent(pct, false),
        Seg::plain(")"),
    ];
    if let Some(s) = suffix {
        value.push(Seg::plain(s));
    }
    Entry { key, value }
}

fn memory(label: &str, swap: bool) -> Vec<Entry> {
    let m = meminfo();
    let (total, free) = if swap {
        (m.get("SwapTotal").copied().unwrap_or(0), m.get("SwapFree").copied().unwrap_or(0))
    } else {
        let total = m.get("MemTotal").copied().unwrap_or(0);
        let avail = m
            .get("MemAvailable")
            .copied()
            .unwrap_or_else(|| m.get("MemFree").copied().unwrap_or(0));
        (total, avail)
    };
    if swap && total == 0 {
        return vec![Entry::text(label, "Disabled")];
    }
    if total == 0 {
        return Vec::new();
    }
    vec![usage_entry(label.to_owned(), total.saturating_sub(free), total, None)]
}

const REAL_FS: &[&str] = &[
    "ext2", "ext3", "ext4", "btrfs", "xfs", "zfs", "f2fs", "bcachefs", "jfs", "reiserfs", "ntfs",
    "ntfs3", "fuseblk", "exfat", "vfat", "nfs", "nfs4", "cifs", "smb3", "fuse.sshfs", "apfs", "hfsplus",
];
const HIDDEN_MOUNTS: &[&str] = &["/boot", "/efi", "/snap", "/var/lib/docker", "/var/lib/containers", "/proc", "/sys", "/dev", "/tmp"];

fn unescape_mount(s: &str) -> String {
    s.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\")
}

fn disks(label: &str) -> Vec<Entry> {
    let Some(mounts) = read("/proc/mounts") else { return Vec::new() };
    let mut seen_dev: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in mounts.lines() {
        let mut f = line.split_whitespace();
        let (Some(dev), Some(mnt), Some(fs_type)) = (f.next(), f.next(), f.next()) else { continue };
        let mnt = unescape_mount(mnt);
        let is_root = mnt == "/";
        if !is_root {
            if !REAL_FS.contains(&fs_type) {
                continue;
            }
            if HIDDEN_MOUNTS.iter().any(|h| mnt == *h || mnt.starts_with(&format!("{h}/"))) {
                continue;
            }
            if mnt.starts_with("/run") && !mnt.starts_with("/run/media") {
                continue;
            }
        }
        // btrfs subvolumes (/, /home, /var/log...) share one device: show it once.
        if dev.starts_with('/') && seen_dev.iter().any(|d| d == dev) {
            continue;
        }
        let Some((total, avail)) = sys::statvfs(&mnt) else { continue };
        if total == 0 {
            continue;
        }
        seen_dev.push(dev.to_owned());
        out.push(usage_entry(
            format!("{label} ({mnt})"),
            total.saturating_sub(avail),
            total,
            Some(format!(" - {fs_type}")),
        ));
    }
    out
}

fn default_iface() -> Option<String> {
    let route = read("/proc/net/route")?;
    route.lines().skip(1).find_map(|l| {
        let mut f = l.split_whitespace();
        let iface = f.next()?;
        (f.next()? == "00000000").then(|| iface.to_owned())
    })
}

fn local_ip(label: &str) -> Vec<Entry> {
    let addrs = sys::ipv4_addrs();
    let def = default_iface();
    let pick = addrs
        .iter()
        .find(|(n, _, _)| Some(n) == def.as_ref())
        .or_else(|| addrs.iter().find(|(_, ip, _)| !ip.is_loopback()));
    match pick {
        Some((iface, ip, prefix)) => vec![Entry::text(format!("{label} ({iface})"), format!("{ip}/{prefix}"))],
        None => Vec::new(),
    }
}

fn batteries(label: &str) -> Vec<Entry> {
    let Ok(dir) = fs::read_dir("/sys/class/power_supply") else { return Vec::new() };
    let mut bats: Vec<(String, f64, String)> = dir
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if read_trim(p.join("type"))? != "Battery" {
                return None;
            }
            // Skip mice, headsets and other peripherals.
            if read_trim(p.join("scope")).as_deref() == Some("Device") {
                return None;
            }
            let cap: f64 = read_trim(p.join("capacity"))?.parse().ok()?;
            let status = read_trim(p.join("status")).unwrap_or_default();
            Some((e.file_name().to_string_lossy().into_owned(), cap, status))
        })
        .collect();
    bats.sort_by(|a, b| a.0.cmp(&b.0));
    let multi = bats.len() > 1;
    bats.into_iter()
        .map(|(name, cap, status)| {
            let mut value = vec![percent(cap, true)];
            if !status.is_empty() && status != "Unknown" {
                value.push(Seg::plain(format!(" [{status}]")));
            }
            let key = if multi { format!("{label} ({name})") } else { label.to_owned() };
            Entry { key, value }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_names() {
        assert_eq!(
            clean_cpu_name("Intel(R) Core(TM) i5-8350U CPU @ 1.70GHz"),
            "Intel Core i5-8350U"
        );
        assert_eq!(
            clean_cpu_name("AMD Ryzen 7 5800X 8-Core Processor"),
            "AMD Ryzen 7 5800X"
        );
        assert_eq!(
            clean_cpu_name("AMD Ryzen 7 7840U w/ Radeon 780M Graphics"),
            "AMD Ryzen 7 7840U w/ Radeon 780M Graphics"
        );
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("GNU bash, version 5.2.37(1)-release (x86_64-pc-linux-gnu)").as_deref(), Some("5.2.37"));
        assert_eq!(parse_version("zsh 5.9 (x86_64-pc-linux-gnu)").as_deref(), Some("5.9"));
        assert_eq!(parse_version("fish, version 4.0.2").as_deref(), Some("4.0.2"));
        assert_eq!(parse_version("0.104.0\n").as_deref(), Some("0.104.0"));
        assert_eq!(parse_version("dash: 0: Illegal option --"), None);
    }

    #[test]
    fn pci() {
        let db = "# c\n8086  Intel Corporation\n\t5916  HD Graphics 620\n\t5917  UHD Graphics 620\n\t\t17aa 225e  ThinkPad\n10de  NVIDIA Corporation\n\t1f08  TU106 [GeForce RTX 2060 Rev. A]\n";
        let (v, d) = pci_lookup(db, "8086", "5917");
        assert_eq!(v.as_deref(), Some("Intel Corporation"));
        assert_eq!(d.as_deref(), Some("UHD Graphics 620"));
        let (_, d) = pci_lookup(db, "10de", "1f08");
        assert_eq!(clean_gpu_name(&d.unwrap()), "GeForce RTX 2060 Rev. A");
        assert_eq!(pci_lookup(db, "8086", "1f08").1, None);
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(59), "0 mins");
        assert_eq!(fmt_duration(3600 + 60), "1 hour, 1 min");
        assert_eq!(fmt_duration(2 * 86400 + 5 * 60), "2 days, 5 mins");
    }

    #[test]
    fn host_junk() {
        assert!(junk("To Be Filled By O.E.M."));
        assert!(junk("1.0"));
        assert!(!junk("ThinkPad T480s"));
    }
}
