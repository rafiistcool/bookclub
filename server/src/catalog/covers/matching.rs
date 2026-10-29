//! The matching rule of the cover search (docs/phase2/notes/cover.md §2): pure functions, so
//! every case is a table test. Conservative throughout: a wrong cover is worse than none.
//!
//! - [`fold`] and [`TitleKeys`]: titles compared without case, accents, punctuation, a leading
//!   article and genre words; the main title, the rest (subtitles and later segments) and the
//!   numbers are compared apart.
//! - [`Person`]: one surname rule for our authors and Open Library's names.
//! - [`verdict`]: clear (attach), uncertain (suggest) or no match, and which cover.

use std::collections::{BTreeSet, HashSet};

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::catalog::shared::ingest::author_surname;
use crate::catalog::shared::lookup::{CoverAnswer, CoverDoc, CoverQuery};
use crate::text::{casefold, collapse_ws};

/// Articles dropped from the start of a title (one, and only when more words follow).
const ARTICLES: [&str; 23] = [
    "the", "a", "an", "der", "die", "das", "ein", "eine", "le", "la", "les", "l", "un", "une",
    "el", "los", "las", "il", "lo", "gli", "de", "het", "een",
];

/// Segments of a title or subtitle that name a genre or a format, not the book: dropped from
/// the rest. Compared after [`fold`], so `Ein Roman` is `roman` and `A Novel` is `novel`.
const GENERIC_SEGMENTS: [&str; 22] = [
    "roman",
    "ein roman",
    "thriller",
    "krimi",
    "kriminalroman",
    "erzählung",
    "erzählungen",
    "novelle",
    "a novel",
    "novel",
    "a thriller",
    "a mystery",
    "mystery",
    "jugendbuch",
    "kinderbuch",
    "deutsche erstausgabe",
    "taschenbuch",
    "paperback",
    "roman taschenbuch",
    "a memoir",
    "psychothriller",
    "historischer roman",
];

/// Collection titles one author has many distinct books of: such a title only ever suggests.
const GENERIC_TITLES: [&str; 20] = [
    "gedichte",
    "erzählungen",
    "briefe",
    "tagebücher",
    "werke",
    "gesammelte werke",
    "ausgewählte werke",
    "sämtliche werke",
    "märchen",
    "kurzgeschichten",
    "novellen",
    "essays",
    "poems",
    "selected poems",
    "collected poems",
    "stories",
    "short stories",
    "collected stories",
    "letters",
    "selected letters",
];

/// Words that mark a volume; deleted with the numbers when two main titles are compared
/// without their numbers.
const VOLUME_WORDS: [&str; 14] = [
    "band", "bd", "volume", "vol", "teil", "tome", "tomo", "nr", "no", "folge", "part", "livre",
    "libro", "deel",
];

/// Roman numerals counted as numbers (2 to 20; `i` is a word too often).
const ROMAN: [&str; 19] = [
    "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv", "xv",
    "xvi", "xvii", "xviii", "xix", "xx",
];

/// Name suffixes skipped when the last word of a name is taken as the surname.
const NAME_SUFFIXES: [&str; 6] = ["jr", "sr", "junior", "senior", "ii", "iii"];

/// MARC language codes (as Open Library and the DNB write them) and the ISO 639-1 code the
/// search sends as `lang`.
const LANGUAGES: [(&str, &str); 19] = [
    ("ger", "de"),
    ("eng", "en"),
    ("fre", "fr"),
    ("spa", "es"),
    ("ita", "it"),
    ("dut", "nl"),
    ("por", "pt"),
    ("swe", "sv"),
    ("dan", "da"),
    ("nor", "no"),
    ("fin", "fi"),
    ("pol", "pl"),
    ("cze", "cs"),
    ("hun", "hu"),
    ("rus", "ru"),
    ("tur", "tr"),
    ("gre", "el"),
    ("jpn", "ja"),
    ("chi", "zh"),
];

/// Characters with a meaning in Open Library's (Lucene) query syntax.
const LUCENE_SPECIAL: &str = "+-&|!(){}[]^\"~*?:\\/";

fn is_apostrophe(c: char) -> bool {
    matches!(
        c,
        '\'' | '\u{2019}' | '\u{2018}' | '\u{02bc}' | '`' | '\u{00b4}'
    )
}

/// Diacritics dropped (NFKD without combining marks), then case-folded (`ß` → `ss`).
fn plain(text: &str) -> String {
    let stripped: String = text.nfkd().filter(|c| !is_combining_mark(*c)).collect();
    casefold(&stripped)
}

