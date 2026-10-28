//! Text helpers with Python semantics where the specs depend on them
//! (docs/phase1/conventions.md §12). Lengths are code points, never bytes.

/// Python's `str.isspace()`: Unicode whitespace plus the separators U+001C..U+001F.
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Python's `" ".join(s.split())`: trims and collapses every whitespace run to one space.
pub fn collapse_ws(s: &str) -> String {
    s.split(is_py_space)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Length in code points (Python `len()`). Never use `str::len()` for limits.
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// The first `n` code points of `s` (Python `s[:n]`).
pub fn truncate_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((index, _)) => &s[..index],
        None => s,
    }
}

/// Python's `str.casefold()` (full Unicode case folding: "Straße" → "strasse").
pub fn casefold(s: &str) -> String {
    caseless::default_case_fold_str(s)
}

/// `s` when it has at most `max` code points, else its first `max - 3` plus "…" (U+2026).
/// With `max` = 90 this is the legacy note excerpt (at most 88 characters).
pub fn excerpt(s: &str, max: usize) -> String {
    if char_len(s) <= max {
        s.to_owned()
    } else {
        format!("{}…", truncate_chars(s, max.saturating_sub(3)))
    }
}

/// Python's `round(x, digits)`: half to even on the exact decimal value of `x`.
pub fn round_half_even(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_matches_python() {
        assert_eq!(py_strip("  ada\t\n"), "ada");
        assert_eq!(py_strip("\u{1c}\u{1f}ada\u{1d}"), "ada");
        assert_eq!(py_strip("\u{a0}\u{2003}ada\u{3000}"), "ada");
        assert_eq!(py_strip("\u{85}ada"), "ada");
        assert_eq!(py_strip("\u{200b}ada"), "\u{200b}ada");
        assert_eq!(py_strip("   "), "");
    }

    #[test]
    fn collapse_matches_python_split_join() {
        assert_eq!(
            collapse_ws("  The   Left\tHand\nof \u{1e} Darkness "),
            "The Left Hand of Darkness"
        );
        assert_eq!(collapse_ws(""), "");
        assert_eq!(collapse_ws(" \u{3000} "), "");
    }

    #[test]
    fn lengths_count_code_points() {
        assert_eq!(char_len("pässwörd"), 8);
        assert_eq!("pässwörd".len(), 10);
        assert_eq!(char_len("📚x"), 2);
    }

    #[test]
    fn truncation_never_splits_a_character() {
        assert_eq!(truncate_chars("pässwörd", 2), "pä");
        assert_eq!(truncate_chars("📚📚📚", 1), "📚");
        assert_eq!(truncate_chars("abc", 5), "abc");
        assert_eq!(truncate_chars("abc", 0), "");
    }

    #[test]
    fn casefold_is_full_folding() {
        assert_eq!(casefold("Straße"), "strasse");
        assert_eq!(casefold("ÉCOLE"), "école");
        assert_eq!(casefold("\u{212a}elvin"), "kelvin");
    }

    #[test]
    fn excerpt_rule() {
        let short = "a".repeat(90);
        assert_eq!(excerpt(&short, 90), short);
        let long = "ä".repeat(91);
        let cut = excerpt(&long, 90);
        assert_eq!(char_len(&cut), 88);
        assert!(cut.ends_with('…'));
        assert_eq!(cut, format!("{}…", "ä".repeat(87)));
    }

    #[test]
    fn rounding_matches_python_round() {
        // Python: round(2.5) == 2, round(3.5) == 4, round(0.125, 2) == 0.12,
        // round(2.675, 2) == 2.67 (2.675 is 2.67499999… in binary), round(4.35, 1) == 4.3.
        assert_eq!(round_half_even(2.5, 0), 2.0);
        assert_eq!(round_half_even(3.5, 0), 4.0);
        assert_eq!(round_half_even(0.125, 2), 0.12);
        assert_eq!(round_half_even(0.375, 2), 0.38);
        assert_eq!(round_half_even(2.675, 2), 2.67);
        assert_eq!(round_half_even(4.35, 1), 4.3);
        assert_eq!(round_half_even(1.23456, 1), 1.2);
        assert_eq!(round_half_even(-2.5, 0), -2.0);
        assert_eq!(round_half_even(4.0, 1), 4.0);
    }
}
