//! The Open Library client of `CatalogService` (#41): the client-side rate limit, the full work
//! record a first detail view or a refresh imports (`ol_work_record`), the response caches, and
//! `CatalogLookup::resolve_text`.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bookclub_server::catalog::cache::Fetch;
use bookclub_server::catalog::search::{SearchParams, run_search};
use bookclub_server::catalog::shared::lookup::{CatalogLookup, LookupError};
use bookclub_server::catalog::{CatalogService, RATE_LIMITED};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn circe_doc() -> Value {
    json!({
        "key": "/works/OL1W",
        "title": "Circe",
        "author_name": ["Madeline Miller"],
        "cover_i": 123,
        "first_publish_year": 2018,
        "number_of_pages_median": 393,
        "ratings_average": 4.27,
        "ratings_count": 812,
        "subject": ["Greek mythology"]
    })
}

fn work_json() -> Value {
    json!({
        "title": "Circe",
        "description": "A plain string description.",
        "subjects": ["Mythology", "mythology", "Greek literature", "", "award:hugo_award=1970"],
        "covers": [-1, 555],
        "authors": [{"author": {"key": "/authors/OL2A"}}]
    })
}

async fn mount_work(app: &TestApp, work: Value, doc: Value) {
    Mock::given(method("GET"))
        .and(path("/works/OL1W.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(work))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "key:/works/OL1W"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"numFound": 1, "docs": [doc]})),
        )
        .mount(&app.upstream)
        .await;
}

async fn mount_author(app: &TestApp, name: &str) {
    Mock::given(method("GET"))
        .and(path("/authors/OL2A.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": name})))
        .mount(&app.upstream)
        .await;
}

async fn count(app: &TestApp, needle: &str) -> usize {
    app.upstream_requests()
        .await
        .iter()
        .filter(|r| r.url.path().contains(needle))
        .count()
}

