//! The one DNB MARC21-xml mapping, shared by the OAI-PMH harvest (HAR) and the SRU lookup (CAT)
//! (docs/phase1/conventions.md §7.5). Each caller hands it one `<record>` element; envelopes,
//! OAI headers and deleted records stay with the caller.
//!
//! Only CC0 fields are read: 001, 008, 020, 041, 100/700, 245, 260/264, 263, 300, 337/338 and
//! the `(Produktform)` 653. Every other field, notably 520 (VLB blurbs) and 856 (links to VLB
//! texts and publisher sites), is skipped while parsing and never reaches a record.

use std::sync::LazyLock;

use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use regex::Regex;
use time::{Date, Month};
use unicode_normalization::UnicodeNormalization;

use super::ingest::{AuthorRecord, ContributorRole, EditionRecord, Format, Source, WorkRecord};
use super::isbn::{Isbn13, parse_isbn, registration_group};
use crate::extract::Patch;

/// One mapped DNB record: the work facts and one edition per ISBN (one without an ISBN when the
/// record has none).
#[derive(Debug, Clone, PartialEq)]
pub struct DnbRecord {
    /// MARC 001.
    pub idn: String,
    pub work: WorkRecord,
    pub editions: Vec<EditionRecord>,
}

/// Why a record cannot be mapped.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MarcError {
    #[error("malformed MARC21-xml: {0}")]
    Xml(String),
    #[error("the record has no 001 control field")]
    NoIdn,
    #[error("the record has no usable title")]
    NoTitle,
}

/// The data fields the mapping reads. Everything else is dropped by the parser.
const READ_TAGS: &[&str] = &[
    "020", "041", "100", "245", "260", "263", "264", "300", "337", "338", "653", "700",
];

/// DNB announces books a year or two ahead. A year further out than this is one of its
/// placeholders for "not known yet" (recorded records carry 2040, 2050 and 9999, with 263 dates
/// to match) and counts as missing, in 260/264 `$c` and in 263 alike.
const ANNOUNCEMENT_HORIZON_YEARS: i32 = 5;

/// The latest year a record can carry: the current year plus [`ANNOUNCEMENT_HORIZON_YEARS`].
fn latest_year() -> i32 {
    time::OffsetDateTime::now_utc().year() + ANNOUNCEMENT_HORIZON_YEARS
}

static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(\d{4})\b").unwrap());
static PAGES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+)\s*(?:Seiten|S\.|pages|p\.)").unwrap());

#[derive(Debug, Default)]
struct DataField {
    tag: String,
    ind2: String,
    subfields: Vec<(String, String)>,
}

impl DataField {
    fn all<'a>(&'a self, code: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.subfields
            .iter()
            .filter(move |(c, _)| c == code)
            .map(|(_, v)| v.as_str())
    }

    fn first(&self, code: &str) -> Option<&str> {
        self.subfields
            .iter()
            .find(|(c, _)| c == code)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Default)]
struct Marc {
    control: Vec<(String, String)>,
    data: Vec<DataField>,
}

impl Marc {
    fn control(&self, tag: &str) -> Option<&str> {
        self.control
            .iter()
            .find(|(t, _)| t == tag)
            .map(|(_, v)| v.as_str())
    }

    fn fields<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a DataField> + 'a {
        self.data.iter().filter(move |f| f.tag == tag)
    }
}

/// Where the parser is inside the record.
enum Inside {
    Nothing,
    Control(String),
    Data,
    Subfield(String),
}

fn xml_error(error: impl std::fmt::Display) -> MarcError {
    MarcError::Xml(error.to_string())
}

fn attribute(e: &quick_xml::events::BytesStart<'_>, name: &str) -> Result<String, MarcError> {
    let value = e.try_get_attribute(name).map_err(xml_error)?;
    match value {
        Some(attr) => quick_xml::escape::unescape(&attr.value)
            .map(|v| v.into_owned())
            .map_err(xml_error),
        None => Ok(String::new()),
    }
}

