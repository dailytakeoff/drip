//! Everything needed to produce one fetch: settings resolved from CLI +
//! config, the gathered module results, the logo and its colors.
//! The static output and the rain animation both drive this.

use crate::cli::Args;
use crate::color::{Color, Seg};
use crate::config::{self, Config};
use crate::logo::{self, Logo};
use crate::modules::{self, Ctx};
use crate::render::{self, Gathered, Item, Opts};
use crate::sys;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// Modules whose values change while drip is running.
const LIVE: &[&str] = &["uptime", "memory", "swap", "disk", "battery", "localip"];

pub struct Fetch {
    ctx: Ctx,
    items: Vec<Item>,
    labels: Vec<String>,
    results: Vec<Option<Gathered>>,
    pub logo: Option<Logo>,
    pub palette: Vec<Color>,
    pub opts: Opts,
    pub gap: usize,
}

impl Fetch {
    /// Resolves settings and gathers every module once.
    /// `width` is the terminal width used to pick a small logo.
    pub fn new(args: &Args, cfg: &Config, color: bool, width: Option<usize>, want_logo: bool) -> Fetch {
        let ctx = Ctx {
            os: sys::os_release(),
            uname: sys::uname(),
            config_home: sys::config_home(),
        };

        // ---- which items, in which style
        let style = args.style.clone().or(cfg.style.clone()).unwrap_or_else(|| "classic".into());
        let tree = match style.as_str() {
            "tree" => true,
            "classic" => false,
            other => {
                eprintln!("drip: unknown style '{other}', using classic");
                false
            }
        };
        let names: Vec<String> = args.modules.clone().or(cfg.modules.clone()).unwrap_or_else(|| {
            let d = if tree { config::TREE_MODULES } else { config::CLASSIC_MODULES };
            d.iter().map(|s| s.to_string()).collect()
        });
        let items: Vec<Item> = names.iter().map(|n| Item::parse(n)).collect();
        let labels = items
            .iter()
            .map(|it| match it {
                Item::Info(n) => {
                    if modules::default_label(n).is_none() {
                        eprintln!("drip: unknown module '{n}' (see --list-modules)");
                    }
                    cfg.keys
                        .get(n)
                        .cloned()
                        .unwrap_or_else(|| modules::default_label(n).unwrap_or(n).to_owned())
                }
                _ => String::new(),
            })
            .collect();

        // ---- logo and colors
        let setting = args.logo.clone().or(cfg.logo.clone()).unwrap_or_else(|| "auto".into());
        let resolved = if want_logo { resolve_logo(&setting, &ctx.os, width) } else { None };
        let colors_name = resolved
            .as_ref()
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| logo::detect(&ctx.os));
        let mut palette = parse_colors(&cfg.logo_colors, "logo_colors");
        if palette.is_empty() {
            palette = logo::default_colors(&colors_name);
        }
        let key = match cfg.key_color.as_deref() {
            None | Some("auto") => palette[0],
            Some(c) => Color::parse(c).unwrap_or_else(|| {
                eprintln!("drip: bad key_color '{c}'");
                palette[0]
            }),
        };
        let logo = resolved.map(|(_, text)| logo::parse(&text, &palette));

        let opts = Opts {
            tree,
            key,
            sep: cfg.separator.clone().unwrap_or_else(|| ": ".into()),
            stat: args.stat,
            color,
            user: sys::username().unwrap_or_else(|| "user".into()),
            host: ctx.uname.as_ref().map(|u| u.nodename.clone()).unwrap_or_else(|| "localhost".into()),
        };

        let mut f = Fetch {
            results: (0..items.len()).map(|_| None).collect(),
            ctx,
            items,
            labels,
            logo,
            palette,
            opts,
            gap: cfg.logo_gap.unwrap_or(3),
        };
        f.gather(|_| true);
        f
    }

    /// Runs the selected modules: slow ones each on their own thread,
    /// the rest on this thread while those are busy.
    fn gather(&mut self, wanted: impl Fn(&str) -> bool) {
        let ctx = &self.ctx;
        let timed = |name: &str, label: &str| {
            let t = Instant::now();
            let entries = modules::gather(name, label, ctx);
            Gathered { entries, time: t.elapsed() }
        };
        let jobs: Vec<(usize, &str, &str)> = self
            .items
            .iter()
            .zip(&self.labels)
            .enumerate()
            .filter_map(|(i, (it, label))| match it {
                Item::Info(n) if wanted(n) => Some((i, n.as_str(), label.as_str())),
                _ => None,
            })
            .collect();

        let mut done: Vec<(usize, Gathered)> = Vec::with_capacity(jobs.len());
        std::thread::scope(|s| {
            let handles: Vec<_> = jobs
                .iter()
                .filter(|(_, n, _)| modules::is_slow(n))
                .map(|&(i, n, l)| (i, s.spawn(move || timed(n, l))))
                .collect();
            for &(i, n, l) in jobs.iter().filter(|(_, n, _)| !modules::is_slow(n)) {
                done.push((i, timed(n, l)));
            }
            for (i, h) in handles {
                if let Ok(g) = h.join() {
                    done.push((i, g));
                }
            }
        });
        for (i, g) in done {
            self.results[i] = Some(g);
        }
    }

    /// Re-reads only the values that change over time.
    pub fn refresh_live(&mut self) {
        self.gather(|n| LIVE.contains(&n));
    }

    pub fn info_lines(&self) -> Vec<Vec<Seg>> {
        render::info_lines(&self.items, &self.results, &self.opts)
    }

    pub fn json(&self) -> String {
        render::json(&self.items, &self.results)
    }
}

pub fn parse_colors(list: &[String], what: &str) -> Vec<Color> {
    list.iter()
        .filter_map(|c| {
            let p = Color::parse(c);
            if p.is_none() {
                eprintln!("drip: bad color '{c}' in {what}");
            }
            p
        })
        .collect()
}

fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), sys::home()) {
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(p),
    }
}

/// Turns the logo setting into (name used for default colors, logo text).
fn resolve_logo(setting: &str, os: &HashMap<String, String>, width: Option<usize>) -> Option<(String, String)> {
    match setting {
        "none" | "off" | "false" => None,
        s if s.contains('/') || s.ends_with(".txt") => match std::fs::read_to_string(expand_home(s)) {
            Ok(text) => Some((String::new(), text)),
            Err(e) => {
                eprintln!("drip: can't read logo {s}: {e}");
                resolve_logo("auto", os, width)
            }
        },
        "auto" | "" => {
            let mut name = logo::detect(os);
            let full = logo::builtin(&name).unwrap_or("");
            let full_w = full.lines().map(|l| l.chars().count()).max().unwrap_or(0);
            // On a narrow terminal, prefer the distro's small logo.
            if let Some(w) = width {
                let small = format!("{name}_small");
                if w < full_w + 50 && logo::builtin(&small).is_some() {
                    name = small;
                }
            }
            logo::builtin(&name).map(|t| (name, t.to_owned()))
        }
        s => {
            let key = s.to_ascii_lowercase().replace('-', "_");
            match logo::builtin(&key) {
                Some(t) => Some((key, t.to_owned())),
                None => {
                    eprintln!("drip: no built-in logo '{s}' (see --list-logos)");
                    resolve_logo("auto", os, width)
                }
            }
        }
    }
}
