//! ISBN lookups (#41): `GET /api/books/isbn/{isbn}` and `CatalogService`'s `CatalogLookup`
//! with the routing by registration group of conventions §7.4: 978-3 → DNB SRU → Open Library,
//! every other group → Open Library only. Upstream answers are recorded fixtures
//! (`tests/fixtures/dnb_sru/`, `tests/fixtures/openlibrary/`, recorded 2026-10-27) or the
//! payloads of the Python fakes.

mod common;

use std::time::Duration;

use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::catalog::shared::lookup::{CatalogLookup, LookupError};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

const SRU_HIT: &str = include_str!("fixtures/dnb_sru/hit_9783694050797.xml");
const SRU_MISS: &str = include_str!("fixtures/dnb_sru/miss.xml");
const OL_SEARCH_CIRCE: &str = include_str!("fixtures/openlibrary/search_isbn_9780316556347.json");
const OL_EDITION_CIRCE: &str = include_str!("fixtures/openlibrary/isbn_9780316556347.json");
/// A German ISBN with a DNB record (Patrice McDonough, "Der Mörder von Whitechapel").
const GERMAN: &str = "9783694050797";
/// The English ISBN of Circe (Little, Brown).
const ENGLISH: &str = "9780316556347";

/// `SEARCH_JSON` of the Python `test_book_details.py` fake.
fn search_json() -> Value {
    json!({
        "numFound": 1,
        "docs": [{
            "key": "/works/OL1W",
            "title": "Circe",
            "author_name": ["Madeline Miller"],
            "cover_i": 123,
            "first_publish_year": 2018,
            "number_of_pages_median": 393,
            "ratings_average": 4.27,
            "ratings_count": 812,
            "subject": ["Greek mythology"]
        }]
    })
}

async fn mount_ol_isbn_search(app: &TestApp, isbn: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", format!("isbn:{isbn}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&app.upstream)
        .await;
}

