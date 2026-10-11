//! Terminal colors and styled text segments.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// One of the 256 palette colors; 0-15 follow the terminal's theme.
    Ansi(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    pub const RED: Color = Color::Ansi(1);
    pub const GREEN: Color = Color::Ansi(2);
    pub const YELLOW: Color = Color::Ansi(3);
    pub const GRAY: Color = Color::Ansi(8);

    /// Accepts color names (`cyan`, `bright_blue`), palette numbers
    /// (`0`-`255`) and hex (`#8747ff`, `8747ff`, `#fff`).
    pub fn parse(s: &str) -> Option<Color> {
        let s = s.trim().to_ascii_lowercase();
        const NAMES: [&str; 8] = [
            "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
        ];
        if let Some(i) = NAMES.iter().position(|n| *n == s) {
            return Some(Color::Ansi(i as u8));
        }
        if let Some(rest) = s.strip_prefix("bright_").or_else(|| s.strip_prefix("bright-")) {
            if let Some(i) = NAMES.iter().position(|n| *n == rest) {
                return Some(Color::Ansi(i as u8 + 8));
            }
        }
        if s == "gray" || s == "grey" {
            return Some(Color::GRAY);
        }
        if let Ok(n) = s.parse::<u8>() {
            return Some(Color::Ansi(n));
        }
        let hex = s.strip_prefix('#').unwrap_or(&s);
        let expand = |h: &str| -> Option<u8> { u8::from_str_radix(h, 16).ok() };
        match hex.len() {
            6 => Some(Color::Rgb(
                expand(&hex[0..2])?,
                expand(&hex[2..4])?,
                expand(&hex[4..6])?,
            )),
            3 => {
                let c = |i: usize| expand(&hex[i..i + 1]).map(|v| v * 17);
                Some(Color::Rgb(c(0)?, c(1)?, c(2)?))
            }
            _ => None,
        }
    }

    fn fg(self, out: &mut String) {
        match self {
            Color::Ansi(n) if n < 8 => out.push_str(&(30 + n as u16).to_string()),
            Color::Ansi(n) if n < 16 => out.push_str(&(90 + n as u16 - 8).to_string()),
            Color::Ansi(n) => out.push_str(&format!("38;5;{n}")),
            Color::Rgb(r, g, b) => out.push_str(&format!("38;2;{r};{g};{b}")),
        }
    }

    fn bg(self, out: &mut String) {
        match self {
            Color::Ansi(n) if n < 8 => out.push_str(&(40 + n as u16).to_string()),
            Color::Ansi(n) if n < 16 => out.push_str(&(100 + n as u16 - 8).to_string()),
            Color::Ansi(n) => out.push_str(&format!("48;5;{n}")),
            Color::Rgb(r, g, b) => out.push_str(&format!("48;2;{r};{g};{b}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
}

impl Style {
    pub fn fg(c: Color) -> Style {
        Style { fg: Some(c), ..Style::default() }
    }
    pub fn bold(self) -> Style {
        Style { bold: true, ..self }
    }
    pub fn is_plain(&self) -> bool {
        *self == Style::default()
    }
}

#[derive(Clone, Debug)]
pub struct Seg {
    pub text: String,
    pub style: Style,
}

impl Seg {
    pub fn plain(text: impl Into<String>) -> Seg {
        Seg { text: text.into(), style: Style::default() }
    }
    pub fn styled(text: impl Into<String>, style: Style) -> Seg {
        Seg { text: text.into(), style }
    }
    pub fn width(&self) -> usize {
        self.text.chars().count()
    }
}

pub fn line_width(line: &[Seg]) -> usize {
    line.iter().map(Seg::width).sum()
}

#[cfg(test)]
pub fn line_text(line: &[Seg]) -> String {
    line.iter().map(|s| s.text.as_str()).collect()
}

/// Appends a segment to `out`, wrapped in SGR escapes when color is on.
pub fn paint(out: &mut String, seg: &Seg, color: bool) {
    if !color || seg.style.is_plain() || seg.text.is_empty() {
        out.push_str(&seg.text);
        return;
    }
    out.push_str("\x1b[");
    let mut first = true;
    let mut sep = |out: &mut String| {
        if !first {
            out.push(';');
        }
        first = false;
    };
    if seg.style.bold {
        sep(out);
        out.push('1');
    }
    if seg.style.dim {
        sep(out);
        out.push('2');
    }
    if let Some(c) = seg.style.fg {
        sep(out);
        c.fg(out);
    }
    if let Some(c) = seg.style.bg {
        sep(out);
        c.bg(out);
    }
    out.push('m');
    out.push_str(&seg.text);
    out.push_str("\x1b[0m");
}

/// A percentage colored like fastfetch: green / yellow / red.
/// `higher_is_better` flips the scale (battery charge).
pub fn percent(value: f64, higher_is_better: bool) -> Seg {
    let v = if higher_is_better { 100.0 - value } else { value };
    let c = if v < 50.0 {
        Color::GREEN
    } else if v < 80.0 {
        Color::YELLOW
    } else {
        Color::RED
    };
    Seg::styled(format!("{value:.0}%"), Style::fg(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colors() {
        assert_eq!(Color::parse("cyan"), Some(Color::Ansi(6)));
        assert_eq!(Color::parse("bright_blue"), Some(Color::Ansi(12)));
        assert_eq!(Color::parse("#8747FF"), Some(Color::Rgb(0x87, 0x47, 0xff)));
        assert_eq!(Color::parse("fff"), Some(Color::Rgb(255, 255, 255)));
        assert_eq!(Color::parse("208"), Some(Color::Ansi(208)));
        assert_eq!(Color::parse("nope"), None);
    }

    #[test]
    fn paints_sgr() {
        let mut s = String::new();
        paint(&mut s, &Seg::styled("x", Style::fg(Color::Ansi(12)).bold()), true);
        assert_eq!(s, "\x1b[1;94mx\x1b[0m");
        s.clear();
        paint(&mut s, &Seg::styled("x", Style::fg(Color::RED)), false);
        assert_eq!(s, "x");
    }
}