/// The words of `text` after [`plain`]: an apostrophe after a word-initial `l`, `d` or `j` is a
/// space (`L'Étranger` → `l etranger`), every other apostrophe is dropped (`Hitchhiker's` →
/// `hitchhikers`), `&` is `and`, and every other character that is not a letter or digit
/// separates words.
fn words(text: &str) -> Vec<String> {
    let text = plain(text);
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            word.push(c);
        } else if is_apostrophe(c) {
            if matches!(word.as_str(), "l" | "d" | "j") {
                out.push(std::mem::take(&mut word));
            }
        } else {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if c == '&' {
                out.push("and".to_owned());
            }
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// A title as a comparable key (notes §2.2): diacritics and case folded, `&`, `und` and `et` as
/// `and`, apostrophes and punctuation as described in [`words`], one leading article dropped.
pub fn fold(text: &str) -> String {
    let mut words: Vec<String> = words(text)
        .into_iter()
        .map(|word| match word.as_str() {
            "und" | "et" => "and".to_owned(),
            _ => word,
        })
        .collect();
    if words.len() > 1 && ARTICLES.contains(&words[0].as_str()) {
        words.remove(0);
    }
    words.join(" ")
}

/// `text` without the parts in `(…)` and `[…]` (nested ones included).
fn strip_brackets(text: &str) -> String {
    let mut depth = 0usize;
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The segments of a title: split at `:`, ` - `, ` – `, ` — `, `;` and ` / `.
fn segments(text: &str) -> Vec<String> {
    let mut text = text.to_owned();
    for separator in [" - ", " – ", " — ", " / "] {
        text = text.replace(separator, "\u{1f}");
    }
    text.split(['\u{1f}', ':', ';'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

fn generic_segment(folded: &str) -> bool {
    GENERIC_SEGMENTS
        .iter()
        .any(|generic| fold(generic) == folded)
}

fn is_roman(word: &str) -> bool {
    ROMAN.contains(&word)
}

/// The numbers in a title (brackets included): digit runs without leading zeros, and the
/// whole-word Roman numerals ii to xx as their value. Years count too.
pub fn numbers(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for word in words(text) {
        if is_roman(&word) {
            let value = ROMAN.iter().position(|r| *r == word).unwrap_or(0) + 2;
            out.insert(value.to_string());
            continue;
        }
        let mut run = String::new();
        for c in word.chars().chain(std::iter::once(' ')) {
            if c.is_ascii_digit() {
                run.push(c);
            } else if !run.is_empty() {
                let trimmed = run.trim_start_matches('0');
                out.insert(if trimmed.is_empty() { "0" } else { trimmed }.to_owned());
                run.clear();
            }
        }
    }
    out
}

/// A key without its number words and volume markers.
fn without_numbers(key: &str) -> String {
    key.split(' ')
        .filter(|word| {
            !word.is_empty()
                && !word.chars().any(|c| c.is_ascii_digit())
                && !is_roman(word)
                && !VOLUME_WORDS.contains(word)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One title variant (a title and its subtitle) as the matching rule compares it (notes §2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleKeys {
    /// The title without brackets, cut at the first separator, folded.
    pub main: String,
    /// The later segments and the subtitle, folded, genre and format words dropped.
    pub rest: String,
    /// `main` and `rest` together.
    pub full: String,
    /// The numbers in the whole title and subtitle.
    pub numbers: BTreeSet<String>,
}

impl TitleKeys {
    /// The keys of a (title, subtitle) pair.
    pub fn new(title: &str, subtitle: &str) -> TitleKeys {
        let mut title_segments = segments(&strip_brackets(title)).into_iter();
        let main = title_segments.next().map(|s| fold(&s)).unwrap_or_default();
        let rest: Vec<String> = title_segments
            .chain(segments(&strip_brackets(subtitle)))
            .map(|segment| fold(&segment))
            .filter(|segment| !segment.is_empty() && !generic_segment(segment))
            .collect();
        let rest = rest.join(" ");
        let full = collapse_ws(&format!("{main} {rest}"));
        TitleKeys {
            numbers: numbers(&format!("{title} {subtitle}")),
            main,
            rest,
            full,
        }
    }

    fn bare_main(&self) -> String {
        without_numbers(&self.main)
    }

    /// The main key is a collection title one author has many distinct books of (notes §2.3).
    pub fn generic(&self) -> bool {
        GENERIC_TITLES.iter().any(|title| fold(title) == self.main)
    }
}

/// How two title variants relate, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Relation {
    /// Same main title, same rest, same numbers.
    Equal,
    /// Same main title once the numbers are deleted, other numbers.
    NumberDiffers,
    /// Same main title and numbers, another rest (or only one side has one).
    RestDiffers,
    /// Every word of the shorter full title is a word of the longer one.
    Similar,
    None,
}

/// How `ours` relates to `theirs`.
pub fn relate(ours: &TitleKeys, theirs: &TitleKeys) -> Relation {
    if !ours.main.is_empty() && ours.main == theirs.main {
        if ours.numbers == theirs.numbers {
            return if ours.rest == theirs.rest {
                Relation::Equal
            } else {
                Relation::RestDiffers
            };
        }
        return Relation::NumberDiffers;
    }
    let (ours_bare, theirs_bare) = (ours.bare_main(), theirs.bare_main());
    if !ours_bare.is_empty() && ours_bare == theirs_bare && ours.numbers != theirs.numbers {
        return Relation::NumberDiffers;
    }
    let ours_words: HashSet<&str> = ours.full.split(' ').filter(|w| !w.is_empty()).collect();
    let theirs_words: HashSet<&str> = theirs.full.split(' ').filter(|w| !w.is_empty()).collect();
    let (short, long) = if ours_words.len() <= theirs_words.len() {
        (&ours_words, &theirs_words)
    } else {
        (&theirs_words, &ours_words)
    };
    if !short.is_empty() && short.is_subset(long) {
        Relation::Similar
    } else {
        Relation::None
    }
}

/// A name without its trailing life dates: each last comma part that holds a digit and at most
/// three letters (`1749-1832`, `b. 1950`, `fl. 1200`, `ca. 1500-1560`) is cut.
fn without_dates(name: &str) -> &str {
    let mut name = name.trim_end();
    while let Some((head, tail)) = name.rsplit_once(',') {
        let letters = tail.chars().filter(|c| c.is_alphabetic()).count();
        if tail.chars().any(|c| c.is_ascii_digit()) && letters <= 3 {
            name = head.trim_end();
        } else {
            break;
        }
    }
    name
}

/// The words of a name, without words made only of digits (a year is no surname).
fn name_words(text: &str) -> Vec<String> {
    words(text)
        .into_iter()
        .filter(|word| !word.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// An author as the matching rule compares names (notes §2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// The last word of the surname part, folded (`Le Guin` → `guin`).
    pub surname: String,
    /// The first letter of the first forename, if known.
    pub initial: Option<char>,
}

impl Person {
    /// `name` (natural order, or inverted with exactly one comma), or `sort_name` (`Surname,
    /// Forenames`) when it has a comma. Life dates are no part of a name: a trailing `, 1749-1832`
    /// (catalog records imported from MARC write `Goethe, Johann Wolfgang von, 1749-1832`) is cut
    /// before the commas are counted, and words made only of digits are dropped. `None` when no
    /// word is left.
    pub fn new(name: &str, sort_name: Option<&str>) -> Option<Person> {
        let name = without_dates(name);
        let sort_name = sort_name.map(without_dates);
        let inverted = sort_name.and_then(|sort| sort.split_once(',')).or_else(|| {
            (name.matches(',').count() == 1)
                .then(|| name.split_once(','))
                .flatten()
        });
        let (surname_words, initial) = match inverted {
            Some((surname, forenames)) => (
                name_words(surname),
                name_words(forenames)
                    .first()
                    .and_then(|word| word.chars().next()),
            ),
            None => {
                let all = name_words(name);
                let initial = (all.len() > 1)
                    .then(|| all.first().and_then(|word| word.chars().next()))
                    .flatten();
                (all, initial)
            }
        };
        let mut surname_words = surname_words;
        if surname_words.len() > 1
            && surname_words
                .last()
                .is_some_and(|last| NAME_SUFFIXES.contains(&last.as_str()))
        {
            surname_words.pop();
        }
        let surname = surname_words.pop()?;
        Some(Person { surname, initial })
    }

    /// Equal surnames, and equal initials unless one is unknown.
    pub fn agrees(&self, other: &Person) -> bool {
        self.surname == other.surname
            && match (self.initial, other.initial) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

/// Lucene's special characters become spaces, whitespace collapses: a stray quote or slash in
/// a title must not turn the query into an error answer.
pub fn escape(value: &str) -> String {
    let replaced: String = value
        .chars()
        .map(|c| if LUCENE_SPECIAL.contains(c) { ' ' } else { c })
        .collect();
    collapse_ws(&replaced)
}

/// A language as the search uses it: the MARC code editions carry, and the ISO 639-1 code
/// Open Library's `lang` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    pub marc: &'static str,
    pub iso: &'static str,
}

impl Language {
    fn from_marc(code: &str) -> Option<Language> {
        LANGUAGES
            .iter()
            .find(|(marc, _)| *marc == code)
            .map(|(marc, iso)| Language { marc, iso })
    }

    fn from_iso(code: &str) -> Option<Language> {
        LANGUAGES
            .iter()
            .find(|(_, iso)| *iso == code)
            .map(|(marc, iso)| Language { marc, iso })
    }
}

/// The book's language: the most frequent known MARC code of its editions, else the
/// registration group of its first ISBN that names one (978-3 German, 978-0 and 978-1
/// English, 978-2 and 979-10 French), else unknown.
pub fn language_hint(edition_languages: &[String], isbns: &[String]) -> Option<Language> {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for code in edition_languages {
        if let Some(language) = Language::from_marc(code) {
            match counts.iter_mut().find(|(marc, _)| *marc == language.marc) {
                Some((_, n)) => *n += 1,
                None => counts.push((language.marc, 1)),
            }
        }
    }
    // The most frequent; on a tie the first seen.
    let mut best: Option<(&'static str, usize)> = None;
    for (marc, n) in counts {
        if best.is_none_or(|(_, top)| n > top) {
            best = Some((marc, n));
        }
    }
    if let Some((marc, _)) = best {
        return Language::from_marc(marc);
    }
    isbns.iter().find_map(|isbn| {
        let iso = if isbn.starts_with("9783") {
            "de"
        } else if isbn.starts_with("9780") || isbn.starts_with("9781") {
            "en"
        } else if isbn.starts_with("9782") || isbn.starts_with("97910") {
            "fr"
        } else {
            return None;
        };
        Language::from_iso(iso)
    })
}

/// What the search knows about one of our works.
#[derive(Debug, Clone, Default)]
pub struct Book {
    pub title: String,
    pub subtitle: String,
    /// The (title, subtitle) of each edition whose title differs from the work's (a
    /// translation's own title).
    pub edition_titles: Vec<(String, String)>,
    /// The first author's name and sort name.
    pub author: Option<(String, Option<String>)>,
    pub ol_work_id: Option<String>,
    pub language: Option<Language>,
}

impl Book {
    fn variants(&self) -> Vec<TitleKeys> {
        let mut out = vec![TitleKeys::new(&self.title, &self.subtitle)];
        for (title, subtitle) in &self.edition_titles {
            let keys = TitleKeys::new(title, subtitle);
            if !out.contains(&keys) {
                out.push(keys);
            }
        }
        out
    }

    fn person(&self) -> Option<Person> {
        let (name, sort) = self.author.as_ref()?;
        Person::new(name, sort.as_deref())
    }

    /// The main title as written (brackets dropped, cut at the first separator).
    pub fn main_title(&self) -> String {
        segments(&strip_brackets(&self.title))
            .into_iter()
            .next()
            .unwrap_or_default()
    }

    /// The request for Open Library (notes §2.1); `None` when the title escapes to nothing.
    pub fn query(&self) -> Option<CoverQuery> {
        let title = self.main_title();
        if escape(&title).is_empty() {
            return None;
        }
        let author = self
            .author
            .as_ref()
            .map(|(name, sort)| author_surname(name, sort.as_deref()))
            .filter(|surname| !escape(surname).is_empty());
        Some(CoverQuery {
            title,
            author,
            language: self.language.map(|language| language.iso.to_owned()),
        })
    }

    /// Our main title is a collection title (`Gedichte`, `Selected Poems`).
    pub fn generic_title(&self) -> bool {
        TitleKeys::new(&self.title, &self.subtitle).generic()
    }
}

/// Why a match is only a suggestion (`cover_suggestions.reason`), in the order a suggestion
/// is chosen by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    SeveralWorks,
    GenericTitle,
    NumberDiffers,
    TitleDiffers,
    NoAuthor,
    /// A clear match for a book whose cover was taken down (set by the search, not here).
    AfterTakedown,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::SeveralWorks => "several_works",
            Reason::GenericTitle => "generic_title",
            Reason::NumberDiffers => "number_differs",
            Reason::TitleDiffers => "title_differs",
            Reason::NoAuthor => "no_author",
            Reason::AfterTakedown => "after_takedown",
        }
    }
}

/// The candidate picture a verdict chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub ol_work_id: String,
    /// Set when the cover is the edition's.
    pub ol_edition_id: Option<String>,
    pub cover_id: i64,
    /// Open Library's title (and subtitle) of what the cover shows.
    pub title: String,
    pub authors: String,
    pub year: Option<i32>,
    /// The edition's MARC language, when the cover is an edition's.
    pub language: Option<String>,
}

/// The outcome of the matching rule (notes §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Clear(Pick),
    Uncertain(Pick, Reason),
    NoMatch,
}

/// One doc of the answer, related to our book.
struct Related<'a> {
    index: usize,
    doc: &'a CoverDoc,
    relation: Relation,
    /// The best relation came from the doc's best edition (a translation's title).
    edition_matched: bool,
    author_agrees: bool,
    /// One of our title variants that is a collection title relates `Equal` to this doc (an
    /// edition titled `Gedichte` of a work with another title).
    generic: bool,
}

fn relate_doc<'a>(
    index: usize,
    doc: &'a CoverDoc,
    ours: &[TitleKeys],
    person: Option<&Person>,
) -> Related<'a> {
    let theirs_doc = TitleKeys::new(&doc.title, &doc.subtitle);
    let theirs_edition = doc
        .edition
        .as_ref()
        .filter(|edition| !edition.title.is_empty())
        .map(|edition| TitleKeys::new(&edition.title, &edition.subtitle));
    let mut best = (Relation::None, false);
    let mut generic = false;
    for our in ours {
        let by_doc = relate(our, &theirs_doc);
        if by_doc < best.0 {
            best = (by_doc, false);
        }
        let mut equal = by_doc == Relation::Equal;
        if let Some(theirs) = &theirs_edition {
            let by_edition = relate(our, theirs);
            if by_edition < best.0 || (by_edition == best.0 && by_edition != Relation::None) {
                best = (by_edition, true);
            }
            equal |= by_edition == Relation::Equal;
        }
        generic |= equal && our.generic();
    }
    let author_agrees = person.is_some_and(|person| {
        doc.authors
            .iter()
            .filter_map(|name| Person::new(name, None))
            .any(|theirs| person.agrees(&theirs))
    });
    Related {
        index,
        doc,
        relation: best.0,
        edition_matched: best.1,
        author_agrees,
        generic,
    }
}

fn usable(cover: Option<i64>, unusable: &HashSet<i64>) -> Option<i64> {
    cover.filter(|id| *id > 0 && !unusable.contains(id))
}

fn joined(title: &str, subtitle: &str) -> String {
    if subtitle.is_empty() {
        title.to_owned()
    } else {
        format!("{title}: {subtitle}")
    }
}

/// Which cover of a matched work (notes §2.4): the best edition's when it is in the book's
/// language or its title is the one that matched; else the work's; else the edition's in
/// another language.
fn choose_cover(
    related: &Related<'_>,
    language: Option<Language>,
    unusable: &HashSet<i64>,
) -> Option<Pick> {
    let doc = related.doc;
    let authors = doc.authors.join(", ");
    let from_edition = |cover_id: i64| {
        let edition = doc.edition.as_ref().expect("an edition");
        let title = if edition.title.is_empty() {
            joined(&doc.title, &doc.subtitle)
        } else {
            joined(&edition.title, &edition.subtitle)
        };
        Pick {
            ol_work_id: doc.ol_work_id.clone(),
            ol_edition_id: Some(edition.ol_edition_id.clone()),
            cover_id,
            title,
            authors: authors.clone(),
            year: doc.first_publish_year,
            language: edition.languages.first().cloned(),
        }
    };
    let edition_cover = doc
        .edition
        .as_ref()
        .and_then(|edition| usable(edition.cover_id, unusable));
    if let (Some(cover_id), Some(edition)) = (edition_cover, &doc.edition) {
        let in_language =
            language.is_some_and(|language| edition.languages.iter().any(|l| l == language.marc));
        if related.edition_matched || in_language {
            return Some(from_edition(cover_id));
        }
    }
    if let Some(cover_id) = usable(doc.cover_id, unusable) {
        return Some(Pick {
            ol_work_id: doc.ol_work_id.clone(),
            ol_edition_id: None,
            cover_id,
            title: joined(&doc.title, &doc.subtitle),
            authors,
            year: doc.first_publish_year,
            language: None,
        });
    }
    edition_cover.map(from_edition)
}

/// `docs[i]` dominates the others: its known edition count is at least 3 times and at least 5
/// more than each other's, and every count is known and at least 1. Needs at least one other.
fn dominates(candidate: &Related<'_>, others: &[&Related<'_>]) -> bool {
    let Some(count) = candidate.doc.edition_count.filter(|n| *n >= 1) else {
        return false;
    };
    !others.is_empty()
        && others.iter().all(|other| {
            other
                .doc
                .edition_count
                .filter(|n| *n >= 1)
                .is_some_and(|n| count >= 3 * n && count >= n + 5)
        })
}

/// The verdict for `book` on Open Library's `answer` (notes §2.3). `blocked`: Open Library
/// works rejected or approved for the book (dropped first). `unusable`: cover ids whose media
/// row is removed or gone (they count as absent).
pub fn verdict(
    book: &Book,
    answer: &CoverAnswer,
    blocked: &HashSet<String>,
    unusable: &HashSet<i64>,
) -> Verdict {
    let ours = book.variants();
    let person = book.person();
    let related: Vec<Related<'_>> = answer
        .docs
        .iter()
        .filter(|doc| !blocked.contains(&doc.ol_work_id))
        .enumerate()
        .map(|(index, doc)| relate_doc(index, doc, &ours, person.as_ref()))
        .collect();

    // Identity: Open Library added a cover to our own record since we stored it.
    if let Some(own) = book.ol_work_id.as_deref()
        && let Some(identity) = related.iter().find(|r| r.doc.ol_work_id == own)
        && let Some(pick) = choose_cover(identity, book.language, unusable)
    {
        return Verdict::Clear(pick);
    }

    let mut candidates: Vec<(Reason, &Related<'_>)> = Vec::new();
    if person.is_none() {
        candidates.extend(
            related
                .iter()
                .filter(|r| r.relation == Relation::Equal)
                .map(|r| (Reason::NoAuthor, r)),
        );
    } else {
        let equal: Vec<&Related<'_>> = related
            .iter()
            .filter(|r| r.relation == Relation::Equal && r.author_agrees)
            .collect();
        if book.generic_title() || equal.iter().any(|r| r.generic) {
            candidates.extend(equal.iter().map(|r| (Reason::GenericTitle, *r)));
        } else if !equal.is_empty() {
            let chosen = if equal.len() == 1 && answer.num_found <= 10 {
                Some(equal[0])
            } else {
                equal.iter().copied().find(|candidate| {
                    let others: Vec<&Related<'_>> = equal
                        .iter()
                        .copied()
                        .filter(|other| other.index != candidate.index)
                        .collect();
                    dominates(candidate, &others)
                })
            };
            if let Some(chosen) = chosen {
                return match choose_cover(chosen, book.language, unusable) {
                    Some(pick) => Verdict::Clear(pick),
                    None => Verdict::NoMatch,
                };
            }
            candidates.extend(equal.iter().map(|r| (Reason::SeveralWorks, *r)));
        }
        for r in related.iter().filter(|r| r.author_agrees) {
            match r.relation {
                Relation::NumberDiffers => candidates.push((Reason::NumberDiffers, r)),
                Relation::RestDiffers | Relation::Similar => {
                    candidates.push((Reason::TitleDiffers, r))
                }
                Relation::Equal | Relation::None => {}
            }
        }
    }
    candidates.sort_by_key(|(reason, r)| {
        (
            *reason,
            std::cmp::Reverse(r.doc.edition_count.unwrap_or(0)),
            r.index,
        )
    });
    candidates
        .into_iter()
        .find_map(|(reason, r)| {
            choose_cover(r, book.language, unusable).map(|pick| Verdict::Uncertain(pick, reason))
        })
        .unwrap_or(Verdict::NoMatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::shared::lookup::CoverEdition;

    #[test]
    fn fold_drops_case_accents_articles_and_punctuation() {
        let cases = [
            ("Der Distelfink", "distelfink"),
            ("Die Straße", "strasse"),
            ("Ça et là", "ca and la"),
            ("Krieg und Frieden", "krieg and frieden"),
            ("Pride & Prejudice", "pride and prejudice"),
            ("L'Étranger", "etranger"),
            ("The Hitchhiker's Guide", "hitchhikers guide"),
            ("Il nome della rosa", "nome della rosa"),
            ("El amor en los tiempos", "amor en los tiempos"),
            ("Het diner", "diner"),
            ("Une saison", "saison"),
            ("Die", "die"),
            ("Momo!", "momo"),
            ("  Über   alles?! ", "uber alles"),
            ("d'Artagnan", "d artagnan"),
        ];
        for (input, expected) in cases {
            assert_eq!(fold(input), expected, "{input}");
        }
    }

    #[test]
    fn split_reads_main_rest_and_numbers() {
        let keys = TitleKeys::new("Percy Jackson – Im Bann des Zyklopen", "");
        assert_eq!(keys.main, "percy jackson");
        assert_eq!(keys.rest, "im bann des zyklopen");
        let keys = TitleKeys::new("Der Distelfink", "Roman");
        assert_eq!((keys.main.as_str(), keys.rest.as_str()), ("distelfink", ""));
        let keys = TitleKeys::new("Harry Potter und der Stein der Weisen (Band 1)", "");
        assert_eq!(keys.main, "harry potter and der stein der weisen");
        assert_eq!(keys.rest, "");
        assert_eq!(keys.numbers, BTreeSet::from(["1".to_owned()]));
        let keys = TitleKeys::new("Gregs Tagebuch - Jetzt reicht's!", "Ein Roman");
        assert_eq!(keys.main, "gregs tagebuch");
        assert_eq!(keys.rest, "jetzt reichts");
        let keys = TitleKeys::new("Dune: House Harkonnen", "");
        assert_eq!(
            (keys.main.as_str(), keys.rest.as_str()),
            ("dune", "house harkonnen")
        );
        let keys = TitleKeys::new("Die Chroniken von Narnia", "Gesamtausgabe im Schuber TB");
        assert_eq!(keys.rest, "gesamtausgabe im schuber tb");
        let keys = TitleKeys::new("Eins; Zwei / Drei", "A Novel");
        assert_eq!(
            (keys.main.as_str(), keys.rest.as_str()),
            ("eins", "zwei drei")
        );
        assert_eq!(keys.full, "eins zwei drei");
        let keys = TitleKeys::new("[Untitled] (draft)", "");
        assert_eq!(keys.main, "");
    }

    #[test]
    fn numbers_read_digits_romans_and_years() {
        let n = |text: &str| numbers(text).into_iter().collect::<Vec<_>>();
        assert_eq!(n("Band 2"), ["2"]);
        assert_eq!(n("#2"), ["2"]);
        assert_eq!(n("Volume II"), ["2"]);
        assert_eq!(n("Leviathan Wakes (The Expanse, 3)"), ["3"]);
        assert_eq!(n("1984"), ["1984"]);
        assert_eq!(n("Agent 007"), ["7"]);
        assert_eq!(n("Louis XIV"), ["14"]);
        assert_eq!(n("I, Robot"), Vec::<String>::new());
        assert_eq!(n("Catch-22"), ["22"]);
    }

    #[test]
    fn names_follow_one_surname_rule() {
        let p = |name: &str, sort: Option<&str>| Person::new(name, sort).unwrap();
        let le_guin = p("Ursula K. Le Guin", None);
        assert_eq!(le_guin.surname, "guin");
        assert!(le_guin.agrees(&p("Le Guin, Ursula K.", None)));
        assert!(le_guin.agrees(&p("Ursula K. Le Guin", Some("Le Guin, Ursula K."))));
        let saint = p("Antoine de Saint-Exupéry", None);
        assert_eq!(saint.surname, "exupery");
        assert!(saint.agrees(&p("Saint-Exupéry, Antoine de", None)));
        assert!(p("Simone de Beauvoir", None).agrees(&p("de Beauvoir, Simone", None)));
        assert_eq!(
            p("Marius Müller-Westernhagen", None).surname,
            "westernhagen"
        );
        let tolkien = p("J. R. R. Tolkien", None);
        assert!(tolkien.agrees(&p("John Ronald Reuel Tolkien", None)));
        assert!(tolkien.agrees(&p("Tolkien", None)));
        assert!(!p("Stephen King", None).agrees(&p("Tabitha King", None)));
        assert!(p("Martin Luther King Jr.", None).agrees(&p("Martin Luther King", None)));
        assert_eq!(Person::new("  ", None), None);
        // Life dates, as records imported from MARC carry them, are no surname.
        let goethe = p("Goethe, Johann Wolfgang von, 1749-1832", None);
        assert_eq!(
            (goethe.surname.as_str(), goethe.initial),
            ("goethe", Some('j'))
        );
        assert!(goethe.agrees(&p("Johann Wolfgang von Goethe", None)));
        assert_eq!(p("Tolkien, J. R. R. 1892-1973", None).surname, "tolkien");
        assert_eq!(p("Ende, Michael, b. 1929", None).surname, "ende");
        assert_eq!(
            p(
                "Brecht, Bertolt, 1898-1956",
                Some("Brecht, Bertolt, 1898-1956")
            )
            .initial,
            Some('b')
        );
        assert!(
            !p("King, Stephen, 1947-", None).agrees(&p("King, Tabitha, 1949-", None)),
            "two dated names agree only on their surname and initial"
        );
        assert_eq!(Person::new("1749-1832", None), None);
    }

    #[test]
    fn the_query_escapes_lucene_syntax() {
        for c in LUCENE_SPECIAL.chars() {
            assert_eq!(escape(&format!("a{c}b")), "a b", "{c}");
        }
        assert_eq!(escape("Say \"Hello"), "Say Hello");
        assert_eq!(escape("?!*"), "");
        let book = Book {
            title: "?!*".into(),
            ..Book::default()
        };
        assert_eq!(book.query(), None);
        let book = Book {
            title: "Die Tribute von Panem – Gefährliche Liebe".into(),
            author: Some((
                "Ursula K. Le Guin".into(),
                Some("Le Guin, Ursula K.".into()),
            )),
            language: Language::from_marc("ger"),
            ..Book::default()
        };
        assert_eq!(
            book.query(),
            Some(CoverQuery {
                title: "Die Tribute von Panem".into(),
                author: Some("Le Guin".into()),
                language: Some("de".into()),
            })
        );
    }

    #[test]
    fn the_language_comes_from_editions_then_the_isbn() {
        let marc = |codes: &[&str]| codes.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        assert_eq!(
            language_hint(&marc(&["eng", "ger", "ger"]), &[]).map(|l| l.iso),
            Some("de")
        );
        assert_eq!(
            language_hint(&[], &marc(&["9783161484100"])).map(|l| l.iso),
            Some("de")
        );
        assert_eq!(
            language_hint(&[], &marc(&["9791032000000"])).map(|l| l.iso),
            Some("fr")
        );
        assert_eq!(
            language_hint(&marc(&["xxx"]), &marc(&["9784000000000"])),
            None
        );
    }

    // ---- verdicts -------------------------------------------------------------------------

    fn doc(id: u32, title: &str, subtitle: &str, author: &str) -> CoverDoc {
        CoverDoc {
            ol_work_id: format!("OL{id}W"),
            title: title.into(),
            subtitle: subtitle.into(),
            authors: vec![author.into()],
            cover_id: Some(i64::from(id) * 100),
            edition_count: Some(10),
            first_publish_year: Some(2000),
            languages: vec!["eng".into()],
            edition: None,
        }
    }

    fn with_edition(mut d: CoverDoc, title: &str, subtitle: &str, lang: &[&str]) -> CoverDoc {
        let n = d
            .ol_work_id
            .trim_start_matches("OL")
            .trim_end_matches('W')
            .to_owned();
        d.edition = Some(CoverEdition {
            ol_edition_id: format!("OL{n}M"),
            title: title.into(),
            subtitle: subtitle.into(),
            cover_id: Some(d.cover_id.unwrap() + 1),
            languages: lang.iter().map(|l| l.to_string()).collect(),
        });
        d
    }

    fn book(title: &str, author: &str) -> Book {
        Book {
            title: title.into(),
            author: (!author.is_empty()).then(|| (author.to_owned(), None)),
            ..Book::default()
        }
    }

    fn answer(docs: Vec<CoverDoc>) -> CoverAnswer {
        CoverAnswer {
            num_found: docs.len() as i64,
            docs,
            refused: None,
        }
    }

    fn judge(book: &Book, docs: Vec<CoverDoc>) -> Verdict {
        verdict(book, &answer(docs), &HashSet::new(), &HashSet::new())
    }

    fn clear_cover(v: &Verdict) -> Option<i64> {
        match v {
            Verdict::Clear(pick) => Some(pick.cover_id),
            _ => None,
        }
    }

    fn reason(v: &Verdict) -> Option<Reason> {
        match v {
            Verdict::Uncertain(_, reason) => Some(*reason),
            _ => None,
        }
    }

    #[test]
    fn an_equal_title_by_the_same_author_is_clear() {
        let v = judge(
            &book("Momo", "Michael Ende"),
            vec![doc(1, "Momo", "", "Michael Ende")],
        );
        assert_eq!(clear_cover(&v), Some(100));
        // The same title by another author is another book.
        let v = judge(
            &book("Momo", "Michael Ende"),
            vec![doc(1, "Momo", "", "Anna Ende")],
        );
        assert_eq!(v, Verdict::NoMatch);
        let v = judge(
            &book("Momo", "Michael Ende"),
            vec![doc(1, "Momo", "", "Kai Meyer")],
        );
        assert_eq!(v, Verdict::NoMatch);
    }

    #[test]
    fn identity_is_clear_whatever_the_titles_say() {
        let mut ours = book("Eine ganz andere Geschichte", "");
        ours.ol_work_id = Some("OL7W".into());
        let v = judge(&ours, vec![doc(7, "Something else", "", "Nobody")]);
        assert_eq!(clear_cover(&v), Some(700));
    }

    #[test]
    fn dominance_needs_three_times_and_five_more() {
        let ours = book("Circe", "Madeline Miller");
        let mut main = doc(1, "Circe", "", "Madeline Miller");
        let mut stub = doc(2, "Circe", "", "Madeline Miller");
        let cases = [
            (Some(15), Some(5), true),
            (Some(14), Some(5), false), // under 3 times
            (Some(6), Some(2), false),  // 3 times, but not 5 more
            (Some(7), Some(2), true),
            (Some(15), None, false),    // unknown
            (Some(15), Some(0), false), // zero is not a known count
        ];
        for (a, b, clear) in cases {
            main.edition_count = a;
            stub.edition_count = b;
            let v = judge(&ours, vec![stub.clone(), main.clone()]);
            if clear {
                assert_eq!(clear_cover(&v), Some(100), "{a:?} {b:?}");
            } else {
                assert_eq!(reason(&v), Some(Reason::SeveralWorks), "{a:?} {b:?}");
                // The suggestion is the doc with the most editions.
                if let Verdict::Uncertain(pick, _) = &v
                    && a.unwrap_or(0) > b.unwrap_or(0)
                {
                    assert_eq!(pick.cover_id, 100);
                }
            }
        }
    }

    #[test]
    fn only_counts_when_nothing_was_cut_off() {
        let ours = book("Circe", "Madeline Miller");
        let mut many = answer(vec![doc(1, "Circe", "", "Madeline Miller")]);
        many.num_found = 11;
        let v = verdict(&ours, &many, &HashSet::new(), &HashSet::new());
        assert_eq!(reason(&v), Some(Reason::SeveralWorks));
        many.num_found = 10;
        let v = verdict(&ours, &many, &HashSet::new(), &HashSet::new());
        assert_eq!(clear_cover(&v), Some(100));
    }

    #[test]
    fn collection_titles_and_books_without_an_author_only_suggest() {
        let v = judge(
            &book("Gedichte", "Rainer Maria Rilke"),
            vec![doc(1, "Gedichte", "", "Rilke")],
        );
        assert_eq!(reason(&v), Some(Reason::GenericTitle));
        let v = judge(
            &book("Selected Poems", "Sylvia Plath"),
            vec![doc(1, "Selected poems", "", "Sylvia Plath")],
        );
        assert_eq!(reason(&v), Some(Reason::GenericTitle));
        // The equal title may come from one of our editions: a work with another title whose
        // edition is called "Gedichte" still only gets a suggestion.
        let mut ours = book("Die Sonette an Orpheus", "Rainer Maria Rilke");
        ours.edition_titles = vec![("Gedichte".into(), String::new())];
        let v = judge(&ours, vec![doc(1, "Gedichte", "", "Rainer Maria Rilke")]);
        assert_eq!(reason(&v), Some(Reason::GenericTitle));
        let v = judge(&book("Momo", ""), vec![doc(1, "Momo", "", "Michael Ende")]);
        assert_eq!(reason(&v), Some(Reason::NoAuthor));
        // No author and only a similar title: nothing.
        let v = judge(
            &book("Momo", ""),
            vec![doc(1, "Momo und die Zeit", "", "Michael Ende")],
        );
        assert_eq!(v, Verdict::NoMatch);
    }

    #[test]
    fn numbers_must_agree() {
        let v = judge(
            &book("Die Tribute von Panem", "Suzanne Collins"),
            vec![doc(1, "Die Tribute von Panem 1", "", "Suzanne Collins")],
        );
        assert_eq!(reason(&v), Some(Reason::NumberDiffers));
        let v = judge(
            &book("Harry Potter und der Stein der Weisen", "J. K. Rowling"),
            vec![doc(
                1,
                "Harry Potter und der Stein der Weisen (Band 1)",
                "",
                "J.K. Rowling",
            )],
        );
        assert_eq!(reason(&v), Some(Reason::NumberDiffers));
        let v = judge(
            &book("1984", "George Orwell"),
            vec![doc(1, "1984", "", "George Orwell")],
        );
        assert_eq!(clear_cover(&v), Some(100));
    }

    #[test]
    fn series_volumes_never_attach() {
        let cases = [
            (
                "Percy Jackson – Im Bann des Zyklopen",
                "Rick Riordan",
                ("Percy Jackson", ""),
            ),
            (
                "Gregs Tagebuch - Jetzt reicht's!",
                "Jeff Kinney",
                ("Gregs Tagebuch", ""),
            ),
            ("Dune: House Harkonnen", "Brian Herbert", ("Dune", "")),
            (
                "Sherlock Holmes - Der Hund der Baskervilles",
                "Arthur Conan Doyle",
                ("Sherlock Holmes", ""),
            ),
            (
                "Der Herr der Ringe - Die Gefährten",
                "J. R. R. Tolkien",
                ("Der Herr der Ringe", ""),
            ),
            (
                "Die Chroniken von Narnia - Der König von Narnia",
                "C. S. Lewis",
                ("Die Chroniken von Narnia", "Gesamtausgabe im Schuber TB"),
            ),
            (
                "Harry Potter: Der Gefangene von Askaban",
                "J. K. Rowling",
                ("Harry Potter", ""),
            ),
            (
                "Percy Jackson – Im Bann des Zyklopen",
                "Rick Riordan",
                ("Im Bann des Zyklopen", "Percy Jackson 2"),
            ),
        ];
        for (title, author, (their_title, their_subtitle)) in cases {
            let mut big = doc(1, their_title, their_subtitle, author);
            big.edition_count = Some(500);
            let v = judge(
                &book(title, author),
                vec![big, doc(2, "Unrelated", "", author)],
            );
            assert!(
                matches!(
                    v,
                    Verdict::Uncertain(_, Reason::TitleDiffers | Reason::NumberDiffers)
                ),
                "{title}: {v:?}"
            );
        }
        // Volume 2 never gets volume 1's cover, also through the German edition's title.
        let v = judge(
            &book("Dune: House Harkonnen", "Brian Herbert"),
            vec![with_edition(
                doc(1, "Dune", "", "Frank Herbert"),
                "Dune",
                "",
                &["ger"],
            )],
        );
        assert_eq!(v, Verdict::NoMatch, "another author");
    }

    #[test]
    fn panem_two_picks_catching_fire_through_its_german_edition() {
        let mut ours = book(
            "Die Tribute von Panem – Gefährliche Liebe",
            "Suzanne Collins",
        );
        ours.language = Language::from_marc("ger");
        let hunger = with_edition(
            doc(1, "The Hunger Games", "", "Suzanne Collins"),
            "Die Tribute von Panem",
            "Tödliche Spiele",
            &[],
        );
        let catching = with_edition(
            doc(2, "Catching Fire", "", "Suzanne Collins"),
            "Die Tribute von Panem",
            "Gefährliche Liebe",
            &[],
        );
        let mocking = with_edition(
            doc(3, "Mockingjay", "", "Suzanne Collins"),
            "Die Tribute von Panem",
            "Flammender Zorn",
            &["ger"],
        );
        let v = judge(&ours, vec![hunger, catching, mocking]);
        match v {
            Verdict::Clear(pick) => {
                assert_eq!(pick.ol_work_id, "OL2W");
                assert_eq!(pick.ol_edition_id.as_deref(), Some("OL2M"));
                assert_eq!(pick.cover_id, 201);
                assert_eq!(pick.title, "Die Tribute von Panem: Gefährliche Liebe");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_translation_matches_through_its_edition_and_takes_its_cover() {
        let mut ours = book("Der Distelfink", "Donna Tartt");
        ours.language = Language::from_marc("ger");
        let goldfinch = with_edition(
            doc(16809803, "The Goldfinch", "", "Donna Tartt"),
            "Der Distelfink",
            "Roman",
            &[],
        );
        let v = judge(&ours, vec![goldfinch]);
        assert_eq!(clear_cover(&v), Some(1_680_980_301));
        // Without a German edition, the English title is no match for a German one.
        let v = judge(
            &ours,
            vec![doc(16809803, "The Goldfinch", "", "Donna Tartt")],
        );
        assert_eq!(v, Verdict::NoMatch);
    }

    #[test]
    fn the_edition_in_the_books_language_wins_then_the_work_then_another_language() {
        let mut ours = book("Circe", "Madeline Miller");
        ours.language = Language::from_marc("ger");
        let base = || doc(1, "Circe", "", "Madeline Miller");
        // The edition is German: its cover.
        let v = judge(&ours, vec![with_edition(base(), "Circe", "", &["ger"])]);
        assert_eq!(clear_cover(&v), Some(101));
        // An English edition whose title matched: matched variant wins too.
        let v = judge(&ours, vec![with_edition(base(), "Circe", "", &["eng"])]);
        assert_eq!(clear_cover(&v), Some(101));
        // An edition with another title in another language: the work's cover.
        let v = judge(&ours, vec![with_edition(base(), "Kirke", "", &["dan"])]);
        assert_eq!(clear_cover(&v), Some(100));
        // No work cover: the other-language edition's.
        let mut bare = with_edition(base(), "Kirke", "", &["dan"]);
        bare.cover_id = None;
        let v = judge(&ours, vec![bare]);
        assert_eq!(clear_cover(&v), Some(101));
    }

    #[test]
    fn rejected_approved_removed_and_gone_candidates_are_out() {
        let ours = book("Momo", "Michael Ende");
        let docs = vec![doc(1, "Momo", "", "Michael Ende")];
        let blocked = HashSet::from(["OL1W".to_owned()]);
        assert_eq!(
            verdict(&ours, &answer(docs.clone()), &blocked, &HashSet::new()),
            Verdict::NoMatch
        );
        let unusable = HashSet::from([100]);
        assert_eq!(
            verdict(&ours, &answer(docs.clone()), &HashSet::new(), &unusable),
            Verdict::NoMatch
        );
        // A rejected duplicate no longer keeps the other record from being the only one.
        let docs = vec![
            doc(1, "Momo", "", "Michael Ende"),
            doc(2, "Momo", "", "Michael Ende"),
        ];
        assert_eq!(
            clear_cover(&verdict(&ours, &answer(docs), &blocked, &HashSet::new())),
            Some(200)
        );
    }

    #[test]
    fn a_dominating_doc_without_a_cover_is_no_match() {
        let ours = book("Circe", "Madeline Miller");
        let mut main = doc(1, "Circe", "", "Madeline Miller");
        main.edition_count = Some(50);
        main.cover_id = None;
        let mut stub = doc(2, "Circe", "", "Madeline Miller");
        stub.edition_count = Some(1);
        assert_eq!(judge(&ours, vec![main, stub]), Verdict::NoMatch);
    }

    #[test]
    fn similar_titles_and_subtitles_only_suggest() {
        let v = judge(
            &book("Dune", "Frank Herbert"),
            vec![doc(1, "Dune Messiah", "", "Frank Herbert")],
        );
        assert_eq!(reason(&v), Some(Reason::TitleDiffers));
        // A genre subtitle is no difference.
        let v = judge(
            &book("Der Schwarm", "Frank Schätzing"),
            vec![doc(1, "Der Schwarm", "Roman", "Frank Schätzing")],
        );
        assert_eq!(clear_cover(&v), Some(100));
        // Suggestions prefer the reason order, then the edition count.
        let mut similar = doc(1, "Dune Messiah", "", "Frank Herbert");
        similar.edition_count = Some(900);
        let numbered = doc(2, "Dune 2", "", "Frank Herbert");
        let v = judge(&book("Dune", "Frank Herbert"), vec![similar, numbered]);
        assert_eq!(reason(&v), Some(Reason::NumberDiffers));
    }
}
