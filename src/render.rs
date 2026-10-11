//! Turns gathered info into lines and lays them out beside the logo.

use crate::color::{line_width, paint, Color, Seg, Style};
use crate::logo::Logo;
use crate::modules::Entry;
use std::time::Duration;

pub enum Item {
    Title,
    Separator,
    Break,
    Colors,
    Section(String),
    Info(String),
}

impl Item {
    pub fn parse(s: &str) -> Item {
        match s {
            "title" => Item::Title,
            "separator" => Item::Separator,
            "break" => Item::Break,
            "colors" => Item::Colors,
            _ => match s.strip_prefix("section:") {
                Some(name) => Item::Section(name.to_owned()),
                None => Item::Info(s.to_owned()),
            },
        }
    }
}

pub struct Gathered {
    pub entries: Vec<Entry>,
    pub time: Duration,
}

pub struct Opts {
    pub tree: bool,
    pub key: Color,
    pub sep: String,
    pub stat: bool,
    pub color: bool,
    pub user: String,
    pub host: String,
}

enum Row {
    Plain(Vec<Seg>),
    Entry(Vec<Seg>, Vec<Seg>), // key, value
    Section(String, Vec<(Vec<Seg>, Vec<Seg>)>),
}

fn stat_seg(t: Duration) -> Seg {
    Seg::styled(format!("  {:.2}ms", t.as_secs_f64() * 1000.0), Style { dim: true, ..Style::default() })
}

fn entry_kv(e: &Entry, first: bool, g: &Gathered, o: &Opts) -> (Vec<Seg>, Vec<Seg>) {
    let mut value = e.value.clone();
    if o.stat && first {
        value.push(stat_seg(g.time));
    }
    (vec![Seg::plain(e.key.clone())], value)
}

/// `results[i]` holds the gathered data for `items[i]` when it is Info.
pub fn info_lines(items: &[Item], results: &[Option<Gathered>], o: &Opts) -> Vec<Vec<Seg>> {
    let key_style = Style::fg(o.key).bold();
    let title = vec![
        Seg::styled(o.user.clone(), key_style),
        Seg::plain("@"),
        Seg::styled(o.host.clone(), key_style),
    ];
    let title_w = line_width(&title);

    // Group into rows; in tree style, info after a section header belongs to it.
    let mut rows: Vec<Row> = Vec::new();
    for (item, res) in items.iter().zip(results) {
        match item {
            Item::Title => rows.push(Row::Plain(title.clone())),
            Item::Separator => rows.push(Row::Plain(vec![Seg::plain("-".repeat(title_w))])),
            Item::Break => rows.push(Row::Plain(vec![])),
            Item::Colors => {
                if o.color {
                    for base in [0u8, 8] {
                        let line = (base..base + 8)
                            .map(|c| Seg::styled("   ", Style { bg: Some(Color::Ansi(c)), ..Style::default() }))
                            .collect();
                        rows.push(Row::Plain(line));
                    }
                }
            }
            Item::Section(name) => rows.push(Row::Section(name.clone(), Vec::new())),
            Item::Info(_) => {
                let Some(g) = res else { continue };
                for (i, e) in g.entries.iter().enumerate() {
                    let kv = entry_kv(e, i == 0, g, o);
                    match rows.last_mut() {
                        Some(Row::Section(_, children)) if o.tree => children.push(kv),
                        _ => rows.push(Row::Entry(kv.0, kv.1)),
                    }
                }
            }
        }
    }

    if o.tree {
        rows.retain(|r| !matches!(r, Row::Section(_, c) if c.is_empty()));
        tree_lines(rows, o)
    } else {
        classic_lines(rows, o)
    }
}

fn classic_lines(rows: Vec<Row>, o: &Opts) -> Vec<Vec<Seg>> {
    let key_style = Style::fg(o.key).bold();
    let mut out = Vec::new();
    for row in rows {
        match row {
            Row::Plain(l) => out.push(l),
            Row::Section(name, _) => out.push(vec![Seg::styled(name, key_style)]),
            Row::Entry(k, v) => {
                let mut l: Vec<Seg> = k.into_iter().map(|s| Seg::styled(s.text, key_style)).collect();
                l.push(Seg::plain(o.sep.clone()));
                l.extend(v);
                out.push(l);
            }
        }
    }
    out
}

