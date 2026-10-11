//! `drip --drip`: the fetch centered on screen with rain falling around it.
//!
//! Every frame: read keys, move the drops, paint everything into a grid of
//! cells, then send the terminal only the cells that changed since the
//! last frame.

use crate::color::{sgr, Color, Seg, Style};
use crate::config::Rain as RainConfig;
use crate::fetch::{parse_colors, Fetch};
use crate::render;
use crate::sys;
use crate::term::{self, RawTerm, STOP};
use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Collision {
    Box,
    Shape,
    None,
}

struct Settings {
    intensity: f32,
    chars: Vec<char>,
    colors: Vec<Color>,
    speed: (f32, f32),
    length: (f32, f32),
    collision: Collision,
    frame: Duration,
    refresh: Duration,
}

impl Settings {
    fn from(cfg: &RainConfig, logo_colors: &[Color]) -> Settings {
        let chars: Vec<char> = cfg.chars.iter().filter_map(|s| s.chars().next()).collect();
        let mut colors = parse_colors(&cfg.colors, "rain.colors");
        if colors.is_empty() {
            colors = logo_colors.to_vec();
        }
        let collision = match cfg.collision.as_deref().unwrap_or("box") {
            "shape" => Collision::Shape,
            "none" => Collision::None,
            "box" => Collision::Box,
            other => {
                eprintln!("drip: unknown rain.collision '{other}', using box");
                Collision::Box
            }
        };
        let [smin, smax] = cfg.speed.unwrap_or([10.0, 35.0]);
        let [lmin, lmax] = cfg.length.unwrap_or([4, 16]);
        Settings {
            intensity: cfg.intensity.unwrap_or(60.0).max(0.0),
            chars: if chars.is_empty() { vec!['│'] } else { chars },
            colors,
            speed: (smin.max(0.5), smax.max(smin.max(0.5))),
            length: (lmin.max(1) as f32, lmax.max(lmin.max(1)) as f32),
            collision,
            frame: Duration::from_secs_f64(1.0 / cfg.fps.unwrap_or(30).clamp(1, 240) as f64),
            refresh: Duration::from_secs_f32(cfg.refresh.unwrap_or(2.0).max(0.2)),
        }
    }
}

/// xorshift64*: tiny, fast, and plenty random for rain.
struct Rng(u64);

impl Rng {
    fn seeded() -> Rng {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        Rng(t | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next() % v.len() as u64) as usize]
    }
}

/// One falling streak. `head` and `tail` are row positions (fractional so
/// slow drops still move smoothly); the drop covers tail..=head.
struct Drop {
    x: usize,
    head: f32,
    tail: f32,
    speed: f32,
    ch: char,
    color: Color,
}

#[derive(Clone, Copy, PartialEq)]
struct Cell {
    ch: char,
    style: Style,
}

const BLANK: Cell = Cell { ch: ' ', style: Style { fg: None, bg: None, bold: false, dim: false } };

/// The fetch, flattened into cells and placed on screen.
struct Block {
    x0: usize,
    y0: usize,
    w: usize,
    cells: Vec<Vec<Cell>>,
}

impl Block {
    fn build(lines: &[Vec<Seg>], cols: usize, rows: usize) -> Block {
        let cells: Vec<Vec<Cell>> = lines
            .iter()
            .map(|l| {
                l.iter()
                    .flat_map(|s| s.text.chars().map(move |ch| Cell { ch, style: s.style }))
                    .collect()
            })
            .collect();
        let w = cells.iter().map(Vec::len).max().unwrap_or(0);
        let h = cells.len();
        Block { x0: cols.saturating_sub(w) / 2, y0: rows.saturating_sub(h) / 2, w, cells }
    }

    /// Is there something at (x, y) for rain to land on? A single space
    /// between two characters counts too, so drops don't fall between words.
    fn solid(&self, x: usize, y: usize) -> bool {
        if y < self.y0 || x < self.x0 {
            return false;
        }
        let Some(row) = self.cells.get(y - self.y0) else { return false };
        let c = x - self.x0;
        let ink = |i: usize| row.get(i).is_some_and(|cell| cell.ch != ' ');
        ink(c) || (c > 0 && ink(c - 1) && ink(c + 1))
    }

    /// For each column, the row where a falling drop must stop.
    fn stop_rows(&self, cols: usize, rows: usize, mode: Collision) -> Vec<usize> {
        let mut stop = vec![rows; cols];
        match mode {
            Collision::None => {}
            Collision::Box => {
                // One blank column/row of breathing room around the fetch.
                let top = self.y0.saturating_sub(1);
                let left = self.x0.saturating_sub(2);
                let right = (self.x0 + self.w + 2).min(cols);
                for s in &mut stop[left..right] {
                    *s = top;
                }
            }
            Collision::Shape => {
                for (x, s) in stop.iter_mut().enumerate() {
                    if let Some(r) = (self.y0..rows).find(|&y| self.solid(x, y)) {
                        *s = r;
                    }
                }
            }
        }
        stop
    }
}

