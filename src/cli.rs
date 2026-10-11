//! Hand-rolled argument parsing (no dependency, no startup cost).

use std::path::PathBuf;

pub enum Action {
    Run,
    Help,
    Version,
    ListLogos,
    ListModules,
    PrintConfig,
    GenConfig { force: bool },
}

pub struct Args {
    pub action: Action,
    pub logo: Option<String>,
    pub style: Option<String>,
    pub modules: Option<Vec<String>>,
    pub config: Option<PathBuf>,
    pub color: Option<bool>,
    pub json: bool,
    pub stat: bool,
}

pub const HELP: &str = "\
drip — a fast system fetch, in the spirit of fastfetch and dripfetch

USAGE:
    drip [OPTIONS]

OPTIONS:
    -l, --logo <NAME|PATH|none>   Logo to show (default: detected from your OS)
    -s, --style <classic|tree>    Layout style
    -m, --modules <a,b,c>         Modules to show, comma separated
    -c, --config <PATH>           Config file (default: ~/.config/drip/config.toml)
        --json                    Print info as JSON instead
        --stat                    Show how long each module took
        --color / --no-color      Force colors on or off (NO_COLOR is respected)
        --list-logos              List built-in logos
        --list-modules            List info modules
        --print-config            Print the default config
        --gen-config [--force]    Write the default config file
    -h, --help                    Show this help
    -V, --version                 Show version
";

pub fn parse() -> Result<Args, String> {
    parse_from(std::env::args().skip(1).collect())
}

pub fn parse_from(raw: Vec<String>) -> Result<Args, String> {
    let mut a = Args {
        action: Action::Run,
        logo: None,
        style: None,
        modules: None,
        config: None,
        color: None,
        json: false,
        stat: false,
    };
    let mut force = false;
    let mut gen = false;
    let mut it = raw.into_iter();

    while let Some(arg) = it.next() {
        // Support both `--opt value` and `--opt=value`.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline.clone().or_else(|| it.next()).ok_or_else(|| format!("{name} needs a value"))
        };
        match flag.as_str() {
            "-l" | "--logo" => a.logo = Some(value("--logo")?),
            "-s" | "--style" => a.style = Some(value("--style")?),
            "-m" | "--modules" => {
                a.modules = Some(
                    value("--modules")?
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect(),
                )
            }
            "-c" | "--config" => a.config = Some(PathBuf::from(value("--config")?)),
            "--json" => a.json = true,
            "--stat" => a.stat = true,
            "--color" => a.color = Some(true),
            "--no-color" => a.color = Some(false),
            "--list-logos" => a.action = Action::ListLogos,
            "--list-modules" => a.action = Action::ListModules,
            "--print-config" => a.action = Action::PrintConfig,
            "--gen-config" => gen = true,
            "--force" => force = true,
            "-h" | "--help" => a.action = Action::Help,
            "-V" | "--version" => a.action = Action::Version,
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    if gen {
        a.action = Action::GenConfig { force };
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Args {
        parse_from(v.iter().map(|s| s.to_string()).collect()).unwrap()
    }

    #[test]
    fn flags() {
        let a = p(&["--logo=arch", "-s", "tree", "-m", "os, cpu", "--no-color"]);
        assert_eq!(a.logo.as_deref(), Some("arch"));
        assert_eq!(a.style.as_deref(), Some("tree"));
        assert_eq!(a.modules.unwrap(), vec!["os", "cpu"]);
        assert_eq!(a.color, Some(false));
        assert!(parse_from(vec!["--bogus".into()]).is_err());
        assert!(parse_from(vec!["--logo".into()]).is_err());
    }
}
