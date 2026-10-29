//! The Open Library client (docs/phase1/spec/catalog.md §2.4–§2.16): search, trending, subjects,
//! works, authors, search by work key, ISBN search, the `/isbn/{isbn}.json` cover check and the
//! cover search by title and author (#74, docs/phase2/notes/cover.md §2.1). Every
//! call goes through `outbound::get_json` (the 3 req/s throttle, cooldown after a 429, one retry
//! on 502/503/504) and the 18-hour response cache with request coalescing.
//!
//! Payloads are read as `serde_json::Value` and coerced field by field with Python's leniency
//! (spec R13), so one odd field never drops a page.

use std::sync::Arc;

use serde_json::{Value, json};
use url::Url;

use super::cache::Fetch;
use super::covers::matching::escape;
use super::service::CatalogService;
use super::shared::ingest::{
    AuthorRecord, EditionRecord, Source, WorkRecord, clean_description, clean_line, clean_subjects,
};
use super::shared::isbn::{Isbn13, parse_isbn};
use super::shared::lookup::{CoverAnswer, CoverDoc, CoverEdition, CoverQuery};
use crate::outbound::{RetryPolicy, UpstreamError, get_json};
use crate::text::{py_strip, round_half_even};

/// `fields` of a search or trending page. `isbn` is left out on purpose: Open Library would
/// send every edition's ISBN (about ten times the payload).
pub const SEARCH_FIELDS: &str =
    "key,title,author_name,cover_i,first_publish_year,cover_edition_key";
/// `fields` of the search-by-key call behind a work import.
pub const WORK_SEARCH_FIELDS: &str = "key,title,author_name,cover_i,first_publish_year,\
     number_of_pages_median,ratings_average,ratings_count,subject";
/// `fields` of an ISBN search.
pub const ISBN_SEARCH_FIELDS: &str =
    "key,title,author_name,cover_i,first_publish_year,number_of_pages_median";
/// `fields` and `limit` of a title-and-author lookup (`CatalogLookup::resolve_text`).
pub const TEXT_SEARCH_FIELDS: &str = "key,title,author_name,cover_i,first_publish_year";
pub const TEXT_SEARCH_LIMIT: u32 = 5;
/// At most this many author records are fetched for one work.
pub const MAX_DETAIL_AUTHORS: usize = 3;
/// `fields` of the cover search (#74): both subtitles, the edition count and the languages, and
/// the best edition for the query and `lang`.
pub const COVER_SEARCH_FIELDS: &str = "key,title,subtitle,author_name,cover_i,edition_count,\
     first_publish_year,language,editions,editions.key,editions.title,editions.subtitle,\
     editions.cover_i,editions.language";
/// Works per cover search: enough to see Open Library's duplicate records of one book.
pub const COVER_SEARCH_LIMIT: u32 = 10;
/// The marker of a refused cover search inside the response cache (never cached itself).
const REFUSED: &str = "__bookclub_refused";

/// Search sort orders (`sort` query parameter); `relevance` is Open Library's default and is
/// not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Readinglog,
    New,
    Title,
    Relevance,
}

impl Sort {
    pub fn parse(value: &str) -> Option<Sort> {
        match value {
            "readinglog" => Some(Sort::Readinglog),
            "new" => Some(Sort::New),
            "title" => Some(Sort::Title),
            "relevance" => Some(Sort::Relevance),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Readinglog => "readinglog",
            Sort::New => "new",
            Sort::Title => "title",
            Sort::Relevance => "relevance",
        }
    }
}

/// One work from a search, trending or subject payload.
#[derive(Debug, Clone, PartialEq)]
pub struct OlDoc {
    /// `OL…W`.
    pub ol_work_id: String,
    pub title: String,
    /// Names joined with `", "`; may be empty.
    pub authors: String,
    /// A positive cover id.
    pub cover_id: Option<i64>,
    pub year: Option<i32>,
    /// The first valid ISBN of the doc's `isbn` field (only an ISBN search asks for it).
    pub isbn: Option<Isbn13>,
    /// `number_of_pages_median`, when the call asked for it.
    pub pages: Option<i32>,
}

