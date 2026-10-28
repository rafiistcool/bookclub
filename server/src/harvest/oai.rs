//! The OAI-PMH envelope of the DNB `ListRecords` answers: request URLs, record headers, the
//! resumption token and OAI errors. This module never reads a MARC field: the MARC21-xml
//! `<record>` inside each `<metadata>` is handed over as text, for
//! `catalog::shared::dnb_marc::map_record` (conventions §7.5).

use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use time::OffsetDateTime;
use time::macros::format_description;
use url::Url;

/// The metadata format the harvester asks for.
pub const METADATA_PREFIX: &str = "MARC21-xml";

/// Longest IDN a deleted header may name (`editions.dnb_idn` holds at most 32 characters).
const MAX_IDN_CHARS: usize = 32;

/// The first `ListRecords` request of a run: one set, every record changed since `from`
/// (the DNB's granularity is seconds, `YYYY-MM-DDThh:mm:ssZ`).
pub fn list_records_url(base: &Url, set: &str, from: OffsetDateTime) -> Url {
    let from = from
        .to_offset(time::UtcOffset::UTC)
        .format(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
        ))
        .expect("a UTC timestamp formats");
    let mut url = base.clone();
    url.query_pairs_mut()
        .append_pair("verb", "ListRecords")
        .append_pair("metadataPrefix", METADATA_PREFIX)
        .append_pair("set", set)
        .append_pair("from", &from);
    url
}

/// A follow-up request. OAI-PMH 2.0 makes `resumptionToken` an exclusive argument: the verb and
/// the token only.
pub fn resume_url(base: &Url, token: &str) -> Url {
    let mut url = base.clone();
    url.query_pairs_mut()
        .append_pair("verb", "ListRecords")
        .append_pair("resumptionToken", token);
    url
}

/// One `<record>` of a `ListRecords` page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OaiRecord {
    /// `oai:dnb.de/<setSpec>/<IDN>`.
    pub identifier: String,
    pub datestamp: String,
    /// `<header status="deleted">`. The DNB still sends the metadata of a deleted record; the
    /// harvester must not map it.
    pub deleted: bool,
    /// The content of `<metadata>`: the MARC21-xml `<record>` as text. `None` when absent.
    pub metadata: Option<String>,
}

impl OaiRecord {
    /// The IDN in the identifier (its last `/` segment), for a deleted header. `None` for an
    /// identifier the DNB never sends (empty, too long, or with characters other than
    /// letters and digits).
    pub fn idn(&self) -> Option<&str> {
        let idn = self.identifier.rsplit('/').next()?.trim();
        let ok = !idn.is_empty()
            && idn.chars().count() <= MAX_IDN_CHARS
            && idn.chars().all(|c| c.is_ascii_alphanumeric());
        ok.then_some(idn)
    }
}

/// One `ListRecords` page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub records: Vec<OaiRecord>,
    /// The token for the next page. `None` when the list is complete: the DNB sends an empty
    /// `<resumptionToken/>` on the last page of a long list and no element at all when the list
    /// fits on one page.
    pub resumption_token: Option<String>,
}

/// What a `ListRecords` request answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Page(Page),
    /// `noRecordsMatch`: nothing changed since `from`, a complete, empty list.
    NoRecordsMatch,
    /// `badResumptionToken`: the token expired or was never valid; the run restarts.
    BadResumptionToken(String),
    /// Any other OAI error (`badArgument`, `cannotDisseminateFormat`, `noSetHierarchy`, …).
    Error {
        code: String,
        message: String,
    },
}

/// Where the reader is inside the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Text {
    None,
    Identifier,
    Datestamp,
    Token,
    Error,
}

fn envelope_error(error: impl std::fmt::Display) -> String {
    format!("malformed OAI-PMH response: {error}")
}

fn attribute(e: &BytesStart<'_>, name: &str) -> Result<Option<String>, String> {
    match e.try_get_attribute(name).map_err(envelope_error)? {
        Some(attr) => quick_xml::escape::unescape(&attr.value)
            .map(|v| Some(v.into_owned()))
            .map_err(envelope_error),
        None => Ok(None),
    }
}