/// #41 acceptance: the client-side rate limit is enforced. With the production throttle
/// (Open Library: 3 requests a second, evenly spaced) a burst of searches is spread out, and
/// what cannot get a slot within 2 s answers 429 `The library is busy. Wait a few seconds and
/// try again.` without a request.
#[sqlx::test]
async fn open_library_requests_are_throttled(db: PgPool) {
    let upstream = MockServer::start().await;
    Mock::given(path("/search.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"numFound": 1, "docs": [circe_doc()]})),
        )
        .mount(&upstream)
        .await;
    let uri = upstream.uri();
    let state = test_state_with(db, test_config(&[("OPENLIBRARY_URL", uri.as_str())]));
    let started = Instant::now();
    let searches = (0..10).map(|i| {
        let state = state.clone();
        async move {
            let params = SearchParams {
                q: format!("query number {i}"),
                subject: String::new(),
                sort: None,
                page: 1,
                limit: 5,
            };
            run_search(&state, 0, params).await
        }
    });
    let results = futures::future::join_all(searches).await;
    let elapsed = started.elapsed();
    let ok = results.iter().filter(|r| r.is_ok()).count();
    let busy: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    for error in &busy {
        assert_eq!(error.status.as_u16(), 429);
        assert_eq!(error.detail, RATE_LIMITED);
    }
    assert!(busy.len() >= 2, "{} throttled", busy.len());
    assert!(ok >= 5, "{ok} answered");
    assert_eq!(upstream.received_requests().await.unwrap().len(), ok);
    assert!(elapsed >= Duration::from_millis(1300), "{elapsed:?}");
}

/// The first import of an Open Library work (the legacy `test_search.py` detail fixture): the
/// description and subjects are cleaned, the work's first positive cover wins, and the authors
/// come from the search doc without an author request.
#[sqlx::test]
async fn a_work_record_takes_names_from_the_search_doc(db: PgPool) {
    let app = TestApp::new(db).await;
    mount_work(&app, work_json(), circe_doc()).await;
    let record = app
        .state
        .catalog
        .ol_work_record("OL1W", false)
        .await
        .unwrap();
    assert_eq!(record.ol_work_id.as_deref(), Some("OL1W"));
    assert_eq!(record.title, "Circe");
    assert_eq!(record.author_names, "Madeline Miller");
    assert!(record.authors.is_empty());
    assert_eq!(
        record.description.as_deref(),
        Some("A plain string description.")
    );
    assert_eq!(
        record.subjects,
        Some(vec!["Mythology".to_owned(), "Greek literature".to_owned()])
    );
    assert_eq!(record.ol_cover_id, Some(555));
    assert_eq!(record.first_publish_year, Some(2018));
    assert_eq!(record.pages, Some(393));
    assert_eq!(record.ol_rating, Some(4.27));
    assert_eq!(record.ol_rating_count, Some(812));
    assert!(record.full_details);
    assert_eq!(count(&app, "/authors/").await, 0);

    // Cached: a second import makes no request; after clearing the caches it fetches again.
    app.state
        .catalog
        .ol_work_record("OL1W", false)
        .await
        .unwrap();
    assert_eq!(app.upstream_requests().await.len(), 2);
    app.state.catalog.clear_caches();
    app.state
        .catalog
        .ol_work_record("OL1W", false)
        .await
        .unwrap();
    assert_eq!(app.upstream_requests().await.len(), 4);
}

/// A dict description is unwrapped and trimmed, no subjects anywhere is `None`, and without work
/// covers the search doc's cover is used. Markdown and the source trailer are flattened
/// (both legacy cleaners).
#[sqlx::test]
async fn a_work_record_cleans_descriptions_and_falls_back_to_the_doc(db: PgPool) {
    let app = TestApp::new(db).await;
    let mut doc = circe_doc();
    doc.as_object_mut().unwrap().remove("subject");
    mount_work(
        &app,
        json!({"title": "Circe", "description": {"type": "/type/text", "value": "  A dict description.  "}}),
        doc,
    )
    .await;
    let record = app
        .state
        .catalog
        .ol_work_record("OL1W", false)
        .await
        .unwrap();
    assert_eq!(record.description.as_deref(), Some("A dict description."));
    assert_eq!(record.subjects, None);
    assert_eq!(record.ol_cover_id, Some(123));

    let app = TestApp::new(app.db.clone()).await;
    mount_work(
        &app,
        json!({
            "title": "Circe",
            "description": "[Comment by Kim Stanley Robinson][1] and [a note](http://example.com):\n\n\
                > One of my favorite novels.\n> Truly.\n\n\n\nThe end.\n\n[1]: http://example.com/robinson\n\
                \n----------\n[Source](https://x)",
            "subjects": ["Greek mythology", "Fiction", "Witches", "Fiction, fantasy, general", "x".repeat(50)]
        }),
        circe_doc(),
    )
    .await;
    let record = app
        .state
        .catalog
        .ol_work_record("OL1W", false)
        .await
        .unwrap();
    assert_eq!(
        record.description.as_deref(),
        Some(
            "Comment by Kim Stanley Robinson and a note:\n\nOne of my favorite novels.\nTruly.\n\nThe end."
        )
    );
    assert_eq!(
        record.subjects,
        Some(vec![
            "Greek mythology".to_owned(),
            "Fiction".to_owned(),
            "Witches".to_owned()
        ])
    );
}

/// Without names in the search doc the author API is asked (at most 3 authors, with their Open
/// Library ids); a failing author request leaves the doc's names. A refresh asks the author API
/// first and bypasses the cached answers.
#[sqlx::test]
async fn a_work_record_asks_the_author_api_when_needed(db: PgPool) {
    let app = TestApp::new(db).await;
    let mut doc = circe_doc();
    doc["author_name"] = json!([]);
    mount_work(&app, work_json(), doc).await;
    mount_author(&app, "Madeline Miller").await;
    let record = app
        .state
        .catalog
        .ol_work_record("OL1W", false)
        .await
        .unwrap();
    assert_eq!(record.author_names, "Madeline Miller");
    assert_eq!(record.authors.len(), 1);
    assert_eq!(record.authors[0].ol_author_id.as_deref(), Some("OL2A"));

    // The author request fails: the doc's names stay.
    let app = TestApp::new(app.db.clone()).await;
    mount_work(&app, work_json(), circe_doc()).await;
    Mock::given(path_regex("^/authors/"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&app.upstream)
        .await;
    let refreshed = app
        .state
        .catalog
        .ol_work_record("OL1W", true)
        .await
        .unwrap();
    assert_eq!(refreshed.author_names, "Madeline Miller");
    assert!(refreshed.authors.is_empty());

    // A refresh: the author API wins over the doc, and cached answers are not used.
    let app = TestApp::new(app.db.clone()).await;
    mount_work(&app, work_json(), circe_doc()).await;
    mount_author(&app, "M. Miller").await;
    let first = app
        .state
        .catalog
        .ol_work_record("OL1W", false)
        .await
        .unwrap();
    assert_eq!(first.author_names, "Madeline Miller");
    let refreshed = app
        .state
        .catalog
        .ol_work_record("OL1W", true)
        .await
        .unwrap();
    assert_eq!(refreshed.author_names, "M. Miller");
    assert_eq!(count(&app, "/works/OL1W.json").await, 2);
}

/// A missing work is `Fetch::Missing` (the detail view's 404 `No such book.`); a failing search
/// doc is best effort; a work without any title is named after its id.
#[sqlx::test]
async fn a_work_record_needs_the_work_but_not_the_doc(db: PgPool) {
    let app = TestApp::new(db).await;
    Mock::given(path("/works/OL404W.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&app.upstream)
        .await;
    Mock::given(path("/works/OL777W.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"covers": [-1]})))
        .mount(&app.upstream)
        .await;
    Mock::given(path("/search.json"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&app.upstream)
        .await;
    let missing = app.state.catalog.ol_work_record("OL404W", false).await;
    assert_eq!(missing.unwrap_err(), Fetch::Missing);
    let bare = app
        .state
        .catalog
        .ol_work_record("OL777W", false)
        .await
        .unwrap();
    assert_eq!(bare.title, "OL777W");
    assert_eq!(bare.author_names, "");
    assert_eq!(bare.ol_cover_id, None);
}

/// `resolve_text`: a local work with that title (case and accents aside) and those authors
/// answers without a request; otherwise the first Open Library hit of "{title} {authors}"
/// (limit 5) is stored as a stub. A miss is `Ok(None)`, a busy upstream an error.
#[sqlx::test]
async fn resolve_text_is_local_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let catalog: Arc<CatalogService> = app.state.catalog.clone();
    let mut conn = app.db.acquire().await.unwrap();
    let local = app.ol_work("OL9W", "Die Brücke", "Ada Lovelace").await;
    let found = catalog
        .resolve_text(&mut conn, "die brucke", "Ada Lovelace")
        .await
        .unwrap();
    assert_eq!(found.map(|r| r.work_id), Some(local.id));
    assert!(app.upstream_requests().await.is_empty());

    Mock::given(path("/search.json"))
        .and(query_param("q", "Circe Madeline Miller"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"numFound": 1, "docs": [circe_doc()]})),
        )
        .mount(&app.upstream)
        .await;
    Mock::given(path("/search.json"))
        .and(query_param("q", "Nothing Nobody"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"numFound": 0, "docs": []})))
        .mount(&app.upstream)
        .await;
    let remote = catalog
        .resolve_text(&mut conn, "Circe", "Madeline Miller")
        .await
        .unwrap()
        .unwrap();
    let ol: Option<String> = sqlx::query_scalar("SELECT ol_work_id FROM works WHERE id = $1")
        .bind(remote.work_id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(ol.as_deref(), Some("OL1W"));
    let sent = &app.upstream_requests().await[0];
    let limit = sent
        .url
        .query_pairs()
        .find(|(k, _)| k == "limit")
        .unwrap()
        .1;
    assert_eq!(limit, "5");

    // Nothing anywhere: no match.
    assert_eq!(
        catalog
            .resolve_text(&mut conn, "Nothing", "Nobody")
            .await
            .unwrap(),
        None
    );
    // Same title, other authors: not the local work; Open Library is asked instead.
    Mock::given(path("/search.json"))
        .and(query_param("q", "Die Brücke Someone Else"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"numFound": 0, "docs": []})))
        .mount(&app.upstream)
        .await;
    assert_eq!(
        catalog
            .resolve_text(&mut conn, "Die Brücke", "Someone Else")
            .await
            .unwrap(),
        None
    );
    let asked = app
        .upstream_requests()
        .await
        .iter()
        .filter(|r| {
            r.url
                .query_pairs()
                .any(|(k, v)| k == "q" && v == "Die Brücke Someone Else")
        })
        .count();
    assert_eq!(asked, 1);
    assert_eq!(
        catalog.resolve_text(&mut conn, "  ", "x").await.unwrap(),
        None
    );
    app.state.outbound.ol_api.cool_down(Duration::from_secs(5));
    let busy = catalog.resolve_text(&mut conn, "Unknown", "Someone").await;
    assert!(matches!(busy, Err(LookupError::Throttled)), "{busy:?}");
}
