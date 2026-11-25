//! In-place terminal progress, with plain append-only logs for redirected stderr.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{self, IsTerminal, Write},
    sync::Mutex,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

static RENDERER: Mutex<Renderer> = Mutex::new(Renderer {
    active: false,
    color: true,
});

struct Renderer {
    active: bool,
    color: bool,
}

impl Renderer {
    fn render(
        &mut self,
        output: &mut impl Write,
        text: &str,
        columns: Option<usize>,
    ) -> io::Result<()> {
        if let Some(columns) = columns {
            let line = fit_line(text, columns);
            let line = if self.color {
                crate::util::style::stderr(&line, crate::util::style::Tone::Accent)
            } else {
                line
            };
            write!(output, "\r\x1b[2K{line}")?;
            self.active = true;
        } else {
            self.finish(output)?;
            writeln!(output, "{text}")?;
        }
        output.flush()
    }

    fn finish(&mut self, output: &mut impl Write) -> io::Result<()> {
        if self.active {
            self.active = false;
            write!(output, "\r\x1b[2K")?;
            output.flush()?;
        }
        Ok(())
    }

    fn message(&mut self, output: &mut impl Write, message: fmt::Arguments<'_>) -> io::Result<()> {
        self.finish(output)?;
        writeln!(output, "{message}")
    }
}

fn fit_line(text: &str, columns: usize) -> String {
    // Reserve the last terminal column to avoid automatic line wrapping.
    let limit = columns.saturating_sub(1);
    if text.width() <= limit {
        return text.to_owned();
    }
    if limit <= 3 {
        return ".".repeat(limit);
    }
    let mut line = String::new();
    let mut width = 0;
    for c in text.chars() {
        let next = c.width().unwrap_or(0);
        if width + next > limit - 3 {
            break;
        }
        line.push(c);
        width += next;
    }
    line.push_str("...");
    line
}

fn interactive_columns() -> Option<usize> {
    if !io::stderr().is_terminal() || std::env::var("TERM").as_deref() == Ok("dumb") {
        return None;
    }
    Some(
        terminal_columns()
            .or_else(|| {
                std::env::var("COLUMNS")
                    .ok()?
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
            })
            .unwrap_or(80),
    )
}

#[cfg(unix)]
fn terminal_columns() -> Option<usize> {
    // SAFETY: winsize contains only integers and ioctl receives a valid writable
    // pointer. Its fields are read only after the terminal query succeeds.
    unsafe {
        let mut size: libc::winsize = std::mem::zeroed();
        (libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) == 0 && size.ws_col > 0)
            .then_some(size.ws_col as usize)
    }
}

#[cfg(not(unix))]
fn terminal_columns() -> Option<usize> {
    None
}

/// Clear the active progress row before stdout, child output or fatal errors.
pub fn finish() {
    let mut renderer = RENDERER.lock().unwrap_or_else(|e| e.into_inner());
    let _ = renderer.finish(&mut io::stderr().lock());
}

/// Keep warnings visible and serialize them with progress from worker threads.
pub fn message(message: fmt::Arguments<'_>) {
    let text = message.to_string();
    let text = if text.starts_with("tnpm: warning:") || text.starts_with("tnpm: skipped optional") {
        crate::util::style::stderr(&text, crate::util::style::Tone::Warning)
    } else {
        text
    };
    let mut renderer = RENDERER.lock().unwrap_or_else(|e| e.into_inner());
    let _ = renderer.message(&mut io::stderr().lock(), format_args!("{text}"));
}

pub fn single_line_message(message_args: fmt::Arguments<'_>) {
    let text = escape_controls(&message_args.to_string());
    message(format_args!("{text}"));
}

fn escape_controls(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            escaped.extend(c.escape_default());
        } else {
            escaped.push(c);
        }
    }
    escaped
}

pub fn active() -> bool {
    RENDERER.lock().unwrap_or_else(|e| e.into_inner()).active
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Resolving,
    Preparing,
    Linking,
    Removing,
    RunningScripts,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Resolving => "Resolving",
            Self::Preparing => "Preparing",
            Self::Linking => "Linking",
            Self::Removing => "Removing",
            Self::RunningScripts => "Running scripts",
        }
    }
}

#[derive(Default)]
struct Counts {
    discovered: BTreeSet<String>,
    finished: BTreeSet<String>,
}

