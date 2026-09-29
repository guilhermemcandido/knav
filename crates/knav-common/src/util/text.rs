//! Measuring and shortening text in terminal cells.

pub fn cell_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// Cuts `s` to `max` cells, ending in an ellipsis when it had to cut.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if cell_width(s) <= max {
        return s.to_string();
    }
    // Leave a cell for the ellipsis, cutting on whole characters.
    let (mut out, mut used) = (String::new(), 0);
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// `bytes` in the largest unit that keeps it at or above 1, one decimal.
pub fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1}{}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_counts_terminal_cells_not_characters() {
        assert_eq!(cell_width("日本語"), 6);
        let cut = truncate("日本語日本語", 7);
        assert!(cell_width(&cut) <= 7 && cut.ends_with('…'), "{cut}");
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 0), "");
    }
}
