//! ISBNs (docs/phase1/conventions.md §7.1). Wave 1 step 2 adds `parse_isbn`, `isbn10_of`,
//! `looks_like_isbn` and `is_german_group`.

/// A checked ISBN-13: 13 digits, valid checksum. Built only by `parse_isbn`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Isbn13(pub(crate) String);

impl Isbn13 {
    /// The 13 digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