/// Reads one `ListRecords` answer. `Err` for anything that is not an OAI-PMH response: HTML
/// from a proxy, a truncated page, or neither `ListRecords` nor `error`.
pub fn parse_list_records(xml: &str) -> Result<Answer, String> {
    let mut reader = Reader::from_str(xml);
    let mut root_seen = false;
    let mut root_closed = false;
    let mut list_seen = false;
    let mut records = Vec::new();
    let mut current: Option<OaiRecord> = None;
    let mut token: Option<String> = None;
    let mut error: Option<(String, String)> = None;
    let mut reading = Text::None;
    let mut text = String::new();
    loop {
        let event = reader.read_event().map_err(envelope_error)?;
        match event {
            Event::Start(e) => match e.local_name().as_ref() {
                "OAI-PMH" => root_seen = true,
                "ListRecords" => list_seen = true,
                "record" if list_seen && current.is_none() => {
                    current = Some(OaiRecord {
                        identifier: String::new(),
                        datestamp: String::new(),
                        deleted: false,
                        metadata: None,
                    });
                }
                "header" => {
                    if let Some(record) = current.as_mut() {
                        record.deleted = attribute(&e, "status")?.as_deref() == Some("deleted");
                    }
                }
                "identifier" if current.is_some() => {
                    start(&mut reading, &mut text, Text::Identifier)
                }
                "datestamp" if current.is_some() => start(&mut reading, &mut text, Text::Datestamp),
                "metadata" if current.is_some() => {
                    // The span covers the content between `>` and `</metadata`.
                    let span = reader.read_to_end(e.name()).map_err(envelope_error)?;
                    let inner = usize::try_from(span.start)
                        .ok()
                        .zip(usize::try_from(span.end).ok())
                        .and_then(|(start, end)| xml.get(start..end))
                        .ok_or_else(|| envelope_error("metadata out of bounds"))?;
                    if let Some(record) = current.as_mut() {
                        let inner = inner.trim();
                        record.metadata = (!inner.is_empty()).then(|| inner.to_owned());
                    }
                }
                "resumptionToken" if list_seen => start(&mut reading, &mut text, Text::Token),
                "error" if root_seen => {
                    error = Some((attribute(&e, "code")?.unwrap_or_default(), String::new()));
                    start(&mut reading, &mut text, Text::Error);
                }
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                "header" => {
                    if let Some(record) = current.as_mut() {
                        record.deleted = attribute(&e, "status")?.as_deref() == Some("deleted");
                    }
                }
                "resumptionToken" if list_seen => token = None,
                "error" if root_seen => {
                    error = Some((attribute(&e, "code")?.unwrap_or_default(), String::new()));
                }
                _ => {}
            },
            Event::Text(t) if reading != Text::None => text.push_str(&t.xml10_content()),
            Event::CData(t) if reading != Text::None => {
                text.push_str(&t.into_inner());
            }
            Event::GeneralRef(r) if reading != Text::None => {
                match r.resolve_char_ref().map_err(envelope_error)? {
                    Some(c) => text.push(c),
                    None => text.push_str(
                        resolve_xml_entity(&r)
                            .ok_or_else(|| envelope_error(format!("unknown entity &{};", &*r)))?,
                    ),
                }
            }
            Event::End(e) => {
                let value = || text.trim().to_owned();
                match (e.local_name().as_ref(), reading) {
                    ("identifier", Text::Identifier) => {
                        if let Some(record) = current.as_mut() {
                            record.identifier = value();
                        }
                        reading = Text::None;
                    }
                    ("datestamp", Text::Datestamp) => {
                        if let Some(record) = current.as_mut() {
                            record.datestamp = value();
                        }
                        reading = Text::None;
                    }
                    ("resumptionToken", Text::Token) => {
                        let found = value();
                        token = (!found.is_empty()).then_some(found);
                        reading = Text::None;
                    }
                    ("error", Text::Error) => {
                        if let Some((_, message)) = error.as_mut() {
                            *message = value();
                        }
                        reading = Text::None;
                    }
                    ("record", _) => records.extend(current.take()),
                    ("OAI-PMH", _) => root_closed = true,
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if let Some((code, message)) = error {
        return Ok(match code.as_str() {
            "noRecordsMatch" => Answer::NoRecordsMatch,
            "badResumptionToken" => Answer::BadResumptionToken(message),
            _ => Answer::Error { code, message },
        });
    }
    if !root_seen || !list_seen {
        return Err(envelope_error("neither ListRecords nor an OAI error"));
    }
    // A page cut off between two records would otherwise pass as a complete list.
    if !root_closed || current.is_some() {
        return Err(envelope_error("the page is truncated"));
    }
    Ok(Answer::Page(Page {
        records,
        resumption_token: token,
    }))
}

fn start(reading: &mut Text, text: &mut String, what: Text) {
    *reading = what;
    text.clear();
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    const PAGE_1: &str = include_str!("../../tests/fixtures/dnb_oai/reiheN_page1.xml");
    const PAGE_2: &str = include_str!("../../tests/fixtures/dnb_oai/reiheN_page2.xml");
    const COMPLETE: &str = include_str!("../../tests/fixtures/dnb_oai/reiheA_complete.xml");
    const BAD_TOKEN: &str = include_str!("../../tests/fixtures/dnb_oai/bad_resumption_token.xml");
    const NO_RECORDS: &str = include_str!("../../tests/fixtures/dnb_oai/no_records_match.xml");

    fn page(xml: &str) -> Page {
        match parse_list_records(xml).unwrap() {
            Answer::Page(page) => page,
            other => panic!("not a page: {other:?}"),
        }
    }

    #[test]
    fn urls_follow_oai_pmh() {
        let base = Url::parse("https://services.dnb.de/oai/repository").unwrap();
        assert_eq!(
            list_records_url(
                &base,
                "dnb-all:reiheN",
                datetime!(2026-07-30 08:15:00.5 +02:00)
            )
            .as_str(),
            "https://services.dnb.de/oai/repository?verb=ListRecords&metadataPrefix=MARC21-xml\
             &set=dnb-all%3AreiheN&from=2026-07-30T06%3A15%3A00Z"
        );
        assert_eq!(
            resume_url(&base, "ab+c/=").as_str(),
            "https://services.dnb.de/oai/repository?verb=ListRecords&resumptionToken=ab%2Bc%2F%3D"
        );
    }

    #[test]
    fn a_recorded_page_has_records_and_a_token() {
        let page = page(PAGE_1);
        assert_eq!(page.records.len(), 5);
        assert_eq!(
            page.resumption_token.as_deref(),
            Some(
                "jqTteNozNLc0NjQxNjUzNzC2lPV1DHI2MtStyM2RS8lL0k3MybEqSs3MSPWTc5MzNZAzBCo2MLU0MgABOQU5\
                 WagisATIFHOwBIxnZmlqaWpqaGYhZ2pibGlgaGSqa25mLGcoZ2JoaAQAEBMb3Q"
            )
        );
        let first = &page.records[0];
        assert_eq!(first.identifier, "oai:dnb.de/dnb-all:reiheN/1289274576");
        assert_eq!(first.datestamp, "2026-10-27T12:06:14Z");
        assert!(!first.deleted);
        let marc = first.metadata.as_deref().unwrap();
        assert!(marc.starts_with("<record type=\"Bibliographic\""));
        assert!(marc.ends_with("</record>"));
        assert!(!marc.contains("<header"));
        // The MARC record maps on its own.
        let mapped = crate::catalog::shared::dnb_marc::map_record(marc).unwrap();
        assert_eq!(mapped.idn, "1289274576");

        let deleted = &page.records[1];
        assert!(deleted.deleted);
        assert_eq!(deleted.idn(), Some("1168979633"));
        // The DNB still sends the metadata of a deleted record.
        assert!(deleted.metadata.is_some());
        assert!(page.records[2..].iter().all(|r| !r.deleted));
    }

    #[test]
    fn the_end_of_a_list_has_no_token() {
        // The last page of a long list: an empty <resumptionToken/>.
        let last = page(PAGE_2);
        assert_eq!(last.records.len(), 3);
        assert_eq!(last.resumption_token, None);
        // A list that fits on one page: no element at all.
        let complete = page(COMPLETE);
        assert_eq!(complete.records.len(), 3);
        assert_eq!(complete.resumption_token, None);
        // An empty token element with text around it counts as empty too.
        let spaced = PAGE_2.replace(
            "<resumptionToken cursor=\"100\" completeListSize=\"4112\"/>",
            "<resumptionToken cursor=\"100\" completeListSize=\"4112\">  </resumptionToken>",
        );
        assert_eq!(page(&spaced).resumption_token, None);
    }

    #[test]
    fn oai_errors_are_answers() {
        assert_eq!(
            parse_list_records(BAD_TOKEN).unwrap(),
            Answer::BadResumptionToken("ResumptionToken is incomplete".into())
        );
        assert_eq!(
            parse_list_records(NO_RECORDS).unwrap(),
            Answer::NoRecordsMatch
        );
        let other = r#"<OAI-PMH xmlns="http://www.openarchives.org/OAI/2.0/"><error code="badArgument">Illegal argument &amp; more</error></OAI-PMH>"#;
        assert_eq!(
            parse_list_records(other).unwrap(),
            Answer::Error {
                code: "badArgument".into(),
                message: "Illegal argument & more".into()
            }
        );
    }

    #[test]
    fn anything_else_is_an_error() {
        assert!(parse_list_records("<html><body>Bad gateway</body></html>").is_err());
        assert!(parse_list_records("").is_err());
        assert!(parse_list_records("not xml <<<").is_err());
        let truncated = &PAGE_1[..PAGE_1.len() / 2];
        assert!(parse_list_records(truncated).is_err());
        // Cut right after a record: well-formed so far, but not the whole page.
        let after_record =
            PAGE_1.find("</metadata></record>").unwrap() + "</metadata></record>".len();
        assert!(parse_list_records(&PAGE_1[..after_record]).is_err());
    }

    #[test]
    fn idns_come_from_the_identifier() {
        let record = |identifier: &str| OaiRecord {
            identifier: identifier.into(),
            datestamp: String::new(),
            deleted: true,
            metadata: None,
        };
        assert_eq!(
            record("oai:dnb.de/dnb-all:reiheN/117392468X").idn(),
            Some("117392468X")
        );
        assert_eq!(record("1168979633").idn(), Some("1168979633"));
        assert_eq!(record("oai:dnb.de/dnb-all:reiheN/").idn(), None);
        assert_eq!(record("oai:dnb.de/x/12 34").idn(), None);
        assert_eq!(
            record(&format!("oai:dnb.de/x/{}", "1".repeat(33))).idn(),
            None
        );
    }
}
