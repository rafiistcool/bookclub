//! ISBNs (docs/phase1/conventions.md §7.1). Every ISBN is stored and looked up as a checked
//! ISBN-13: ISBN-10 input becomes its 978 form, and 979 ISBNs have no ISBN-10.

use std::fmt;

use crate::text::is_py_space;

/// A checked ISBN-13: 13 digits starting with 978 or 979, valid checksum. Built only by
/// [`parse_isbn`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Isbn13(pub(crate) String);

impl Isbn13 {
    /// The 13 digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Isbn13 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The legacy `extract_isbn` normalisation: drops whitespace and `-`, uppercases.
fn compact(raw: &str) -> String {
    raw.chars()
        .filter(|&c| c != '-' && !is_py_space(c))
        .flat_map(char::to_uppercase)
        .collect()
}

fn is_isbn10_shape(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[..9].iter().all(u8::is_ascii_digit)
        && (b[9].is_ascii_digit() || b[9] == b'X')
}

fn is_isbn13_shape(s: &str) -> bool {
    s.len() == 13 && s.bytes().all(|b| b.is_ascii_digit())
}

fn digit(b: u8) -> u32 {
    u32::from(b - b'0')
}

/// The ISBN-10 check character for nine digits.
fn isbn10_check(nine: &[u8]) -> char {
    let sum: u32 = nine
        .iter()
        .zip((2..=10).rev())
        .map(|(&b, weight)| digit(b) * weight)
        .sum();
    match (11 - sum % 11) % 11 {
        10 => 'X',
        d => char::from_digit(d, 10).expect("a digit"),
    }
}

/// The ISBN-13 check digit for twelve digits.
fn isbn13_check(twelve: &[u8]) -> char {
    let sum: u32 = twelve
        .iter()
        .enumerate()
        .map(|(i, &b)| digit(b) * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    char::from_digit((10 - sum % 10) % 10, 10).expect("a digit")
}

/// Parses an ISBN-10 or ISBN-13 as typed: spaces and hyphens are ignored, `x` counts as `X`, the
/// checksum must hold, and an ISBN-13 must start with 978 or 979. ISBN-10 input is converted to
/// its 978 ISBN-13.
pub fn parse_isbn(raw: &str) -> Option<Isbn13> {
    let s = compact(raw);
    let b = s.as_bytes();
    if is_isbn10_shape(&s) {
        if isbn10_check(&b[..9]) != char::from(b[9]) {
            return None;
        }
        let mut twelve = format!("978{}", &s[..9]);
        let check = isbn13_check(twelve.as_bytes());
        twelve.push(check);
        return Some(Isbn13(twelve));
    }
    let valid = is_isbn13_shape(&s)
        && (s.starts_with("978") || s.starts_with("979"))
        && isbn13_check(&b[..12]) == char::from(b[12]);
    valid.then_some(Isbn13(s))
}

/// The ISBN-10 form of a 978 ISBN; `None` for 979 ISBNs, which have none. `EditionOut.isbn10` is
/// always derived through this.
pub fn isbn10_of(isbn: &Isbn13) -> Option<String> {
    let nine = isbn.0.strip_prefix("978")?.get(..9)?;
    let mut ten = nine.to_owned();
    ten.push(isbn10_check(nine.as_bytes()));
    Some(ten)
}

/// Whether a search query is shaped like an ISBN-10 or ISBN-13 (no checksum check), for routing
/// it to the ISBN lookup.
pub fn looks_like_isbn(q: &str) -> bool {
    let s = compact(q);
    is_isbn10_shape(&s) || is_isbn13_shape(&s)
}

/// 978-3: the German-language registration group, looked up at DNB first (conventions §7.4).
pub fn is_german_group(isbn: &Isbn13) -> bool {
    isbn.0.starts_with("9783")
}

/// The prefix plus registration group, e.g. `"9783"` (German), `"97888"` (Italian),
/// `"979"` + `"10"` (French). Used to drop foreign co-publisher ISBNs from a DNB record.
pub fn registration_group(isbn: &Isbn13) -> &str {
    let s = isbn.0.as_str();
    let rest = &s[3..];
    let num = |n: usize| rest[..n].parse::<u32>().unwrap_or(0);
    let len = if s.starts_with("979") {
        if rest.starts_with('8') { 1 } else { 2 }
    } else {
        match num(1) {
            0..=5 | 7 => 1,
            8 => 2,
            9 => match num(2) {
                90..=94 => 2,
                95..=98 => 3,
                _ if num(4) <= 9989 => 4,
                _ => 5,
            },
            _ => match num(3) {
                600..=649 => 3,
                650..=659 => 2,
                _ => 3,
            },
        }
    };
    &s[..3 + len]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isbn(raw: &str) -> Isbn13 {
        parse_isbn(raw).unwrap_or_else(|| panic!("{raw} should parse"))
    }

    #[test]
    fn isbn10_input_becomes_978_isbn13() {
        assert_eq!(isbn("0-8044-2957-X").as_str(), "9780804429573");
        assert_eq!(isbn("080442957x").as_str(), "9780804429573");
        assert_eq!(isbn(" 3 89561 419 X ").as_str(), "9783895614194");
        assert_eq!(isbn("3518368818").as_str(), "9783518368817");
        // The legacy parity fixture: all zeros is checksum-valid.
        assert_eq!(isbn("0000000000").as_str(), "9780000000002");
    }

    #[test]
    fn isbn13_input_is_kept_when_valid() {
        assert_eq!(isbn("978-0-316-76948-8").as_str(), "9780316769488");
        assert_eq!(isbn("9780316556347").as_str(), "9780316556347");
        assert_eq!(isbn("979-10-90636-07-1").as_str(), "9791090636071");
    }

    #[test]
    fn bad_checksums_and_shapes_are_refused() {
        for raw in [
            "0804429578",     // wrong ISBN-10 check digit
            "9780316769487",  // wrong ISBN-13 check digit
            "9771234567003",  // valid EAN, not an ISBN prefix
            "X804429573",     // X only as the last ISBN-10 character
            "97803167694888", // 14 digits
            "",
            "isbn",
            "978031676948８", // a full-width digit is not an ASCII digit
        ] {
            assert_eq!(parse_isbn(raw), None, "{raw}");
        }
    }

    #[test]
    fn isbn10_is_derived_for_978_only() {
        assert_eq!(
            isbn10_of(&isbn("9780804429573")).as_deref(),
            Some("080442957X")
        );
        assert_eq!(
            isbn10_of(&isbn("9783895614194")).as_deref(),
            Some("389561419X")
        );
        assert_eq!(
            isbn10_of(&isbn("9780316769488")).as_deref(),
            Some("0316769487")
        );
        assert_eq!(isbn10_of(&isbn("9791090636071")), None);
        // Round trip through both forms.
        for raw in ["9783648139776", "9783112270158", "9780000000002"] {
            let ten = isbn10_of(&isbn(raw)).unwrap();
            assert_eq!(isbn(&ten).as_str(), raw);
        }
    }

    #[test]
    fn query_shapes_ignore_checksums() {
        assert!(looks_like_isbn("978-0-316-76948-7"));
        assert!(looks_like_isbn("080442957x"));
        assert!(looks_like_isbn(" 0 8044 2957 8 "));
        assert!(!looks_like_isbn("Circe"));
        assert!(!looks_like_isbn("97803167694"));
        assert!(!looks_like_isbn("X804429573"));
    }

    #[test]
    fn german_group_is_978_3() {
        assert!(is_german_group(&isbn("9783895614194")));
        assert!(is_german_group(&isbn("3-518-36881-8")));
        assert!(!is_german_group(&isbn("9780316769488")));
        assert!(!is_german_group(&isbn("9788368826104")));
        // 979-3… does not exist as a German group; 979 is never German.
        assert!(!is_german_group(&isbn("9791090636071")));
    }

    #[test]
    fn registration_groups_follow_the_range_table() {
        assert_eq!(registration_group(&isbn("9783895614194")), "9783");
        assert_eq!(registration_group(&isbn("9780316769488")), "9780");
        assert_eq!(registration_group(&isbn("9788368826104")), "97883");
        assert_eq!(registration_group(&isbn("9791090636071")), "97910");
        assert_eq!(registration_group(&isbn("9798000000007")), "9798");
        assert_eq!(registration_group(&isbn("9786000000004")), "978600");
        assert_eq!(registration_group(&isbn("9789500000000")), "978950");
        assert_eq!(registration_group(&isbn("9789900000006")), "9789900");
        assert_eq!(registration_group(&isbn("9789999100007")), "97899991");
    }
}