pub fn run(fetch: &mut Fetch, cfg: &RainConfig, color: bool) -> std::io::Result<()> {
    if !sys::is_tty(libc::STDIN_FILENO) || !sys::is_tty(libc::STDOUT_FILENO) {
        eprintln!("drip: --drip needs an interactive terminal");
        std::process::exit(1);
    }
    let set = Settings::from(cfg, &fetch.palette);
    let _guard = RawTerm::enter()?;
    let mut out = std::io::stdout().lock();

    let mut rng = Rng::seeded();
    let mut drops: Vec<Drop> = Vec::new();
    let (mut cols, mut rows) = sys::term_size().unwrap_or((80, 24));
    let make_block = |f: &Fetch, cols: usize, rows: usize| {
        let lines = render::layout(f.logo.as_ref(), f.gap, &f.info_lines(), Some(cols));
        Block::build(&lines, cols, rows)
    };
    let mut block = make_block(fetch, cols, rows);
    let mut stop = block.stop_rows(cols, rows, set.collision);
    let mut prev: Vec<Cell> = Vec::new(); // empty forces a full first paint
    let mut grid = vec![BLANK; cols * rows];

    let started = Instant::now();
    let mut last_tick = Instant::now();
    let mut last_refresh = Instant::now();
    let mut paused = false;
    let mut density = 1.0f32;

    while !STOP.load(Ordering::SeqCst) {
        // ---- resize / refresh: rebuild the block and stop rows
        let size = sys::term_size().unwrap_or((cols, rows));
        let resized = size != (cols, rows);
        if resized {
            (cols, rows) = size;
            grid = vec![BLANK; cols * rows];
            prev.clear();
            drops.retain(|d| d.x < cols);
            out.write_all(b"\x1b[2J")?;
        }
        let mut rebuild = resized;
        if last_refresh.elapsed() >= set.refresh {
            fetch.refresh_live();
            last_refresh = Instant::now();
            rebuild = true;
        }
        if rebuild {
            block = make_block(fetch, cols, rows);
            stop = block.stop_rows(cols, rows, set.collision);
        }

        // ---- simulate
        let now = Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f32().min(0.1);
        last_tick = now;
        if !paused {
            step(&mut drops, &stop, dt, rows);
            spawn(&mut drops, &mut rng, &set, density, cols, rows);
        }

        // ---- draw
        grid.fill(BLANK);
        draw_drops(&mut grid, &drops, &stop, cols, rows, color);
        draw_block(&mut grid, &block, cols, rows, set.collision, color);
        if started.elapsed() < Duration::from_secs(4) {
            draw_hint(&mut grid, cols, rows, color);
        }
        flush_diff(&mut out, &grid, &mut prev, cols)?;

        // ---- wait for the next frame, reacting to keys meanwhile
        let left = set.frame.saturating_sub(now.elapsed());
        if let Some(keys) = term::read_input(left.as_millis() as i32) {
            match keys.as_slice() {
                b"q" | b"Q" | [0x1b] | [0x03] => break,
                b" " => paused = !paused,
                b"+" | b"=" => density = (density * 1.25).min(8.0),
                b"-" | b"_" => density = (density / 1.25).max(0.05),
                _ => {}
            }
        }
    }
    Ok(())
}

fn step(drops: &mut Vec<Drop>, stop: &[usize], dt: f32, rows: usize) {
    for d in drops.iter_mut() {
        let floor = stop.get(d.x).copied().unwrap_or(rows) as f32;
        let mv = d.speed * dt;
        // The head halts on whatever it hits; the tail keeps draining into it.
        d.head = (d.head + mv).min(floor);
        d.tail += mv;
    }
    drops.retain(|d| d.tail < d.head);
}

fn spawn(drops: &mut Vec<Drop>, rng: &mut Rng, set: &Settings, density: f32, cols: usize, rows: usize) {
    let target = (set.intensity * density * (cols * rows) as f32 / (80.0 * 24.0)) as usize;
    let missing = target.saturating_sub(drops.len());
    // Only fill part of the gap each frame so drops arrive staggered.
    for _ in 0..missing {
        if rng.unit() > 0.15 {
            continue;
        }
        let len = rng.range(set.length.0, set.length.1);
        let head = -rng.range(0.0, rows as f32 * 0.3);
        drops.push(Drop {
            x: (rng.next() % cols.max(1) as u64) as usize,
            head,
            tail: head - len,
            speed: rng.range(set.speed.0, set.speed.1),
            ch: rng.pick(&set.chars),
            color: rng.pick(&set.colors),
        });
    }
}

