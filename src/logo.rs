//! Built-in ASCII logos (fastfetch `$N` color-marker format).

use crate::color::{Color, Seg, Style};
use std::collections::HashMap;

include!(concat!(env!("OUT_DIR"), "/logos.rs"));

pub fn names() -> impl Iterator<Item = &'static str> {
    LOGOS.iter().map(|(n, _)| *n)
}

pub fn builtin(name: &str) -> Option<&'static str> {
    LOGOS
        .binary_search_by_key(&name, |(n, _)| *n)
        .ok()
        .map(|i| LOGOS[i].1)
}

pub struct Logo {
    pub lines: Vec<Vec<Seg>>,
    pub width: usize,
}

/// Turns raw logo text into colored lines. `$1`..`$9` switch to the
/// matching palette entry (wrapping around), and the color carries over
/// into following lines until the next marker; `$$` is a literal `$`.
pub fn parse(src: &str, colors: &[Color]) -> Logo {
    let palette: &[Color] = if colors.is_empty() { &[Color::Ansi(7)] } else { colors };
    let mut style = Style::fg(palette[0]).bold();
    let mut lines = Vec::new();

    for raw in src.trim_end_matches(['\n', '\r']).lines() {
        let raw = raw.replace('\t', "    ");
        let mut segs: Vec<Seg> = Vec::new();
        let mut buf = String::new();
        let mut chars = raw.chars().peekable();

        while let Some(c) = chars.next() {
            if c != '$' {
                buf.push(c);
                continue;
            }
            match chars.peek().copied() {
                Some('$') => {
                    chars.next();
                    buf.push('$');
                }
                Some(d @ '1'..='9') => {
                    chars.next();
                    if !buf.is_empty() {
                        segs.push(Seg::styled(std::mem::take(&mut buf), style));
                    }
                    let idx = (d as usize - '1' as usize) % palette.len();
                    style = Style::fg(palette[idx]).bold();
                }
                _ => buf.push('$'),
            }
        }
        if !buf.is_empty() {
            segs.push(Seg::styled(buf, style));
        }
        lines.push(segs);
    }

    let width = lines.iter().map(|l| crate::color::line_width(l)).max().unwrap_or(0);
    Logo { lines, width }
}

/// Picks a built-in logo from /etc/os-release: `ID`, then each `ID_LIKE`.
pub fn detect(os: &HashMap<String, String>) -> String {
    if cfg!(target_os = "macos") {
        return "macos".into();
    }
    let mut candidates: Vec<String> = Vec::new();
    if let Some(id) = os.get("ID") {
        candidates.push(id.clone());
    }
    if let Some(like) = os.get("ID_LIKE") {
        candidates.extend(like.split_whitespace().map(str::to_owned));
    }
    for c in candidates {
        let c = c.to_ascii_lowercase().replace('-', "_");
        if builtin(&c).is_some() {
            return c;
        }
        // e.g. "opensuse_tumbleweed" -> "opensuse"
        if let Some((base, _)) = c.split_once('_') {
            if builtin(base).is_some() {
                return base.to_owned();
            }
        }
    }
    "linux".into()
}

/// Default palette per distro, roughly matching fastfetch. Numbers are the
/// terminal's own 16 colors, so logos follow the user's color scheme.
pub fn default_colors(name: &str) -> Vec<Color> {
    let base = name
        .trim_end_matches("_small")
        .trim_end_matches("_old")
        .trim_end_matches(|c: char| c.is_ascii_digit());
    let ansi: &[u8] = match base {
        "arch" | "archcraft" | "archlabs" | "arcolinux" | "artix" | "artix_small" => &[6, 6, 6],
        "cachyos" => &[6, 2, 8],
        "gentoo" | "funtoo" => &[5, 7],
        "void" => &[2, 8],
        "nixos" => &[4, 6],
        "debian" | "devuan" => &[1, 7, 3],
        "ubuntu" | "kubuntu" | "xubuntu" | "lubuntu" | "ubuntu_budgie" => &[1, 7],
        "fedora" | "nobara" => &[4, 7],
        "endeavouros" => &[5, 1, 4],
        "manjaro" => &[2, 2],
        "linuxmint" | "mint" => &[2, 7],
        "pop" => &[6, 7],
        "opensuse" | "opensuse_tumbleweed" | "opensuse_leap" | "opensuse_microos" => &[2, 7],
        "alpine" => &[4, 7],
        "kali" => &[4, 8],
        "garuda" => &[1, 3, 5],
        "slackware" => &[4, 7],
        "elementary" | "mx" => &[7, 7],
        "zorin" => &[4, 4],
        "centos" => &[3, 2, 4, 5, 7],
        "rocky" => &[2, 2],
        "almalinux" => &[1, 3, 4, 2, 6],
        "rhel" | "redhat" => &[1, 7],
        "freebsd" => &[1, 7],
        "openbsd" => &[3, 7, 6, 1, 8],
        "macos" => &[2, 3, 1, 1, 5, 4],
        "android" => &[2, 7],
        "raspbian" => &[2, 1],
        "linux" => &[7, 8, 3],
        _ => &[4, 6, 5, 2, 3, 1],
    };
    ansi.iter().map(|&n| Color::Ansi(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_and_carry_over() {
        let c = [Color::Ansi(1), Color::Ansi(2)];
        let logo = parse("a$2b\nc$$d$1e\n", &c);
        assert_eq!(logo.lines.len(), 2);
        assert_eq!(logo.lines[0][0].style.fg, Some(Color::Ansi(1)));
        assert_eq!(logo.lines[0][1].text, "b");
        // color 2 carries into the next line
        assert_eq!(logo.lines[1][0].text, "c$d");
        assert_eq!(logo.lines[1][0].style.fg, Some(Color::Ansi(2)));
        assert_eq!(logo.lines[1][1].style.fg, Some(Color::Ansi(1)));
        assert_eq!(logo.width, 4);
    }

    #[test]
    fn palette_wraps() {
        let logo = parse("$5x", &[Color::Ansi(1), Color::Ansi(2)]);
        assert_eq!(logo.lines[0][0].style.fg, Some(Color::Ansi(1)));
    }

    #[test]
    fn table_is_sorted_and_has_common_logos() {
        assert!(LOGOS.windows(2).all(|w| w[0].0 < w[1].0));
        for n in ["arch", "artix", "cachyos", "gentoo", "void", "linux"] {
            assert!(builtin(n).is_some(), "{n}");
        }
    }

    #[test]
    fn detects_via_id_like() {
        let mut os = HashMap::new();
        os.insert("ID".into(), "somethingnew".into());
        os.insert("ID_LIKE".into(), "arch".into());
        assert_eq!(detect(&os), "arch");
    }
}
