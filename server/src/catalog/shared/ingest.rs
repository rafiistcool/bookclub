//! Ingest: the only writer of catalog fields, with field-level provenance
//! (docs/phase1/conventions.md §7.2, §7.3).
//!
//! Every write loads the row `FOR UPDATE`, merges the record into it under the provenance rules
//! ([`decide`]), and writes it back once. Derived columns (`search_extra`, `match_key`) and the
//! NULL fill of work fields from editions are maintained after every edition write. Callers run
//! multi-statement writes in a transaction; the advisory locks taken here then serialise
//! concurrent ingests of the same ISBN, IDN, OL edition or DNB title. A transaction over several
//! records can still deadlock with another one and is retried (§7.2 "Deadlocks").

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sqlx::types::Json;
use sqlx::{Connection, PgConnection};
use time::Date;
use time::macros::format_description;
use unicode_normalization::UnicodeNormalization;

use super::books::{follow_merges, follow_merges_many};
use super::dnb_marc::DnbRecord;
use super::isbn::Isbn13;
use crate::error::{ApiResult, AppError, is_unique_violation, msg};
use crate::extract::Patch;
use crate::media::shared::urls::new_public_id;
use crate::text::{casefold, char_len, collapse_ws, py_strip, truncate_chars};

// ---------------------------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------------------------

/// Who wrote a catalog field. `Admin` is field provenance only, never a row source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    OpenLibrary,
    Dnb,
    User,
    Legacy,
    Admin,
}

impl Source {
    /// Legacy 1, OpenLibrary/Dnb 2, User 3, Admin 4. A source may overwrite a field of equal or
    /// lower rank.
    pub fn rank(self) -> u8 {
        match self {
            Source::Legacy => 1,
            Source::OpenLibrary | Source::Dnb => 2,
            Source::User => 3,
            Source::Admin => 4,
        }
    }

    /// The value stored in `source` columns and `field_sources`.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::OpenLibrary => "openlibrary",
            Source::Dnb => "dnb",
            Source::User => "user",
            Source::Legacy => "legacy",
            Source::Admin => "admin",
        }
    }

    /// The inverse of [`Source::as_str`].
    pub fn parse(value: &str) -> Option<Source> {
        Some(match value {
            "openlibrary" => Source::OpenLibrary,
            "dnb" => Source::Dnb,
            "user" => Source::User,
            "legacy" => Source::Legacy,
            "admin" => Source::Admin,
            _ => return None,
        })
    }
}

/// An edition contributor's role (`edition_contributors.role`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContributorRole {
    Translator,
    Illustrator,
    Editor,
}

impl ContributorRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ContributorRole::Translator => "translator",
            ContributorRole::Illustrator => "illustrator",
            ContributorRole::Editor => "editor",
        }
    }
}

/// An edition's format (`editions.format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Hardcover,
    Paperback,
    Ebook,
    Audiobook,
    Other,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Hardcover => "hardcover",
            Format::Paperback => "paperback",
            Format::Ebook => "ebook",
            Format::Audiobook => "audiobook",
            Format::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Option<Format> {
        Some(match value {
            "hardcover" => Format::Hardcover,
            "paperback" => Format::Paperback,
            "ebook" => Format::Ebook,
            "audiobook" => Format::Audiobook,
            "other" => Format::Other,
            _ => return None,
        })
    }
}

/// An author as a source delivers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorRecord {
    /// Natural order: "Udo Paulitz".
    pub name: String,
    /// Inverted form when known: "Paulitz, Udo".
    pub sort_name: Option<String>,
    pub ol_author_id: Option<String>,
    pub gnd_id: Option<String>,
}

impl AuthorRecord {
    /// An author known only by name.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sort_name: None,
            ol_author_id: None,
            gnd_id: None,
        }
    }
}

/// Work-level facts from one source. `None` (or an empty string or list) means "this record
/// has no value"; what that does depends on the [`Mode`].
#[derive(Debug, Clone, PartialEq)]
pub struct WorkRecord {
    pub source: Source,
    pub ol_work_id: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    /// Display string, natural order, ", "-joined. Derived from `authors` when empty.
    pub author_names: String,
    pub authors: Vec<AuthorRecord>,
    pub first_publish_year: Option<i32>,
    pub description: Option<String>,
    pub subjects: Option<Vec<String>>,
    pub pages: Option<i32>,
    pub ol_rating: Option<f64>,
    pub ol_rating_count: Option<i32>,
    pub ol_cover_id: Option<i64>,
    /// Sets `details_fetched_at` (the full record, not a search stub).
    pub full_details: bool,
    pub created_by: Option<i64>,
}

impl WorkRecord {
    /// A record with only a title; set the other fields with struct update syntax.
    pub fn new(source: Source, title: impl Into<String>) -> Self {
        Self {
            source,
            ol_work_id: None,
            title: title.into(),
            subtitle: None,
            author_names: String::new(),
            authors: Vec::new(),
            first_publish_year: None,
            description: None,
            subjects: None,
            pages: None,
            ol_rating: None,
            ol_rating_count: None,
            ol_cover_id: None,
            full_details: false,
            created_by: None,
        }
    }
}

/// Edition-level facts from one source.
#[derive(Debug, Clone, PartialEq)]
pub struct EditionRecord {
    pub source: Source,
    pub isbn13: Option<Isbn13>,
    pub ol_edition_id: Option<String>,
    /// MARC 001 of the record, on the first ISBN's edition only (`editions.dnb_idn` is unique).
    pub dnb_idn: Option<String>,
    /// MARC 001 of the delivering DNB record, on every edition of the record
    /// (`editions.dnb_record_idn`, not unique). `None` falls back to `dnb_idn`.
    pub dnb_record_idn: Option<String>,
    /// Stored only when it differs (by case fold) from the work title.
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub publisher: Option<String>,
    /// ISO 639-2/B, e.g. `ger`.
    pub language: Option<String>,
    pub format: Option<Format>,
    pub pages: Option<i32>,
    pub publication_year: Option<i32>,
    /// Missing keeps the stored date in every mode, Null clears it, Value sets it.
    pub planned_publication_date: Patch<Date>,
    pub ol_cover_id: Option<i64>,
    pub contributors: Vec<(AuthorRecord, ContributorRole)>,
    pub created_by: Option<i64>,
}

impl EditionRecord {
    /// A record with no facts; set the fields with struct update syntax.
    pub fn new(source: Source) -> Self {
        Self {
            source,
            isbn13: None,
            ol_edition_id: None,
            dnb_idn: None,
            dnb_record_idn: None,
            title: None,
            subtitle: None,
            publisher: None,
            language: None,
            format: None,
            pages: None,
            publication_year: None,
            planned_publication_date: Patch::Missing,
            ol_cover_id: None,
            contributors: Vec::new(),
            created_by: None,
        }
    }
}

/// What a record does to fields it has no value for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Existing rows are returned untouched (search stubs, one batched statement).
    InsertOnly,
    /// Some values fill empty fields and overwrite fields of equal or lower rank; None keeps the
    /// old value.
    Refresh,
    /// A complete record from an authoritative feed (DNB OAI and SRU): as Refresh, and None also
    /// clears a field whose provenance is this same source. `planned_publication_date` follows its
    /// Patch (Missing keeps, Null clears, Value sets), and a record without a cover never clears
    /// one.
    Replace,
}

/// How `upsert_edition_with_work` finds the work for a new edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkGrouping {
    /// Create a new work from the WorkRecord (no deduplication; the caller decided). A record
    /// with an `ol_work_id` still finds that work.
    NewWork,
    /// Attach to this existing work (following merges; Err for a missing work).
    Into(i64),
    /// DNB: an existing non-merged `source='dnb'` work with the same match_key, else a new work.
    /// A record without an author always gets a new work.
    DnbTitleAuthor,
}

/// A row that has a cover (and edit-request target).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverTarget {
    Work(i64),
    Edition(i64),
}

// ---------------------------------------------------------------------------------------------
// Pure rules
// ---------------------------------------------------------------------------------------------

static OL_WORK_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new("^OL[0-9]+W$").unwrap());
static OL_EDITION_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new("^OL[0-9]+M$").unwrap());
static OL_AUTHOR_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new("^OL[0-9]+A$").unwrap());
static LANGUAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new("^[a-z]{3}$").unwrap());

static DESCRIPTION_TRAILER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n+-{3,}[\s\S]*$").unwrap());
static REFERENCE_DEFINITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*\[[^\]]+\]:[ \t]*\S+.*$").unwrap());
static INLINE_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap());
static REFERENCE_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\[[^\]]*\]").unwrap());
static BLOCKQUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*>[ \t]?").unwrap());
static BLANK_LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

/// The union of the two legacy description cleaners (docs/phase1/spec/catalog.md §2.11): the
/// `----` source trailer goes, markdown links and blockquotes are flattened.
pub fn clean_description(raw: &str) -> String {
    let text = py_strip(raw);
    let text = DESCRIPTION_TRAILER.replace(text, "");
    let text = REFERENCE_DEFINITION.replace_all(&text, "");
    let text = INLINE_LINK.replace_all(&text, "$1");
    let text = REFERENCE_LINK.replace_all(&text, "$1");
    let text = BLOCKQUOTE.replace_all(&text, "");
    let text = BLANK_LINES.replace_all(&text, "\n\n");
    py_strip(&text).to_owned()
}

/// Subject cleaning (docs/phase1/spec/catalog.md §2.12, unified): drops blanks and entries that
/// contain `:` or `,` or exceed 40 characters (machine tags included), dedupes
/// case-insensitively, keeps at most 12.
pub fn clean_subjects(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .iter()
        .map(|value| py_strip(value))
        .filter(|name| {
            !name.is_empty() && char_len(name) <= 40 && !name.contains(':') && !name.contains(',')
        })
        .filter(|name| seen.insert(name.to_lowercase()))
        .take(12)
        .map(str::to_owned)
        .collect()
}

/// `authors.name_key`: `casefold(collapse_ws(name))`.
pub fn name_key(name: &str) -> String {
    truncate_chars(&casefold(&collapse_ws(name)), 900).to_owned()
}

/// The surname used by `match_key`: the part before the comma of an inverted name (`sort_name`
/// when known), else the last word of the natural-order name.
pub fn author_surname(name: &str, sort_name: Option<&str>) -> String {
    if let Some(sort) = sort_name
        && let Some((surname, _)) = sort.split_once(',')
    {
        return collapse_ws(surname);
    }
    let first = name.split(',').next().unwrap_or("");
    collapse_ws(first)
        .rsplit(' ')
        .next()
        .unwrap_or("")
        .to_owned()
}

/// `works.match_key`: case-folded title, U+001F, case-folded surname of the first author.
pub fn match_key(title: &str, surname: &str) -> String {
    let key = format!(
        "{}\u{1f}{}",
        casefold(&collapse_ws(title)),
        casefold(&collapse_ws(surname))
    );
    truncate_chars(&key, 1500).to_owned()
}

/// `works.search_extra`: the edition titles that differ by case fold from the work title,
/// deduplicated the same way, " / "-joined.
pub fn search_extra(work_title: &str, edition_titles: &[String]) -> String {
    let mut seen = HashSet::from([casefold(&collapse_ws(work_title))]);
    let titles: Vec<&str> = edition_titles
        .iter()
        .map(|title| title.as_str())
        .filter(|title| seen.insert(casefold(&collapse_ws(title))))
        .collect();
    truncate_chars(&titles.join(" / "), 5000).to_owned()
}