fn draw_drops(grid: &mut [Cell], drops: &[Drop], stop: &[usize], cols: usize, rows: usize, color: bool) {
    for d in drops {
        let floor = stop.get(d.x).copied().unwrap_or(rows);
        let head = d.head.floor() as i64;
        let len = (d.head - d.tail).max(1.0);
        let from = d.tail.ceil().max(0.0) as i64;
        let to = head.min(floor as i64 - 1).min(rows as i64 - 1);
        for y in from..=to {
            // 0 at the head, 1 at the far end of the tail.
            let t = (d.head - y as f32) / len;
            let style = if !color {
                Style::default()
            } else if y == head {
                Style::fg(d.color).bold()
            } else if t > 0.55 {
                Style { fg: Some(d.color), dim: true, ..Style::default() }
            } else {
                Style::fg(d.color)
            };
            grid[y as usize * cols + d.x] = Cell { ch: d.ch, style };
        }
    }
}

fn draw_block(grid: &mut [Cell], b: &Block, cols: usize, rows: usize, mode: Collision, color: bool) {
    for (r, line) in b.cells.iter().enumerate() {
        let y = b.y0 + r;
        if y >= rows {
            break;
        }
        for (c, cell) in line.iter().enumerate() {
            let x = b.x0 + c;
            if x >= cols {
                break;
            }
            // In box mode the block is opaque; otherwise rain shows through gaps.
            if cell.ch != ' ' || mode == Collision::Box {
                let style = if color { cell.style } else { Style::default() };
                grid[y * cols + x] = Cell { ch: cell.ch, style };
            }
        }
    }
}

fn draw_hint(grid: &mut [Cell], cols: usize, rows: usize, color: bool) {
    let hint = " q quit · space pause · +/- rain ";
    let w = hint.chars().count();
    if rows == 0 || cols < w {
        return;
    }
    let style = if color { Style { dim: true, ..Style::default() } } else { Style::default() };
    let y = rows - 1;
    for (i, ch) in hint.chars().enumerate() {
        grid[y * cols + cols - w + i] = Cell { ch, style };
    }
}

/// Sends only the cells that differ from the previous frame, wrapped in a
/// synchronized-update so terminals that support it never show half a frame.
fn flush_diff(out: &mut impl Write, grid: &[Cell], prev: &mut Vec<Cell>, cols: usize) -> std::io::Result<()> {
    let full = prev.len() != grid.len();
    let mut buf = String::with_capacity(if full { grid.len() * 8 } else { 4096 });
    buf.push_str("\x1b[?2026h");
    let mut cursor: Option<usize> = None; // index the terminal cursor is at
    let mut current: Option<Style> = None;

    for (i, cell) in grid.iter().enumerate() {
        if !full && prev[i] == *cell {
            continue;
        }
        if cursor != Some(i) {
            buf.push_str(&format!("\x1b[{};{}H", i / cols + 1, i % cols + 1));
        }
        if current != Some(cell.style) {
            sgr(&cell.style, &mut buf);
            current = Some(cell.style);
        }
        buf.push(cell.ch);
        // Writing in the last column doesn't advance the cursor reliably.
        cursor = if (i + 1) % cols == 0 { None } else { Some(i + 1) };
    }
    buf.push_str("\x1b[0m\x1b[?2026l");
    out.write_all(buf.as_bytes())?;
    out.flush()?;
    prev.clear();
    prev.extend_from_slice(grid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_stop_and_drain() {
        let stop = vec![10usize; 4];
        let mut drops = vec![Drop { x: 1, head: 8.0, tail: 4.0, speed: 10.0, ch: '|', color: Color::Ansi(1) }];
        step(&mut drops, &stop, 0.5, 20); // would move 5 rows
        assert_eq!(drops[0].head, 10.0); // halted on the obstacle
        assert_eq!(drops[0].tail, 9.0);
        step(&mut drops, &stop, 0.5, 20);
        assert!(drops.is_empty()); // fully drained
    }

    #[test]
    fn box_stops_columns_around_block() {
        let lines = vec![vec![Seg::plain("ab")], vec![Seg::plain("cd")]];
        let b = Block::build(&lines, 10, 10); // 2x2 centered at (4,4)
        let stop = b.stop_rows(10, 10, Collision::Box);
        assert_eq!(stop, vec![10, 10, 3, 3, 3, 3, 3, 3, 10, 10]);
    }

    #[test]
    fn shape_stops_on_text() {
        let lines = vec![vec![Seg::plain("a ")], vec![Seg::plain(" b")]];
        let b = Block::build(&lines, 4, 4); // at x0=1, y0=1
        let stop = b.stop_rows(4, 4, Collision::Shape);
        assert_eq!(stop, vec![4, 1, 2, 4]);
    }

    #[test]
    fn diff_sends_only_changes() {
        let mut prev = Vec::new();
        let mut grid = vec![BLANK; 6];
        let mut out = Vec::new();
        flush_diff(&mut out, &grid, &mut prev, 3).unwrap();
        grid[4].ch = 'x';
        out.clear();
        flush_diff(&mut out, &grid, &mut prev, 3).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b[2;2H"));
        assert_eq!(s.matches('x').count(), 1);
        assert!(!s.contains("\x1b[1;1H"));
    }
}
