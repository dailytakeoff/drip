# drip

A fast system fetch written in Rust: fastfetch-style output with the logo
collection and tree layout from [dripfetch](https://github.com/a-shygun/dripfetch).

Everything is read straight from `/proc`, `/sys` and config files. The only
process it ever starts is `<your shell> --version`, and slow modules run on
their own threads, so a full fetch takes a few milliseconds.

```
drip                 # fastfetch-style
drip -s tree         # dripfetch-style sections
drip --stat          # time per module
drip --json          # machine-readable
drip --drip          # full-screen, with rain (q quit, space pause, +/- density)
drip -l gentoo       # any of the 487 built-in logos
```

## Install

Needs a Rust toolchain (`pacman -S rust` or rustup).

```sh
cargo build --release
install -Dm755 target/release/drip ~/.local/bin/drip
```

or `cargo install --path .`

## Modules

| Module | Source |
|---|---|
| os | `/etc/os-release` + `uname` |
| host | DMI (`product_name (product_version)`), device tree on ARM |
| kernel | `uname` |
| init | `/proc/1/comm` — systemd, dinit, runit, OpenRC, s6, SysVinit, Shepherd |
| uptime | `/proc/uptime` |
| packages | pacman, portage, dpkg, apk, xbps, rpm, flatpak, snap (counted from their databases) |
| shell | parent process + `--version` |
| terminal | walks the process tree past shells, tmux, sudo... |
| de / wm | `XDG_CURRENT_DESKTOP`, compositor sockets, then a process scan (niri, Hyprland, Sway, river, mango, labwc, KWin, i3, dwm...) |
| display | `/sys/class/drm` connected outputs |
| theme / icons / font / cursor | GTK 3/4 `settings.ini`, `kdeglobals`, `XCURSOR_*` |
| cpu | `/proc/cpuinfo` + highest `cpuinfo_max_freq` |
| gpu | `/sys/bus/pci` + `pci.ids` |
| memory / swap | `/proc/meminfo` (used = total − available) |
| disk | `/proc/mounts` + `statvfs`; btrfs subvolumes shown once; `/boot`, snaps etc. hidden |
| localip | default-route interface from `/proc/net/route` |
| battery | `/sys/class/power_supply`, peripherals skipped |
| locale | `LC_ALL` / `LANG` |

Plus layout items: `title`, `separator`, `break`, `colors`, `section:NAME`.
Modules with nothing to report are simply left out.

## Rain

`drip --drip` centers the fetch on screen with rain falling around it, in
your logo's colors. Uptime, memory, disk, battery and IP refresh every two
seconds. Tune it under `[rain]` in the config:

```toml
[rain]
intensity = 60          # drops on an 80x24 screen; scales with size
chars = ["│"]
colors = []             # empty = logo colors
speed = [10.0, 35.0]    # rows per second
length = [4, 16]
collision = "box"       # "box", "shape" (lands on the logo/text) or "none"
fps = 30
refresh = 2.0
```

Alias it if you want it every time: `alias fetch='drip --drip'`.

## Config

`drip --gen-config` writes a commented `~/.config/drip/config.toml`:

```toml
logo = "auto"            # name, path to a file, or "none"
logo_colors = []         # override $1, $2... (names, 0-255, "#hex")
style = "classic"        # or "tree"
key_color = "auto"       # logo's first color
modules = ["title", "separator", "os", "kernel", "init", "cpu", "memory", "break", "colors"]

[keys]
localip = "IP"
```

Custom logos use fastfetch syntax: `$1`..`$9` switch color, `$$` is a literal `$`.

## Credits

Logos come from dripfetch (MIT, © 2026 Amir Shygun), which collected them
from fastfetch (MIT). See `LICENSE`.
