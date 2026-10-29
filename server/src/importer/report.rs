//! The import report (docs/phase1/legacy-mapping.md §0 "Report"): one line per entity,
//! `source rows → target rows` with merged, dropped and normalised counts, then the details a
//! dry run has to show (every normalisation, every merge, every ISBN outcome).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

/// The width of the entity name column (the longest legacy table name).
const NAME_WIDTH: usize = 22;

/// One entity's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityLine {
    pub name: String,
    /// Rows in the legacy table; `None` for entities with no legacy table (the club).
    pub source: Option<u64>,
    /// Rows written (or, for a dropped table, `None`).
    pub target: Option<u64>,
    /// Text after the counts, e.g. `(1 used, 1 unused imported as revoked)`.
    pub detail: String,
}

/// What a legacy `ISBN…` key resolved to (legacy-mapping.md §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsbnOutcome {
    /// A source or the local catalog knew the ISBN.
    Matched { work_id: i64, title: String },
    /// Every source answered and none knew it: bare facts.
    Unmatched,
    /// A source was unavailable (`--allow-unmatched`): bare facts.
    Error,
    /// The key is not a valid ISBN: bare facts without an edition.
    Invalid,
    /// Another legacy key with the same ISBN came first.
    SameAs { legacy_book_id: i64 },
}

/// One legacy `ISBN…` key and its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsbnLine {
    pub legacy_book_id: i64,
    /// The key as stored (`/works/ISBN…`).
    pub key: String,
    /// The ISBN-13 it parsed to.
    pub isbn13: Option<String>,
    pub outcome: IsbnOutcome,
}

/// Accumulates the report while the import runs.
#[derive(Debug, Clone, Default)]
pub struct ReportBuilder {
    pub entities: Vec<EntityLine>,
    pub normalised: BTreeMap<String, u64>,
    pub merges: Vec<String>,
    pub differences: Vec<String>,
    pub isbns: Vec<IsbnLine>,
    pub lookup_wait: Duration,
    pub notes: Vec<String>,
}

impl ReportBuilder {
    /// Adds an entity line.
    pub fn entity(
        &mut self,
        name: &str,
        source: Option<u64>,
        target: Option<u64>,
        detail: impl Into<String>,
    ) {
        self.entities.push(EntityLine {
            name: name.to_owned(),
            source,
            target,
            detail: detail.into(),
        });
    }

    /// Counts one repaired value of `table.column`.
    pub fn normalised(&mut self, column: &str) {
        self.normalised_by(column, 1);
    }

    /// Counts `n` repaired values of `table.column`. Nothing repaired adds no entry, so the
    /// line does not list the column with 0.
    pub fn normalised_by(&mut self, column: &str, n: u64) {
        if n == 0 {
            return;
        }
        *self.normalised.entry(column.to_owned()).or_default() += n;
    }

    /// The rendered lines.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for e in &self.entities {
            let source = e.source.map_or("-".to_owned(), |n| n.to_string());
            let target = e.target.map_or("DROPPED".to_owned(), |n| n.to_string());
            let mut line = format!("{:<NAME_WIDTH$} {source} → {target}", e.name);
            if !e.detail.is_empty() {
                line.push(' ');
                line.push_str(&e.detail);
            }
            lines.push(line);
        }
        if self.normalised.is_empty() {
            lines.push(format!("{:<NAME_WIDTH$} none", "normalised"));
        } else {
            let parts: Vec<String> = self
                .normalised
                .iter()
                .map(|(column, n)| format!("{column} {n}"))
                .collect();
            lines.push(format!(
                "{:<NAME_WIDTH$} {}",
                "normalised",
                parts.join(", ")
            ));
        }
        let secs = self.lookup_wait.as_secs_f64();
        let mut wait = String::new();
        let _ = write!(wait, "{:<NAME_WIDTH$} {secs:.1} s", "lookup wait");
        lines.push(wait);
        section(&mut lines, "merges", &self.merges);
        section(&mut lines, "differences", &self.differences);
        let isbns: Vec<String> = self.isbns.iter().map(isbn_line).collect();
        section(&mut lines, "isbn keys", &isbns);
        section(&mut lines, "notes", &self.notes);
        lines
    }
}

fn section(lines: &mut Vec<String>, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    lines.push(format!("{title}:"));
    lines.extend(items.iter().map(|item| format!("  {item}")));
}

fn isbn_line(line: &IsbnLine) -> String {
    let isbn = line.isbn13.as_deref().unwrap_or("-");
    let outcome = match &line.outcome {
        // The work id is left out: sequences advance in a dry run, so it would differ from the
        // real run's.
        IsbnOutcome::Matched { title, .. } => format!("matched \"{title}\""),
        IsbnOutcome::Unmatched => "unmatched (bare facts)".to_owned(),
        IsbnOutcome::Error => "error: source unavailable (bare facts)".to_owned(),
        IsbnOutcome::Invalid => "invalid ISBN (bare facts, no edition)".to_owned(),
        IsbnOutcome::SameAs { legacy_book_id } => format!("same ISBN as book {legacy_book_id}"),
    };
    format!(
        "{} (book {}, isbn {isbn}): {outcome}",
        line.key, line.legacy_book_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_show_counts_details_and_sections() {
        let mut r = ReportBuilder::default();
        r.entity("users", Some(3), Some(3), "");
        r.entity("club", None, Some(1), "\"Readers\" (readers-k3x9)");
        r.entity("push_subscriptions", Some(2), None, "(Phase 4)");
        r.normalised("books.pages");
        r.normalised("books.pages");
        r.normalised("books.year");
        r.merges.push("book 10 → the work of book 9".into());
        r.isbns.push(IsbnLine {
            legacy_book_id: 10,
            key: "/works/ISBN9780000000019".into(),
            isbn13: Some("9780000000019".into()),
            outcome: IsbnOutcome::Unmatched,
        });
        assert_eq!(
            r.lines(),
            vec![
                "users                  3 → 3",
                "club                   - → 1 \"Readers\" (readers-k3x9)",
                "push_subscriptions     2 → DROPPED (Phase 4)",
                "normalised             books.pages 2, books.year 1",
                "lookup wait            0.0 s",
                "merges:",
                "  book 10 → the work of book 9",
                "isbn keys:",
                "  /works/ISBN9780000000019 (book 10, isbn 9780000000019): unmatched (bare facts)",
            ]
        );
    }

    /// A step that repaired nothing (CUT2's rehearsal printed `events.payload.rating_take 0`)
    /// leaves the line as it is.
    #[test]
    fn a_count_of_zero_is_not_listed() {
        let mut r = ReportBuilder::default();
        r.normalised_by("events.payload.rating_take", 0);
        assert!(
            r.lines()
                .contains(&format!("{:<NAME_WIDTH$} none", "normalised"))
        );
        r.normalised_by("book_posts.pick_id", 1);
        r.normalised_by("events.payload.rating_take", 0);
        assert!(
            r.lines().contains(&format!(
                "{:<NAME_WIDTH$} book_posts.pick_id 1",
                "normalised"
            )),
            "{:?}",
            r.lines()
        );
    }
}
