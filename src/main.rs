mod cli;
mod color;
mod config;
mod logo;
mod modules;
mod render;
mod sys;

use color::Color;
use modules::{Ctx, MODULES};
use render::{Gathered, Item, Opts};
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let args = match cli::parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("drip: {e}\nTry 'drip --help'.");
            std::process::exit(2);
        }
    };
    let out = match args.action {
        cli::Action::Help => cli::HELP.to_owned(),
        cli::Action::Version => format!("drip {}\n", env!("CARGO_PKG_VERSION")),
        cli::Action::ListLogos => logo::names().collect::<Vec<_>>().join("\n") + "\n",
        cli::Action::ListModules => MODULES
            .iter()
            .map(|(n, _, d)| format!("{n:<10} {d}\n"))
            .collect(),
        cli::Action::PrintConfig => config::DEFAULT_CONFIG.to_owned(),
        cli::Action::GenConfig { force } => {
            gen_config(args.config.clone(), force);
            return;
        }
        cli::Action::Run => run(&args),
    };
    let mut stdout = std::io::stdout().lock();
    // A closed pipe (`drip | head`) is not an error worth reporting.
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
}

fn gen_config(path: Option<PathBuf>, force: bool) {
    let Some(path) = path.or_else(config::default_path) else {
        eprintln!("drip: can't work out a config directory (is HOME set?)");
        std::process::exit(1);
    };
    if path.exists() && !force {
        eprintln!("drip: {} already exists (add --force to overwrite)", path.display());
        std::process::exit(1);
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, config::DEFAULT_CONFIG) {
        Ok(()) => println!("Wrote {}", path.display()),
        Err(e) => {
            eprintln!("drip: can't write {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}

fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), sys::home()) {
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(p),
    }
}

/// Resolves the logo setting to (name for colors, logo text).
fn resolve_logo(setting: &str, os: &std::collections::HashMap<String, String>, width: Option<usize>) -> Option<(String, String)> {
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

fn run(args: &cli::Args) -> String {
    let start = Instant::now();
    let cfg = config::load(args.config.as_deref());

    let color = !args.json
        && args.color.unwrap_or_else(|| sys::env("NO_COLOR").is_none() && sys::is_tty(libc::STDOUT_FILENO));
    let width = if sys::is_tty(libc::STDOUT_FILENO) && cfg.truncate.unwrap_or(true) {
        sys::term_width()
    } else {
        None
    };

    let ctx = Ctx {
        os: sys::os_release(),
        uname: sys::uname(),
        config_home: sys::config_home(),
    };

    // ---- what to show
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
    for it in &items {
        if let Item::Info(n) = it {
            if modules::default_label(n).is_none() {
                eprintln!("drip: unknown module '{n}' (see --list-modules)");
            }
        }
    }

    // ---- gather: slow modules on threads, the rest inline meanwhile
    let label = |n: &str| -> String {
        cfg.keys
            .get(n)
            .cloned()
            .unwrap_or_else(|| modules::default_label(n).unwrap_or(n).to_owned())
    };
    let timed = |n: &str| {
        let t = Instant::now();
        let entries = modules::gather(n, &label(n), &ctx);
        Gathered { entries, time: t.elapsed() }
    };
    let mut results: Vec<Option<Gathered>> = (0..items.len()).map(|_| None).collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| match it {
                Item::Info(n) if modules::is_slow(n) => Some((i, s.spawn(|| timed(n)))),
                _ => None,
            })
            .collect();
        for (i, it) in items.iter().enumerate() {
            if let Item::Info(n) = it {
                if !modules::is_slow(n) {
                    results[i] = Some(timed(n));
                }
            }
        }
        for (i, h) in handles {
            results[i] = h.join().ok();
        }
    });

    if args.json {
        return render::json(&items, &results);
    }

    // ---- logo and colors
    let logo_setting = args.logo.clone().or(cfg.logo.clone()).unwrap_or_else(|| "auto".into());
    let resolved = resolve_logo(&logo_setting, &ctx.os, width);
    let colors_name = resolved.as_ref().map(|(n, _)| n.clone()).unwrap_or_else(|| logo::detect(&ctx.os));
    let mut palette: Vec<Color> = cfg
        .logo_colors
        .iter()
        .filter_map(|c| {
            let p = Color::parse(c);
            if p.is_none() {
                eprintln!("drip: bad color '{c}' in logo_colors");
            }
            p
        })
        .collect();
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
    let mut lines = render::info_lines(&items, &results, &opts);
    if args.stat {
        lines.push(vec![]);
        lines.push(vec![color::Seg::plain(format!(
            "Total: {:.2}ms",
            start.elapsed().as_secs_f64() * 1000.0
        ))]);
    }
    render::compose(logo.as_ref(), cfg.logo_gap.unwrap_or(3), &lines, width, color)
}