/// Reads the control fields and the data fields of [`READ_TAGS`].
fn parse(xml: &str) -> Result<Marc, MarcError> {
    let mut reader = Reader::from_str(xml);
    let mut marc = Marc::default();
    let mut inside = Inside::Nothing;
    let mut field: Option<DataField> = None;
    let mut text = String::new();
    loop {
        match reader.read_event().map_err(xml_error)? {
            Event::Start(e) => match e.local_name().as_ref() {
                "controlfield" => {
                    inside = Inside::Control(attribute(&e, "tag")?);
                    text.clear();
                }
                "datafield" => {
                    let tag = attribute(&e, "tag")?;
                    field = READ_TAGS.contains(&tag.as_str()).then(|| DataField {
                        tag,
                        ind2: attribute(&e, "ind2").unwrap_or_default(),
                        subfields: Vec::new(),
                    });
                    inside = Inside::Data;
                }
                "subfield" if field.is_some() => {
                    inside = Inside::Subfield(attribute(&e, "code")?);
                    text.clear();
                }
                _ => {}
            },
            Event::Text(t) => {
                if matches!(inside, Inside::Control(_) | Inside::Subfield(_)) {
                    text.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(r) => {
                if matches!(inside, Inside::Control(_) | Inside::Subfield(_)) {
                    match r.resolve_char_ref().map_err(xml_error)? {
                        Some(c) => text.push(c),
                        None => text.push_str(
                            resolve_xml_entity(&r)
                                .ok_or_else(|| xml_error(format!("unknown entity &{};", &*r)))?,
                        ),
                    }
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                "controlfield" => {
                    if let Inside::Control(tag) = std::mem::replace(&mut inside, Inside::Nothing) {
                        marc.control.push((tag, std::mem::take(&mut text)));
                    }
                }
                "subfield" => {
                    if let Inside::Subfield(code) = std::mem::replace(&mut inside, Inside::Data)
                        && let Some(field) = field.as_mut()
                    {
                        field.subfields.push((code, std::mem::take(&mut text)));
                    }
                }
                "datafield" => {
                    marc.data.extend(field.take());
                    inside = Inside::Nothing;
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(marc)
}

/// Drops DNB's non-filing markers (U+0098 … U+009C around "Die", "The"), composes the
/// decomposed Unicode DNB delivers (NFC: "o" + U+0308 → "ö"), collapses whitespace and trims
/// trailing ISBD separators.
fn clean(value: &str) -> String {
    let value: String = value
        .chars()
        .filter(|c| !matches!(c, '\u{98}' | '\u{9c}'))
        .nfc()
        .collect();
    let mut value = crate::text::collapse_ws(&value);
    loop {
        let trimmed = value
            .trim_end_matches([' ', '/', ':', ';', '=', ','])
            .to_owned();
        if trimmed == value {
            return value;
        }
        value = trimmed;
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

/// 245: `$a`, then any `$n` (part number) and `$p` (part name), ISBD-joined; `$b` subtitle.
fn title(marc: &Marc) -> (Option<String>, Option<String>) {
    let Some(field) = marc.fields("245").next() else {
        return (None, None);
    };
    let mut title = String::new();
    let mut previous = "";
    for (code, value) in &field.subfields {
        let value = clean(value);
        if value.is_empty() {
            continue;
        }
        match code.as_str() {
            "a" if title.is_empty() => title = value,
            "n" | "p" if !title.is_empty() => {
                title.push_str(if code == "p" && previous == "n" {
                    ", "
                } else {
                    ". "
                });
                title.push_str(&value);
            }
            _ => continue,
        }
        previous = code;
    }
    let subtitle = field.first("b").map(clean).and_then(non_empty);
    (non_empty(title), subtitle)
}

/// "Paulitz, Udo" → ("Udo Paulitz", Some("Paulitz, Udo")); a name without a comma is kept.
fn person_name(raw: &str) -> Option<(String, Option<String>)> {
    let inverted = clean(raw);
    match inverted.split_once(',') {
        Some((surname, forename)) if !forename.trim().is_empty() => Some((
            format!("{} {}", forename.trim(), surname.trim()),
            Some(inverted.clone()),
        )),
        _ => non_empty(inverted).map(|name| (name, None)),
    }
}

/// The GND id from the `$0` values: `(DE-588)118540238` → `118540238`.
fn gnd_id(field: &DataField) -> Option<String> {
    field
        .all("0")
        .find_map(|v| v.strip_prefix("(DE-588)"))
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
}

/// Authors (`aut`; a 100 without `$4` counts as `aut`) and contributors (`trl`, `ill`, `edt`),
/// each person once per role, in record order.
fn people(marc: &Marc) -> (Vec<AuthorRecord>, Vec<(AuthorRecord, ContributorRole)>) {
    let mut authors: Vec<AuthorRecord> = Vec::new();
    let mut contributors: Vec<(AuthorRecord, ContributorRole)> = Vec::new();
    for field in marc
        .data
        .iter()
        .filter(|f| f.tag == "100" || f.tag == "700")
    {
        let Some((name, sort_name)) = field.first("a").and_then(person_name) else {
            continue;
        };
        let person = AuthorRecord {
            name,
            sort_name,
            ol_author_id: None,
            gnd_id: gnd_id(field),
        };
        let mut relators: Vec<&str> = field.all("4").map(str::trim).collect();
        if relators.is_empty() && field.tag == "100" {
            relators.push("aut");
        }
        for relator in relators {
            let role = match relator {
                "aut" => {
                    if !authors.contains(&person) {
                        authors.push(person.clone());
                    }
                    continue;
                }
                "trl" => ContributorRole::Translator,
                "ill" => ContributorRole::Illustrator,
                "edt" => ContributorRole::Editor,
                _ => continue,
            };
            if !contributors.iter().any(|(p, r)| *p == person && *r == role) {
                contributors.push((person.clone(), role));
            }
        }
    }
    (authors, contributors)
}

/// A publication year from a 260/264 `$c` ("2026", "[2026]", "September 2026", "© 2025"); a
/// year after `latest` is a placeholder.
fn year_of(value: &str, latest: i32) -> Option<i32> {
    let year: i32 = YEAR.captures(value)?[1].parse().ok()?;
    (year <= latest).then_some(year)
}

/// The publication statements (`$b` publisher, `$c` year): 264 with ind2 1 first, then other
/// 264s, then 260.
fn imprints(marc: &Marc, latest: i32) -> Vec<(Option<String>, Option<i32>)> {
    let published = marc.fields("264").filter(|f| f.ind2.trim() == "1");
    let other = marc.fields("264").filter(|f| f.ind2.trim() != "1");
    published
        .chain(other)
        .chain(marc.fields("260"))
        .map(|f| {
            (
                f.first("b").map(clean).and_then(non_empty),
                f.all("c").find_map(|c| year_of(c, latest)),
            )
        })
        .collect()
}

/// 041 `$a`, else 008/35-37.
fn language(marc: &Marc) -> Option<String> {
    let valid = |code: &str| {
        let code = code.trim().to_ascii_lowercase();
        (code.len() == 3 && code.bytes().all(|b| b.is_ascii_lowercase())).then_some(code)
    };
    marc.fields("041")
        .flat_map(|f| f.all("a"))
        .find_map(valid)
        .or_else(|| {
            marc.control("008")
                .and_then(|f| f.get(35..38))
                .and_then(valid)
        })
}

/// 300 `$a`: "352 Seiten", "XXI, 337 S.", "1 Online-Ressource (352 Seiten)".
fn pages(marc: &Marc) -> Option<i32> {
    marc.fields("300")
        .flat_map(|f| f.all("a"))
        .find_map(|v| PAGES.captures(v)?[1].parse().ok())
}

/// 263 `$a` `YYYYMM` → the first of that month; a year after `latest` is a placeholder.
fn planned_date(marc: &Marc, latest: i32) -> Option<Date> {
    let value = marc.fields("263").find_map(|f| f.first("a"))?.trim();
    let year: i32 = value.get(0..4)?.parse().ok().filter(|y| *y <= latest)?;
    let month: u8 = value.get(4..6)?.parse().ok()?;
    Date::from_calendar_date(year, Month::try_from(month).ok()?, 1).ok()
}

/// A 653 `(Produktform)` value.
fn produktform_format(value: &str) -> Format {
    let value = value.trim();
    if value == "Hardback" {
        Format::Hardcover
    } else if value.starts_with("Paperback") {
        Format::Paperback
    } else if value.starts_with("Digital") || value.to_lowercase().starts_with("e-book") {
        Format::Ebook
    } else if value.starts_with("Audio") {
        Format::Audiobook
    } else {
        Format::Other
    }
}

/// The binding of a 020 `$c` ("Gb. : EUR 22.00", "kart.", "Broschur : EUR 12.00").
fn binding_format(value: &str) -> Option<Format> {
    let binding = value.split(':').next()?.to_lowercase();
    let words: Vec<&str> = binding
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .collect();
    const HARD: &[&str] = &[
        "gb",
        "geb",
        "gebunden",
        "gewebe",
        "hldr",
        "pp",
        "hperg",
        "festeinband",
        "leinen",
        "ln",
        "lw",
        "hardcover",
    ];
    const SOFT: &[&str] = &[
        "kart",
        "kt",
        "broschur",
        "brosch",
        "broschiert",
        "klappenbroschur",
        "pb",
        "paperback",
        "taschenbuch",
    ];
    if words.iter().any(|w| HARD.contains(w)) {
        Some(Format::Hardcover)
    } else if words.iter().any(|w| SOFT.contains(w)) {
        Some(Format::Paperback)
    } else {
        None
    }
}

/// 338 carrier (`cr` online, audio carriers `s…`), else 337 media type (`c`, `s`).
fn carrier_format(marc: &Marc) -> Option<Format> {
    const AUDIO: &[&str] = &["sd", "se", "sg", "si", "sq", "ss", "st", "sw", "sz"];
    let carriers: Vec<&str> = marc.fields("338").flat_map(|f| f.all("b")).collect();
    if carriers.contains(&"cr") {
        return Some(Format::Ebook);
    }
    if carriers.iter().any(|c| AUDIO.contains(c)) {
        return Some(Format::Audiobook);
    }
    let media: Vec<&str> = marc.fields("337").flat_map(|f| f.all("b")).collect();
    if media.contains(&"s") {
        Some(Format::Audiobook)
    } else if media.contains(&"c") {
        Some(Format::Ebook)
    } else {
        None
    }
}

/// One 020 ISBN with its own binding and publisher qualifier.
struct IsbnEntry {
    isbn: Isbn13,
    binding: Option<String>,
    qualifier: Option<String>,
}

/// The record's ISBNs in order, deduplicated (ISBN-10 and ISBN-13 of one book), restricted to
/// the registration group of the first one.
fn isbns(marc: &Marc) -> Vec<IsbnEntry> {
    let mut entries: Vec<IsbnEntry> = Vec::new();
    for field in marc.fields("020") {
        let Some(isbn) = field.all("a").find_map(parse_isbn) else {
            continue;
        };
        let binding = field.first("c").map(str::to_owned);
        let qualifier = field.first("q").map(clean).and_then(non_empty);
        match entries.iter_mut().find(|e| e.isbn == isbn) {
            Some(entry) => {
                entry.binding = entry.binding.take().or(binding);
                entry.qualifier = entry.qualifier.take().or(qualifier);
            }
            None => entries.push(IsbnEntry {
                isbn,
                binding,
                qualifier,
            }),
        }
    }
    if let Some(first) = entries.first() {
        let group = registration_group(&first.isbn).to_owned();
        entries.retain(|e| registration_group(&e.isbn) == group);
    }
    entries
}

/// Maps one MARC21-xml `<record>`. Err for a record without 001 or without a usable title.
pub fn map_record(record_xml: &str) -> Result<DnbRecord, MarcError> {
    map_record_until(record_xml, latest_year())
}

/// [`map_record`] with years after `latest` treated as placeholders.
fn map_record_until(record_xml: &str, latest: i32) -> Result<DnbRecord, MarcError> {
    let marc = parse(record_xml)?;
    let idn = marc
        .control("001")
        .map(str::trim)
        .filter(|idn| !idn.is_empty())
        .ok_or(MarcError::NoIdn)?
        .to_owned();
    let (title, subtitle) = title(&marc);
    let title = title.ok_or(MarcError::NoTitle)?;
    let (authors, contributors) = people(&marc);
    let imprints = imprints(&marc, latest);
    let first_publisher = imprints.iter().find_map(|(p, _)| p.clone());
    let publication_year = imprints.iter().find_map(|(_, y)| *y);
    let language = language(&marc);
    let pages = pages(&marc);
    let planned = planned_date(&marc, latest);
    let produktform: Vec<&str> = marc
        .fields("653")
        .flat_map(|f| f.all("a"))
        .filter_map(|v| v.strip_prefix("(Produktform)"))
        .collect();
    let carrier = carrier_format(&marc);
    let isbns = isbns(&marc);

    let format_of = |binding: Option<&str>| -> Format {
        let binding = binding.and_then(binding_format);
        if isbns.len() <= 1 {
            produktform
                .first()
                .copied()
                .map(produktform_format)
                .or(binding)
                .or(carrier)
                .unwrap_or(Format::Other)
        } else {
            let single = match produktform.as_slice() {
                [only] => Some(produktform_format(only)),
                _ => None,
            };
            binding.or(single).or(carrier).unwrap_or(Format::Other)
        }
    };
    let publisher_of = |qualifier: Option<&str>| -> Option<String> {
        qualifier
            .and_then(|q| {
                let q = q.to_lowercase();
                imprints
                    .iter()
                    .filter_map(|(p, _)| p.as_ref())
                    .find(|p| p.to_lowercase().contains(&q))
                    .cloned()
            })
            .or_else(|| first_publisher.clone())
    };
    let edition = |isbn: Option<&IsbnEntry>, first: bool| EditionRecord {
        isbn13: isbn.map(|e| e.isbn.clone()),
        // editions.dnb_idn is unique: only the first ISBN's edition carries the record's IDN.
        dnb_idn: first.then(|| idn.clone()),
        // Every edition carries the delivering record's IDN, so a deleted header withdraws all.
        dnb_record_idn: Some(idn.clone()),
        title: Some(title.clone()),
        subtitle: subtitle.clone(),
        publisher: publisher_of(isbn.and_then(|e| e.qualifier.as_deref())),
        language: language.clone(),
        format: Some(format_of(isbn.and_then(|e| e.binding.as_deref()))),
        pages,
        publication_year,
        planned_publication_date: planned.map_or(Patch::Missing, Patch::Value),
        contributors: contributors.clone(),
        ..EditionRecord::new(Source::Dnb)
    };
    let editions: Vec<EditionRecord> = if isbns.is_empty() {
        vec![edition(None, true)]
    } else {
        isbns
            .iter()
            .enumerate()
            .map(|(i, e)| edition(Some(e), i == 0))
            .collect()
    };

    let author_names = authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let work = WorkRecord {
        subtitle,
        author_names,
        authors,
        first_publish_year: publication_year.or(planned.map(|d| d.year())),
        pages,
        ..WorkRecord::new(Source::Dnb, title)
    };
    Ok(DnbRecord {
        idn,
        work,
        editions,
    })
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;

    /// A minimal record around `fields` (raw MARC21-xml datafields).
    fn record(fields: &str) -> String {
        format!(
            r#"<record xmlns="http://www.loc.gov/MARC21/slim" type="Bibliographic">
                 <leader>00000nam a22000008c 4500</leader>
                 <controlfield tag="001">123456789X</controlfield>
                 <controlfield tag="008">261019s2026    gw ||||| |||| 00||||ger  </controlfield>
                 <datafield tag="245" ind1="1" ind2="0"><subfield code="a">Titel</subfield></datafield>
                 {fields}
               </record>"#
        )
    }

    fn isbn_field(isbn: &str, binding: &str) -> String {
        format!(
            r#"<datafield tag="020" ind1=" " ind2=" "><subfield code="a">{isbn}</subfield><subfield code="c">{binding}</subfield></datafield>"#
        )
    }

    /// One datafield with these subfields.
    fn datafield(tag: &str, ind2: &str, subfields: &[(&str, &str)]) -> String {
        let subfields: String = subfields
            .iter()
            .map(|(code, value)| format!(r#"<subfield code="{code}">{value}</subfield>"#))
            .collect();
        format!(r#"<datafield tag="{tag}" ind1=" " ind2="{ind2}">{subfields}</datafield>"#)
    }

    fn formats(rec: &DnbRecord) -> Vec<Option<Format>> {
        rec.editions.iter().map(|e| e.format).collect()
    }

    #[test]
    fn multi_isbn_records_use_each_binding_and_drop_foreign_groups() {
        let rec = map_record(&record(&format!(
            "{}{}{}{}",
            isbn_field("9783787306503", "kart. : DM 36.00"),
            isbn_field("9783857680540", "Gewebe : DM 48.00"),
            isbn_field("9783895614194", "Hperg. : DM 120.00"),
            isbn_field("9788368826104", "Pb."),
        )))
        .unwrap();
        let isbns: Vec<&str> = rec
            .editions
            .iter()
            .map(|e| e.isbn13.as_ref().unwrap().as_str())
            .collect();
        assert_eq!(isbns, ["9783787306503", "9783857680540", "9783895614194"]);
        assert_eq!(
            formats(&rec),
            [
                Some(Format::Paperback),
                Some(Format::Hardcover),
                Some(Format::Hardcover)
            ]
        );
        // Only the first edition carries the IDN (editions.dnb_idn is unique); every edition
        // carries the record's.
        let idns: Vec<Option<&str>> = rec.editions.iter().map(|e| e.dnb_idn.as_deref()).collect();
        assert_eq!(idns, [Some("123456789X"), None, None]);
        let record_idns: Vec<Option<&str>> = rec
            .editions
            .iter()
            .map(|e| e.dnb_record_idn.as_deref())
            .collect();
        assert_eq!(record_idns, [Some("123456789X"); 3]);
    }

    #[test]
    fn multi_isbn_records_fall_back_to_a_single_produktform_then_the_carrier() {
        let produktform = r#"<datafield tag="653" ind1=" " ind2=" "><subfield code="a">(Produktform)Hardback</subfield></datafield>"#;
        let rec = map_record(&record(&format!(
            "{}{}{produktform}",
            isbn_field("9783787306503", ": EUR 10.00"),
            isbn_field("9783857680540", "kart."),
        )))
        .unwrap();
        assert_eq!(
            formats(&rec),
            [Some(Format::Hardcover), Some(Format::Paperback)]
        );

        let online = r#"<datafield tag="338" ind1=" " ind2=" "><subfield code="b">cr</subfield></datafield>"#;
        let rec = map_record(&record(&format!(
            "{}{}{online}",
            isbn_field("9783787306503", ""),
            isbn_field("9783857680540", ""),
        )))
        .unwrap();
        assert_eq!(formats(&rec), [Some(Format::Ebook), Some(Format::Ebook)]);
    }

    #[test]
    fn single_isbn_records_prefer_the_produktform() {
        let cases = [
            ("Hardback", Format::Hardcover),
            ("Paperback / softback", Format::Paperback),
            ("Digital (delivered electronically)", Format::Ebook),
            ("E-book", Format::Ebook),
            ("Audio CD", Format::Audiobook),
            ("Calendar", Format::Other),
            ("Multiple-item retail product", Format::Other),
        ];
        for (value, expected) in cases {
            let rec = map_record(&record(&format!(
                r#"{}<datafield tag="653" ind1=" " ind2=" "><subfield code="a">(Produktform){value}</subfield></datafield>"#,
                isbn_field("9783787306503", "Gb. : EUR 20.00")
            )))
            .unwrap();
            assert_eq!(formats(&rec), [Some(expected)], "{value}");
        }
        // Without a Produktform: the binding, then the carrier, then other.
        let rec = map_record(&record(&isbn_field("9783787306503", "Broschur"))).unwrap();
        assert_eq!(formats(&rec), [Some(Format::Paperback)]);
        let audio = r#"<datafield tag="338" ind1=" " ind2=" "><subfield code="b">sd</subfield></datafield>"#;
        let rec = map_record(&record(&format!(
            "{}{audio}",
            isbn_field("9783787306503", ": EUR 20.00")
        )))
        .unwrap();
        assert_eq!(formats(&rec), [Some(Format::Audiobook)]);
        let rec = map_record(&record(&isbn_field("9783787306503", ": EUR 20.00"))).unwrap();
        assert_eq!(formats(&rec), [Some(Format::Other)]);
    }

    #[test]
    fn bindings() {
        for value in [
            "Gb. : EUR 22.00",
            "geb.",
            "Gebunden",
            "Gewebe",
            "Hldr.",
            "Pp. : DM 68.00",
            "Hperg.",
            "Festeinband",
            "Leinen",
            "Ln.",
            "Lw. : DM 40.00",
            "Hardcover",
        ] {
            assert_eq!(binding_format(value), Some(Format::Hardcover), "{value}");
        }
        for value in [
            "kart. : DM 28.00",
            "KT",
            "Broschur",
            "brosch.",
            "Broschiert",
            "Klappenbroschur : EUR 25.00 (DE)",
            "PB. : ca. EUR 14.95",
            "Paperback",
            "Taschenbuch",
        ] {
            assert_eq!(binding_format(value), Some(Format::Paperback), "{value}");
        }
        assert_eq!(binding_format(": EUR 12.00 (DE)"), None);
        // Words only, and only before the first colon.
        assert_eq!(binding_format("Gbx."), None);
        assert_eq!(binding_format("EUR 12.00 : kart."), None);
    }

    #[test]
    fn titles_join_parts_and_drop_non_filing_markers() {
        let rec = map_record(&record("").replace(
            r#"<subfield code="a">Titel</subfield>"#,
            r#"<subfield code="a">&#152;Die&#156; Tribute von Panem /</subfield><subfield code="n">3</subfield><subfield code="p">Flammender Zorn</subfield><subfield code="b">Roman :</subfield><subfield code="c">Suzanne Collins</subfield>"#,
        ))
        .unwrap();
        assert_eq!(rec.work.title, "Die Tribute von Panem. 3, Flammender Zorn");
        assert_eq!(rec.work.subtitle.as_deref(), Some("Roman"));
        // DNB sends decomposed umlauts; the mapping composes them.
        assert_eq!(clean("Scho\u{308}ffling ;"), "Schöffling");
    }

    #[test]
    fn names_come_in_natural_order() {
        assert_eq!(
            person_name("Paulitz, Udo"),
            Some(("Udo Paulitz".into(), Some("Paulitz, Udo".into())))
        );
        assert_eq!(person_name("Homer"), Some(("Homer".into(), None)));
        assert_eq!(person_name("  "), None);
    }

    #[test]
    fn dates_years_pages_and_language() {
        let rec = map_record_until(
            &record(
                r#"<datafield tag="263" ind1=" " ind2=" "><subfield code="a">202609</subfield></datafield>
               <datafield tag="264" ind1=" " ind2="1"><subfield code="b">Reclam</subfield><subfield code="c">2099</subfield></datafield>
               <datafield tag="300" ind1=" " ind2=" "><subfield code="a">1 Online-Ressource (352 Seiten)</subfield></datafield>"#,
            ),
            2031,
        )
        .unwrap();
        let edition = &rec.editions[0];
        assert_eq!(
            edition.planned_publication_date,
            Patch::Value(date!(2026 - 09 - 01))
        );
        assert_eq!(edition.publication_year, None, "2099 is a placeholder");
        assert_eq!(rec.work.first_publish_year, Some(2026));
        assert_eq!(edition.pages, Some(352));
        assert_eq!(edition.language.as_deref(), Some("ger"), "from 008");
        assert_eq!(edition.isbn13, None);
        assert_eq!(edition.dnb_idn.as_deref(), Some("123456789X"));
        assert_eq!(edition.dnb_record_idn.as_deref(), Some("123456789X"));
        assert_eq!(year_of("September 2026", 2031), Some(2026));
        assert_eq!(year_of("[2026]", 2031), Some(2026));
        assert_eq!(year_of("2031", 2031), Some(2031));
        assert_eq!(year_of("9999", 2031), None);
    }

    #[test]
    fn far_future_years_are_placeholders() {
        // translators_gnd.xml and deleted_header.xml carry 2040/204001 and 2050/205004.
        let placeholder = record(&format!(
            "{}{}",
            datafield("263", " ", &[("a", "204001")]),
            datafield("264", "1", &[("b", "Schöffling"), ("c", "2040")])
        ));
        let rec = map_record_until(&placeholder, 2031).unwrap();
        let edition = &rec.editions[0];
        assert_eq!(edition.publication_year, None);
        assert_eq!(edition.planned_publication_date, Patch::Missing);
        assert_eq!(rec.work.first_publish_year, None);
        assert_eq!(edition.publisher.as_deref(), Some("Schöffling"));
        // An announcement within the horizon is kept.
        let rec = map_record_until(&placeholder, 2040).unwrap();
        assert_eq!(rec.editions[0].publication_year, Some(2040));
        assert_eq!(
            rec.editions[0].planned_publication_date,
            Patch::Value(date!(2040 - 01 - 01))
        );
    }

    #[test]
    fn relators_decide_the_role() {
        let rec = map_record(&record(&format!(
            "{}{}{}{}",
            // Older records carry no relator on the main entry: it is the author.
            datafield(
                "100",
                " ",
                &[("a", "Kafka, Franz"), ("0", "(DE-588)118559230")]
            ),
            // An added entry without a relator says nothing about the role.
            datafield("700", " ", &[("a", "Brod, Max")]),
            datafield("700", " ", &[("a", "Kubin, Alfred"), ("4", "ill")]),
            datafield("700", " ", &[("a", "Kubin, Alfred"), ("4", "ill")]),
        )))
        .unwrap();
        assert_eq!(rec.work.author_names, "Franz Kafka");
        assert_eq!(rec.work.authors.len(), 1);
        assert_eq!(rec.work.authors[0].gnd_id.as_deref(), Some("118559230"));
        let contributors: Vec<(&str, ContributorRole)> = rec.editions[0]
            .contributors
            .iter()
            .map(|(person, role)| (person.name.as_str(), *role))
            .collect();
        assert_eq!(
            contributors,
            [("Alfred Kubin", ContributorRole::Illustrator)]
        );
    }

    #[test]
    fn imprints_prefer_the_publication_264_then_260() {
        let rec = map_record(&record(&datafield(
            "260",
            " ",
            &[("a", "Stuttgart"), ("b", "Klett :"), ("c", "1998")],
        )))
        .unwrap();
        assert_eq!(rec.editions[0].publisher.as_deref(), Some("Klett"));
        assert_eq!(rec.editions[0].publication_year, Some(1998));

        let rec = map_record(&record(&format!(
            "{}{}{}",
            datafield("264", "3", &[("b", "Druckerei"), ("c", "2019")]),
            datafield("260", " ", &[("b", "Alt"), ("c", "2018")]),
            datafield("264", "1", &[("b", "Verlag"), ("c", "2020")]),
        )))
        .unwrap();
        assert_eq!(rec.editions[0].publisher.as_deref(), Some("Verlag"));
        assert_eq!(rec.editions[0].publication_year, Some(2020));
    }

    #[test]
    fn the_041_language_wins_over_008() {
        let rec = map_record(&record(&datafield("041", " ", &[("a", "eng")]))).unwrap();
        assert_eq!(rec.editions[0].language.as_deref(), Some("eng"));
    }

    #[test]
    fn unusable_records_are_errors() {
        assert_eq!(
            map_record(
                &record("").replace(r#"<controlfield tag="001">123456789X</controlfield>"#, "")
            ),
            Err(MarcError::NoIdn)
        );
        assert_eq!(
            map_record(&record("").replace(
                r#"<subfield code="a">Titel</subfield>"#,
                r#"<subfield code="a"> / </subfield>"#
            )),
            Err(MarcError::NoTitle)
        );
        assert!(matches!(
            map_record("<record><controlfield"),
            Err(MarcError::Xml(_))
        ));
    }
}