async fn mount_sru(app: &TestApp, body: &str) {
    Mock::given(method("GET"))
        .and(path("/sru/dnb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(body.as_bytes().to_vec(), "text/xml;charset=UTF-8"),
        )
        .mount(&app.upstream)
        .await;
}

async fn requests_to(app: &TestApp, needle: &str) -> Vec<wiremock::Request> {
    app.upstream_requests()
        .await
        .into_iter()
        .filter(|r| r.url.path().contains(needle))
        .collect()
}

async fn shelve(app: &TestApp, who: &TestClient, work_id: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ($1, $2, 'want_to_read')
         RETURNING id",
    )
    .bind(who.id())
    .bind(work_id)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// parity: test_book_details.py::test_isbn_lookup
/// Adapted: the hit is stored (edition and work), so the repeat after shelving is local; the
/// answer carries `work_id`/`edition_id`, and `on_shelf` is a boolean next to `shelf_id`.
#[sqlx::test]
async fn isbn_lookup(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    // 0000000000 is a valid ISBN-10 (9780000000002); Open Library knows nothing about it.
    mount_ol_isbn_search(&app, "9780000000002", json!({"numFound": 0, "docs": []})).await;
    mount_ol_isbn_search(&app, ENGLISH, search_json()).await;
    let ada = app.user("ada").await;

    ada.get("/api/books/isbn/12345")
        .await
        .assert_detail(400, "That is not a valid ISBN-10 or ISBN-13");
    ada.get("/api/books/isbn/0000000000")
        .await
        .assert_detail(404, "No book found for that ISBN");

    let hit = ada.get("/api/books/isbn/978-0-316-55634-7").await;
    hit.assert_status(200);
    let body = hit.json();
    assert_eq!(body["isbn"], ENGLISH);
    assert_eq!(body["ol_work_key"], "/works/OL1W");
    assert_eq!(body["title"], "Circe");
    assert_eq!(body["authors"], "Madeline Miller");
    assert_eq!(body["year"], 2018);
    assert_eq!(body["pages"], 393);
    assert_eq!(body["on_shelf"], false);
    assert!(body["shelf_id"].is_null());
    assert!(body["cover_url"].as_str().unwrap().starts_with("/media/"));

    let work_id = body["work_id"].as_i64().unwrap();
    let entry = shelve(&app, &ada, work_id).await;
    let calls = app.upstream_requests().await.len();
    let again = ada.get(&format!("/api/books/isbn/{ENGLISH}")).await.json();
    assert_eq!(again["on_shelf"], true);
    assert_eq!(again["shelf_id"], entry);
    assert_eq!(again["edition_id"], body["edition_id"]);
    assert_eq!(app.upstream_requests().await.len(), calls);
    // An English ISBN is never asked at the DNB.
    assert!(requests_to(&app, "/sru/").await.is_empty());
}

/// parity: test_book_details.py::test_isbn10_with_x_check_digit_is_accepted
/// parity: test_google_books.py::test_isbn_x_and_X_are_the_same_row
/// (KEEP, the lookup half: `x` and `X` are the same edition; the second lookup is local.)
/// parity: test_google_books.py::test_google_catalog_ids
/// (KEEP, the ISBN normalisation half: hyphens and the case of `X` do not matter; the Google
/// id formats are dropped.)
#[sqlx::test]
async fn isbn10_with_x_check_digit_is_accepted(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_ol_isbn_search(&app, "9780804429573", search_json()).await;
    let ada = app.user("ada").await;
    let upper = ada.get("/api/books/isbn/080442957X").await;
    upper.assert_status(200);
    assert_eq!(upper.json()["isbn"], "9780804429573");
    let lower = ada.get("/api/books/isbn/080442957x").await;
    lower.assert_status(200);
    assert_eq!(lower.json()["edition_id"], upper.json()["edition_id"]);
    assert_eq!(requests_to(&app, "search.json").await.len(), 1);
}

/// #41 acceptance (English ISBN) with recorded Open Library answers.
/// parity: test_google_books.py::test_isbn_lookup_prefers_google
/// (KEEP: an ISBN lookup returns title, authors, year, pages and cover.)
#[sqlx::test]
async fn an_english_isbn_asks_open_library_only(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", format!("isbn:{ENGLISH}")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(OL_SEARCH_CIRCE, "application/json"))
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    let body = ada.get(&format!("/api/books/isbn/{ENGLISH}")).await.json();
    assert_eq!(body["title"], "Circe");
    assert_eq!(body["authors"], "Madeline Miller");
    assert_eq!(body["year"], 2018);
    assert_eq!(body["pages"], 404);
    assert_eq!(body["ol_work_key"], "/works/OL18012166W");
    assert!(body["cover_url"].as_str().unwrap().starts_with("/media/"));
    assert!(requests_to(&app, "/sru/").await.is_empty());
    let search = requests_to(&app, "search.json").await;
    assert_eq!(search.len(), 1);
    let fields: String = search[0]
        .url
        .query_pairs()
        .find(|(k, _)| k == "fields")
        .unwrap()
        .1
        .into_owned();
    assert!(!fields.split(',').any(|f| f == "isbn"));
    let edition: (String, Option<String>) =
        sqlx::query_as("SELECT source, isbn13 FROM editions WHERE id = $1")
            .bind(body["edition_id"].as_i64().unwrap())
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(
        edition,
        ("openlibrary".to_owned(), Some(ENGLISH.to_owned()))
    );
}

/// #41 acceptance (German ISBN): DNB SRU first, mapped by the one MARC mapping and stored with
/// `Mode::Replace`; then one Open Library cover check by ISBN. Open Library search is not asked.
#[sqlx::test]
async fn a_german_isbn_asks_the_dnb_first(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_sru(&app, SRU_HIT).await;
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{GERMAN}.json")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    let res = ada.get(&format!("/api/books/isbn/{GERMAN}")).await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["title"], "Der Mörder von Whitechapel");
    assert_eq!(body["authors"], "Patrice McDonough");
    assert_eq!(body["year"], 2026);
    assert_eq!(body["pages"], 350);
    assert!(body["ol_work_key"].is_null());
    assert!(body["cover_url"].is_null());

    let sru = requests_to(&app, "/sru/dnb").await;
    assert_eq!(sru.len(), 1);
    let params: std::collections::HashMap<String, String> =
        sru[0].url.query_pairs().into_owned().collect();
    assert_eq!(params["query"], format!("num={GERMAN}"));
    assert_eq!(params["recordSchema"], "MARC21-xml");
    assert_eq!(params["operation"], "searchRetrieve");
    let agent = sru[0].headers.get("user-agent").unwrap().to_str().unwrap();
    assert!(agent.starts_with("Bookclub/1.0"), "{agent}");
    assert_eq!(
        requests_to(&app, &format!("/isbn/{GERMAN}.json"))
            .await
            .len(),
        1
    );
    assert!(requests_to(&app, "search.json").await.is_empty());

    let row: (String, String, Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT w.source, e.publisher, e.format, e.language, w.details_fetched_at IS NOT NULL
         FROM editions e JOIN works w ON w.id = e.work_id WHERE e.isbn13 = $1",
    )
    .bind(GERMAN)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "dnb".to_owned(),
            "dp Verlag".to_owned(),
            Some("paperback".to_owned()),
            Some("ger".to_owned()),
            true
        )
    );
    let translator: String = sqlx::query_scalar(
        "SELECT a.name FROM edition_contributors c JOIN authors a ON a.id = c.author_id
         JOIN editions e ON e.id = c.edition_id WHERE e.isbn13 = $1 AND c.role = 'translator'",
    )
    .bind(GERMAN)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(translator, "Jeanette Mey");

    // Known now: the next lookup is local.
    let calls = app.upstream_requests().await.len();
    ada.get(&format!("/api/books/isbn/{GERMAN}"))
        .await
        .assert_status(200);
    assert_eq!(app.upstream_requests().await.len(), calls);
}

