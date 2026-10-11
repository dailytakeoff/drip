//! Thin wrappers over libc and /proc used by the modules.

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::io::Read;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// Reads a file and returns its trimmed contents, or None if empty.
pub fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    let s = read(path)?;
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

pub fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

pub fn parse_kv(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_owned(), v.trim().trim_matches(['"', '\'']).to_owned()))
        })
        .collect()
}

pub fn os_release() -> HashMap<String, String> {
    read("/etc/os-release")
        .or_else(|| read("/usr/lib/os-release"))
        .map(|t| parse_kv(&t))
        .unwrap_or_default()
}

pub fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(cmd))
        .find(|p| p.is_file())
}

/// Runs a command with no stdin and returns stdout, giving up after
/// `timeout` so a misbehaving program can't stall the whole fetch.
pub fn run(cmd: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_micros(300)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout?.read_to_string(&mut out).ok()?;
    Some(out)
}

pub struct Uname {
    pub sysname: String,
    pub nodename: String,
    pub release: String,
    pub machine: String,
}

fn cstr_field(buf: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn uname() -> Option<Uname> {
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) != 0 {
            return None;
        }
        Some(Uname {
            sysname: cstr_field(&u.sysname),
            nodename: cstr_field(&u.nodename),
            release: cstr_field(&u.release),
            machine: cstr_field(&u.machine),
        })
    }
}

pub fn username() -> Option<String> {
    env("USER").or_else(|| env("LOGNAME")).or_else(|| unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_name.is_null() {
            return None;
        }
        Some(CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned())
    })
}

pub fn home() -> Option<PathBuf> {
    env("HOME").map(PathBuf::from)
}

pub fn config_home() -> Option<PathBuf> {
    env("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".config")))
}

/// (total bytes, available-to-user bytes)
pub fn statvfs(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    unsafe {
        let mut s: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut s) != 0 {
            return None;
        }
        let frsize = s.f_frsize as u64;
        Some((s.f_blocks as u64 * frsize, s.f_bavail as u64 * frsize))
    }
}

pub fn is_tty(fd: i32) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

pub fn term_width() -> Option<usize> {
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO, libc::STDIN_FILENO] {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return Some(ws.ws_col as usize);
            }
        }
    }
    env("COLUMNS").and_then(|c| c.parse().ok())
}

/// Every up IPv4 address: (interface, address, prefix length).
pub fn ipv4_addrs() -> Vec<(String, Ipv4Addr, u32)> {
    let mut out = Vec::new();
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut cur = head;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
                continue;
            }
            if (*ifa.ifa_addr).sa_family as i32 != libc::AF_INET {
                continue;
            }
            if ifa.ifa_flags & libc::IFF_UP as u32 == 0 {
                continue;
            }
            let addr = &*(ifa.ifa_addr as *const libc::sockaddr_in);
            let ip = Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
            let prefix = if ifa.ifa_netmask.is_null() {
                32
            } else {
                let m = &*(ifa.ifa_netmask as *const libc::sockaddr_in);
                m.sin_addr.s_addr.count_ones()
            };
            let name = CStr::from_ptr(ifa.ifa_name).to_string_lossy().into_owned();
            out.push((name, ip, prefix));
        }
        libc::freeifaddrs(head);
    }
    out
}

// ---- /proc process helpers -------------------------------------------------

pub fn ppid_of(pid: u32) -> Option<u32> {
    let stat = read(format!("/proc/{pid}/stat"))?;
    // The command name may contain spaces and parens; fields resume after the last ')'.
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut it = rest.split_whitespace();
    it.next()?; // state
    it.next()?.parse().ok()
}

pub fn comm_of(pid: u32) -> Option<String> {
    read_trim(format!("/proc/{pid}/comm")).map(|c| c.trim_start_matches('-').to_owned())
}

pub fn exe_of(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
}

pub fn parent_pid() -> u32 {
    unsafe { libc::getppid() as u32 }
}

/// Names of all running processes (best effort).
pub fn all_comms() -> Vec<String> {
    let Ok(dir) = fs::read_dir("/proc") else { return Vec::new() };
    dir.filter_map(|e| e.ok())
        .filter(|e| e.file_name().as_encoded_bytes().iter().all(u8::is_ascii_digit))
        .filter_map(|e| read_trim(e.path().join("comm")))
        .collect()
}

pub fn count_dirs(path: impl AsRef<Path>) -> usize {
    fs::read_dir(path)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

pub fn fmt_bytes(b: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = b as f64;
    if b >= KIB * KIB * KIB * KIB {
        format!("{:.2} TiB", b / (KIB * KIB * KIB * KIB))
    } else if b >= KIB * KIB * KIB {
        format!("{:.2} GiB", b / (KIB * KIB * KIB))
    } else if b >= KIB * KIB {
        format!("{:.2} MiB", b / (KIB * KIB))
    } else {
        format!("{:.2} KiB", b / KIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(fmt_bytes(16 * 1024 * 1024 * 1024), "16.00 GiB");
        assert_eq!(fmt_bytes(512 * 1024 * 1024), "512.00 MiB");
    }

    #[test]
    fn own_process_is_visible() {
        let me = std::process::id();
        assert!(comm_of(me).is_some());
        assert!(ppid_of(me).is_some());
    }
}
