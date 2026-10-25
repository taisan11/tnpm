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
                crate::style::stderr(&line, crate::style::Tone::Accent)
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
            writeln!(output)?;
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

/// Complete the active progress row before stdout, child output or fatal errors.
/// Returns whether a live terminal row was completed.
pub fn finish() -> bool {
    let mut renderer = RENDERER.lock().unwrap_or_else(|e| e.into_inner());
    let active = renderer.active;
    let _ = renderer.finish(&mut io::stderr().lock());
    active
}

/// Keep warnings visible and serialize them with progress from worker threads.
pub fn message(message: fmt::Arguments<'_>) {
    let text = message.to_string();
    let text = if text.starts_with("tnpm: warning:") || text.starts_with("tnpm: skipped optional") {
        crate::style::stderr(&text, crate::style::Tone::Warning)
    } else {
        text
    };
    let mut renderer = RENDERER.lock().unwrap_or_else(|e| e.into_inner());
    let _ = renderer.message(&mut io::stderr().lock(), format_args!("{text}"));
}

pub fn active() -> bool {
    RENDERER.lock().unwrap_or_else(|e| e.into_inner()).active
}

#[derive(Default)]
struct Phase {
    discovered: BTreeSet<String>,
    finished: BTreeSet<String>,
}

pub struct Progress {
    enabled: bool,
    phases: RefCell<BTreeMap<&'static str, Phase>>,
}

impl Progress {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            phases: RefCell::new(BTreeMap::new()),
        }
    }

    pub fn start(&self, phase: &'static str, package: &str) {
        if !self.enabled {
            return;
        }
        let inserted = self
            .phases
            .borrow_mut()
            .entry(phase)
            .or_default()
            .discovered
            .insert(package.into());
        if inserted {
            self.event(phase, &format!("checking {package}"));
        }
    }

    pub fn finish(&self, phase: &'static str, package: &str, outcome: &str) {
        if !self.enabled {
            return;
        }
        let mut phases = self.phases.borrow_mut();
        let state = phases.entry(phase).or_default();
        state.discovered.insert(package.into());
        let inserted = state.finished.insert(package.into());
        drop(phases);
        if inserted {
            self.event(phase, &format!("{outcome} {package}"));
        }
    }

    pub fn event(&self, phase: &'static str, detail: &str) {
        if !self.enabled {
            return;
        }
        let phases = self.phases.borrow();
        let (done, total) = phases
            .get(phase)
            .map(|p| (p.finished.len(), p.discovered.len()))
            .unwrap_or_default();
        // Never allow package/registry data to inject terminal escape sequences.
        let mut clean = String::new();
        for c in detail.chars() {
            if c.is_control() {
                clean.extend(c.escape_default());
            } else {
                clean.push(c);
            }
        }
        let line = format!("tnpm: [{done}/{total}] {phase}: {clean}");
        let columns = interactive_columns();
        let mut renderer = RENDERER.lock().unwrap_or_else(|e| e.into_inner());
        let _ = renderer.render(&mut io::stderr().lock(), &line, columns);
    }

    pub fn url(url: &reqwest::Url) -> String {
        // Signed URL queries, fragments and URL credentials must not enter logs.
        format!("{}{}", url.origin().ascii_serialization(), url.path())
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
    fn counters_are_deduplicated_and_allow_discovery_after_completion() {
        let p = Progress::new(true);
        p.start("resolve", "a");
        p.finish("resolve", "a", "resolved");
        p.start("resolve", "b");
        p.finish("resolve", "a", "resolved");
        let phases = p.phases.borrow();
        assert_eq!(phases["resolve"].discovered.len(), 2);
        assert_eq!(phases["resolve"].finished.len(), 1);
    }

    #[test]
    fn logged_urls_do_not_expose_credentials_or_signed_queries() {
        let url =
            reqwest::Url::parse("https://user:password@registry.test/pkg.tgz?token=secret#private")
                .unwrap();
        assert_eq!(Progress::url(&url), "https://registry.test/pkg.tgz");
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
            "\r\x1b[2Klong progress frame\r\x1b[2Kshort\nwarning: keep me\n\r\x1b[2Kfinal progress\n"
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
