//! Per-stream color policy; colored supplies the styles after width calculations.
use colored::Colorize;
use std::io::{self, IsTerminal};

#[derive(Clone, Copy)]
pub enum Tone {
    Accent,
    Success,
    Warning,
    Error,
    Muted,
}

pub fn init() {
    // colored normally checks stdout only. Gate each stream below so stderr
    // progress can still be colored when JSON stdout is redirected.
    colored::control::set_override(true);
}

fn enabled(terminal: bool) -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("CLICOLOR_FORCE").is_ok_and(|v| v != "0" && !v.is_empty()) {
        return true;
    }
    terminal
        && std::env::var("TERM").as_deref() != Ok("dumb")
        && std::env::var("CLICOLOR").as_deref() != Ok("0")
}

fn paint(text: &str, tone: Tone, terminal: bool) -> String {
    if !enabled(terminal) {
        return text.to_owned();
    }
    match tone {
        Tone::Accent => text.cyan().bold(),
        Tone::Success => text.green(),
        Tone::Warning => text.yellow(),
        Tone::Error => text.red().bold(),
        Tone::Muted => text.dimmed(),
    }
    .to_string()
}

pub fn stdout(text: &str, tone: Tone) -> String {
    paint(text, tone, io::stdout().is_terminal())
}
pub fn stderr(text: &str, tone: Tone) -> String {
    paint(text, tone, io::stderr().is_terminal())
}