/// The Open Library cover check after a DNB hit attaches `covers[0]` (the first positive id)
/// by cover id, with Open Library provenance, to the edition and the work.
#[sqlx::test]
async fn a_dnb_hit_gets_its_cover_from_open_library(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_sru(&app, SRU_HIT).await;
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{GERMAN}.json")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"covers": [-1, 8283869]})))
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    let body = ada.get(&format!("/api/books/isbn/{GERMAN}")).await.json();
    let public_id = body["cover_public_id"].as_str().unwrap();
    assert_eq!(body["cover_url"], format!("/media/{public_id}/l"));
    let row: (i64, Value, bool) = sqlx::query_as(
        "SELECT m.ol_cover_id, e.field_sources, COALESCE(w.cover_media_id = m.id, false)
         FROM editions e JOIN media m ON m.id = e.cover_media_id
         JOIN works w ON w.id = e.work_id WHERE e.isbn13 = $1",
    )
    .bind(GERMAN)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(row.0, 8283869);
    assert_eq!(row.1["cover_media_id"], "openlibrary");
    assert!(row.2, "the work's empty cover is filled from the edition");
}

/// #41 acceptance: a German ISBN falls through to Open Library while the DNB cools down (no
/// SRU request is made); the hit is stored with Open Library provenance.
#[sqlx::test]
async fn a_german_isbn_falls_through_while_the_dnb_cools_down(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_sru(&app, SRU_HIT).await;
    let mut doc = search_json();
    doc["docs"][0]["key"] = json!("/works/OL5W");
    doc["docs"][0]["title"] = json!("Der Mörder von Whitechapel");
    mount_ol_isbn_search(&app, GERMAN, doc).await;
    app.state
        .outbound
        .dnb_sru
        .cool_down(Duration::from_secs(60));
    let ada = app.user("ada").await;
    let body = ada.get(&format!("/api/books/isbn/{GERMAN}")).await.json();
    assert_eq!(body["ol_work_key"], "/works/OL5W");
    assert!(requests_to(&app, "/sru/").await.is_empty());
    let source: String = sqlx::query_scalar("SELECT source FROM editions WHERE isbn13 = $1")
        .bind(GERMAN)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(source, "openlibrary");
}

