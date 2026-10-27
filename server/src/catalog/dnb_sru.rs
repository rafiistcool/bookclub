//! The DNB SRU client: the fallback lookup of German-language ISBNs (978-3, conventions §7.4).
//! One `searchRetrieve` per ISBN (`query=num={isbn}`, `recordSchema=MARC21-xml`) through
//! `outbound::get_bytes` and the DNB SRU throttle (1 request per second, a cooldown after a
//! 429). This module only reads the SRU envelope; every `<record>` is mapped by the one MARC
//! mapping, `catalog::shared::dnb_marc::map_record`.

use std::sync::Arc;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use url::Url;

use super::cache::Fetch;
use super::service::CatalogService;
use super::shared::dnb_marc::{DnbRecord, map_record};
use super::shared::isbn::Isbn13;
use crate::outbound::{RetryPolicy, get_bytes};

/// Records asked for per lookup. One ISBN normally has one record; an announcement and the
/// published title can both carry it.
pub const MAX_RECORDS: u32 = 5;

/// The `searchRetrieve` URL for one ISBN.
pub fn sru_url(base: &Url, isbn: &Isbn13) -> Url {
    let mut url = base.clone();
    url.query_pairs_mut()
        .append_pair("version", "1.1")
        .append_pair("operation", "searchRetrieve")
        .append_pair("query", &format!("num={isbn}"))
        .append_pair("recordSchema", "MARC21-xml")
        .append_pair("maximumRecords", &MAX_RECORDS.to_string());
    url
}

/// The MARC21-xml `<record>` elements of an SRU `searchRetrieveResponse`, as text for
/// `map_record`. A response without `numberOfRecords` (an SRU diagnostic) is an error; zero
/// records is a definite miss.
pub fn sru_records(xml: &str) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_str(xml);
    let mut records = Vec::new();
    let mut number_of_records: Option<u64> = None;
    let mut in_count = false;
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(e) => match e.local_name().as_ref() {
                "numberOfRecords" => in_count = true,
                "recordData" => {
                    // The span covers the content between `>` and `</recordData`.
                    let span = reader
                        .read_to_end(e.name())
                        .map_err(|error| error.to_string())?;
                    let inner = usize::try_from(span.start)
                        .ok()
                        .zip(usize::try_from(span.end).ok())
                        .and_then(|(start, end)| xml.get(start..end))
                        .ok_or_else(|| "recordData out of bounds".to_owned())?;
                    records.push(inner.trim().to_owned());
                }
                _ => {}
            },
            Event::Text(t) if in_count => {
                let raw = t.xml10_content();
                number_of_records = Some(
                    raw.trim()
                        .parse()
                        .map_err(|_| format!("numberOfRecords {raw:?}"))?,
                );
            }
            Event::End(e) if e.local_name().as_ref() == "numberOfRecords" => in_count = false,
            Event::Eof => break,
            _ => {}
        }
    }
    match number_of_records {
        Some(_) => Ok(records),
        None => Err("no numberOfRecords in the SRU response".to_owned()),
    }
}

/// The first mapped record with an edition of `isbn`. Records the mapping refuses are skipped.
pub fn record_for(records: &[String], isbn: &Isbn13) -> Option<DnbRecord> {
    records.iter().find_map(|xml| match map_record(xml) {
        Ok(record) => record
            .editions
            .iter()
            .any(|edition| edition.isbn13.as_ref() == Some(isbn))
            .then_some(record),
        Err(error) => {
            tracing::warn!(%error, "skipping a DNB SRU record");
            None
        }
    })
}

impl CatalogService {
    /// The DNB record of `isbn`: `Ok(Some)` a hit, `Ok(None)` a definite miss, `Err` no answer
    /// (throttled, cooling down or failing). Answers with records are cached for 18 hours,
    /// misses never.
    pub async fn dnb_record(&self, isbn: &Isbn13) -> Result<Option<DnbRecord>, Fetch> {
        let outbound = self.outbound.clone();
        let url = sru_url(&outbound.urls.dnb_sru, isbn);
        let records = self
            .sru
            .get_or_fetch(
                isbn.as_str().to_owned(),
                false,
                |records: &Arc<Vec<String>>| !records.is_empty(),
                || async move {
                    let answer =
                        get_bytes(&outbound, &outbound.dnb_sru, url, &RetryPolicy::REQUEST)
                            .await
                            .map_err(Fetch::from)?;
                    let xml = std::str::from_utf8(&answer.body)
                        .map_err(|error| Fetch::Failed(error.to_string()))?;
                    sru_records(xml).map(Arc::new).map_err(Fetch::Failed)
                },
            )
            .await?;
        Ok(record_for(&records, isbn))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::shared::isbn::parse_isbn;

    const MISS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<searchRetrieveResponse xmlns="http://www.loc.gov/zing/srw/"><version>1.1</version><numberOfRecords>0</numberOfRecords><records/></searchRetrieveResponse>"#;

    const DIAGNOSTIC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<searchRetrieveResponse xmlns="http://www.loc.gov/zing/srw/"><version>1.1</version><diagnostics><diagnostic xmlns="http://www.loc.gov/zing/srw/diagnostic/"><uri>info:srw/diagnostic/1/10</uri><message>Query syntax error</message></diagnostic></diagnostics></searchRetrieveResponse>"#;

    #[test]
    fn the_url_asks_for_marc_by_isbn() {
        let base = Url::parse("https://services.dnb.de/sru/dnb").unwrap();
        let isbn = parse_isbn("9783694050797").unwrap();
        let url = sru_url(&base, &isbn);
        assert_eq!(
            url.as_str(),
            "https://services.dnb.de/sru/dnb?version=1.1&operation=searchRetrieve\
             &query=num%3D9783694050797&recordSchema=MARC21-xml&maximumRecords=5"
        );
    }

    #[test]
    fn envelopes_give_records_misses_and_errors() {
        assert_eq!(sru_records(MISS), Ok(vec![]));
        assert!(sru_records(DIAGNOSTIC).is_err());
        assert!(sru_records("<html>").is_err());
        let hit = include_str!("../../tests/fixtures/dnb_sru/hit_9783694050797.xml");
        let records = sru_records(hit).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].starts_with("<record"));
        let isbn = parse_isbn("9783694050797").unwrap();
        let record = record_for(&records, &isbn).expect("the record has the ISBN");
        assert_eq!(record.idn, "1422369358");
        let other = parse_isbn("9780316556347").unwrap();
        assert!(record_for(&records, &other).is_none());
    }
}
