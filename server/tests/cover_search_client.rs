//! The Open Library side of the cover search (#74, docs/phase2/notes/cover.md §2.1, §4.6):
//! `CatalogLookup::cover_candidates` and `cover_by_isbn` of `CatalogService` against the mock
//! upstream. The request (`q`, `fields`, `limit`, `lang`, the User-Agent), the mapping of the
//! recorded Goldfinch answer and of junk, the cache, and what counts as which answer.

mod common;

use std::path::Path;

use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::catalog::shared::lookup::{
    CatalogLookup, CoverAnswer, CoverEdition, CoverQuery, LookupError,
};
use bookclub_server::outbound::user_agent;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

const FIELDS: &str = "key,title,subtitle,author_name,cover_i,edition_count,first_publish_year,\
     language,editions,editions.key,editions.title,editions.subtitle,editions.cover_i,\
     editions.language";

fn goldfinch() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/openlibrary/cover_search_goldfinch.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn distelfink() -> CoverQuery {
    CoverQuery {
        title: "Der Distelfink".into(),
        author: Some("Tartt".into()),
        language: Some("de".into()),
    }
}

async fn mount_search(app: &TestApp, status: u16, body: Value) {
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&app.upstream)
        .await;
}

async fn searches(app: &TestApp) -> Vec<wiremock::Request> {
    app.upstream_requests()
        .await
        .into_iter()
        .filter(|r| r.url.path() == "/search.json")
        .collect()
}

fn param(request: &wiremock::Request, name: &str) -> Option<String> {
    request
        .url
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// The request is the fielded query of the design (the `title=`/`author=` parameters match work
/// titles only and never find a translation), with both subtitles in `fields`, 10 works and
/// the book's language; our User-Agent goes with it.
#[sqlx::test]
async fn the_request_is_the_fielded_query_with_both_subtitles(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_search(&app, 200, goldfinch()).await;
    app.state
        .catalog
        .cover_candidates(&distelfink())
        .await
        .unwrap();
    let requests = searches(&app).await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        param(request, "q").as_deref(),
        Some("title:(Der Distelfink) author:(Tartt)")
    );
    assert_eq!(param(request, "fields").as_deref(), Some(FIELDS));
    assert_eq!(param(request, "limit").as_deref(), Some("10"));
    assert_eq!(param(request, "lang").as_deref(), Some("de"));
    assert_eq!(param(request, "title"), None);
    assert_eq!(
        request
            .headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok()),
        Some(user_agent(app.config()).as_str())
    );

    // Lucene's special characters never reach Open Library; no author, no language: left out.
    let query = CoverQuery {
        title: "Wer hat Angst vor Virginia Woolf? (Drama) \"Neu\"".into(),
        author: None,
        language: None,
    };
    app.state.catalog.cover_candidates(&query).await.unwrap();
    let requests = searches(&app).await;
    let request = requests.last().unwrap();
    assert_eq!(
        param(request, "q").as_deref(),
        Some("title:(Wer hat Angst vor Virginia Woolf Drama Neu)")
    );
    assert_eq!(param(request, "lang"), None);
}

/// The recorded answer of 2026-10-29 maps to the English work with its German edition (whose
/// language Open Library leaves out); junk is skipped or cleaned.
#[sqlx::test]
async fn the_answer_is_mapped_leniently(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_search(&app, 200, goldfinch()).await;
    let answer = app
        .state
        .catalog
        .cover_candidates(&distelfink())
        .await
        .unwrap();
    assert_eq!(answer.num_found, 1);
    assert_eq!(answer.refused, None);
    let doc = &answer.docs[0];
    assert_eq!(doc.ol_work_id, "OL16809803W");
    assert_eq!(doc.title, "The Goldfinch");
    assert_eq!(doc.authors, ["Donna Tartt", "Katia Benovich"]);
    assert_eq!(doc.cover_id, Some(8771366));
    assert_eq!(doc.edition_count, Some(42));
    assert_eq!(doc.first_publish_year, Some(2013));
    assert!(doc.languages.contains(&"eng".to_owned()));
    assert_eq!(
        doc.edition,
        Some(CoverEdition {
            ol_edition_id: "OL59468694M".into(),
            title: "Der Distelfink".into(),
            subtitle: "Roman".into(),
            cover_id: Some(15099932),
            languages: Vec::new(),
        })
    );

    let junk = json!({"numFound": 7, "docs": [
        "junk",
        {"key": "/books/OL1M", "title": "An edition"},
        {"key": "/works/OL2W", "title": " \u{0000} "},
        {"key": "/works/OL3W", "title": "Nul\u{0000}l  title ", "subtitle": "x".repeat(1200),
         "author_name": "One Name", "cover_i": 0, "first_publish_year": 99999,
         "language": ["GER", "deutsch", 5],
         "editions": {"docs": [{"key": "/books/bad", "title": "Bad edition"}]}},
        {"key": "/works/OL3W", "title": "A duplicate"},
        {"key": "/works/OL4W", "title": "Counted", "edition_count": "12", "cover_i": -1,
         "editions": {"docs": [{"key": "/books/OL4M", "cover_i": 44, "language": "ger"}]}}
    ]});
    let app = TestApp::new(app.db.clone()).await;
    mount_search(&app, 200, junk).await;
    let answer = app
        .state
        .catalog
        .cover_candidates(&distelfink())
        .await
        .unwrap();
    assert_eq!(answer.num_found, 7);
    let ids: Vec<&str> = answer.docs.iter().map(|d| d.ol_work_id.as_str()).collect();
    assert_eq!(ids, ["OL3W", "OL4W"]);
    let first = &answer.docs[0];
    assert_eq!(first.title, "Null title");
    assert_eq!(first.subtitle.chars().count(), 1000);
    assert_eq!(first.authors, ["One Name"]);
    assert_eq!(first.cover_id, None);
    assert_eq!(first.edition_count, None, "a missing count stays unknown");
    assert_eq!(first.first_publish_year, None);
    assert_eq!(first.languages, ["ger"]);
    assert_eq!(first.edition, None);
    let second = &answer.docs[1];
    assert_eq!(second.edition_count, Some(12));
    assert_eq!(second.cover_id, None);
    let edition = second.edition.as_ref().unwrap();
    assert_eq!(
        (edition.ol_edition_id.as_str(), edition.title.as_str()),
        ("OL4M", "")
    );
    assert_eq!(
        (edition.cover_id, edition.languages.clone()),
        (Some(44), vec!["ger".to_owned()])
    );
}