/// The surname `match_key` uses for a work record.
fn record_surname(authors: &[AuthorRecord], author_names: &str) -> String {
    match authors.first() {
        Some(author) => author_surname(&author.name, author.sort_name.as_deref()),
        None => author_surname(author_names, None),
    }
}

/// What one record does to one field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Keep,
    Write,
    Clear,
}

/// The provenance rules for one field: an empty field may be filled, unless an admin cleared it
/// (provenance `admin` without a value: only an admin fills it again); a value overwrites a field
/// of equal or lower rank; under Replace, a missing value clears the source's own field.
fn decide(has_value: bool, owner: Source, incoming: bool, by: Source, mode: Mode) -> Action {
    if incoming {
        if (!has_value && owner != Source::Admin) || owner.rank() <= by.rank() {
            Action::Write
        } else {
            Action::Keep
        }
    } else if mode == Mode::Replace && has_value && owner == by {
        Action::Clear
    } else {
        Action::Keep
    }
}

/// `field_sources`: which source wrote each field that holds a value, and `admin` for a field an
/// admin cleared (it stays empty until an admin sets it).
#[derive(Debug, Clone, Default)]
struct Provenance(Map<String, Value>);

impl Provenance {
    fn get(&self, field: &str) -> Option<Source> {
        self.0.get(field)?.as_str().and_then(Source::parse)
    }

    fn set(&mut self, field: &str, by: Source) {
        self.0
            .insert(field.to_owned(), Value::String(by.as_str().to_owned()));
    }

    fn remove(&mut self, field: &str) {
        self.0.remove(field);
    }

    fn to_value(&self) -> Value {
        Value::Object(self.0.clone())
    }
}

/// Merges one record into one row, field by field.
struct Merger {
    prov: Provenance,
    /// The owner of a field that holds a value without recorded provenance: the row's source.
    fallback: Source,
    by: Source,
    mode: Mode,
    /// Only `legacy`-ranked values may change (the parent work of a found edition, §7.2).
    only_legacy: bool,
    changed: bool,
}

impl Merger {
    fn new(prov: Map<String, Value>, fallback: Source, by: Source, mode: Mode) -> Self {
        Self {
            prov: Provenance(prov),
            fallback,
            by,
            mode,
            only_legacy: false,
            changed: false,
        }
    }

    fn owner(&self, field: &str) -> Source {
        self.prov.get(field).unwrap_or(self.fallback)
    }

    fn action(&self, field: &str, has_value: bool, incoming: bool) -> Action {
        let owner = self.owner(field);
        if self.only_legacy && !(has_value && owner == Source::Legacy) {
            return Action::Keep;
        }
        decide(has_value, owner, incoming, self.by, self.mode)
    }

    fn mark(&mut self, field: &str, action: Action) {
        match action {
            Action::Write if self.prov.get(field) != Some(self.by) => {
                self.prov.set(field, self.by);
                self.changed = true;
            }
            Action::Clear => {
                self.prov.remove(field);
                self.changed = true;
            }
            _ => {}
        }
    }

    /// Merges a field whose empty value is `T::default()` (`''`, NULL, `{}`).
    fn field<T: PartialEq + Default>(
        &mut self,
        field: &str,
        current: &mut T,
        incoming: Option<T>,
    ) -> Action {
        let empty = T::default();
        let has_value = *current != empty;
        let incoming = incoming.filter(|value| *value != empty);
        let action = self.action(field, has_value, incoming.is_some());
        match (action, incoming) {
            (Action::Write, Some(value)) => {
                if *current != value {
                    *current = value;
                    self.changed = true;
                }
            }
            (Action::Clear, _) => *current = empty,
            _ => {}
        }
        self.mark(field, action);
        action
    }

    /// A field that only changes when the record carries a value: a NOT NULL title, and covers
    /// (a record without a cover never removes one).
    fn value_only<T: PartialEq + Default>(
        &mut self,
        field: &str,
        current: &mut T,
        incoming: Option<T>,
    ) -> Action {
        if incoming.is_none() {
            return Action::Keep;
        }
        self.field(field, current, incoming)
    }

