//! The deadlock retries of the catalog's ingest callers (#41, conventions §7.2 "Deadlocks"): an
//! ingest that Postgres aborts to break a deadlock (SQLSTATE 40P01) is rolled back and retried
//! once, for an ISBN lookup, the hits of a search, a work import and `resolve_text`. A trigger
//! fakes the deadlock.

mod common;

use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::catalog::shared::lookup::{CatalogLookup, LookupError};
use bookclub_server::error::is_deadlock;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

const OL_SEARCH_CIRCE: &str = include_str!("fixtures/openlibrary/search_isbn_9780316556347.json");
/// The English ISBN of Circe (Little, Brown).
const ENGLISH: &str = "9780316556347";

fn circe_doc() -> Value {
    json!({
        "key": "/works/OL1W",
        "title": "Circe",
        "author_name": ["Madeline Miller"],
        "first_publish_year": 2018
    })
}

/// Makes the first `failures` inserts into `table` fail with SQLSTATE 40P01, as when Postgres
/// aborts a transaction to break a deadlock. A sequence counts the inserts: unlike a row, it is
/// not rolled back with the aborted transaction.
async fn fake_deadlocks(app: &TestApp, table: &str, failures: i64) {
    app.exec(&format!(
        r#"CREATE SEQUENCE fake_deadlock_calls;
           CREATE FUNCTION fake_deadlock() RETURNS trigger LANGUAGE plpgsql AS $$
           BEGIN
               IF nextval('fake_deadlock_calls') <= {failures} THEN
                   RAISE EXCEPTION 'fake deadlock' USING ERRCODE = '40P01';
               END IF;
               RETURN NEW;
           END $$;
           CREATE TRIGGER fake_deadlock BEFORE INSERT ON {table}
               FOR EACH ROW EXECUTE FUNCTION fake_deadlock();"#
    ))
    .await;
}

/// How many inserts the trigger saw.
async fn inserts(app: &TestApp) -> i64 {
    app.scalar_i64("SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM fake_deadlock_calls")
        .await
}

async fn mount_isbn_search(app: &TestApp) {
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", format!("isbn:{ENGLISH}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(OL_SEARCH_CIRCE.as_bytes().to_vec(), "application/json"),
        )
        .mount(&app.upstream)
        .await;
}

/// An ISBN lookup whose ingest deadlocks is retried once and then stored.
#[sqlx::test]
async fn an_isbn_lookup_is_retried_once_after_a_deadlock(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_isbn_search(&app).await;
    fake_deadlocks(&app, "editions", 1).await;
    let isbn = parse_isbn(ENGLISH).unwrap();
    let mut conn = app.db.acquire().await.unwrap();
    let found = app
        .state
        .catalog
        .resolve_isbn_remote(&mut conn, &isbn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inserts(&app).await, 2);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE isbn13 = '{ENGLISH}' AND work_id = {}",
            found.work_id
        ))
        .await,
        1
    );
}

/// Only once: a second deadlock is the caller's error, and nothing is stored.
#[sqlx::test]
async fn a_second_deadlock_fails_the_isbn_lookup(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_isbn_search(&app).await;
    fake_deadlocks(&app, "editions", 2).await;
    let isbn = parse_isbn(ENGLISH).unwrap();
    let mut conn = app.db.acquire().await.unwrap();
    let failed = app
        .state
        .catalog
        .resolve_isbn_remote(&mut conn, &isbn)
        .await;
    match failed {
        Err(LookupError::Db(error)) => assert!(is_deadlock(&error), "{error}"),
        other => panic!("expected a deadlock, got {other:?}"),
    }
    assert_eq!(inserts(&app).await, 2);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM editions").await, 0);
    // The API answers 500 and stores nothing either (two more deadlocks).
    app.exec("ALTER SEQUENCE fake_deadlock_calls RESTART").await;
    let ada = app.user("ada").await;
    ada.get(&format!("/api/books/isbn/{ENGLISH}"))
        .await
        .assert_status(500);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM works").await, 0);
}

/// The hits of a search are stored after one retry.
#[sqlx::test]
async fn search_hits_are_stored_after_one_retry(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"numFound": 1, "docs": [circe_doc()]})),
        )
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    fake_deadlocks(&app, "works", 1).await;
    let res = ada.get("/api/books/search?q=circe").await;
    res.assert_status(200);
    assert_eq!(res.json()["items"][0]["ol_work_key"], "/works/OL1W");
    assert_eq!(inserts(&app).await, 2);
}

/// The first view of an unknown Open Library work stores its record after one retry.
#[sqlx::test]
async fn a_work_import_is_stored_after_one_retry(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    Mock::given(method("GET"))
        .and(path("/works/OL1W.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "Circe",
            "description": "A witch on an island."
        })))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "key:/works/OL1W"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"numFound": 1, "docs": [circe_doc()]})),
        )
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    fake_deadlocks(&app, "works", 1).await;
    let res = ada.get("/api/books/works/OL1W").await;
    res.assert_status(200);
    assert_eq!(res.json()["description"], "A witch on an island.");
    assert_eq!(inserts(&app).await, 2);
}

/// `resolve_text` (SHELF's Goodreads import) stores its Open Library hit after one retry.
#[sqlx::test]
async fn resolve_text_stores_its_hit_after_one_retry(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "Circe Madeline Miller"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"numFound": 1, "docs": [circe_doc()]})),
        )
        .mount(&app.upstream)
        .await;
    fake_deadlocks(&app, "works", 1).await;
    let mut conn = app.db.acquire().await.unwrap();
    let found = app
        .state
        .catalog
        .resolve_text(&mut conn, "Circe", "Madeline Miller")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inserts(&app).await, 2);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {} AND ol_work_id = 'OL1W'",
            found.work_id
        ))
        .await,
        1
    );
}