fn tree_lines(rows: Vec<Row>, o: &Opts) -> Vec<Vec<Seg>> {
    let glyph = Style::fg(o.key);
    let head = Style::fg(o.key).bold();
    let key_w = rows
        .iter()
        .filter_map(|r| match r {
            Row::Section(_, c) => c.iter().map(|(k, _)| line_width(k)).max(),
            _ => None,
        })
        .max()
        .unwrap_or(0);

    let section_count = rows.iter().filter(|r| matches!(r, Row::Section(..))).count();
    let mut seen = 0;
    let mut out = Vec::new();
    for row in rows {
        match row {
            Row::Plain(l) => out.push(l),
            Row::Entry(k, v) => {
                let mut l: Vec<Seg> = k.into_iter().map(|s| Seg::styled(s.text, head)).collect();
                l.push(Seg::plain(o.sep.clone()));
                l.extend(v);
                out.push(l);
            }
            Row::Section(name, children) => {
                let last = seen + 1 == section_count;
                let corner = if seen == 0 { "┌─ " } else if last { "└─ " } else { "├─ " };
                if seen > 0 {
                    out.push(vec![Seg::styled("│", glyph)]);
                }
                out.push(vec![Seg::styled(corner, glyph), Seg::styled(name, head)]);
                let rail = if last { "   " } else { "│  " };
                let n = children.len();
                for (i, (k, v)) in children.into_iter().enumerate() {
                    let branch = if i + 1 == n { "└─ " } else { "├─ " };
                    let kw = line_width(&k);
                    let mut l = vec![Seg::styled(format!("{rail}{branch}"), glyph)];
                    l.extend(k);
                    l.push(Seg::plain(" ".repeat(key_w - kw + 2)));
                    l.extend(v);
                    out.push(l);
                }
                seen += 1;
            }
        }
    }
    out
}

/// Cuts a line to `max` visible columns, ending with an ellipsis.
fn truncate(line: &[Seg], max: usize) -> Vec<Seg> {
    if line_width(line) <= max {
        return line.to_vec();
    }
    let mut out = Vec::new();
    let mut left = max.saturating_sub(1);
    for seg in line {
        if left == 0 {
            break;
        }
        let w = seg.width();
        if w <= left {
            out.push(seg.clone());
            left -= w;
        } else {
            out.push(Seg::styled(seg.text.chars().take(left).collect::<String>(), seg.style));
            left = 0;
        }
    }
    out.push(Seg::plain("…"));
    out
}

/// Places the logo and info side by side. Each returned line is the full
/// row as segments; used both for printing and as the rain's centerpiece.
pub fn layout(logo: Option<&Logo>, gap: usize, info: &[Vec<Seg>], width: Option<usize>) -> Vec<Vec<Seg>> {
    let left = logo.map(|l| l.width + gap).unwrap_or(0);
    let avail = width.map(|w| w.saturating_sub(left));
    let logo_rows = logo.map(|l| l.lines.len()).unwrap_or(0);
    let rows = logo_rows.max(info.len());
    let mut out = Vec::with_capacity(rows);

    for i in 0..rows {
        let mut row: Vec<Seg> = Vec::new();
        if let Some(l) = logo {
            let segs = l.lines.get(i).map(Vec::as_slice).unwrap_or(&[]);
            row.extend_from_slice(segs);
            if i < info.len() {
                row.push(Seg::plain(" ".repeat(left - line_width(segs))));
            }
        }
        if let Some(line) = info.get(i) {
            match avail {
                Some(a) if a > 1 => row.extend(truncate(line, a)),
                _ => row.extend_from_slice(line),
            }
        }
        out.push(row);
    }
    out
}

pub fn paint_lines(lines: &[Vec<Seg>], color: bool) -> String {
    let mut out = String::with_capacity(lines.len() * 96);
    for line in lines {
        for s in line {
            paint(&mut out, s, color);
        }
        // Drop trailing padding (a logo shorter than the info column).
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    out
}

pub fn json(items: &[Item], results: &[Option<Gathered>]) -> String {
    fn esc(s: &str) -> String {
        let mut o = String::with_capacity(s.len() + 2);
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
        o
    }
    let mut parts = Vec::new();
    for (item, res) in items.iter().zip(results) {
        if let (Item::Info(name), Some(g)) = (item, res) {
            for e in &g.entries {
                let value: String = e.value.iter().map(|s| s.text.as_str()).collect();
                parts.push(format!(
                    "  {{\"module\": \"{}\", \"key\": \"{}\", \"value\": \"{}\"}}",
                    esc(name),
                    esc(&e.key),
                    esc(&value)
                ));
            }
        }
    }
    format!("[\n{}\n]\n", parts.join(",\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_with_ellipsis() {
        let l = vec![Seg::plain("abc"), Seg::plain("defgh")];
        let t = truncate(&l, 5);
        assert_eq!(crate::color::line_text(&t), "abcd…");
        assert_eq!(line_width(&t), 5);
    }

    #[test]
    fn logo_and_info_align() {
        let logo = crate::logo::parse("ab\nabcd\n", &[Color::Ansi(1)]);
        let info = vec![vec![Seg::plain("x")], vec![Seg::plain("y")], vec![Seg::plain("z")]];
        let out = paint_lines(&layout(Some(&logo), 2, &info, None), false);
        assert_eq!(out, "ab    x\nabcd  y\n      z\n");
    }
}