    /// `planned_publication_date`: Missing keeps (every mode), Null clears under the rank
    /// rule, Value writes like any field.
    fn patch<T: PartialEq + Clone>(
        &mut self,
        field: &str,
        current: &mut Option<T>,
        incoming: &Patch<T>,
    ) {
        match incoming {
            Patch::Missing => {}
            Patch::Value(value) => {
                self.field(field, current, Some(Some(value.clone())));
            }
            Patch::Null => {
                let owner = self.owner(field);
                let allowed = !self.only_legacy || owner == Source::Legacy;
                if current.is_some() && allowed && owner.rank() <= self.by.rank() {
                    *current = None;
                    self.mark(field, Action::Clear);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Sanitising: values outside the schema's CHECKs are dropped or truncated, never refused.
// ---------------------------------------------------------------------------------------------

/// Catalog text is stored composed (NFC), so the same title from two sources compares equal.
/// Control characters other than tab and line breaks are dropped: Postgres `text` refuses U+0000,
/// which Open Library JSON sometimes carries, and the others are never meant as text.
pub(crate) fn nfc(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .nfc()
        .collect()
}

/// [`nfc`], whitespace collapsed, cut to `max` characters; `None` when nothing is left.
pub(crate) fn clean_line(value: Option<&str>, max: usize) -> Option<String> {
    let value = collapse_ws(&nfc(value?));
    (!value.is_empty()).then(|| truncate_chars(&value, max).to_owned())
}

fn clean_pages(pages: Option<i32>) -> Option<i32> {
    pages.filter(|p| (1..=100_000).contains(p))
}

/// The years `works.first_publish_year` and `editions.publication_year` accept.
const YEARS: std::ops::RangeInclusive<i32> = -5000..=3000;

fn clean_year(year: Option<i32>) -> Option<i32> {
    year.filter(|y| YEARS.contains(y))
}

/// A planned date whose year the work's `first_publish_year` could not take is dropped (the
/// record has no value, which keeps the stored date).
fn clean_planned(date: &Patch<Date>) -> Patch<Date> {
    match date {
        Patch::Value(d) if !YEARS.contains(&d.year()) => Patch::Missing,
        other => other.clone(),
    }
}

fn clean_id(value: Option<&str>, pattern: &Regex) -> Option<String> {
    value.filter(|v| pattern.is_match(v)).map(str::to_owned)
}

fn clean_language(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    LANGUAGE.is_match(&value).then_some(value)
}

fn clean_author(author: &AuthorRecord) -> Option<AuthorRecord> {
    Some(AuthorRecord {
        name: clean_line(Some(&author.name), 300)?,
        sort_name: clean_line(author.sort_name.as_deref(), 300),
        ol_author_id: clean_id(author.ol_author_id.as_deref(), &OL_AUTHOR_ID),
        gnd_id: clean_line(author.gnd_id.as_deref(), 32).filter(|g| !g.contains(' ')),
    })
}

/// A work record with every value inside the schema's limits.
struct CleanWork {
    title: Option<String>,
    subtitle: Option<String>,
    author_names: Option<String>,
    authors: Vec<AuthorRecord>,
    first_publish_year: Option<i32>,
    description: Option<String>,
    subjects: Option<Vec<String>>,
    pages: Option<i32>,
    ol_rating: Option<f64>,
    ol_rating_count: Option<i32>,
    ol_cover_id: Option<i64>,
    /// The usable media row for `ol_cover_id`, resolved by [`CleanWork::resolve_cover`] before
    /// any catalog row is locked (see [`CleanEdition::resolve_cover`]).
    cover: Option<i64>,
    ol_work_id: Option<String>,
}

impl CleanWork {
    fn new(rec: &WorkRecord) -> Self {
        let from_feed = matches!(rec.source, Source::OpenLibrary | Source::Dnb);
        let authors: Vec<AuthorRecord> = rec.authors.iter().filter_map(clean_author).collect();
        let author_names = clean_line(Some(&rec.author_names), 2000).or_else(|| {
            let joined = authors
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            clean_line(Some(&joined), 2000)
        });
        let description = rec.description.as_deref().map(|raw| {
            let raw = nfc(raw);
            let text = if from_feed {
                clean_description(&raw)
            } else {
                py_strip(&raw).to_owned()
            };
            truncate_chars(&text, 50_000).to_owned()
        });
        let subjects = rec.subjects.as_ref().map(|values| {
            if from_feed {
                clean_subjects(&values.iter().map(|v| nfc(v)).collect::<Vec<_>>())
            } else {
                values
                    .iter()
                    .filter_map(|s| clean_line(Some(s), 200))
                    .take(50)
                    .collect()
            }
        });
        Self {
            title: clean_line(Some(&rec.title), 1000),
            subtitle: clean_line(rec.subtitle.as_deref(), 1000),
            author_names,
            authors,
            first_publish_year: clean_year(rec.first_publish_year),
            description: description.filter(|d| !d.is_empty()),
            subjects: subjects.filter(|s| !s.is_empty()),
            pages: clean_pages(rec.pages),
            ol_rating: rec
                .ol_rating
                .filter(|r| r.is_finite() && (0.0..=5.0).contains(r)),
            ol_rating_count: rec.ol_rating_count.filter(|c| *c >= 0),
            ol_cover_id: rec.ol_cover_id.filter(|id| *id > 0),
            cover: None,
            ol_work_id: clean_id(rec.ol_work_id.as_deref(), &OL_WORK_ID),
        }
    }

    async fn resolve_cover(&mut self, conn: &mut PgConnection) -> sqlx::Result<()> {
        self.cover = cover_for(conn, self.ol_cover_id).await?;
        Ok(())
    }

    fn surname(&self) -> String {
        record_surname(&self.authors, self.author_names.as_deref().unwrap_or(""))
    }
}

/// An edition record with every value inside the schema's limits.
struct CleanEdition {
    source: Source,
    isbn13: Option<String>,
    ol_edition_id: Option<String>,
    dnb_idn: Option<String>,
    /// The record's IDN, else `dnb_idn`: an edition that carries an IDN was delivered by it.
    dnb_record_idn: Option<String>,
    title: Option<String>,
    subtitle: Option<String>,
    publisher: Option<String>,
    language: Option<String>,
    format: Option<String>,
    pages: Option<i32>,
    publication_year: Option<i32>,
    planned_publication_date: Patch<Date>,
    ol_cover_id: Option<i64>,
    /// The usable media row for `ol_cover_id`, resolved by [`CleanEdition::resolve_cover`]
    /// before any catalog row is locked.
    cover: Option<i64>,
    contributors: Vec<(AuthorRecord, ContributorRole)>,
    created_by: Option<i64>,
}

impl CleanEdition {
    fn new(rec: &EditionRecord) -> Self {
        // Identifiers are dropped, never truncated, when they break the schema's limits.
        let idn = |value: Option<&str>| clean_line(value, 33).filter(|idn| char_len(idn) <= 32);
        let dnb_idn = idn(rec.dnb_idn.as_deref());
        Self {
            source: rec.source,
            isbn13: rec.isbn13.as_ref().map(|isbn| isbn.as_str().to_owned()),
            ol_edition_id: clean_id(rec.ol_edition_id.as_deref(), &OL_EDITION_ID),
            dnb_record_idn: idn(rec.dnb_record_idn.as_deref()).or_else(|| dnb_idn.clone()),
            dnb_idn,
            title: clean_line(rec.title.as_deref(), 1000),
            subtitle: clean_line(rec.subtitle.as_deref(), 1000),
            publisher: clean_line(rec.publisher.as_deref(), 300),
            language: clean_language(rec.language.as_deref()),
            format: rec.format.map(|f| f.as_str().to_owned()),
            pages: clean_pages(rec.pages),
            publication_year: clean_year(rec.publication_year),
            planned_publication_date: clean_planned(&rec.planned_publication_date),
            ol_cover_id: rec.ol_cover_id.filter(|id| *id > 0),
            cover: None,
            contributors: rec
                .contributors
                .iter()
                .filter_map(|(author, role)| Some((clean_author(author)?, *role)))
                .collect(),
            created_by: rec.created_by,
        }
    }

    /// The edition's own title: `None` when it equals the work title by case fold.
    fn own_title(&self, work_title: &str) -> Option<String> {
        self.title
            .clone()
            .filter(|t| casefold(&collapse_ws(t)) != casefold(&collapse_ws(work_title)))
    }

    /// Resolves (and share-locks) the cover's media row. Called first, before the edition or its
    /// work is locked, so a takedown that holds the media row never waits for a row this ingest
    /// holds.
    async fn resolve_cover(&mut self, conn: &mut PgConnection) -> sqlx::Result<()> {
        self.cover = cover_for(conn, self.ol_cover_id).await?;
        Ok(())
    }

    /// The advisory lock keys of this edition's identifiers.
    fn lock_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(isbn) = &self.isbn13 {
            keys.push(format!("isbn:{isbn}"));
        }
        if let Some(idn) = &self.dnb_idn {
            keys.push(format!("idn:{idn}"));
        }
        if let Some(ol) = &self.ol_edition_id {
            keys.push(format!("olm:{ol}"));
        }
        keys
    }
}

fn row_source(rec_source: Source) -> sqlx::Result<&'static str> {
    if rec_source == Source::Admin {
        debug_assert!(false, "Source::Admin is field provenance only");
        return Err(sqlx::Error::Protocol(
            "Source::Admin is not a row source".into(),
        ));
    }
    Ok(rec_source.as_str())
}

/// The row source after a record of `by` was ingested into a row of `current`: a legacy row
/// becomes dnb or openlibrary; every other row keeps its origin.
fn upgraded_source(current: &str, by: Source) -> String {
    if current == Source::Legacy.as_str() && matches!(by, Source::Dnb | Source::OpenLibrary) {
        by.as_str().to_owned()
    } else {
        current.to_owned()
    }
}

// ---------------------------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------------------------

/// The first key of the two-key advisory lock space ingest uses (separate from the one-key
/// space of `jobs::lock_ids`).
const LOCK_SPACE: i32 = 0x6263_0100;

/// Takes transaction-level advisory locks on `keys`, in a fixed order, so two calls never
/// deadlock over the keys they share. Locks taken by earlier calls in the same transaction, and
/// row locks, are outside that order (§7.2 "Deadlocks", [`crate::error::is_deadlock`]). Outside
/// a transaction they end with the statement, which is harmless.
async fn lock_keys(conn: &mut PgConnection, keys: &[String]) -> sqlx::Result<()> {
    let mut ids: Vec<i32> = keys
        .iter()
        .map(|key| {
            let digest = Sha256::digest(key.as_bytes());
            i32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]])
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        sqlx::query_scalar!(
            r#"SELECT 1 AS "locked!" FROM pg_advisory_xact_lock($1, $2)"#,
            LOCK_SPACE,
            id
        )
        .fetch_one(&mut *conn)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Authors
// ---------------------------------------------------------------------------------------------

async fn find_author(conn: &mut PgConnection, a: &AuthorRecord) -> sqlx::Result<Option<i64>> {
    if a.ol_author_id.is_some() || a.gnd_id.is_some() {
        sqlx::query_scalar!(
            "SELECT id FROM authors WHERE ol_author_id = $1 OR gnd_id = $2
             ORDER BY (ol_author_id = $1) IS TRUE DESC, id LIMIT 1",
            a.ol_author_id,
            a.gnd_id
        )
        .fetch_optional(conn)
        .await
    } else {
        sqlx::query_scalar!(
            "SELECT id FROM authors
             WHERE name_key = $1 AND ol_author_id IS NULL AND gnd_id IS NULL",
            name_key(&a.name)
        )
        .fetch_optional(conn)
        .await
    }
}

/// The `authors` row for a (clean) author: by OL id, then GND id; id-less names by `name_key`.
async fn author_id(conn: &mut PgConnection, a: &AuthorRecord) -> sqlx::Result<i64> {
    if let Some(id) = find_author(conn, a).await? {
        if a.sort_name.is_some() {
            sqlx::query!(
                "UPDATE authors SET sort_name = $2 WHERE id = $1 AND sort_name IS NULL",
                id,
                a.sort_name
            )
            .execute(&mut *conn)
            .await?;
        }
        return Ok(id);
    }
    let inserted = sqlx::query_scalar!(
        "INSERT INTO authors (name, name_key, sort_name, ol_author_id, gnd_id)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING
         RETURNING id",
        a.name,
        name_key(&a.name),
        a.sort_name,
        a.ol_author_id,
        a.gnd_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    match inserted {
        Some(id) => Ok(id),
        None => find_author(conn, a).await?.ok_or(sqlx::Error::RowNotFound),
    }
}

/// Replaces the work's author links with `authors`, in order.
async fn replace_work_authors(
    conn: &mut PgConnection,
    work_id: i64,
    authors: &[AuthorRecord],
) -> sqlx::Result<()> {
    let mut ids = Vec::new();
    for author in authors {
        let id = author_id(conn, author).await?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let current = sqlx::query_scalar!(
        "SELECT author_id FROM work_authors WHERE work_id = $1 ORDER BY position",
        work_id
    )
    .fetch_all(&mut *conn)
    .await?;
    if current == ids {
        return Ok(());
    }
    sqlx::query!("DELETE FROM work_authors WHERE work_id = $1", work_id)
        .execute(&mut *conn)
        .await?;
    let positions: Vec<i32> = (0..ids.len() as i32).collect();
    sqlx::query!(
        "INSERT INTO work_authors (work_id, author_id, position)
         SELECT $1, author_id, position FROM unnest($2::int8[], $3::int4[]) AS t (author_id, position)",
        work_id,
        &ids,
        &positions
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Replaces the edition's contributors, in record order.
async fn replace_contributors(
    conn: &mut PgConnection,
    edition_id: i64,
    contributors: &[(AuthorRecord, ContributorRole)],
) -> sqlx::Result<()> {
    let (mut ids, mut roles, mut positions) = (Vec::new(), Vec::new(), Vec::new());
    for (author, role) in contributors {
        let id = author_id(conn, author).await?;
        let role = role.as_str().to_owned();
        if !ids.iter().zip(&roles).any(|(i, r)| *i == id && *r == role) {
            positions.push(ids.len() as i32);
            ids.push(id);
            roles.push(role);
        }
    }
    let current: Vec<(i64, String)> = sqlx::query!(
        "SELECT author_id, role FROM edition_contributors WHERE edition_id = $1 ORDER BY position",
        edition_id
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| (row.author_id, row.role))
    .collect();
    let wanted: Vec<(i64, String)> = ids.iter().copied().zip(roles.iter().cloned()).collect();
    if current == wanted {
        return Ok(());
    }
    sqlx::query!(
        "DELETE FROM edition_contributors WHERE edition_id = $1",
        edition_id
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "INSERT INTO edition_contributors (edition_id, author_id, role, position)
         SELECT $1, author_id, role, position
         FROM unnest($2::int8[], $3::text[], $4::int4[]) AS t (author_id, role, position)",
        edition_id,
        &ids,
        &roles,
        &positions
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------------------------

/// The media row for an Open Library cover id (insert-only). `None` when that row was removed by
/// a takedown or Open Library confirmed it gone (`media.ol_gone_at`, the cover search's dead
/// check), so no caller can attach it again, and for ids ≤ 0.
pub async fn ol_cover_media(
    conn: &mut PgConnection,
    ol_cover_id: i64,
) -> sqlx::Result<Option<i64>> {
    if ol_cover_id <= 0 {
        return Ok(None);
    }
    Ok(ol_cover_medias(conn, &[ol_cover_id])
        .await?
        .remove(&ol_cover_id))
}

/// [`ol_cover_media`] for many cover ids at once: the usable (not removed, not gone) media id per
/// cover id.
/// The rows are locked `FOR SHARE` until the transaction ends, so a takedown's
/// `UPDATE media SET removed_at` waits for this ingest to commit and its `clear_media` then sees
/// the reference.
async fn ol_cover_medias(
    conn: &mut PgConnection,
    ol_cover_ids: &[i64],
) -> sqlx::Result<HashMap<i64, i64>> {
    let mut wanted: Vec<i64> = ol_cover_ids.iter().copied().filter(|id| *id > 0).collect();
    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() {
        return Ok(HashMap::new());
    }
    let existing: HashSet<i64> = sqlx::query_scalar!(
        r#"SELECT ol_cover_id AS "ol_cover_id!" FROM media WHERE ol_cover_id = ANY($1)"#,
        &wanted
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let missing: Vec<i64> = wanted
        .iter()
        .copied()
        .filter(|id| !existing.contains(id))
        .collect();
    if !missing.is_empty() {
        let public_ids: Vec<String> = missing.iter().map(|_| new_public_id()).collect();
        sqlx::query!(
            "INSERT INTO media (public_id, kind, source, ol_cover_id)
             SELECT public_id, 'cover', 'openlibrary', ol_cover_id
             FROM unnest($1::text[], $2::int8[]) AS t (public_id, ol_cover_id)
             ON CONFLICT (ol_cover_id) WHERE ol_cover_id IS NOT NULL DO NOTHING",
            &public_ids,
            &missing
        )
        .execute(&mut *conn)
        .await?;
    }
    let rows = sqlx::query!(
        r#"SELECT id, ol_cover_id AS "ol_cover_id!" FROM media
           WHERE ol_cover_id = ANY($1) AND removed_at IS NULL AND ol_gone_at IS NULL
           ORDER BY id
           FOR SHARE"#,
        &wanted
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|r| (r.ol_cover_id, r.id)).collect())
}

/// The usable media id for an optional Open Library cover id.
async fn cover_for(conn: &mut PgConnection, ol_cover_id: Option<i64>) -> sqlx::Result<Option<i64>> {
    match ol_cover_id {
        Some(id) => ol_cover_media(conn, id).await,
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------------------------
// Works
// ---------------------------------------------------------------------------------------------

struct WorkRow {
    id: i64,
    title: String,
    subtitle: String,
    author_names: String,
    first_publish_year: Option<i32>,
    description: String,
    subjects: Vec<String>,
    pages: Option<i32>,
    ol_rating: Option<f64>,
    ol_rating_count: Option<i32>,
    ol_work_id: Option<String>,
    source: String,
    cover_media_id: Option<i64>,
    field_sources: Json<Map<String, Value>>,
}

async fn load_work(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<Option<WorkRow>> {
    sqlx::query_as!(
        WorkRow,
        r#"SELECT id, title, subtitle, author_names, first_publish_year, description, subjects,
                  pages, ol_rating, ol_rating_count, ol_work_id, source, cover_media_id,
                  field_sources AS "field_sources: Json<Map<String, Value>>"
           FROM works WHERE id = $1 FOR UPDATE"#,
        work_id
    )
    .fetch_optional(conn)
    .await
}

async fn find_work_by_ol(conn: &mut PgConnection, ol_work_id: &str) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!(
        "SELECT id FROM works WHERE ol_work_id = $1 FOR UPDATE",
        ol_work_id
    )
    .fetch_optional(conn)
    .await
}

/// Which work fields a record may change.
#[derive(Clone, Copy)]
enum WorkScope {
    /// Every field (`upsert_work`).
    Full,
    /// Title, subtitle and authors only, for the parent of a found edition. `open`: they merge
    /// normally; otherwise only `legacy`-ranked values are replaced.
    ParentOfEdition { open: bool },
}

/// Inserts a new work; `None` when a concurrent insert took its `ol_work_id`.
async fn insert_work(
    conn: &mut PgConnection,
    rec: &WorkRecord,
    w: &CleanWork,
) -> sqlx::Result<Option<i64>> {
    let source = row_source(rec.source)?;
    let cover = w.cover;
    let mut prov = Provenance::default();
    let title = w.title.clone().unwrap_or_else(|| "Untitled".to_owned());
    prov.set("title", rec.source);
    let set = |prov: &mut Provenance, field: &str, present: bool| {
        if present {
            prov.set(field, rec.source);
        }
    };
    set(&mut prov, "subtitle", w.subtitle.is_some());
    set(&mut prov, "author_names", w.author_names.is_some());
    set(
        &mut prov,
        "first_publish_year",
        w.first_publish_year.is_some(),
    );
    set(&mut prov, "description", w.description.is_some());
    set(&mut prov, "subjects", w.subjects.is_some());
    set(&mut prov, "pages", w.pages.is_some());
    set(&mut prov, "cover_media_id", cover.is_some());
    let key = match_key(&title, &w.surname());

    let id = sqlx::query_scalar!(
        "INSERT INTO works (title, subtitle, author_names, first_publish_year, description,
                            subjects, pages, ol_work_id, ol_rating, ol_rating_count, source,
                            cover_media_id, created_by, details_fetched_at, field_sources,
                            match_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                 CASE WHEN $14 THEN now() END, $15, $16)
         ON CONFLICT (ol_work_id) WHERE ol_work_id IS NOT NULL DO NOTHING
         RETURNING id",
        title,
        w.subtitle.clone().unwrap_or_default(),
        w.author_names.clone().unwrap_or_default(),
        w.first_publish_year,
        w.description.clone().unwrap_or_default(),
        &w.subjects.clone().unwrap_or_default(),
        w.pages,
        w.ol_work_id,
        w.ol_rating,
        w.ol_rating_count,
        source,
        cover,
        rec.created_by,
        rec.full_details,
        prov.to_value(),
        key
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = id
        && !w.authors.is_empty()
    {
        replace_work_authors(conn, id, &w.authors).await?;
    }
    Ok(id)
}

/// Gives the work `ol_work_id` when it has none and no other work holds it; returns whether it
/// did. Two works claiming one id at once meet on `works_ol_work_id_key`: the loser's UPDATE fails
/// inside a savepoint, so the caller's transaction goes on.
async fn claim_ol_work_id(
    conn: &mut PgConnection,
    work_id: i64,
    ol_work_id: &str,
) -> sqlx::Result<bool> {
    let mut savepoint = conn.begin().await?;
    let claimed = sqlx::query!(
        "UPDATE works SET ol_work_id = $2, updated_at = now()
         WHERE id = $1 AND ol_work_id IS NULL
           AND NOT EXISTS (SELECT 1 FROM works WHERE ol_work_id = $2)",
        work_id,
        ol_work_id
    )
    .execute(&mut *savepoint)
    .await;
    match claimed {
        Ok(done) => {
            savepoint.commit().await?;
            Ok(done.rows_affected() == 1)
        }
        Err(e) if is_unique_violation(&e, "works_ol_work_id_key") => {
            savepoint.rollback().await?;
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Merges `rec` into the work under the provenance rules.
async fn update_work(
    conn: &mut PgConnection,
    mut row: WorkRow,
    rec: &WorkRecord,
    w: &CleanWork,
    mode: Mode,
    scope: WorkScope,
) -> sqlx::Result<()> {
    row_source(rec.source)?;
    let fallback = Source::parse(&row.source).unwrap_or(Source::Legacy);
    let mut m = Merger::new(row.field_sources.0.clone(), fallback, rec.source, mode);
    if let WorkScope::ParentOfEdition { open } = scope {
        m.only_legacy = !open;
    }
    let old_names = row.author_names.clone();
    m.value_only("title", &mut row.title, w.title.clone());
    m.field("subtitle", &mut row.subtitle, w.subtitle.clone());
    let authors = m.field(
        "author_names",
        &mut row.author_names,
        w.author_names.clone(),
    );
    let full = matches!(scope, WorkScope::Full);
    if full {
        m.field(
            "first_publish_year",
            &mut row.first_publish_year,
            w.first_publish_year.map(Some),
        );
        m.field("description", &mut row.description, w.description.clone());
        m.field("subjects", &mut row.subjects, w.subjects.clone());
        m.field("pages", &mut row.pages, w.pages.map(Some));
        m.value_only("cover_media_id", &mut row.cover_media_id, w.cover.map(Some));
        if w.ol_rating.is_some() && w.ol_rating != row.ol_rating {
            row.ol_rating = w.ol_rating;
            m.changed = true;
        }
        if w.ol_rating_count.is_some() && w.ol_rating_count != row.ol_rating_count {
            row.ol_rating_count = w.ol_rating_count;
            m.changed = true;
        }
    }
    let mut source = upgraded_source(&row.source, rec.source);
    // An `openlibrary` work has an ol_work_id: a legacy work upgraded by an Open Library record
    // takes the record's id, or stays `legacy` (a merge candidate) when another work holds it.
    if source != row.source && rec.source == Source::OpenLibrary && row.ol_work_id.is_none() {
        let claimed = match &w.ol_work_id {
            Some(ol_work_id) => claim_ol_work_id(conn, row.id, ol_work_id).await?,
            None => false,
        };
        if !claimed {
            source = row.source.clone();
        }
    }
    let fetched = full && rec.full_details;
    if m.changed || source != row.source || fetched {
        sqlx::query!(
            "UPDATE works SET title = $2, subtitle = $3, author_names = $4,
                    first_publish_year = $5, description = $6, subjects = $7, pages = $8,
                    ol_rating = $9, ol_rating_count = $10, source = $11, cover_media_id = $12,
                    field_sources = $13,
                    details_fetched_at = CASE WHEN $14 THEN now() ELSE details_fetched_at END,
                    updated_at = now()
             WHERE id = $1",
            row.id,
            row.title,
            row.subtitle,
            row.author_names,
            row.first_publish_year,
            row.description,
            &row.subjects,
            row.pages,
            row.ol_rating,
            row.ol_rating_count,
            source,
            row.cover_media_id,
            m.prov.to_value(),
            fetched
        )
        .execute(&mut *conn)
        .await?;
    }
    match authors {
        Action::Write if !w.authors.is_empty() => {
            replace_work_authors(conn, row.id, &w.authors).await?
        }
        // New names without author records: the old links no longer describe them.
        Action::Write if row.author_names != old_names => {
            replace_work_authors(conn, row.id, &[]).await?
        }
        Action::Clear => replace_work_authors(conn, row.id, &[]).await?,
        _ => {}
    }
    // The parent of an edition is refreshed by finish_edition.
    if full {
        refresh_derived(conn, row.id).await?;
    }
    Ok(())
}

/// A work identified by `ol_work_id` is found and updated per `mode`. With `ol_work_id` None, a
/// new work is ALWAYS created, for any source (no deduplication; the caller groups).
/// `Source::OpenLibrary` requires `ol_work_id` (debug_assert; `Err(Protocol)` in release). Sets
/// `details_fetched_at` when `full_details`. A work found by `ol_work_id` that was merged away
/// resolves to the surviving work, which is returned untouched: it has its own Open Library
/// record, and the merged row is never written again.
pub async fn upsert_work(
    conn: &mut PgConnection,
    rec: &WorkRecord,
    mode: Mode,
) -> sqlx::Result<i64> {
    row_source(rec.source)?;
    let mut w = CleanWork::new(rec);
    let Some(ol_work_id) = w.ol_work_id.clone() else {
        if rec.source == Source::OpenLibrary {
            debug_assert!(false, "an Open Library work record needs its ol_work_id");
            return Err(sqlx::Error::Protocol(
                "an Open Library work record needs a valid ol_work_id".into(),
            ));
        }
        w.resolve_cover(conn).await?;
        return insert_work(conn, rec, &w)
            .await?
            .ok_or(sqlx::Error::RowNotFound);
    };
    w.resolve_cover(conn).await?;
    let existing = match find_work_by_ol(conn, &ol_work_id).await? {
        Some(id) => id,
        None => match insert_work(conn, rec, &w).await? {
            Some(id) => return Ok(id),
            // A concurrent insert won; merge into its row.
            None => find_work_by_ol(conn, &ol_work_id)
                .await?
                .ok_or(sqlx::Error::RowNotFound)?,
        },
    };
    let current = follow_merges(conn, existing)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;
    if current != existing {
        return Ok(current);
    }
    if mode != Mode::InsertOnly
        && let Some(row) = load_work(conn, existing).await?
    {
        update_work(conn, row, rec, &w, mode, WorkScope::Full).await?;
    }
    Ok(existing)
}

/// Search stubs: one batched `INSERT … ON CONFLICT DO NOTHING` for the records with an
/// `ol_work_id` (existing rows are untouched); returns ids in input order, resolved through
/// merges like [`upsert_work`]. Stubs carry
/// `author_names` only; author records come with the full record. Records without an
/// `ol_work_id` are inserted one by one (each is a new work).
pub async fn upsert_work_stubs(
    conn: &mut PgConnection,
    recs: &[WorkRecord],
) -> sqlx::Result<Vec<i64>> {
    let cleaned: Vec<CleanWork> = recs.iter().map(CleanWork::new).collect();
    let cover_ids: Vec<i64> = cleaned.iter().filter_map(|w| w.ol_cover_id).collect();
    let covers = ol_cover_medias(conn, &cover_ids).await?;

    let mut ol_ids = Vec::new();
    let (mut titles, mut subtitles, mut names, mut years, mut pages) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut ratings, mut counts, mut cover_media, mut sources, mut provs) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut keys, mut descriptions, mut subjects, mut creators, mut full) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (rec, w) in recs.iter().zip(&cleaned) {
        let Some(ol_id) = &w.ol_work_id else { continue };
        let source = row_source(rec.source)?;
        let title = w.title.clone().unwrap_or_else(|| "Untitled".to_owned());
        let cover = w.ol_cover_id.and_then(|id| covers.get(&id).copied());
        let mut prov = Provenance::default();
        for (field, present) in [
            ("title", true),
            ("subtitle", w.subtitle.is_some()),
            ("author_names", w.author_names.is_some()),
            ("first_publish_year", w.first_publish_year.is_some()),
            ("description", w.description.is_some()),
            ("subjects", w.subjects.is_some()),
            ("pages", w.pages.is_some()),
            ("cover_media_id", cover.is_some()),
        ] {
            if present {
                prov.set(field, rec.source);
            }
        }
        keys.push(match_key(&title, &w.surname()));
        ol_ids.push(ol_id.clone());
        titles.push(title);
        subtitles.push(w.subtitle.clone().unwrap_or_default());
        names.push(w.author_names.clone().unwrap_or_default());
        years.push(w.first_publish_year);
        pages.push(w.pages);
        ratings.push(w.ol_rating);
        counts.push(w.ol_rating_count);
        cover_media.push(cover);
        sources.push(source.to_owned());
        provs.push(prov.to_value());
        descriptions.push(w.description.clone().unwrap_or_default());
        subjects.push(Value::from(w.subjects.clone().unwrap_or_default()));
        creators.push(rec.created_by);
        full.push(rec.full_details);
    }
    if !ol_ids.is_empty() {
        sqlx::query!(
            "INSERT INTO works (ol_work_id, title, subtitle, author_names, first_publish_year,
                                pages, ol_rating, ol_rating_count, cover_media_id, source,
                                field_sources, match_key, description, subjects, created_by,
                                details_fetched_at)
             SELECT t.ol_work_id, t.title, t.subtitle, t.author_names, t.first_publish_year,
                    t.pages, t.ol_rating, t.ol_rating_count, t.cover_media_id, t.source,
                    t.field_sources, t.match_key, t.description,
                    ARRAY(SELECT jsonb_array_elements_text(t.subjects)), t.created_by,
                    CASE WHEN t.full_details THEN now() END
             FROM unnest($1::text[], $2::text[], $3::text[], $4::text[], $5::int4[],
                         $6::int4[], $7::float8[], $8::int4[], $9::int8[], $10::text[],
                         $11::jsonb[], $12::text[], $13::text[], $14::jsonb[], $15::int8[],
                         $16::bool[])
                  AS t (ol_work_id, title, subtitle, author_names, first_publish_year, pages,
                        ol_rating, ol_rating_count, cover_media_id, source, field_sources,
                        match_key, description, subjects, created_by, full_details)
             ON CONFLICT (ol_work_id) WHERE ol_work_id IS NOT NULL DO NOTHING",
            &ol_ids,
            &titles,
            &subtitles,
            &names,
            &years as &[Option<i32>],
            &pages as &[Option<i32>],
            &ratings as &[Option<f64>],
            &counts as &[Option<i32>],
            &cover_media as &[Option<i64>],
            &sources,
            &provs,
            &keys,
            &descriptions,
            &subjects,
            &creators as &[Option<i64>],
            &full
        )
        .execute(&mut *conn)
        .await?;
    }
    let ids: HashMap<String, i64> = sqlx::query!(
        r#"SELECT id, ol_work_id AS "ol_work_id!" FROM works WHERE ol_work_id = ANY($1)"#,
        &ol_ids
    )
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| (row.ol_work_id, row.id))
    .collect();
    // A merged-away work that kept its ol_work_id stands for the work it was merged into.
    let found: Vec<i64> = ids.values().copied().collect();
    let current = follow_merges_many(conn, &found).await?;

    let mut out = Vec::with_capacity(recs.len());
    for (rec, w) in recs.iter().zip(&cleaned) {
        let id = match &w.ol_work_id {
            Some(ol_id) => ids
                .get(ol_id)
                .and_then(|id| current.get(id))
                .copied()
                .ok_or(sqlx::Error::RowNotFound)?,
            None => upsert_work(conn, rec, Mode::InsertOnly).await?,
        };
        out.push(id);
    }
    Ok(out)
}

/// Rebuilds `search_extra` and `match_key` from the work's title, first author and edition
/// titles.
async fn refresh_derived(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<()> {
    let Some(row) = sqlx::query!(
        r#"SELECT w.title, w.author_names, w.search_extra, w.match_key,
                  fa.name AS "first_name?", fa.sort_name AS "first_sort?",
                  ARRAY(SELECT e.title FROM editions e
                        WHERE e.work_id = w.id AND e.title IS NOT NULL ORDER BY e.id)
                      AS "edition_titles!: Vec<String>"
           FROM works w
           LEFT JOIN LATERAL (
               SELECT a.name, a.sort_name FROM work_authors wa JOIN authors a ON a.id = wa.author_id
               WHERE wa.work_id = w.id ORDER BY wa.position, a.id LIMIT 1
           ) fa ON true
           WHERE w.id = $1"#,
        work_id
    )
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(());
    };
    let surname = match &row.first_name {
        Some(name) => author_surname(name, row.first_sort.as_deref()),
        None => author_surname(&row.author_names, None),
    };
    let key = match_key(&row.title, &surname);
    let extra = search_extra(&row.title, &row.edition_titles);
    if row.match_key.as_deref() != Some(key.as_str()) || row.search_extra != extra {
        sqlx::query!(
            "UPDATE works SET match_key = $2, search_extra = $3 WHERE id = $1",
            work_id,
            key,
            extra
        )
        .execute(conn)
        .await?;
    }
    Ok(())
}

/// Fills the work's NULL `pages`, `first_publish_year` (from `publication_year`, else the year
/// of `planned_publication_date` when `first_publish_year` can take it) and `cover_media_id` from
/// this edition, with the edition's provenance.
async fn fill_work_from_edition(conn: &mut PgConnection, edition_id: i64) -> sqlx::Result<()> {
    sqlx::query!(
        r#"UPDATE works w SET
               pages = COALESCE(w.pages, e.pages),
               first_publish_year = COALESCE(w.first_publish_year, e.publication_year,
                                             e.planned_year),
               cover_media_id = COALESCE(w.cover_media_id, e.cover_media_id),
               field_sources = w.field_sources
                   || CASE WHEN w.pages IS NULL AND e.pages IS NOT NULL
                           THEN jsonb_build_object('pages',
                                COALESCE(e.field_sources -> 'pages', to_jsonb(e.source)))
                           ELSE '{}'::jsonb END
                   || CASE WHEN w.first_publish_year IS NOT NULL THEN '{}'::jsonb
                           WHEN e.publication_year IS NOT NULL
                           THEN jsonb_build_object('first_publish_year',
                                COALESCE(e.field_sources -> 'publication_year', to_jsonb(e.source)))
                           WHEN e.planned_year IS NOT NULL
                           THEN jsonb_build_object('first_publish_year',
                                COALESCE(e.field_sources -> 'planned_publication_date',
                                         to_jsonb(e.source)))
                           ELSE '{}'::jsonb END
                   || CASE WHEN w.cover_media_id IS NULL AND e.cover_media_id IS NOT NULL
                           THEN jsonb_build_object('cover_media_id',
                                COALESCE(e.field_sources -> 'cover_media_id', to_jsonb(e.source)))
                           ELSE '{}'::jsonb END,
               updated_at = now()
           FROM (SELECT e.*,
                        CASE WHEN EXTRACT(YEAR FROM e.planned_publication_date)
                                  BETWEEN -5000 AND 3000
                             THEN EXTRACT(YEAR FROM e.planned_publication_date)::int
                        END AS planned_year
                 FROM editions e WHERE e.id = $1) e
           WHERE w.id = e.work_id
             AND ((w.pages IS NULL AND e.pages IS NOT NULL)
                  OR (w.first_publish_year IS NULL
                      AND (e.publication_year IS NOT NULL OR e.planned_year IS NOT NULL))
                  OR (w.cover_media_id IS NULL AND e.cover_media_id IS NOT NULL))"#,
        edition_id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The rules every edition write ends with (§7.3).
async fn finish_edition(
    conn: &mut PgConnection,
    work_id: i64,
    edition_id: i64,
) -> sqlx::Result<()> {
    fill_work_from_edition(conn, edition_id).await?;
    refresh_derived(conn, work_id).await
}

// ---------------------------------------------------------------------------------------------
// Editions
// ---------------------------------------------------------------------------------------------

struct EditionRow {
    id: i64,
    work_id: i64,
    isbn13: Option<String>,
    title: Option<String>,
    subtitle: Option<String>,
    publisher: String,
    language: Option<String>,
    format: Option<String>,
    pages: Option<i32>,
    publication_year: Option<i32>,
    planned_publication_date: Option<Date>,
    ol_edition_id: Option<String>,
    dnb_idn: Option<String>,
    dnb_record_idn: Option<String>,
    source: String,
    cover_media_id: Option<i64>,
    field_sources: Json<Map<String, Value>>,
    withdrawn: bool,
    contributor_count: i64,
}

/// The edition with the record's ISBN, else its IDN, else its OL edition id.
async fn find_edition(
    conn: &mut PgConnection,
    ed: &CleanEdition,
) -> sqlx::Result<Option<EditionRow>> {
    if ed.isbn13.is_none() && ed.dnb_idn.is_none() && ed.ol_edition_id.is_none() {
        return Ok(None);
    }
    sqlx::query_as!(
        EditionRow,
        r#"SELECT e.id, e.work_id, e.isbn13, e.title, e.subtitle, e.publisher, e.language,
                  e.format, e.pages, e.publication_year, e.planned_publication_date,
                  e.ol_edition_id, e.dnb_idn, e.dnb_record_idn, e.source, e.cover_media_id,
                  e.field_sources AS "field_sources: Json<Map<String, Value>>",
                  e.withdrawn_at IS NOT NULL AS "withdrawn!",
                  (SELECT count(*) FROM edition_contributors c WHERE c.edition_id = e.id)
                      AS "contributor_count!"
           FROM editions e
           WHERE e.isbn13 = $1 OR e.dnb_idn = $2 OR e.ol_edition_id = $3
           ORDER BY (e.isbn13 = $1) IS TRUE DESC, (e.dnb_idn = $2) IS TRUE DESC, e.id
           LIMIT 1
           FOR UPDATE OF e"#,
        ed.isbn13,
        ed.dnb_idn,
        ed.ol_edition_id
    )
    .fetch_optional(conn)
    .await
}

async fn work_title(conn: &mut PgConnection, work_id: i64) -> sqlx::Result<String> {
    sqlx::query_scalar!("SELECT title FROM works WHERE id = $1", work_id)
        .fetch_one(conn)
        .await
}

/// Inserts a new edition of `work_id`; `None` when a concurrent insert took one of its
/// identifiers.
async fn insert_edition(
    conn: &mut PgConnection,
    work_id: i64,
    ed: &CleanEdition,
) -> sqlx::Result<Option<i64>> {
    let source = row_source(ed.source)?;
    let title = ed.own_title(&work_title(conn, work_id).await?);
    let cover = ed.cover;
    let planned = match &ed.planned_publication_date {
        Patch::Value(date) => Some(*date),
        _ => None,
    };
    let mut prov = Provenance::default();
    for (field, present) in [
        ("title", title.is_some()),
        ("subtitle", ed.subtitle.is_some()),
        ("publisher", ed.publisher.is_some()),
        ("language", ed.language.is_some()),
        ("format", ed.format.is_some()),
        ("pages", ed.pages.is_some()),
        ("publication_year", ed.publication_year.is_some()),
        ("planned_publication_date", planned.is_some()),
        ("cover_media_id", cover.is_some()),
        ("contributors", !ed.contributors.is_empty()),
    ] {
        if present {
            prov.set(field, ed.source);
        }
    }
    let id = sqlx::query_scalar!(
        "INSERT INTO editions (work_id, isbn13, title, subtitle, publisher, language, format,
                               pages, publication_year, planned_publication_date, ol_edition_id,
                               dnb_idn, dnb_record_idn, source, cover_media_id, created_by,
                               field_sources)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
         ON CONFLICT DO NOTHING
         RETURNING id",
        work_id,
        ed.isbn13,
        title,
        ed.subtitle,
        ed.publisher.clone().unwrap_or_default(),
        ed.language,
        ed.format,
        ed.pages,
        ed.publication_year,
        planned,
        ed.ol_edition_id,
        ed.dnb_idn,
        ed.dnb_record_idn,
        source,
        cover,
        ed.created_by,
        prov.to_value()
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = id
        && !ed.contributors.is_empty()
    {
        replace_contributors(conn, id, &ed.contributors).await?;
    }
    Ok(id)
}

/// Merges `ed` into a found edition under the provenance rules: the newest record's IDN moves to
/// it, the newest delivering record's IDN replaces `dnb_record_idn`, a missing ISBN or OL edition
/// id is filled, and a withdrawn edition is un-withdrawn.
async fn update_edition(
    conn: &mut PgConnection,
    mut row: EditionRow,
    ed: &CleanEdition,
    mode: Mode,
) -> sqlx::Result<()> {
    row_source(ed.source)?;
    let title = ed.own_title(&work_title(conn, row.work_id).await?);
    let fallback = Source::parse(&row.source).unwrap_or(Source::Legacy);
    let mut m = Merger::new(row.field_sources.0.clone(), fallback, ed.source, mode);
    m.field("title", &mut row.title, title.map(Some));
    m.field("subtitle", &mut row.subtitle, ed.subtitle.clone().map(Some));
    m.field("publisher", &mut row.publisher, ed.publisher.clone());
    m.field("language", &mut row.language, ed.language.clone().map(Some));
    m.field("format", &mut row.format, ed.format.clone().map(Some));
    m.field("pages", &mut row.pages, ed.pages.map(Some));
    m.field(
        "publication_year",
        &mut row.publication_year,
        ed.publication_year.map(Some),
    );
    m.patch(
        "planned_publication_date",
        &mut row.planned_publication_date,
        &ed.planned_publication_date,
    );
    m.value_only(
        "cover_media_id",
        &mut row.cover_media_id,
        ed.cover.map(Some),
    );
    let contributors = m.action(
        "contributors",
        row.contributor_count > 0,
        !ed.contributors.is_empty(),
    );
    m.mark("contributors", contributors);

    let mut identity_changed = row.withdrawn;
    if let Some(idn) = &ed.dnb_idn
        && row.dnb_idn.as_ref() != Some(idn)
    {
        sqlx::query!(
            "UPDATE editions SET dnb_idn = NULL, updated_at = now() WHERE dnb_idn = $1 AND id <> $2",
            idn,
            row.id
        )
        .execute(&mut *conn)
        .await?;
        row.dnb_idn = Some(idn.clone());
        identity_changed = true;
    }
    // Not unique: every edition of a multi-ISBN record carries it, so nothing moves.
    if let Some(idn) = &ed.dnb_record_idn
        && row.dnb_record_idn.as_ref() != Some(idn)
    {
        row.dnb_record_idn = Some(idn.clone());
        identity_changed = true;
    }
    // An edition found by IDN or OL id keeps its ISBN; a missing one is filled when free.
    if row.isbn13.is_none()
        && let Some(isbn) = &ed.isbn13
        && !sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM editions WHERE isbn13 = $1) AS "taken!""#,
            isbn
        )
        .fetch_one(&mut *conn)
        .await?
    {
        row.isbn13 = Some(isbn.clone());
        identity_changed = true;
    }
    if row.ol_edition_id.is_none()
        && let Some(ol_id) = &ed.ol_edition_id
        && !sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM editions WHERE ol_edition_id = $1) AS "taken!""#,
            ol_id
        )
        .fetch_one(&mut *conn)
        .await?
    {
        row.ol_edition_id = Some(ol_id.clone());
        identity_changed = true;
    }
    let source = upgraded_source(&row.source, ed.source);
    if m.changed || identity_changed || source != row.source {
        sqlx::query!(
            "UPDATE editions SET isbn13 = $2, title = $3, subtitle = $4, publisher = $5,
                    language = $6, format = $7, pages = $8, publication_year = $9,
                    planned_publication_date = $10, ol_edition_id = $11, dnb_idn = $12,
                    dnb_record_idn = $13, source = $14, cover_media_id = $15,
                    field_sources = $16, withdrawn_at = NULL, updated_at = now()
             WHERE id = $1",
            row.id,
            row.isbn13,
            row.title,
            row.subtitle,
            row.publisher,
            row.language,
            row.format,
            row.pages,
            row.publication_year,
            row.planned_publication_date,
            row.ol_edition_id,
            row.dnb_idn,
            row.dnb_record_idn,
            source,
            row.cover_media_id,
            m.prov.to_value()
        )
        .execute(&mut *conn)
        .await?;
    }
    match contributors {
        Action::Write => replace_contributors(conn, row.id, &ed.contributors).await?,
        Action::Clear => replace_contributors(conn, row.id, &[]).await?,
        Action::Keep => {}
    }
    Ok(())
}

/// Updates the parent work of a found edition (§7.2): title, subtitle and authors merge normally
/// for a legacy work, or for a single-edition DNB work receiving a DNB record (a provisional VLB
/// title gets the final one); otherwise only their `legacy`-ranked values are replaced.
async fn update_parent_work(
    conn: &mut PgConnection,
    work_id: i64,
    rec: &WorkRecord,
    mode: Mode,
) -> sqlx::Result<()> {
    let Some(row) = load_work(conn, work_id).await? else {
        return Ok(());
    };
    let editions = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM editions WHERE work_id = $1"#,
        work_id
    )
    .fetch_one(&mut *conn)
    .await?;
    let title_owner = Provenance(row.field_sources.0.clone()).get("title");
    let open = row.source == Source::Legacy.as_str()
        || (rec.source == Source::Dnb && title_owner == Some(Source::Dnb) && editions == 1);
    let w = CleanWork::new(rec);
    update_work(
        conn,
        row,
        rec,
        &w,
        mode,
        WorkScope::ParentOfEdition { open },
    )
    .await
}

/// Inserts `ed` under `work_id`, or merges it into the edition a concurrent insert created.
async fn insert_or_merge_edition(
    conn: &mut PgConnection,
    work_id: i64,
    ed: &CleanEdition,
    mode: Mode,
) -> sqlx::Result<(i64, i64)> {
    if let Some(id) = insert_edition(conn, work_id, ed).await? {
        return Ok((work_id, id));
    }
    let row = find_edition(conn, ed)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;
    let found = (row.work_id, row.id);
    if mode != Mode::InsertOnly {
        update_edition(conn, row, ed, mode).await?;
    }
    Ok(found)
}

/// Finds the edition by isbn13, then dnb_idn, then ol_edition_id.
/// - Found: updates it per `mode` and the provenance rules (dnb_idn and dnb_record_idn take the
///   newest record's IDN; a withdrawn edition is un-withdrawn). Then the parent work's title,
///   subtitle, author_names and work_authors are updated from `work` under the same rules when
///   they are `legacy`-ranked, or when they are `dnb`-ranked and this is the work's only edition.
/// - Not found: resolves the work by `grouping` (creating it from `work` when needed) and
///   inserts the edition.
///
/// In both cases NULL work fields are filled from the edition (§7.3). Returns
/// (work_id, edition_id). `InsertOnly` returns a found edition untouched. A legacy work upgraded
/// by an Open Library record takes the record's `ol_work_id`, or stays `legacy` when another work
/// holds it.
pub async fn upsert_edition_with_work(
    conn: &mut PgConnection,
    work: &WorkRecord,
    ed: &EditionRecord,
    grouping: WorkGrouping,
    mode: Mode,
) -> sqlx::Result<(i64, i64)> {
    row_source(work.source)?;
    let mut clean = CleanEdition::new(ed);
    clean.resolve_cover(conn).await?;
    lock_keys(conn, &clean.lock_keys()).await?;
    if let Some(row) = find_edition(conn, &clean).await? {
        let found = (row.work_id, row.id);
        if mode == Mode::InsertOnly {
            return Ok(found);
        }
        update_parent_work(conn, row.work_id, work, mode).await?;
        update_edition(conn, row, &clean, mode).await?;
        finish_edition(conn, found.0, found.1).await?;
        return Ok(found);
    }
    let work_id = match grouping {
        WorkGrouping::NewWork => upsert_work(conn, work, mode).await?,
        WorkGrouping::Into(id) => follow_merges(conn, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?,
        WorkGrouping::DnbTitleAuthor => match dnb_match_key(work) {
            Some(key) => {
                lock_keys(conn, &[format!("match:{key}")]).await?;
                let existing = sqlx::query_scalar!(
                    "SELECT id FROM works
                     WHERE match_key = $1 AND source = 'dnb' AND merged_into_id IS NULL
                     ORDER BY id LIMIT 1",
                    key
                )
                .fetch_optional(&mut *conn)
                .await?;
                match existing {
                    Some(id) => id,
                    None => upsert_work(conn, work, mode).await?,
                }
            }
            None => upsert_work(conn, work, mode).await?,
        },
    };
    let (work_id, edition_id) = insert_or_merge_edition(conn, work_id, &clean, mode).await?;
    finish_edition(conn, work_id, edition_id).await?;
    Ok((work_id, edition_id))
}

/// The key `WorkGrouping::DnbTitleAuthor` groups by; `None` for a record without an author, whose
/// title alone would group unrelated anthologies, calendars and yearbooks into one work.
fn dnb_match_key(work: &WorkRecord) -> Option<String> {
    let w = CleanWork::new(work);
    let surname = w.surname();
    (!surname.is_empty()).then(|| match_key(w.title.as_deref().unwrap_or("Untitled"), &surname))
}

/// One mapped DNB record (§7.5), for HAR and CAT's SRU lookup: the first edition finds or creates
/// its work with `WorkGrouping::DnbTitleAuthor`, the others join that work
/// (`WorkGrouping::Into`), so a record's co-publisher editions never split across works. The
/// advisory locks of every edition and of the match key are taken first, in one call. Returns
/// (work_id, edition_id) per edition, in record order; an edition found under another work keeps
/// it, like in [`upsert_edition_with_work`].
pub async fn upsert_dnb_record(
    conn: &mut PgConnection,
    rec: &DnbRecord,
    mode: Mode,
) -> sqlx::Result<Vec<(i64, i64)>> {
    let mut keys: Vec<String> = rec
        .editions
        .iter()
        .flat_map(|edition| CleanEdition::new(edition).lock_keys())
        .collect();
    keys.extend(dnb_match_key(&rec.work).map(|key| format!("match:{key}")));
    lock_keys(conn, &keys).await?;
    let mut grouping = WorkGrouping::DnbTitleAuthor;
    let mut out = Vec::with_capacity(rec.editions.len());
    for edition in &rec.editions {
        let ids = upsert_edition_with_work(conn, &rec.work, edition, grouping, mode).await?;
        if out.is_empty() {
            grouping = WorkGrouping::Into(ids.0);
        }
        out.push(ids);
    }
    Ok(out)
}

/// An edition of a known work (OL editions list, a user adding an edition). Same find-or-insert
/// as [`upsert_edition_with_work`], but never touches the work's own fields except the NULL fill
/// of §7.3. A found edition of another work keeps its work; the returned work id says so.
pub async fn upsert_edition(
    conn: &mut PgConnection,
    work_id: i64,
    rec: &EditionRecord,
    mode: Mode,
) -> sqlx::Result<(i64, i64)> {
    let mut clean = CleanEdition::new(rec);
    clean.resolve_cover(conn).await?;
    lock_keys(conn, &clean.lock_keys()).await?;
    let (work_id, edition_id) = match find_edition(conn, &clean).await? {
        Some(row) => {
            let found = (row.work_id, row.id);
            if mode == Mode::InsertOnly {
                return Ok(found);
            }
            update_edition(conn, row, &clean, mode).await?;
            found
        }
        None => {
            let work_id = follow_merges(conn, work_id)
                .await?
                .ok_or(sqlx::Error::RowNotFound)?;
            insert_or_merge_edition(conn, work_id, &clean, mode).await?
        }
    };
    finish_edition(conn, work_id, edition_id).await?;
    Ok((work_id, edition_id))
}

/// DNB delivered `status="deleted"` for this IDN: sets `editions.withdrawn_at` on every edition
/// whose newest delivering record it is (`editions.dnb_record_idn`), so a multi-ISBN record is
/// withdrawn with all its ISBNs. An edition a newer record delivered again stays live. Never
/// inserts.
pub async fn withdraw_dnb_edition(conn: &mut PgConnection, dnb_idn: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE editions SET withdrawn_at = now(), updated_at = now()
         WHERE dnb_record_idn = $1 AND withdrawn_at IS NULL",
        dnb_idn
    )
    .execute(conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Covers, takedowns, admin edits
// ---------------------------------------------------------------------------------------------

/// Sets the cover under the provenance rules (`by` = the writer). With `only_if_empty` the
/// UPDATE carries `AND cover_media_id IS NULL` (a first user upload; false = someone else was
/// first). Refuses a removed (or non-cover) media row. Returns whether the cover changed. Also
/// fills a NULL work cover from an edition cover.
pub async fn set_cover(
    conn: &mut PgConnection,
    target: CoverTarget,
    media_id: i64,
    by: Source,
    only_if_empty: bool,
) -> sqlx::Result<bool> {
    // Locked first and until the transaction ends, like the media rows ingest attaches: a
    // takedown's UPDATE of the row then waits, and its clear_media sees this reference.
    let usable = sqlx::query_scalar!(
        "SELECT id FROM media WHERE id = $1 AND kind = 'cover' AND removed_at IS NULL FOR SHARE",
        media_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .is_some();
    if !usable {
        return Ok(false);
    }
    let current = match target {
        CoverTarget::Work(id) => sqlx::query!(
            r#"SELECT cover_media_id, field_sources ->> 'cover_media_id' AS "owner?", source
               FROM works WHERE id = $1 FOR UPDATE"#,
            id
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| (r.cover_media_id, r.owner, r.source)),
        CoverTarget::Edition(id) => sqlx::query!(
            r#"SELECT cover_media_id, field_sources ->> 'cover_media_id' AS "owner?", source
               FROM editions WHERE id = $1 FOR UPDATE"#,
            id
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|r| (r.cover_media_id, r.owner, r.source)),
    };
    let Some((cover, owner, row_source)) = current else {
        return Ok(false);
    };
    if cover == Some(media_id) {
        return Ok(false);
    }
    if cover.is_some() {
        let owner = owner
            .as_deref()
            .and_then(Source::parse)
            .or_else(|| Source::parse(&row_source))
            .unwrap_or(Source::Legacy);
        if only_if_empty || owner.rank() > by.rank() {
            return Ok(false);
        }
    }
    let changed = match target {
        CoverTarget::Work(id) => sqlx::query!(
            "UPDATE works SET cover_media_id = $2,
                    field_sources = field_sources || jsonb_build_object('cover_media_id', $3::text),
                    updated_at = now()
             WHERE id = $1 AND cover_media_id IS NOT DISTINCT FROM $4",
            id,
            media_id,
            by.as_str(),
            cover
        )
        .execute(&mut *conn)
        .await?
        .rows_affected(),
        CoverTarget::Edition(id) => sqlx::query!(
            "UPDATE editions SET cover_media_id = $2,
                    field_sources = field_sources || jsonb_build_object('cover_media_id', $3::text),
                    updated_at = now()
             WHERE id = $1 AND cover_media_id IS NOT DISTINCT FROM $4",
            id,
            media_id,
            by.as_str(),
            cover
        )
        .execute(&mut *conn)
        .await?
        .rows_affected(),
    } == 1;
    if changed && let CoverTarget::Edition(id) = target {
        fill_work_from_edition(conn, id).await?;
    }
    Ok(changed)
}

/// Takedown: clears every works/editions `cover_media_id` that points at the media row (and its
/// provenance). Returns the number of rows cleared. Every work it cleared gets the cover
/// search's takedown flag (`cover_search.takedown_at`, #74): from then on the search only
/// suggests a cover for it, a person decides. It is due for the search at once (an earlier
/// miss backoff is dropped: the book had a cover since).
pub async fn clear_media(conn: &mut PgConnection, media_id: i64) -> sqlx::Result<u64> {
    let (works, editions) = clear_references(conn, media_id).await?;
    if !works.is_empty() {
        sqlx::query!(
            "INSERT INTO cover_search (work_id, takedown_at)
             SELECT id, now() FROM unnest($1::int8[]) AS t (id)
             ON CONFLICT (work_id) DO UPDATE SET
                 takedown_at = now(), next_search_at = NULL, misses = 0",
            &works
        )
        .execute(conn)
        .await?;
    }
    Ok(works.len() as u64 + editions)
}

/// An Open Library picture confirmed gone (#74, the cover search's dead check): clears every
/// works/editions reference to the media row and its provenance, like [`clear_media`] but
/// without the takedown flag. The caller locked the media row `FOR UPDATE` first. Returns the
/// works it cleared, which are due for the cover search at once (an earlier miss backoff is
/// dropped: the book had a cover since).
pub async fn clear_gone_media(conn: &mut PgConnection, media_id: i64) -> sqlx::Result<Vec<i64>> {
    let works = clear_references(conn, media_id).await?.0;
    if !works.is_empty() {
        sqlx::query!(
            "UPDATE cover_search SET next_search_at = NULL, misses = 0 WHERE work_id = ANY($1)",
            &works
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(works)
}

/// The works and the number of editions whose cover was `media_id`, cleared (works first).
async fn clear_references(conn: &mut PgConnection, media_id: i64) -> sqlx::Result<(Vec<i64>, u64)> {
    let works = sqlx::query_scalar!(
        "UPDATE works SET cover_media_id = NULL, field_sources = field_sources - 'cover_media_id',
                updated_at = now()
         WHERE cover_media_id = $1
         RETURNING id",
        media_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let editions = sqlx::query!(
        "UPDATE editions SET cover_media_id = NULL,
                field_sources = field_sources - 'cover_media_id', updated_at = now()
         WHERE cover_media_id = $1",
        media_id
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok((works, editions))
}

fn invalid(key: &str) -> AppError {
    AppError::bad_request(format!("Invalid value for {key}"))
}

/// A text value of an edit request: a string within `max` characters, or null (`None`).
fn admin_text(key: &str, value: &Value, max: usize) -> ApiResult<Option<String>> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) => {
            let s = nfc(s);
            let s = py_strip(&s);
            if char_len(s) > max {
                return Err(invalid(key));
            }
            Ok((!s.is_empty()).then(|| s.to_owned()))
        }
        _ => Err(invalid(key)),
    }
}

/// An integer value of an edit request within `range`, or null.
fn admin_int(
    key: &str,
    value: &Value,
    range: std::ops::RangeInclusive<i32>,
) -> ApiResult<Option<i32>> {
    match value {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .filter(|n| range.contains(n))
            .map(Some)
            .ok_or_else(|| invalid(key)),
        _ => Err(invalid(key)),
    }
}

/// Records `admin` provenance for a written value and for a cleared one: no source fills a field
/// an admin cleared (`decide`), so a description removed on notice stays removed.
fn admin_mark(prov: &mut Provenance, field: &str) {
    prov.set(field, Source::Admin);
}

/// An approved edit request: writes the fields and marks them `admin`. `fields` uses the
/// edit-request keys: works `title`, `subtitle`, `authors`, `year`, `description`, `subjects`,
/// `pages`; editions `title`, `subtitle`, `publisher`, `language`, `format`, `pages`,
/// `publication_year`, `planned_publication_date` (`YYYY-MM-DD`). Null clears a field (not a work
/// title) and keeps it `admin`, so no source fills it again. 404 `No such book.` for a missing
/// target; 400 for an unknown key or a bad value.
pub async fn apply_admin_fields(
    conn: &mut PgConnection,
    target: CoverTarget,
    fields: &Map<String, Value>,
) -> ApiResult<()> {
    match target {
        CoverTarget::Work(id) => apply_admin_work(conn, id, fields).await,
        CoverTarget::Edition(id) => apply_admin_edition(conn, id, fields).await,
    }
}

async fn apply_admin_work(
    conn: &mut PgConnection,
    work_id: i64,
    fields: &Map<String, Value>,
) -> ApiResult<()> {
    let mut row = load_work(conn, work_id)
        .await?
        .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    let mut prov = Provenance(row.field_sources.0.clone());
    let mut authors_changed = false;
    for (key, value) in fields {
        match key.as_str() {
            "title" => {
                row.title = admin_text(key, value, 1000)?.ok_or_else(|| invalid(key))?;
                admin_mark(&mut prov, "title");
            }
            "subtitle" => {
                row.subtitle = admin_text(key, value, 1000)?.unwrap_or_default();
                admin_mark(&mut prov, "subtitle");
            }
            "authors" => {
                let names = admin_text(key, value, 2000)?.unwrap_or_default();
                authors_changed |= names != row.author_names;
                row.author_names = names;
                admin_mark(&mut prov, "author_names");
            }
            "year" => {
                row.first_publish_year = admin_int(key, value, YEARS)?;
                admin_mark(&mut prov, "first_publish_year");
            }
            "description" => {
                row.description = admin_text(key, value, 50_000)?.unwrap_or_default();
                admin_mark(&mut prov, "description");
            }
            "subjects" => {
                row.subjects = match value {
                    Value::Null => Vec::new(),
                    Value::Array(items) if items.len() <= 50 => items
                        .iter()
                        .map(|item| admin_text(key, item, 200))
                        .collect::<ApiResult<Vec<_>>>()?
                        .into_iter()
                        .flatten()
                        .collect(),
                    _ => return Err(invalid(key)),
                };
                admin_mark(&mut prov, "subjects");
            }
            "pages" => {
                row.pages = admin_int(key, value, 1..=100_000)?;
                admin_mark(&mut prov, "pages");
            }
            _ => return Err(AppError::bad_request(format!("Unknown field: {key}"))),
        }
    }
    sqlx::query!(
        "UPDATE works SET title = $2, subtitle = $3, author_names = $4, first_publish_year = $5,
                description = $6, subjects = $7, pages = $8, field_sources = $9, updated_at = now()
         WHERE id = $1",
        work_id,
        row.title,
        row.subtitle,
        row.author_names,
        row.first_publish_year,
        row.description,
        &row.subjects,
        row.pages,
        prov.to_value()
    )
    .execute(&mut *conn)
    .await?;
    if authors_changed {
        // The admin's names replace whatever the links described.
        replace_work_authors(conn, work_id, &[]).await?;
    }
    refresh_derived(conn, work_id).await?;
    Ok(())
}

async fn apply_admin_edition(
    conn: &mut PgConnection,
    edition_id: i64,
    fields: &Map<String, Value>,
) -> ApiResult<()> {
    let row = sqlx::query!(
        r#"SELECT work_id, title, subtitle, publisher, language, format, pages, publication_year,
                  planned_publication_date,
                  field_sources AS "field_sources: Json<Map<String, Value>>"
           FROM editions WHERE id = $1 FOR UPDATE"#,
        edition_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NO_SUCH_BOOK))?;
    let mut prov = Provenance(row.field_sources.0);
    let (mut title, mut subtitle, mut publisher) = (row.title, row.subtitle, row.publisher);
    let (mut language, mut format, mut pages) = (row.language, row.format, row.pages);
    let (mut year, mut planned) = (row.publication_year, row.planned_publication_date);
    for (key, value) in fields {
        match key.as_str() {
            "title" => {
                title = admin_text(key, value, 1000)?;
                admin_mark(&mut prov, "title");
            }
            "subtitle" => {
                subtitle = admin_text(key, value, 1000)?;
                admin_mark(&mut prov, "subtitle");
            }
            "publisher" => {
                publisher = admin_text(key, value, 300)?.unwrap_or_default();
                admin_mark(&mut prov, "publisher");
            }
            "language" => {
                language = match admin_text(key, value, 3)? {
                    None => None,
                    Some(code) => Some(clean_language(Some(&code)).ok_or_else(|| invalid(key))?),
                };
                admin_mark(&mut prov, "language");
            }
            "format" => {
                format = match admin_text(key, value, 20)? {
                    None => None,
                    Some(f) => Some(
                        Format::parse(&f)
                            .ok_or_else(|| invalid(key))?
                            .as_str()
                            .to_owned(),
                    ),
                };
                admin_mark(&mut prov, "format");
            }
            "pages" => {
                pages = admin_int(key, value, 1..=100_000)?;
                admin_mark(&mut prov, "pages");
            }
            "publication_year" => {
                year = admin_int(key, value, YEARS)?;
                admin_mark(&mut prov, "publication_year");
            }
            "planned_publication_date" => {
                planned = match admin_text(key, value, 10)? {
                    None => None,
                    Some(text) => Some(
                        Date::parse(&text, format_description!("[year]-[month]-[day]"))
                            .ok()
                            .filter(|date| YEARS.contains(&date.year()))
                            .ok_or_else(|| invalid(key))?,
                    ),
                };
                admin_mark(&mut prov, "planned_publication_date");
            }
            _ => return Err(AppError::bad_request(format!("Unknown field: {key}"))),
        }
    }
    sqlx::query!(
        "UPDATE editions SET title = $2, subtitle = $3, publisher = $4, language = $5, format = $6,
                pages = $7, publication_year = $8, planned_publication_date = $9,
                field_sources = $10, updated_at = now()
         WHERE id = $1",
        edition_id,
        title,
        subtitle,
        publisher,
        language,
        format,
        pages,
        year,
        planned,
        prov.to_value()
    )
    .execute(&mut *conn)
    .await?;
    finish_edition(conn, row.work_id, edition_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_order_the_sources() {
        assert!(Source::Admin.rank() > Source::User.rank());
        assert!(Source::User.rank() > Source::Dnb.rank());
        assert_eq!(Source::Dnb.rank(), Source::OpenLibrary.rank());
        assert!(Source::OpenLibrary.rank() > Source::Legacy.rank());
        for source in [
            Source::OpenLibrary,
            Source::Dnb,
            Source::User,
            Source::Legacy,
            Source::Admin,
        ] {
            assert_eq!(Source::parse(source.as_str()), Some(source));
        }
    }

    #[test]
    fn decide_follows_the_rank_rules() {
        use Action::*;
        use Source::*;
        // An empty field is filled, whoever wrote it before ...
        assert_eq!(decide(false, User, true, Legacy, Mode::Refresh), Write);
        assert_eq!(decide(false, Dnb, true, OpenLibrary, Mode::Replace), Write);
        // ... unless an admin cleared it: then only an admin fills it again.
        assert_eq!(decide(false, Admin, true, Legacy, Mode::Refresh), Keep);
        assert_eq!(decide(false, Admin, true, User, Mode::Refresh), Keep);
        assert_eq!(decide(false, Admin, true, Dnb, Mode::Replace), Keep);
        assert_eq!(decide(false, Admin, true, Admin, Mode::Refresh), Write);
        // Equal or lower rank is overwritten, higher rank never.
        assert_eq!(decide(true, Legacy, true, Dnb, Mode::Refresh), Write);
        assert_eq!(decide(true, OpenLibrary, true, Dnb, Mode::Refresh), Write);
        assert_eq!(decide(true, User, true, Dnb, Mode::Refresh), Keep);
        assert_eq!(decide(true, Admin, true, User, Mode::Refresh), Keep);
        // A missing value keeps under Refresh; under Replace it clears only the source's own field.
        assert_eq!(decide(true, Dnb, false, Dnb, Mode::Refresh), Keep);
        assert_eq!(decide(true, Dnb, false, Dnb, Mode::Replace), Clear);
        // Same source, not same rank: DNB's Replace never clears what Open Library wrote.
        assert_eq!(decide(true, OpenLibrary, false, Dnb, Mode::Replace), Keep);
        assert_eq!(decide(true, Dnb, false, OpenLibrary, Mode::Replace), Keep);
        assert_eq!(decide(true, Legacy, false, Dnb, Mode::Replace), Keep);
        assert_eq!(decide(true, User, false, Dnb, Mode::Replace), Keep);
        assert_eq!(decide(false, Dnb, false, Dnb, Mode::Replace), Keep);
    }

    #[test]
    fn description_cleaning_satisfies_both_legacy_fixtures() {
        let a = "[Comment by Kim Stanley Robinson][1] and [a note](http://example.com):\n\n> One of my favorite novels.\n> Truly.\n\n\n\nThe end.\n\n[1]: http://example.com/robinson\n";
        assert_eq!(
            clean_description(a),
            "Comment by Kim Stanley Robinson and a note:\n\nOne of my favorite novels.\nTruly.\n\nThe end."
        );
        let b = "A witch on an island.\n\n----------\n[Source](https://x)";
        assert_eq!(clean_description(b), "A witch on an island.");
        assert_eq!(clean_description("  plain  "), "plain");
    }

    #[test]
    fn subject_cleaning_is_the_union_rule() {
        let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            clean_subjects(&strings(&[
                "Greek mythology",
                "Fiction",
                "Witches",
                "Fiction, fantasy, general",
                &"x".repeat(50)
            ])),
            strings(&["Greek mythology", "Fiction", "Witches"])
        );
        assert_eq!(
            clean_subjects(&strings(&[
                "Mythology",
                "mythology",
                "Greek literature",
                "",
                "award:hugo_award=1970"
            ])),
            strings(&["Mythology", "Greek literature"])
        );
        let many: Vec<String> = (0..20).map(|i| format!("Subject {i}")).collect();
        assert_eq!(clean_subjects(&many).len(), 12);
    }

    #[test]
    fn surnames_and_match_keys() {
        assert_eq!(
            author_surname("Udo Paulitz", Some("Paulitz, Udo")),
            "Paulitz"
        );
        assert_eq!(author_surname("Udo Paulitz", None), "Paulitz");
        assert_eq!(author_surname("Paulitz, Udo", None), "Paulitz");
        assert_eq!(
            author_surname("Ada Lovelace, Charles Babbage", None),
            "Lovelace"
        );
        assert_eq!(author_surname("Homer", None), "Homer");
        assert_eq!(author_surname("", None), "");
        assert_eq!(
            match_key("  Die  Straße ", "Müller"),
            "die strasse\u{1f}müller"
        );
        assert_eq!(match_key("Circe", ""), "circe\u{1f}");
        assert_eq!(name_key("  Udo   PAULITZ "), "udo paulitz");
    }

    #[test]
    fn search_extra_keeps_distinct_edition_titles() {
        let titles = vec![
            "Der Hobbit".to_string(),
            "THE HOBBIT".to_string(),
            "der  hobbit".to_string(),
            "Le Hobbit".to_string(),
        ];
        assert_eq!(
            search_extra("The Hobbit", &titles),
            "Der Hobbit / Le Hobbit"
        );
        assert_eq!(search_extra("The Hobbit", &[]), "");
    }

    #[test]
    fn records_are_sanitised_to_the_schema() {
        let rec = WorkRecord {
            pages: Some(0),
            first_publish_year: Some(99_999),
            ol_rating: Some(7.0),
            ol_rating_count: Some(-1),
            ol_cover_id: Some(-1),
            ol_work_id: Some("works/OL1W".into()),
            subtitle: Some("   ".into()),
            authors: vec![
                AuthorRecord::named("  "),
                AuthorRecord::named("Ada  Lovelace"),
            ],
            ..WorkRecord::new(Source::User, "t".repeat(1200))
        };
        let decomposed = CleanWork::new(&WorkRecord::new(Source::Dnb, "Der Mo\u{308}rder"));
        assert_eq!(decomposed.title.as_deref(), Some("Der Mörder"));
        // Postgres text refuses U+0000; it and the other control characters are dropped.
        let nul = CleanWork::new(&WorkRecord {
            author_names: "Ada\u{0} Lovelace".into(),
            authors: vec![AuthorRecord::named("\u{0}")],
            description: Some("Line one\u{0}\nline\u{7} two\u{85}".into()),
            subjects: Some(vec!["Fic\u{0}tion".into()]),
            ..WorkRecord::new(Source::OpenLibrary, "Cir\u{0}ce\u{1b}")
        });
        assert_eq!(nul.title.as_deref(), Some("Circe"));
        assert_eq!(nul.author_names.as_deref(), Some("Ada Lovelace"));
        assert!(nul.authors.is_empty());
        assert_eq!(nul.description.as_deref(), Some("Line one\nline two"));
        assert_eq!(nul.subjects, Some(vec!["Fiction".to_owned()]));
        let w = CleanWork::new(&rec);
        assert_eq!(w.title.as_deref().map(char_len), Some(1000));
        assert_eq!(w.pages, None);
        assert_eq!(w.first_publish_year, None);
        assert_eq!(w.ol_rating, None);
        assert_eq!(w.ol_rating_count, None);
        assert_eq!(w.ol_cover_id, None);
        assert_eq!(w.ol_work_id, None);
        assert_eq!(w.subtitle, None);
        assert_eq!(w.authors.len(), 1);
        assert_eq!(w.author_names.as_deref(), Some("Ada Lovelace"));

        let ed = CleanEdition::new(&EditionRecord {
            language: Some("GER".into()),
            dnb_idn: Some("x".repeat(40)),
            ..EditionRecord::new(Source::Dnb)
        });
        assert_eq!(ed.language.as_deref(), Some("ger"));
        assert_eq!(ed.dnb_idn, None);
        assert_eq!(ed.dnb_record_idn, None);
        assert_eq!(ed.own_title("Circe"), None);
        // The record's IDN falls back to the edition's own; an overlong one is dropped too.
        let ed = CleanEdition::new(&EditionRecord {
            dnb_idn: Some(" 100 ".into()),
            ..EditionRecord::new(Source::Dnb)
        });
        assert_eq!(
            (ed.dnb_idn.as_deref(), ed.dnb_record_idn.as_deref()),
            (Some("100"), Some("100"))
        );
        let ed = CleanEdition::new(&EditionRecord {
            dnb_idn: Some("100".into()),
            dnb_record_idn: Some("200".into()),
            ..EditionRecord::new(Source::Dnb)
        });
        assert_eq!(ed.dnb_record_idn.as_deref(), Some("200"));
        let ed = CleanEdition::new(&EditionRecord {
            dnb_idn: Some("100".into()),
            dnb_record_idn: Some("x".repeat(40)),
            ..EditionRecord::new(Source::Dnb)
        });
        assert_eq!(ed.dnb_record_idn.as_deref(), Some("100"));
        let ed = CleanEdition::new(&EditionRecord {
            publisher: Some("Reclam\u{0}".into()),
            planned_publication_date: Patch::Value(time::macros::date!(3001 - 01 - 01)),
            ..EditionRecord::new(Source::Dnb)
        });
        assert_eq!(ed.publisher.as_deref(), Some("Reclam"));
        assert_eq!(
            ed.planned_publication_date,
            Patch::Missing,
            "a year works.first_publish_year cannot take"
        );
        let ed = CleanEdition::new(&EditionRecord {
            title: Some("circe ".into()),
            ..EditionRecord::new(Source::Dnb)
        });
        assert_eq!(ed.own_title("Circe"), None);
        assert_eq!(ed.own_title("Kirke").as_deref(), Some("circe"));
    }
}