impl OlDoc {
    /// The stub record a hit is persisted with (insert-only).
    pub fn work_record(&self) -> WorkRecord {
        WorkRecord {
            ol_work_id: Some(self.ol_work_id.clone()),
            author_names: self.authors.clone(),
            first_publish_year: self.year,
            pages: self.pages,
            ol_cover_id: self.cover_id,
            ..WorkRecord::new(Source::OpenLibrary, self.title.clone())
        }
    }
}

/// A search page: the mapped docs and Open Library's total.
#[derive(Debug, Clone, PartialEq)]
pub struct OlPage {
    pub docs: Vec<OlDoc>,
    pub num_found: i64,
}

/// Python `int(value)`: integers, floats truncated toward zero, numeric strings (surrounding
/// whitespace allowed) and booleans; anything else is `None`.
pub fn py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite() && f.abs() < 9.2e18)
                .map(|f| f.trunc() as i64)
        }),
        Value::Bool(b) => Some(i64::from(*b)),
        Value::String(s) => py_strip(s).parse::<i64>().ok(),
        _ => None,
    }
}

/// Python `float(value)` for numbers and numeric strings.
fn py_float(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::String(s) => py_strip(s).parse::<f64>().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

fn py_i32(value: &Value) -> Option<i32> {
    py_int(value).and_then(|n| i32::try_from(n).ok())
}

/// A usable Open Library cover id: only values above 0 (`-1` means "no cover").
pub fn positive_cover_id(value: &Value) -> Option<i64> {
    py_int(value).filter(|&id| id > 0)
}

/// The first positive cover id among `values`, walking lists item by item.
pub fn first_positive_cover(values: &[&Value]) -> Option<i64> {
    values.iter().find_map(|value| match value {
        Value::Array(items) => items.iter().find_map(positive_cover_id),
        other => positive_cover_id(other),
    })
}

/// `round(float(x), 2)` for a rating.
fn rating(value: &Value) -> Option<f64> {
    py_float(value).map(|r| round_half_even(r, 2))
}

/// The stripped string of a JSON string or number (Python `str(value or "").strip()`).
fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => py_strip(s).to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// The `OL…W` id of a `/works/OL…W` key; `None` for editions, authors and malformed ids (they
/// could not be stored as Open Library works).
fn work_id_of(key: &str) -> Option<String> {
    let id = key.strip_prefix("/works/")?;
    let digits = id.strip_prefix("OL")?.strip_suffix('W')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_owned())
}

