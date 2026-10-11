mod cli;
mod color;
mod config;
mod fetch;
mod logo;
mod modules;
mod rain;
mod render;
mod sys;
mod term;

use fetch::Fetch;
use modules::MODULES;
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

/// Returns what to print (empty after the rain animation).
fn run(args: &cli::Args) -> String {
    let start = Instant::now();
    let cfg = config::load(args.config.as_deref());
    let tty = sys::is_tty(libc::STDOUT_FILENO);

    let color = !args.json && args.color.unwrap_or_else(|| sys::env("NO_COLOR").is_none() && tty);
    let width = if tty && cfg.truncate.unwrap_or(true) { sys::term_width() } else { None };

    let mut fetch = Fetch::new(args, &cfg, color, width, !args.json);

    if args.json {
        return fetch.json();
    }
    if args.drip {
        if let Err(e) = rain::run(&mut fetch, &cfg.rain, color) {
            eprintln!("drip: {e}");
            std::process::exit(1);
        }
        return String::new();
    }

    let mut lines = fetch.info_lines();
    if args.stat {
        lines.push(vec![]);
        lines.push(vec![color::Seg::plain(format!(
            "Total: {:.2}ms",
            start.elapsed().as_secs_f64() * 1000.0
        ))]);
    }
    render::paint_lines(&render::layout(fetch.logo.as_ref(), fetch.gap, &lines, width), color)
}