/// A good answer is kept for 18 hours under the folded title, author and language.
#[sqlx::test]
async fn answers_are_cached(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_search(&app, 200, goldfinch()).await;
    let lookup = &app.state.catalog;
    lookup.cover_candidates(&distelfink()).await.unwrap();
    let same = CoverQuery {
        title: "der  DISTELFINK".into(),
        ..distelfink()
    };
    let again = lookup.cover_candidates(&same).await.unwrap();
    assert_eq!(again.docs.len(), 1);
    assert_eq!(searches(&app).await.len(), 1, "served from the cache");
    let english = CoverQuery {
        language: Some("en".into()),
        ..distelfink()
    };
    lookup.cover_candidates(&english).await.unwrap();
    assert_eq!(
        searches(&app).await.len(),
        2,
        "another language is another key"
    );
}

/// A 429 is `Throttled` and starts the cooldown readers' searches see too.
#[sqlx::test]
async fn a_429_is_throttled_and_cools_down(db: PgPool) {
    let app = TestApp::new(db).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30"))
        .mount(&app.upstream)
        .await;
    let error = app
        .state
        .catalog
        .cover_candidates(&distelfink())
        .await
        .unwrap_err();
    assert!(matches!(error, LookupError::Throttled), "{error:?}");
    assert!(app.state.outbound.ol_api.cooldown_remaining().is_some());
}

/// Another 4xx is a query Open Library refuses: an answer without docs that names the status,
/// not cached, so the next search asks again.
#[sqlx::test]
async fn a_400_is_an_empty_answer_that_is_not_cached(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_search(&app, 400, json!({"error": "bad query"})).await;
    let lookup = &app.state.catalog;
    let answer = lookup.cover_candidates(&distelfink()).await.unwrap();
    assert_eq!(
        answer,
        CoverAnswer {
            refused: Some(400),
            ..CoverAnswer::default()
        }
    );
    lookup.cover_candidates(&distelfink()).await.unwrap();
    assert_eq!(searches(&app).await.len(), 2);
}

/// A 200 without a `docs` array, or with an `error` key, is a hiccup of the search index:
/// unavailable, not "no match". So is a 500 after the retry.
#[sqlx::test]
async fn a_payload_without_docs_is_unavailable(db: PgPool) {
    for body in [
        json!({"numFound": 0}),
        json!({"docs": [], "error": "solr timeout"}),
        json!({"docs": "nope"}),
    ] {
        let app = TestApp::new(db.clone()).await;
        mount_search(&app, 200, body.clone()).await;
        let error = app
            .state
            .catalog
            .cover_candidates(&distelfink())
            .await
            .unwrap_err();
        assert!(
            matches!(error, LookupError::Unavailable),
            "{body}: {error:?}"
        );
    }
    let app = TestApp::new(db).await;
    mount_search(&app, 500, json!({})).await;
    let error = app
        .state
        .catalog
        .cover_candidates(&distelfink())
        .await
        .unwrap_err();
    assert!(matches!(error, LookupError::Unavailable), "{error:?}");
}

/// `cover_by_isbn` is the existing `/isbn/{isbn}.json` check: the first positive cover id, none
/// for a 404, unavailable for a failure.
#[sqlx::test]
async fn the_isbn_cover_check_reads_the_edition(db: PgPool) {
    let app = TestApp::new(db).await;
    Mock::given(method("GET"))
        .and(path("/isbn/9783161484100.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"covers": [-1, 4242]})))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/isbn/9780316556347.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&app.upstream)
        .await;
    let lookup = &app.state.catalog;
    let isbn = |s: &str| parse_isbn(s).unwrap();
    assert_eq!(
        lookup.cover_by_isbn(&isbn("9783161484100")).await.unwrap(),
        Some(4242)
    );
    assert_eq!(
        lookup.cover_by_isbn(&isbn("9781234567897")).await.unwrap(),
        None
    );
    assert!(matches!(
        lookup.cover_by_isbn(&isbn("9780316556347")).await,
        Err(LookupError::Unavailable)
    ));
}
