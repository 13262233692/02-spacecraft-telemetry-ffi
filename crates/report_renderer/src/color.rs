//! Minimal ANSI SGR styling with automatic `NO_COLOR` / non-TTY fallback.

use std::sync::atomic::{AtomicBool, Ordering};

static FORCE_COLOR: AtomicBool = AtomicBool::new(false);

/// Explicitly enable or disable ANSI colors. When never called, colors are
/// used automatically on TTYs unless the `NO_COLOR` environment variable is
/// present.
pub fn set_color(enabled: bool) {
    FORCE_COLOR.store(enabled, Ordering::Relaxed);
}

fn auto_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    is_terminal_stdout()
}

#[cfg(not(target_arch = "wasm32"))]
fn is_terminal_stdout() -> bool {
    use std::os::fd::AsRawFd;
    // libc is not a dependency; use the ioctl-based heuristic through isatty.
    unsafe { libc_isatty(std::io::stdout().as_raw_fd()) }
}

#[cfg(not(target_arch = "wasm32"))]
unsafe fn libc_isatty(fd: i32) -> bool {
    extern "C" {
        fn isatty(fd: i32) -> i32;
    }
    isatty(fd) == 1
}

#[cfg(target_arch = "wasm32")]
fn is_terminal_stdout() -> bool {
    false
}

pub fn enabled() -> bool {
    FORCE_COLOR.load(Ordering::Relaxed) || auto_color()
}

#[derive(Debug, Clone, Copy)]
pub enum Style {
    Reset,
    Bold,
    Dim,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Gray,
    RedBold,
    GreenBold,
    YellowBold,
    BlueBold,
    MagentaBold,
    CyanBold,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Style::Reset => "0",
            Style::Bold => "1",
            Style::Dim => "2",
            Style::Red => "31",
            Style::Green => "32",
            Style::Yellow => "33",
            Style::Blue => "34",
            Style::Magenta => "35",
            Style::Cyan => "36",
            Style::White => "37",
            Style::Gray => "90",
            Style::RedBold => "1;31",
            Style::GreenBold => "1;32",
            Style::YellowBold => "1;33",
            Style::BlueBold => "1;34",
            Style::MagentaBold => "1;35",
            Style::CyanBold => "1;36",
        }
    }
}

/// Wrap `text` in an SGR escape sequence when color output is active.
pub fn paint(style: Style, text: &str) -> String {
    if !enabled() {
        return text.to_string();
    }
    format!("\x1b[{}m{}\x1b[0m", style.code(), text)
}

pub fn bold(text: &str) -> String {
    paint(Style::Bold, text)
}

pub fn dim(text: &str) -> String {
    paint(Style::Dim, text)
}