pub struct Progress {
    enabled: bool,
    phases: RefCell<BTreeMap<Phase, Counts>>,
}

impl Progress {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            phases: RefCell::new(BTreeMap::new()),
        }
    }

    pub fn start(&self, phase: Phase, package: &str) {
        self.start_many(phase, std::iter::once(package));
    }

    pub fn start_many<'a>(&self, phase: Phase, packages: impl Iterator<Item = &'a str>) {
        if !self.enabled {
            return;
        }
        let mut phases = self.phases.borrow_mut();
        let discovered = &mut phases.entry(phase).or_default().discovered;
        let previous = discovered.len();
        discovered.extend(packages.map(str::to_owned));
        let changed = previous != discovered.len();
        drop(phases);
        if changed {
            self.show(phase);
        }
    }

    pub fn complete(&self, phase: Phase, package: &str) {
        if !self.enabled {
            return;
        }
        let mut phases = self.phases.borrow_mut();
        let state = phases.entry(phase).or_default();
        state.discovered.insert(package.into());
        let inserted = state.finished.insert(package.into());
        drop(phases);
        if inserted {
            self.show(phase);
        }
    }

    pub fn show(&self, phase: Phase) {
        if !self.enabled {
            return;
        }
        let phases = self.phases.borrow();
        let (done, total) = phases
            .get(&phase)
            .map(|p| (p.finished.len(), p.discovered.len()))
            .unwrap_or_default();
        let label = phase.label();
        let line = if total == 0 {
            label.to_owned()
        } else {
            format!("{label} [{done}/{total}]")
        };
        let columns = interactive_columns();
        let mut renderer = RENDERER.lock().unwrap_or_else(|e| e.into_inner());
        let _ = renderer.render(&mut io::stderr().lock(), &line, columns);
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if self.enabled {
            finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_diagnostics_escape_controls_and_preserve_unicode() {
        assert_eq!(
            escape_controls("archive\u{1b}\n\r\u{9b}日本語"),
            r"archive\u{1b}\n\r\u{9b}日本語"
        );
    }

    #[test]
    fn counters_are_deduplicated_and_allow_discovery_after_completion() {
        let p = Progress::new(true);
        p.start(Phase::Resolving, "a");
        p.complete(Phase::Resolving, "a");
        p.start(Phase::Resolving, "b");
        p.complete(Phase::Resolving, "a");
        let phases = p.phases.borrow();
        assert_eq!(phases[&Phase::Resolving].discovered.len(), 2);
        assert_eq!(phases[&Phase::Resolving].finished.len(), 1);
    }

    #[test]
    fn terminal_frames_replace_one_row_and_warnings_are_preserved() {
        let mut renderer = Renderer {
            active: false,
            color: false,
        };
        let mut output = Vec::new();
        renderer
            .render(&mut output, "long progress frame", Some(80))
            .unwrap();
        renderer.render(&mut output, "short", Some(80)).unwrap();
        assert!(!output.contains(&b'\n'));
        renderer
            .message(&mut output, format_args!("warning: keep me"))
            .unwrap();
        renderer
            .render(&mut output, "final progress", Some(80))
            .unwrap();
        renderer.finish(&mut output).unwrap();
        renderer.finish(&mut output).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\r\x1b[2Klong progress frame\r\x1b[2Kshort\r\x1b[2Kwarning: keep me\n\r\x1b[2Kfinal progress\r\x1b[2K"
        );
    }

    #[test]
    fn redirected_output_remains_plain_text() {
        let mut renderer = Renderer {
            active: false,
            color: false,
        };
        let mut output = Vec::new();
        renderer.render(&mut output, "first", None).unwrap();
        renderer.render(&mut output, "second", None).unwrap();
        renderer.finish(&mut output).unwrap();
        assert_eq!(output, b"first\nsecond\n");
    }

    #[test]
    fn terminal_lines_fit_narrow_and_unicode_terminal_widths() {
        for columns in [0, 1, 2, 4, 12, 24, 80] {
            let text = fit_line(&"tnpm: 日本語👩‍💻e\u{301}".repeat(20), columns);
            assert!(
                text.width() <= columns.saturating_sub(1),
                "{columns}: {text}"
            );
        }
    }
}