/// A DNB miss falls through to Open Library; a miss everywhere is 404.
/// parity: test_google_books.py::test_isbn_lookup_miss_does_not_use_open_library
/// parity: test_google_books.py::test_search_empty_google_does_not_fall_back
/// parity: test_google_books.py::test_empty_google_response_does_not_freeze_retries
/// INVERT: a miss in one source falls through to the next; only a final miss is 404. KEEP (the
/// DNB SRU half of the last one): a DNB miss is not cached.
#[sqlx::test]
async fn a_dnb_miss_falls_through_and_a_final_miss_is_404(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_sru(&app, SRU_MISS).await;
    mount_ol_isbn_search(&app, GERMAN, json!({"numFound": 0, "docs": []})).await;
    let ada = app.user("ada").await;
    ada.get(&format!("/api/books/isbn/{GERMAN}"))
        .await
        .assert_detail(404, "No book found for that ISBN");
    assert_eq!(requests_to(&app, "/sru/").await.len(), 1);
    assert_eq!(requests_to(&app, "search.json").await.len(), 1);
    // Neither miss was cached: the next lookup asks both again.
    ada.get(&format!("/api/books/isbn/{GERMAN}"))
        .await
        .assert_status(404);
    assert_eq!(requests_to(&app, "/sru/").await.len(), 2);
    assert_eq!(requests_to(&app, "search.json").await.len(), 2);
}

/// Without a definite answer the lookup is an error, never a miss: a busy DNB with an Open
/// Library miss is 429; every source busy is 429; a failing Open Library is 502.
#[sqlx::test]
async fn no_definite_answer_is_an_error_not_a_miss(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_ol_isbn_search(&app, GERMAN, json!({"numFound": 0, "docs": []})).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", format!("isbn:{ENGLISH}")))
        .respond_with(ResponseTemplate::new(500))
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    app.state
        .outbound
        .dnb_sru
        .cool_down(Duration::from_secs(30));
    let busy = ada.get(&format!("/api/books/isbn/{GERMAN}")).await;
    busy.assert_status(429);
    assert!(
        busy.detail()
            .starts_with("The library is busy. Try again in")
    );

    ada.get(&format!("/api/books/isbn/{ENGLISH}"))
        .await
        .assert_detail(502, "Could not look that ISBN up right now. Try again.");

    app.state.outbound.ol_api.cool_down(Duration::from_secs(30));
    ada.get(&format!("/api/books/isbn/{GERMAN}"))
        .await
        .assert_status(429);
    ada.get("/api/books/isbn/9780804429573")
        .await
        .assert_status(429);
    assert!(requests_to(&app, "/sru/").await.is_empty());
}

