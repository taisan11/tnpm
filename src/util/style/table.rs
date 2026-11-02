//! Tables padded using raw display widths before adding terminal colors.
use unicode_width::UnicodeWidthStr;

pub fn format<const N: usize>(headers: [&str; N], rows: &[[String; N]]) -> String {
    render(headers, rows, false, true)
}

pub fn format_plain<const N: usize>(headers: [&str; N], rows: &[[String; N]]) -> String {
    render(headers, rows, false, false)
}

pub fn format_bordered<const N: usize>(headers: [&str; N], rows: &[[String; N]]) -> String {
    render(headers, rows, true, true)
}

fn render<const N: usize>(
    headers: [&str; N],
    rows: &[[String; N]],
    bordered: bool,
    colors: bool,
) -> String {
    if N == 0 {
        return String::new();
    }
    let mut widths = headers.map(UnicodeWidthStr::width);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.width());
        }
    }
    let mut output = String::new();
    let rule = |left: char, middle: char, right: char| {
        let mut line = left.to_string();
        for (i, width) in widths.iter().enumerate() {
            if i > 0 {
                line.push(middle);
            }
            line.push_str(&"─".repeat(width + 2));
        }
        line.push(right);
        line.push('\n');
        line
    };
    if bordered {
        output.push_str(&rule('┌', '┬', '┐'));
    }
    for (row_index, row) in std::iter::once(headers)
        .chain(rows.iter().map(|r| r.each_ref().map(String::as_str)))
        .enumerate()
    {
        if bordered {
            output.push('│');
        }
        for (i, cell) in row.iter().enumerate() {
            if bordered {
                output.push(' ');
            }
            let tone = if row_index == 0 || i == 0 {
                crate::util::style::Tone::Accent
            } else if matches!(*cell, "MISSING" | "REMOVED") {
                crate::util::style::Tone::Error
            } else if i + 1 == N {
                crate::util::style::Tone::Success
            } else {
                crate::util::style::Tone::Warning
            };
            if colors {
                output.push_str(&crate::util::style::stdout(cell, tone));
            } else {
                output.push_str(cell);
            }
            if bordered {
                output.push_str(&" ".repeat(widths[i] - cell.width() + 1));
                output.push('│');
            } else if i + 1 < N {
                output.push_str(&" ".repeat(widths[i] - cell.width() + 2));
            }
        }
        output.push('\n');
        if bordered && row_index == 0 {
            output.push_str(&rule('├', '┼', '┤'));
        }
    }
    if bordered {
        output.push_str(&rule('└', '┴', '┘'));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_wide_and_combining_characters_without_tabs_or_trailing_spaces() {
        let rows = [
            ["日本語".into(), "MISSING".into(), "1.0.0".into()],
            ["e\u{301}".into(), "1.0.0".into(), "2.0.0".into()],
            ["👩‍💻".into(), "1.0.0".into(), "2.0.0".into()],
        ];
        let table = format_plain(["Package", "Current", "Latest"], &rows);
        assert!(!table.contains('\t'));
        for line in table.lines() {
            assert!(!line.ends_with(' '));
            let last = line.split_whitespace().last().unwrap();
            let prefix = &line[..line.len() - last.len()];
            assert_eq!(prefix.width(), 18, "{line}");
        }
    }

    #[test]
    fn borders_have_matching_unicode_display_widths() {
        let rows = [
            ["日本語".into(), "1.0.0".into()],
            ["e\u{301}".into(), "2.0.0".into()],
        ];
        // Rendering without colors makes this assertion independent of terminal policy.
        let table = render(["Package", "Latest"], &rows, true, false);
        let width = table.lines().next().unwrap().width();
        assert!(table.starts_with('┌'));
        assert!(table.contains('┼'));
        assert!(table.ends_with("┘\n"));
        for line in table.lines() {
            assert_eq!(line.width(), width, "{line}");
        }
    }
}
