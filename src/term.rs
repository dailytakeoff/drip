//! Raw terminal mode for the rain animation.
//!
//! Whatever happens (q, Ctrl-C, SIGTERM, a closed terminal, even a panic)
//! the terminal must be put back the way we found it, so the original
//! settings are kept in a static and restored from every exit path.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Set by the signal handler; the frame loop checks it every tick.
pub static STOP: AtomicBool = AtomicBool::new(false);
static ORIGINAL: OnceLock<libc::termios> = OnceLock::new();
static RESTORED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    // Only async-signal-safe work is allowed here: flip a flag and return.
    STOP.store(true, Ordering::SeqCst);
}

/// Restores the terminal when dropped.
pub struct RawTerm;

impl RawTerm {
    pub fn enter() -> std::io::Result<RawTerm> {
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let _ = ORIGINAL.set(t);

            // No line buffering, no echo, and Ctrl-C arrives as a byte (0x03)
            // instead of killing us before we can clean up.
            t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            t.c_iflag &= !(libc::IXON | libc::ICRNL);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t) != 0 {
                return Err(std::io::Error::last_os_error());
            }

            let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
                libc::signal(sig, handler);
            }
        }

        // drip is built with panic = "abort", so Drop won't run on a panic;
        // the hook still does.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));

        // Alternate screen (keeps your scrollback intact), hide cursor, clear.
        let mut out = std::io::stdout();
        out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[2J")?;
        out.flush()?;
        Ok(RawTerm)
    }
}

impl Drop for RawTerm {
    fn drop(&mut self) {
        restore();
    }
}

pub fn restore() {
    if RESTORED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(t) = ORIGINAL.get() {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t);
        }
    }
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[0m\x1b[?25h\x1b[?1049l");
    let _ = out.flush();
}

/// Waits up to `timeout_ms` for input and returns whatever bytes arrived.
pub fn read_input(timeout_ms: i32) -> Option<Vec<u8>> {
    let mut fds = libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 };
    let ready = unsafe { libc::poll(&mut fds, 1, timeout_ms.max(0)) };
    if ready <= 0 || fds.revents & libc::POLLIN == 0 {
        return None;
    }
    let mut buf = [0u8; 64];
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    (n > 0).then(|| buf[..n as usize].to_vec())
}