/// Names joined with `", "`. A string is one name (Python iterated its characters; spec R13).
fn joined_names(value: Option<&Value>) -> String {
    let names: Vec<String> = match value {
        Some(Value::Array(items)) => items.iter().map(|item| text(Some(item))).collect(),
        Some(Value::String(name)) => vec![py_strip(name).to_owned()],
        _ => Vec::new(),
    };
    names
        .into_iter()
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The first valid ISBN of a doc's `isbn` field (a string or a list).
fn doc_isbn(value: Option<&Value>) -> Option<Isbn13> {
    match value {
        Some(Value::String(s)) => parse_isbn(s),
        Some(Value::Array(items)) => items
            .iter()
            .find_map(|item| item.as_str().and_then(parse_isbn)),
        _ => None,
    }
}

/// Maps the docs of `search.json` and the works of `trending/*.json` (one shape). Skips
/// non-objects, keys that are not `/works/OL…W`, keys already seen (the first wins) and
/// untitled docs.
pub fn map_search_docs(docs: &Value) -> Vec<OlDoc> {
    map_works(docs, |doc| OlDoc {
        ol_work_id: String::new(),
        title: String::new(),
        authors: joined_names(doc.get("author_name")),
        cover_id: doc.get("cover_i").and_then(positive_cover_id),
        year: doc.get("first_publish_year").and_then(py_i32),
        isbn: doc_isbn(doc.get("isbn")),
        pages: doc
            .get("number_of_pages_median")
            .and_then(py_i32)
            .filter(|&p| p > 0),
    })
}

/// Maps the works of `subjects/*.json`, which use `cover_id` and `authors[].name`.
pub fn map_subject_works(works: &Value) -> Vec<OlDoc> {
    map_works(works, |work| {
        let names = match work.get("authors") {
            Some(Value::Array(entries)) => entries
                .iter()
                .map(|entry| text(entry.get("name")))
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
            _ => String::new(),
        };
        OlDoc {
            ol_work_id: String::new(),
            title: String::new(),
            authors: names,
            cover_id: work.get("cover_id").and_then(positive_cover_id),
            year: work.get("first_publish_year").and_then(py_i32),
            isbn: None,
            pages: None,
        }
    })
}

fn map_works(items: &Value, map: impl Fn(&Value) -> OlDoc) -> Vec<OlDoc> {
    let Value::Array(items) = items else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in items.iter().filter(|item| item.is_object()) {
        let Some(id) = work_id_of(&text(item.get("key"))) else {
            continue;
        };
        let title = text(item.get("title"));
        if title.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        out.push(OlDoc {
            ol_work_id: id,
            title,
            ..map(item)
        });
    }
    out
}

/// A cleaned single-line value of a payload (NFC, no control characters, whitespace collapsed,
/// cut to `max` characters); `""` when nothing is left.
fn clean_text(value: Option<&Value>, max: usize) -> String {
    clean_line(Some(&text(value)), max).unwrap_or_default()
}

/// The MARC language codes of a payload's `language` list (three lowercase letters only).
fn languages(value: Option<&Value>) -> Vec<String> {
    let codes: Vec<&Value> = match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(single @ Value::String(_)) => vec![single],
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for code in codes.into_iter().filter_map(Value::as_str) {
        let code = code.trim().to_ascii_lowercase();
        if code.len() == 3 && code.bytes().all(|b| b.is_ascii_lowercase()) && !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

/// The `OL…M` id of a `/books/OL…M` key.
fn edition_id_of(key: &str) -> Option<String> {
    let id = key.strip_prefix("/books/")?;
    let digits = id.strip_prefix("OL")?.strip_suffix('M')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_owned())
}

/// The best edition of a cover search doc (`editions.docs[0]`), if it is a usable record.
fn cover_edition(editions: Option<&Value>) -> Option<CoverEdition> {
    let first = editions?.get("docs")?.as_array()?.first()?;
    if !first.is_object() {
        return None;
    }
    Some(CoverEdition {
        ol_edition_id: edition_id_of(&text(first.get("key")))?,
        title: clean_text(first.get("title"), 1000),
        subtitle: clean_text(first.get("subtitle"), 1000),
        cover_id: first.get("cover_i").and_then(positive_cover_id),
        languages: languages(first.get("language")),
    })
}

/// Maps the docs of a cover search (notes §2.1), leniently like every payload: non-objects,
/// keys that are not `/works/OL…W`, docs whose cleaned title is empty and duplicates are
/// skipped; cover ids must be positive; a missing `edition_count` stays unknown; a year outside
/// −5000..3000 is dropped.
pub fn map_cover_docs(docs: &Value) -> Vec<CoverDoc> {
    let Value::Array(items) = docs else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in items.iter().filter(|item| item.is_object()) {
        let Some(id) = work_id_of(&text(item.get("key"))) else {
            continue;
        };
        let title = clean_text(item.get("title"), 1000);
        if title.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let authors = match item.get("author_name") {
            Some(Value::Array(names)) => names
                .iter()
                .map(|name| clean_text(Some(name), 300))
                .collect(),
            Some(name @ Value::String(_)) => vec![clean_text(Some(name), 300)],
            _ => Vec::new(),
        }
        .into_iter()
        .filter(|name: &String| !name.is_empty())
        .collect();
        out.push(CoverDoc {
            ol_work_id: id,
            title,
            subtitle: clean_text(item.get("subtitle"), 1000),
            authors,
            cover_id: item.get("cover_i").and_then(positive_cover_id),
            edition_count: item
                .get("edition_count")
                .and_then(py_i32)
                .filter(|n| *n >= 0),
            first_publish_year: item
                .get("first_publish_year")
                .and_then(py_i32)
                .filter(|year| (-5000..=3000).contains(year)),
            languages: languages(item.get("language")),
            edition: cover_edition(item.get("editions")),
        });
    }
    out
}

/// `num_found` (or `numFound`) of a search payload; non-numeric → 0.
pub fn num_found(payload: &Value) -> i64 {
    let raw = match payload.get("num_found") {
        Some(Value::Null) | None => payload.get("numFound"),
        found => found,
    };
    raw.and_then(py_int).unwrap_or(0)
}

/// True when a payload's result list (`key`) is a non-empty array. Empty and missing lists are
/// never cached: a lucky-empty answer must not freeze for 18 hours.
pub fn has_items(payload: &Value, key: &str) -> bool {
    payload
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
}

/// The description of a work payload: a string, or `{"type", "value"}`.
fn work_description(work: &Value) -> String {
    match work.get("description") {
        Some(Value::Object(object)) => text(object.get("value")),
        other => text(other),
    }
}

/// The raw subject strings of a payload's `key` list.
fn subject_list(payload: &Value, key: &str) -> Vec<String> {
    match payload.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// The `OL…A` ids of a work's authors (`authors[].author.key` or `authors[].key`), deduplicated,
/// at most [`MAX_DETAIL_AUTHORS`].
fn author_ids(work: &Value) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let Some(Value::Array(entries)) = work.get("authors") else {
        return ids;
    };
    for entry in entries {
        let key = entry
            .get("author")
            .and_then(|author| author.get("key"))
            .or_else(|| entry.get("key"))
            .and_then(Value::as_str);
        let Some(id) = key.map(|key| key.rsplit('/').next().unwrap_or_default()) else {
            continue;
        };
        let valid = id
            .strip_prefix("OL")
            .and_then(|rest| rest.strip_suffix('A'))
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
        if valid && !ids.iter().any(|known| known == id) {
            ids.push(id.to_owned());
            if ids.len() == MAX_DETAIL_AUTHORS {
                break;
            }
        }
    }
    ids
}

/// Accepts only JSON objects (Open Library answers objects; anything else is a broken answer).
fn object(value: Value) -> Result<Arc<Value>, Fetch> {
    if value.is_object() {
        Ok(Arc::new(value))
    } else {
        Err(Fetch::Failed("not a JSON object".into()))
    }
}

/// `" ".join(s.casefold().split())`: identical searches share a cache key.
pub fn normalize_search_query(query: &str) -> String {
    crate::text::collapse_ws(&crate::text::casefold(query))
}

impl CatalogService {
    fn ol_url(&self, path: &str) -> Url {
        self.outbound
            .urls
            .openlibrary
            .join(path)
            .expect("an Open Library path joins its base URL")
    }

    /// One GET of Open Library JSON through the cache. `cacheable` decides whether a good
    /// answer is kept.
    async fn ol_json(
        &self,
        cache_key: String,
        url: Url,
        bypass: bool,
        cacheable: impl Fn(&Arc<Value>) -> bool,
    ) -> Result<Arc<Value>, Fetch> {
        let outbound = self.outbound.clone();
        self.json
            .get_or_fetch(cache_key, bypass, cacheable, || async move {
                let value = get_json(&outbound, &outbound.ol_api, url, &RetryPolicy::REQUEST)
                    .await
                    .map_err(Fetch::from)?;
                object(value)
            })
            .await
    }

    /// One `search.json` page (spec §2.4): `q`, `page`, `limit`, [`SEARCH_FIELDS`], `lang=en`
    /// when `lang_en`, `sort` unless relevance. Empty pages are not cached.
    pub async fn ol_search_page(
        &self,
        q: &str,
        sort: Sort,
        page: i32,
        limit: i32,
        lang_en: bool,
    ) -> Result<OlPage, Fetch> {
        let mut url = self.ol_url("search.json");
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("q", q)
                .append_pair("page", &page.to_string())
                .append_pair("limit", &limit.to_string())
                .append_pair("fields", SEARCH_FIELDS);
            if lang_en {
                query.append_pair("lang", "en");
            }
            if sort != Sort::Relevance {
                query.append_pair("sort", sort.as_str());
            }
        }
        let key = format!(
            "ol|search|{}|{}|{page}|{limit}",
            normalize_search_query(q),
            sort.as_str()
        );
        let payload = self
            .ol_json(key, url, false, |payload| has_items(payload, "docs"))
            .await?;
        Ok(OlPage {
            docs: payload.get("docs").map(map_search_docs).unwrap_or_default(),
            num_found: num_found(&payload),
        })
    }

    /// `trending/daily.json?limit=` (spec §1.2). Empty lists are not cached.
    pub async fn ol_trending(&self, limit: i32) -> Result<Vec<OlDoc>, Fetch> {
        let mut url = self.ol_url("trending/daily.json");
        url.query_pairs_mut()
            .append_pair("limit", &limit.to_string());
        let payload = self
            .ol_json(format!("trending|{limit}"), url, false, |payload| {
                has_items(payload, "works")
            })
            .await?;
        Ok(payload
            .get("works")
            .map(map_search_docs)
            .unwrap_or_default())
    }

    /// `subjects/{key}.json?limit=&offset=&details=false` (spec §1.3): the works and
    /// `work_count`. `key` is already normalised (`[a-z0-9_]+`). Empty lists are not cached.
    pub async fn ol_subject(
        &self,
        key: &str,
        page: i32,
        limit: i32,
        offset: i64,
    ) -> Result<(Vec<OlDoc>, i64), Fetch> {
        let mut url = self.ol_url(&format!("subjects/{key}.json"));
        url.query_pairs_mut()
            .append_pair("limit", &limit.to_string())
            .append_pair("offset", &offset.to_string())
            .append_pair("details", "false");
        let payload = self
            .ol_json(
                format!("subject|{key}|{page}|{limit}"),
                url,
                false,
                |payload| has_items(payload, "works"),
            )
            .await?;
        let works = payload
            .get("works")
            .map(map_subject_works)
            .unwrap_or_default();
        let work_count = payload.get("work_count").and_then(py_int).unwrap_or(0);
        Ok((works, work_count))
    }

    /// `search.json?q=isbn:{isbn}` (spec §2.10 C): the work of the first doc, or `None` when
    /// Open Library has no such ISBN. Misses are not cached.
    pub async fn ol_isbn_search(&self, isbn: &Isbn13) -> Result<Option<OlDoc>, Fetch> {
        let mut url = self.ol_url("search.json");
        url.query_pairs_mut()
            .append_pair("q", &format!("isbn:{isbn}"))
            .append_pair("fields", ISBN_SEARCH_FIELDS)
            .append_pair("limit", "1");
        let payload = self
            .ol_json(format!("isbn|{isbn}"), url, false, |payload| {
                has_items(payload, "docs")
            })
            .await?;
        let docs = payload.get("docs").map(map_search_docs).unwrap_or_default();
        Ok(docs.into_iter().next())
    }

    /// `search.json?q={title} {authors}&limit=5` (`CatalogLookup::resolve_text`): the first
    /// work, or `None`. Misses are not cached.
    pub async fn ol_text_search(&self, q: &str) -> Result<Option<OlDoc>, Fetch> {
        let mut url = self.ol_url("search.json");
        url.query_pairs_mut()
            .append_pair("q", q)
            .append_pair("limit", &TEXT_SEARCH_LIMIT.to_string())
            .append_pair("fields", TEXT_SEARCH_FIELDS);
        let key = format!("text|{}", normalize_search_query(q));
        let payload = self
            .ol_json(key, url, false, |payload| has_items(payload, "docs"))
            .await?;
        let docs = payload.get("docs").map(map_search_docs).unwrap_or_default();
        Ok(docs.into_iter().next())
    }

    /// The cover search (#74, notes §2.1): `search.json?q=title:(…) author:(…)&limit=10` with
    /// [`COVER_SEARCH_FIELDS`] and `lang` when the book's language is known. Both values are
    /// escaped ([`escape`]). A 200 without a `docs` array (or with an `error` key) is a failure;
    /// any other 4xx than 429 is an answer without docs ([`CoverAnswer::refused`]). Empty and
    /// refused answers are not cached.
    pub async fn ol_cover_search(&self, query: &CoverQuery) -> Result<CoverAnswer, Fetch> {
        let title = escape(&query.title);
        let author = query
            .author
            .as_deref()
            .map(escape)
            .filter(|author| !author.is_empty());
        let mut q = format!("title:({title})");
        if let Some(author) = &author {
            q.push_str(&format!(" author:({author})"));
        }
        let mut url = self.ol_url("search.json");
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("q", &q)
                .append_pair("limit", &COVER_SEARCH_LIMIT.to_string())
                .append_pair("fields", COVER_SEARCH_FIELDS);
            if let Some(language) = &query.language {
                pairs.append_pair("lang", language);
            }
        }
        let key = format!(
            "cover|{}|{}|{}",
            normalize_search_query(&title),
            normalize_search_query(author.as_deref().unwrap_or_default()),
            query.language.as_deref().unwrap_or_default()
        );
        let outbound = self.outbound.clone();
        let payload = self
            .json
            .get_or_fetch(
                key,
                false,
                |payload| has_items(payload, "docs"),
                || async move {
                    match get_json(&outbound, &outbound.ol_api, url, &RetryPolicy::REQUEST).await {
                        Ok(value) => {
                            let has_docs = value.get("docs").is_some_and(Value::is_array);
                            if !has_docs || value.get("error").is_some() {
                                return Err(Fetch::Failed("a search answer without docs".into()));
                            }
                            object(value)
                        }
                        // A query Open Library refuses: no match, not an outage (429 is
                        // `Throttled` already).
                        Err(UpstreamError::Status(status))
                            if (400..500).contains(&status) && status != 429 =>
                        {
                            Ok(Arc::new(json!({ REFUSED: status })))
                        }
                        Err(error) => Err(Fetch::from(error)),
                    }
                },
            )
            .await?;
        if let Some(status) = payload.get(REFUSED).and_then(py_int) {
            return Ok(CoverAnswer {
                refused: u16::try_from(status).ok(),
                ..CoverAnswer::default()
            });
        }
        Ok(CoverAnswer {
            num_found: num_found(&payload),
            docs: payload.get("docs").map(map_cover_docs).unwrap_or_default(),
            refused: None,
        })
    }

    /// `/isbn/{isbn}.json` (Open Library redirects it to the edition): the first positive
    /// cover id. `Ok(None)` when Open Library has no such edition (404) or it has no cover.
    /// The one cover check after a DNB hit (conventions §7.4).
    pub async fn ol_isbn_cover(&self, isbn: &Isbn13) -> Result<Option<i64>, Fetch> {
        let url = self.ol_url(&format!("isbn/{isbn}.json"));
        match self
            .ol_json(format!("isbn-edition|{isbn}"), url, false, |_| true)
            .await
        {
            Ok(edition) => Ok(edition
                .get("covers")
                .and_then(|covers| first_positive_cover(&[covers]))),
            Err(Fetch::Missing) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// `/works/{id}.json`. A refresh (`bypass`) skips the cached answer.
    pub async fn ol_work(&self, ol_work_id: &str, bypass: bool) -> Result<Arc<Value>, Fetch> {
        let url = self.ol_url(&format!("works/{ol_work_id}.json"));
        self.ol_json(format!("work|{ol_work_id}"), url, bypass, |_| true)
            .await
    }

    /// The search doc of one work (`q=key:/works/{id}`, [`WORK_SEARCH_FIELDS`]); best effort:
    /// any failure is an empty object.
    pub async fn ol_work_search_doc(&self, ol_work_id: &str, bypass: bool) -> Value {
        let mut url = self.ol_url("search.json");
        url.query_pairs_mut()
            .append_pair("q", &format!("key:/works/{ol_work_id}"))
            .append_pair("fields", WORK_SEARCH_FIELDS)
            .append_pair("limit", "1");
        let key = format!("work-search|{ol_work_id}");
        match self.ol_json(key, url, bypass, |_| true).await {
            Ok(payload) => match payload.get("docs").and_then(|docs| docs.get(0)) {
                Some(doc) if doc.is_object() => doc.clone(),
                _ => Value::Object(Default::default()),
            },
            Err(_) => Value::Object(Default::default()),
        }
    }

    /// One author's name (`/authors/{id}.json`); any failure is `""`.
    pub async fn ol_author_name(&self, ol_author_id: &str, bypass: bool) -> String {
        let url = self.ol_url(&format!("authors/{ol_author_id}.json"));
        match self
            .ol_json(format!("author|{ol_author_id}"), url, bypass, |_| true)
            .await
        {
            Ok(author) => text(author.get("name")),
            Err(_) => String::new(),
        }
    }

    /// The full record of an Open Library work, for its first detail view and for a refresh
    /// (`refresh` bypasses the caches). One normaliser for the two legacy import paths
    /// (spec §5.7):
    /// - the work JSON and its search doc are fetched together; the work JSON must answer
    ///   (404 → `Fetch::Missing`), the search doc is best effort;
    /// - title: the work's, else the doc's, else the id;
    /// - authors: the doc's names first and the author API (at most 3) only when it has none;
    ///   on a refresh the author API first;
    /// - cover: the work's covers first, then the doc's `cover_i`;
    /// - year, pages, rating and rating count from the doc; subjects from the work, else the
    ///   doc; description from the work. Cleaning is ingest's (§7.3); a value that cleans to
    ///   nothing is absent, so a refresh never wipes a stored one.
    pub async fn ol_work_record(
        &self,
        ol_work_id: &str,
        refresh: bool,
    ) -> Result<WorkRecord, Fetch> {
        let (work, doc) = tokio::join!(
            self.ol_work(ol_work_id, refresh),
            self.ol_work_search_doc(ol_work_id, refresh)
        );
        let work = work?;
        let mut title = text(work.get("title"));
        if title.is_empty() {
            title = text(doc.get("title"));
        }
        if title.is_empty() {
            title = ol_work_id.to_owned();
        }

        let search_names = joined_names(doc.get("author_name"));
        let mut authors = Vec::new();
        if refresh || search_names.is_empty() {
            let ids = author_ids(&work);
            let names =
                futures::future::join_all(ids.iter().map(|id| self.ol_author_name(id, refresh)))
                    .await;
            for (id, name) in ids.iter().zip(names) {
                if !name.is_empty() {
                    authors.push(AuthorRecord {
                        ol_author_id: Some(id.clone()),
                        ..AuthorRecord::named(name)
                    });
                }
            }
        }
        // Author records travel with the names they produced; the doc's names have none.
        let author_names = if authors.is_empty() {
            search_names
        } else {
            authors
                .iter()
                .map(|author| author.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };

        let null = Value::Null;
        let cover = first_positive_cover(&[
            work.get("covers").unwrap_or(&null),
            doc.get("cover_i").unwrap_or(&null),
        ]);
        let mut subjects = clean_subjects(&subject_list(&work, "subjects"));
        if subjects.is_empty() {
            subjects = clean_subjects(&subject_list(&doc, "subject"));
        }
        let description = clean_description(&work_description(&work));

        Ok(WorkRecord {
            ol_work_id: Some(ol_work_id.to_owned()),
            author_names,
            authors,
            first_publish_year: doc.get("first_publish_year").and_then(py_i32),
            description: (!description.is_empty()).then_some(description),
            subjects: (!subjects.is_empty()).then_some(subjects),
            pages: doc
                .get("number_of_pages_median")
                .and_then(py_i32)
                .filter(|&p| p > 0),
            ol_rating: doc.get("ratings_average").and_then(rating),
            ol_rating_count: doc.get("ratings_count").and_then(py_i32),
            ol_cover_id: cover,
            full_details: true,
            ..WorkRecord::new(Source::OpenLibrary, title)
        })
    }
}

/// The edition record of an Open Library ISBN hit: the ISBN only (the search doc describes the
/// work).
pub fn isbn_edition(isbn: &Isbn13) -> EditionRecord {
    EditionRecord {
        isbn13: Some(isbn.clone()),
        ..EditionRecord::new(Source::OpenLibrary)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// parity: test_book_details.py::test_positive_cover_id_rejects_placeholders
    #[test]
    fn cover_ids_must_be_positive() {
        assert_eq!(positive_cover_id(&json!(-1)), None);
        assert_eq!(positive_cover_id(&json!(0)), None);
        assert_eq!(positive_cover_id(&Value::Null), None);
        assert_eq!(positive_cover_id(&json!(12.9)), Some(12));
        assert_eq!(positive_cover_id(&json!("99")), Some(99));
        assert_eq!(
            first_positive_cover(&[&json!(-1), &json!([0, -1, 99]), &json!(12)]),
            Some(99)
        );
        assert_eq!(
            parse_isbn("978-0-316-76948-8").map(|i| i.to_string()),
            Some("9780316769488".to_owned())
        );
        assert_eq!(parse_isbn("it"), None);
    }

    #[test]
    fn python_int_leniency() {
        assert_eq!(py_int(&json!(1999)), Some(1999));
        assert_eq!(py_int(&json!(1999.7)), Some(1999));
        assert_eq!(py_int(&json!(-3.9)), Some(-3));
        assert_eq!(py_int(&json!(" 1999 ")), Some(1999));
        assert_eq!(py_int(&json!("4.5")), None);
        assert_eq!(py_int(&json!(true)), Some(1));
        assert_eq!(py_int(&json!({"a": 1})), None);
        assert_eq!(num_found(&json!({"numFound": 50})), 50);
        assert_eq!(num_found(&json!({"num_found": "7", "numFound": 50})), 7);
        assert_eq!(num_found(&json!({"num_found": "x"})), 0);
    }

    /// parity: test_google_books.py::test_map_volumes_skips_junk_and_duplicates (ADAPT: the
    /// Open Library mappers skip records without an id or a title and dedupe one page by work)
    #[test]
    fn search_docs_skip_junk_and_duplicates() {
        let docs = json!([
            {"key": "/works/OL1W", "title": " Circe ", "author_name": ["Madeline Miller", ""],
             "cover_i": 123, "first_publish_year": 2018},
            {"key": "/books/OL9M", "title": "An edition, skip"},
            {"key": "/works/OL1W", "title": "Circe duplicate work"},
            {"key": "/works/foo", "title": "Malformed id"},
            {"key": "/works/OL2W", "title": "  "},
            "junk",
            {"key": "/works/OL3W", "title": "Ann", "author_name": "Ann Leckie", "cover_i": -1,
             "isbn": ["123", "0-316-55634-3"]},
        ]);
        let hits = map_search_docs(&docs);
        assert_eq!(
            hits.iter()
                .map(|h| h.ol_work_id.as_str())
                .collect::<Vec<_>>(),
            ["OL1W", "OL3W"]
        );
        assert_eq!(hits[0].title, "Circe");
        assert_eq!(hits[0].authors, "Madeline Miller");
        assert_eq!(hits[0].cover_id, Some(123));
        assert_eq!(hits[0].year, Some(2018));
        // A string author_name is one name, not its characters (spec R13, an intentional fix).
        assert_eq!(hits[1].authors, "Ann Leckie");
        assert_eq!(hits[1].cover_id, None);
        assert_eq!(
            hits[1].isbn.as_ref().map(Isbn13::as_str),
            Some("9780316556347")
        );
    }

    #[test]
    fn subject_works_use_their_own_shape() {
        let works = json!([
            {"key": "/works/OL138052W", "title": "Alice's Adventures in Wonderland",
             "authors": [{"key": "/authors/OL22098A", "name": "Lewis Carroll"}],
             "cover_id": 10527843, "first_publish_year": 1865, "isbn": ["0316556343"]},
            {"key": "/authors/OL1A", "title": "Not a work, skip"},
        ]);
        let hits = map_subject_works(&works);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].authors, "Lewis Carroll");
        assert_eq!(hits[0].cover_id, Some(10527843));
        assert_eq!(hits[0].year, Some(1865));
        assert_eq!(hits[0].isbn, None);
    }

    #[test]
    fn author_ids_are_valid_unique_and_at_most_three() {
        let work = json!({"authors": [
            {"author": {"key": "/authors/OL2A"}},
            {"key": "/authors/OL2A"},
            {"author": {"key": "/authors/bad"}},
            {"author": {"key": "/authors/OL3A"}},
            {"author": {"key": "/authors/OL4A"}},
            {"author": {"key": "/authors/OL5A"}},
        ]});
        assert_eq!(author_ids(&work), ["OL2A", "OL3A", "OL4A"]);
    }

    #[test]
    fn empty_and_missing_lists_are_not_cacheable() {
        assert!(!has_items(&json!({"numFound": 0, "docs": []}), "docs"));
        assert!(!has_items(&json!({"totalItems": 0}), "docs"));
        assert!(has_items(&json!({"docs": [{}]}), "docs"));
    }

    #[test]
    fn cache_keys_ignore_case_and_spacing() {
        assert_eq!(normalize_search_query("  CIRCE  "), "circe");
        assert_eq!(normalize_search_query("Straße"), "strasse");
    }
}