/// parity: test_google_books.py::test_empty_google_response_does_not_freeze_retries
/// KEEP (the ISBN cases): an empty Open Library answer (missing or empty `docs`) is a 404 and is
/// not cached; the retry asks again and stores the hit; the third lookup is local.
#[sqlx::test]
async fn empty_isbn_answers_do_not_freeze_retries(db: PgPool) {
    for (round, empty) in [json!({"numFound": 0}), json!({"numFound": 0, "docs": []})]
        .into_iter()
        .enumerate()
    {
        let app = TestApp::with_real_lookup(db.clone()).await;
        app.exec("DELETE FROM editions; DELETE FROM works").await;
        Mock::given(method("GET"))
            .and(path("/search.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&app.upstream)
            .await;
        mount_ol_isbn_search(&app, ENGLISH, search_json()).await;
        let ada = app.user(&format!("ada{round}")).await;
        ada.get(&format!("/api/books/isbn/{ENGLISH}"))
            .await
            .assert_status(404);
        let second = ada.get(&format!("/api/books/isbn/{ENGLISH}")).await;
        second.assert_status(200);
        assert_eq!(requests_to(&app, "search.json").await.len(), 2);
        let third = ada.get(&format!("/api/books/isbn/{ENGLISH}")).await;
        assert_eq!(third.json(), second.json());
        assert_eq!(requests_to(&app, "search.json").await.len(), 2);
    }
}

/// parity: test_google_books.py::test_isbn_search_uses_google_isbn_field (ADAPT, the German
/// half): an ISBN-shaped search for a 978-3 ISBN goes through the DNB.
#[sqlx::test]
async fn a_german_isbn_search_goes_through_the_dnb(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_sru(&app, SRU_HIT).await;
    let ada = app.user("ada").await;
    let res = ada.get("/api/books/search?q=978-3-69405-079-7").await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["items"][0]["title"], "Der Mörder von Whitechapel");
    assert_eq!(body["items"][0]["isbn"], GERMAN);
    assert_eq!(body["items"][0]["source"], "dnb");
    assert_eq!(body["has_more"], false);
    assert_eq!(requests_to(&app, "/sru/").await.len(), 1);
    assert!(requests_to(&app, "search.json").await.is_empty());
    // A checksum that does not hold matches nothing and asks nobody.
    let bad = ada.get("/api/books/search?q=9783694050798").await.json();
    assert_eq!(bad["items"], json!([]));
    assert_eq!(app.upstream_requests().await.len(), 2);
}

/// `resolve_isbn_remote` skips the local step and upgrades an existing edition under the
/// provenance rules: a user-added work keeps its reader-entered title, and the answer names
/// that work (custom books answer 409 with it).
#[sqlx::test]
async fn resolve_isbn_remote_upgrades_an_existing_edition(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_ol_isbn_search(&app, ENGLISH, search_json()).await;
    let mine = app.work("My Circe").await;
    let edition = app.edition(&mine, ENGLISH).await;
    let isbn = parse_isbn(ENGLISH).unwrap();
    let mut conn = app.db.acquire().await.unwrap();
    let local = app
        .state
        .catalog
        .resolve_isbn(&mut conn, &isbn)
        .await
        .unwrap();
    assert_eq!(local.map(|r| r.work_id), Some(mine.id));
    assert!(app.upstream_requests().await.is_empty());

    let remote = app
        .state
        .catalog
        .resolve_isbn_remote(&mut conn, &isbn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(remote.work_id, mine.id);
    assert_eq!(remote.edition_id, Some(edition.id));
    let title: String = sqlx::query_scalar("SELECT title FROM works WHERE id = $1")
        .bind(mine.id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(title, "My Circe");
    assert_eq!(requests_to(&app, "search.json").await.len(), 1);
}

/// The DNB SRU cache keeps records for 18 hours: a second remote lookup of the same German ISBN
/// makes no second SRU request (the edition is stored by then, so only `resolve_isbn_remote`
/// asks again).
#[sqlx::test]
async fn dnb_answers_are_cached(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_sru(&app, SRU_HIT).await;
    Mock::given(method("GET"))
        .and(path(format!("/isbn/{GERMAN}.json")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&app.upstream)
        .await;
    let isbn = parse_isbn(GERMAN).unwrap();
    let mut conn = app.db.acquire().await.unwrap();
    let first = app
        .state
        .catalog
        .resolve_isbn_remote(&mut conn, &isbn)
        .await
        .unwrap();
    let second = app
        .state
        .catalog
        .resolve_isbn_remote(&mut conn, &isbn)
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(requests_to(&app, "/sru/").await.len(), 1);
    // A failing DNB answer is not cached either; it reports no definite answer.
    let other = parse_isbn("9783000000003").unwrap();
    app.state.outbound.dnb_sru.cool_down(Duration::from_secs(5));
    app.state.outbound.ol_api.cool_down(Duration::from_secs(5));
    let busy = app
        .state
        .catalog
        .resolve_isbn_remote(&mut conn, &other)
        .await;
    assert!(matches!(busy, Err(LookupError::Throttled)), "{busy:?}");
}

/// The 429 hint names the cooldown of the source whose answer was missing: Open Library's when
/// its step was throttled (an English ISBN never asks the DNB, so the DNB's cooldown does not
/// matter), the DNB's when it was throttled and Open Library answered with a miss. The same
/// holds for an ISBN-shaped search and for the ISBN check of a user-added book.
#[sqlx::test]
async fn the_busy_hint_names_the_source_that_refused(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_ol_isbn_search(&app, GERMAN, json!({"numFound": 0, "docs": []})).await;
    let ada = app.user("ada").await;
    app.state
        .outbound
        .dnb_sru
        .cool_down(Duration::from_secs(600));

    // The DNB is busy and Open Library does not know the book: the DNB decided.
    let dnb = ada.get(&format!("/api/books/isbn/{GERMAN}")).await;
    dnb.assert_detail(429, "The library is busy. Try again in 600 seconds.");
    assert_eq!(dnb.header("retry-after"), Some("600"));

    // Open Library is busy too: its step decided, for every group and every route.
    app.state.outbound.ol_api.cool_down(Duration::from_secs(5));
    for path in [
        format!("/api/books/isbn/{ENGLISH}"),
        format!("/api/books/isbn/{GERMAN}"),
        format!("/api/books/search?q={ENGLISH}"),
    ] {
        let res = ada.get(&path).await;
        res.assert_detail(429, "The library is busy. Try again in 5 seconds.");
        assert_eq!(res.header("retry-after"), Some("5"), "{path}");
    }
    let custom = ada
        .post(
            "/api/books/custom",
            json!({"title": "Circe", "isbn": ENGLISH}),
        )
        .await;
    custom.assert_detail(429, "The library is busy. Try again in 5 seconds.");
    assert!(requests_to(&app, "/sru/").await.is_empty());
}

/// The ISBN routes hold no pool connection while the sources are asked: with a pool of one
/// connection, another request gets it while Open Library is still answering a lookup or the
/// ISBN check of a user-added book.
#[sqlx::test]
async fn isbn_routes_hold_no_connection_while_asking_upstream(db: PgPool) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.connect_options().as_ref().clone())
        .await
        .unwrap();
    let app = TestApp::with_real_lookup(pool.clone()).await;
    for (isbn, body) in [
        (ENGLISH, search_json()),
        ("9780062060624", json!({"numFound": 0, "docs": []})),
    ] {
        Mock::given(method("GET"))
            .and(path("/search.json"))
            .and(query_param("q", format!("isbn:{isbn}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(body)
                    .set_delay(Duration::from_millis(1500)),
            )
            .mount(&app.upstream)
            .await;
    }
    let ada = app.user("ada").await;
    let probe = |calls: usize| {
        let (app, pool) = (&app, &pool);
        async move {
            for _ in 0..250 {
                if requests_to(app, "search.json").await.len() == calls {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let conn = tokio::time::timeout(Duration::from_millis(500), pool.acquire()).await;
            assert!(
                conn.is_ok(),
                "the pool's only connection is held by the lookup"
            );
        }
    };
    let lookup = format!("/api/books/isbn/{ENGLISH}");
    let (res, ()) = tokio::join!(ada.get(&lookup), probe(1));
    res.assert_status(200);
    let (res, ()) = tokio::join!(
        ada.post(
            "/api/books/custom",
            json!({"title": "The Song of Achilles", "isbn": "9780062060624"}),
        ),
        probe(2)
    );
    res.assert_status(201);
    drop(app);
    pool.close().await;
}

/// A German ISBN-10 becomes its 978-3 ISBN-13, which the DNB route takes.
#[test]
fn german_isbn10_is_routed_to_the_dnb() {
    let isbn = parse_isbn("3-694-05079-5").unwrap();
    assert_eq!(isbn.as_str(), GERMAN);
    assert!(bookclub_server::catalog::shared::isbn::is_german_group(
        &isbn
    ));
}

/// Coverage for `OL_EDITION_CIRCE`: the recorded edition JSON of an English ISBN carries
/// `covers` with a placeholder after the real id, which the cover check skips.
#[test]
fn the_recorded_edition_has_a_cover() {
    let edition: Value = serde_json::from_str(OL_EDITION_CIRCE).unwrap();
    assert_eq!(
        bookclub_server::catalog::openlibrary::first_positive_cover(&[&edition["covers"]]),
        Some(8283869)
    );
}
