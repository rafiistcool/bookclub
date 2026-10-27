//! Book detail and refresh (#41): `GET /api/books/works/{work_id}` and
//! `POST /api/books/works/{work_id}/refresh`. The `test_search.py` and `test_book_details.py`
//! detail ports, the catalog half of `test_local_reads.py`, and the detail and refresh KEEP and
//! ADAPT tests of `test_google_books.py` (docs/phase1/spec/google.md §4).
//!
//! `FakeOl` is the Python `_FakeClient` of both files as one wiremock responder: the work JSON,
//! its search doc, the author API, trending and the `/isbn/{isbn}.json` cover check.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bookclub_server::catalog::shared::books::{BookRef, book_out};
use bookclub_server::catalog::shared::ingest::{self, Mode, Source, WorkRecord};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::{Mock, Request, Respond, ResponseTemplate};

/// `SEARCH_DOCS[0]` of `test_search.py`: the search doc of OL1W.
fn circe_doc() -> Value {
    json!({
        "key": "/works/OL1W",
        "title": "Circe",
        "author_name": ["Madeline Miller"],
        "cover_i": 123,
        "first_publish_year": 2018,
        "number_of_pages_median": 393,
        "ratings_average": 4.27,
        "ratings_count": 812
    })
}

/// `_FakeClient.reset()`'s work payload of `test_search.py`.
fn circe_work() -> Value {
    json!({
        "title": "Circe",
        "description": "A plain string description.",
        "subjects": ["Mythology", "mythology", "Greek literature", "", "award:hugo_award=1970"],
        "covers": [-1, 555],
        "authors": [{"author": {"key": "/authors/OL2A"}}]
    })
}

/// `WORK_JSON` of `test_book_details.py`.
fn details_work() -> Value {
    json!({
        "title": "Circe",
        "description": {"value": "A witch on an island.\n\n----------\n[Source](https://x)"},
        "subjects": ["Greek mythology", "Fiction", "Witches", "Fiction, fantasy, general",
                     "x".repeat(50)],
        "covers": [123]
    })
}

/// `SEARCH_JSON["docs"][0]` of `test_book_details.py`.
fn details_doc() -> Value {
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

fn trending_works() -> Value {
    json!([
        {"key": "/works/OL7W", "title": "Atomic Habits", "author_name": ["James Clear"],
         "cover_i": 999, "first_publish_year": 2016},
        {"key": "/works/OL8W", "title": "No cover", "cover_i": -1}
    ])
}

struct OlState {
    work: Value,
    doc: Value,
    author_name: String,
    /// The `/isbn/{isbn}.json` edition.
    edition: Value,
    /// Path marker → status.
    status_for: Vec<(String, u16)>,
    /// Path markers answered 404.
    missing: Vec<String>,
    delay: Option<Duration>,
}

/// The Python `_FakeClient`s, as a wiremock responder.
#[derive(Clone)]
struct FakeOl {
    state: Arc<Mutex<OlState>>,
}

impl FakeOl {
    async fn mount(app: &TestApp) -> Self {
        Self::mount_with(app, circe_work(), circe_doc()).await
    }

    async fn mount_with(app: &TestApp, work: Value, doc: Value) -> Self {
        let fake = FakeOl {
            state: Arc::new(Mutex::new(OlState {
                work,
                doc,
                author_name: "Madeline Miller".to_owned(),
                edition: json!({"covers": [777]}),
                status_for: Vec::new(),
                missing: Vec::new(),
                delay: None,
            })),
        };
        Mock::given(wiremock::matchers::any())
            .respond_with(fake.clone())
            .mount(&app.upstream)
            .await;
        fake
    }

    fn set_work(&self, work: Value) {
        self.state.lock().unwrap().work = work;
    }

    fn set_doc(&self, doc: Value) {
        self.state.lock().unwrap().doc = doc;
    }

    fn set_author(&self, name: &str) {
        self.state.lock().unwrap().author_name = name.to_owned();
    }

    fn status_for(&self, marker: &str, status: u16) {
        self.state
            .lock()
            .unwrap()
            .status_for
            .push((marker.to_owned(), status));
    }

    fn clear_status(&self) {
        self.state.lock().unwrap().status_for.clear();
    }

    fn missing(&self, marker: &str) {
        self.state.lock().unwrap().missing.push(marker.to_owned());
    }

    fn delay(&self, delay: Duration) {
        self.state.lock().unwrap().delay = Some(delay);
    }
}

impl Respond for FakeOl {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let state = self.state.lock().unwrap();
        let path = request.url.path().to_owned();
        let delayed = |template: ResponseTemplate| match state.delay {
            Some(delay) => template.set_delay(delay),
            None => template,
        };
        if let Some((_, status)) = state.status_for.iter().find(|(m, _)| path.contains(m)) {
            return ResponseTemplate::new(*status);
        }
        if state.missing.iter().any(|m| path.contains(m)) {
            return ResponseTemplate::new(404);
        }
        let q: String = request
            .url
            .query_pairs()
            .find(|(k, _)| k == "q")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        let body = if path.contains("search.json") {
            if q.starts_with("key:") {
                if state.doc.is_null() {
                    json!({"numFound": 0, "docs": []})
                } else {
                    json!({"numFound": 1, "docs": [state.doc.clone()]})
                }
            } else {
                json!({"numFound": 50, "docs": [circe_doc()]})
            }
        } else if path.contains("trending") {
            json!({"works": trending_works()})
        } else if path.contains("/authors/") {
            json!({"name": state.author_name})
        } else if path.contains("/works/") {
            state.work.clone()
        } else if path.contains("/isbn/") {
            state.edition.clone()
        } else {
            return ResponseTemplate::new(404);
        };
        delayed(ResponseTemplate::new(200).set_body_json(body))
    }
}

async fn setup(db: PgPool) -> (TestApp, FakeOl, TestClient) {
    let app = TestApp::with_real_lookup(db).await;
    let fake = FakeOl::mount(&app).await;
    let ada = app.user("ada").await;
    (app, fake, ada)
}

/// Upstream requests whose path contains `needle`. Every request carries the Bookclub
/// User-Agent, as the Python fakes asserted.
async fn calls(app: &TestApp, needle: &str) -> usize {
    let requests = app.upstream_requests().await;
    for request in &requests {
        let agent = request
            .headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(agent.contains("Bookclub/1.0"), "user agent {agent:?}");
    }
    requests
        .iter()
        .filter(|r| r.url.path().contains(needle))
        .count()
}

async fn total_calls(app: &TestApp) -> usize {
    app.upstream_requests().await.len()
}

async fn work_id_of(app: &TestApp, ol_work_id: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM works WHERE ol_work_id = $1")
        .bind(ol_work_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// The Open Library cover id behind a `cover_public_id` (BookDetailOut has no `cover_id`).
async fn ol_cover(app: &TestApp, body: &Value) -> Option<i64> {
    let public_id = body["cover_public_id"].as_str()?;
    sqlx::query_scalar("SELECT ol_cover_id FROM media WHERE public_id = $1")
        .bind(public_id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// An Open Library work whose full record was fetched before (a known book): only the given
/// facts, `details_fetched_at` set.
async fn known_ol_work(
    app: &TestApp,
    ol_work_id: &str,
    title: &str,
    year: Option<i32>,
    cover: Option<i64>,
) -> i64 {
    let record = WorkRecord {
        ol_work_id: Some(ol_work_id.to_owned()),
        author_names: "Madeline Miller".to_owned(),
        first_publish_year: year,
        ol_cover_id: cover,
        full_details: true,
        ..WorkRecord::new(Source::OpenLibrary, title)
    };
    let mut conn = app.db.acquire().await.unwrap();
    ingest::upsert_work(&mut conn, &record, Mode::Refresh)
        .await
        .unwrap()
}

/// A shelf entry inserted directly (SHELF owns `POST /api/shelf`).
async fn shelve(app: &TestApp, who: &TestClient, work_id: i64, status: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(who.id())
    .bind(work_id)
    .bind(status)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// A current pick inserted directly (CLUB owns the pick routes).
async fn pick(app: &TestApp, club: &TestClub, work_id: i64) {
    sqlx::query("INSERT INTO club_picks (club_id, work_id) VALUES ($1, $2)")
        .bind(club.id)
        .bind(work_id)
        .execute(&app.db)
        .await
        .unwrap();
}

fn reader_names(body: &Value) -> Vec<String> {
    body["readers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["username"].as_str().unwrap().to_owned())
        .collect()
}

/// parity: test_search.py::test_first_import_skips_author_api_when_search_has_names
#[sqlx::test]
async fn first_import_skips_author_api_when_search_has_names(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let body = ada.get("/api/books/works/OL1W").await;
    body.assert_status(200);
    assert_eq!(body.json()["authors"], "Madeline Miller");
    assert_eq!(calls(&app, "/authors/").await, 0);
}

/// parity: test_search.py::test_book_detail_resolves_description_subjects_and_authors
/// Adapted: `cover_id` is gone; the cover behind `cover_public_id` is Open Library cover 555.
/// parity: test_google_books.py::test_detail_imports_from_google_not_open_library
/// (KEEP: the second open of a known book is served locally with zero upstream calls.)
#[sqlx::test]
async fn book_detail_resolves_description_subjects_and_authors(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let res = ada.get("/api/books/works/OL1W").await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["ol_work_key"], "/works/OL1W");
    assert_eq!(body["title"], "Circe");
    assert_eq!(body["description"], "A plain string description.");
    // Deduplicated case-insensitively; blanks and machine tags dropped.
    assert_eq!(body["subjects"], json!(["Mythology", "Greek literature"]));
    // The first positive cover wins; -1 is skipped.
    assert_eq!(ol_cover(&app, &body).await, Some(555));
    let cover_url = body["cover_url"].as_str().unwrap();
    assert!(cover_url.starts_with("/media/") && cover_url.ends_with("/l"));
    assert_eq!(body["authors"], "Madeline Miller");
    assert_eq!(body["on_shelf"], Value::Null);
    assert_eq!(body["readers"], json!([]));
    assert_eq!(body["source"], "openlibrary");
    assert_eq!(body["custom"], false);
    assert_eq!(body["can_refresh"], true);

    // The first view imports the work; later reads are local even without the RAM caches.
    app.state.catalog.clear_caches();
    let before = total_calls(&app).await;
    let again = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(again["description"], "A plain string description.");
    assert_eq!(again["subjects"], json!(["Mythology", "Greek literature"]));
    assert_eq!(again["id"], body["id"]);
    let by_id = ada.get(&format!("/api/books/works/{}", body["id"])).await;
    by_id.assert_status(200);
    assert_eq!(total_calls(&app).await, before);
}

/// parity: test_search.py::test_book_detail_accepts_dict_description
#[sqlx::test]
async fn book_detail_accepts_dict_description(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    fake.set_work(json!({
        "title": "Circe",
        "description": {"type": "/type/text", "value": "  A dict description.  "}
    }));
    let body = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(body["description"], "A dict description.");
    assert_eq!(body["subjects"], json!([]));
    // The work JSON has no covers; the search doc still has cover_i.
    assert_eq!(ol_cover(&app, &body).await, Some(123));
}

/// parity: test_search.py::test_book_detail_flattens_markdown_in_description
#[sqlx::test]
async fn book_detail_flattens_markdown_in_description(db: PgPool) {
    let (_app, fake, ada) = setup(db).await;
    fake.set_work(json!({
        "title": "Circe",
        "description": "[Comment by Kim Stanley Robinson][1] and [a note](http://example.com):\n\
                        \n\
                        > One of my favorite novels.\n\
                        > Truly.\n\
                        \n\
                        \n\
                        \n\
                        The end.\n\
                        \n\
                        [1]: http://example.com/robinson\n"
    }));
    let body = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(
        body["description"],
        "Comment by Kim Stanley Robinson and a note:\n\nOne of my favorite novels.\nTruly.\n\nThe end."
    );
}

/// parity: test_search.py::test_book_detail_includes_own_shelf_state_and_other_readers
/// Adapted: the readers share a club (the audience, #46) and a stranger's entry is left out. The
/// Python stub came from the shelf add with client metadata; here the known work carries those
/// facts (year 2018, cover 123, no description) and its full record was fetched before, so the
/// view makes no outbound call.
#[sqlx::test]
async fn book_detail_includes_own_shelf_state_and_other_readers(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let grace = app.user("grace").await;
    let eve = app.user("eve").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let work = known_ol_work(&app, "OL1W", "Circe", Some(2018), Some(123)).await;
    let ada_entry = shelve(&app, &ada, work, "currently_reading").await;
    app.exec(&format!(
        "UPDATE shelf_entries SET progress = 40 WHERE id = {ada_entry}"
    ))
    .await;
    let grace_entry = shelve(&app, &grace, work, "finished").await;
    app.exec(&format!(
        "UPDATE shelf_entries SET rating = 5, take = 'Loved it' WHERE id = {grace_entry}"
    ))
    .await;
    shelve(&app, &eve, work, "finished").await;

    let res = grace.get("/api/books/works/OL1W").await;
    res.assert_status(200);
    assert_eq!(total_calls(&app).await, 0);
    let body = res.json();
    assert_eq!(body["on_shelf"], "finished");
    assert_eq!(body["shelf_id"], grace_entry);
    assert_eq!(body["rating"], 5);
    assert_eq!(body["take"], "Loved it");
    assert_eq!(body["rating_visibility"], "audience");
    assert_eq!(reader_names(&body), ["ada"]);
    assert_eq!(body["readers"][0]["status"], "currently_reading");
    assert_eq!(body["readers"][0]["progress"], 40);
    assert_eq!(body["readers"][0]["rating"], Value::Null);
    // Known book: local fields only.
    assert_eq!(body["year"], 2018);
    assert_eq!(ol_cover(&app, &body).await, Some(123));
    assert_eq!(body["title"], "Circe");
    assert_eq!(body["description"], "");
    assert_eq!(body["club_rating"], 5.0);
    assert_eq!(body["rating_count"], 1);
}

/// parity: test_search.py::test_book_detail_rejects_malformed_work_id
#[sqlx::test]
async fn book_detail_rejects_malformed_work_id(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    for raw in ["not-a-key", "OL1M", "ol1w", "99999999999999999999"] {
        ada.get(&format!("/api/books/works/{raw}"))
            .await
            .assert_detail(404, "No such book.");
    }
    // A numeric id or a legacy key that names no work.
    ada.get("/api/books/works/424242")
        .await
        .assert_detail(404, "No such book.");
    ada.get("/api/books/works/BCdeadbeef01")
        .await
        .assert_detail(404, "No such book.");
    assert_eq!(total_calls(&app).await, 0);
}

/// parity: test_search.py::test_book_detail_propagates_not_found
#[sqlx::test]
async fn book_detail_propagates_not_found(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    fake.missing("/works/");
    ada.get("/api/books/works/OL404W")
        .await
        .assert_detail(404, "No such book.");
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM works WHERE ol_work_id = 'OL404W'")
            .await,
        0
    );
}

/// parity: test_search.py::test_detail_survives_author_lookup_failure
#[sqlx::test]
async fn detail_survives_author_lookup_failure(db: PgPool) {
    let (_app, fake, ada) = setup(db).await;
    fake.missing("/authors/");
    let body = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(body["title"], "Circe");
    // The author API failed; the search doc still has the name.
    assert_eq!(body["authors"], "Madeline Miller");
}

/// parity: test_search.py::test_book_detail_refresh_updates_local_row
/// Adapted: the title-only work is a known Open Library work whose record had nothing else (the
/// legacy shelf stub has no counterpart: shelving needs an existing work). `/work/OL1W` is
/// merged into `/works/` (`pages`, `ol_rating`), and the shelf's `BookOut` is the shared loader.
/// parity: test_google_books.py::test_refresh_updates_google_isbn_book
/// (ADAPT: refreshing a bare record from a CC0 source fills the missing facts.)
#[sqlx::test]
async fn book_detail_refresh_updates_local_row(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let work = known_ol_work(&app, "OL1W", "Circe", None, None).await;
    shelve(&app, &ada, work, "currently_reading").await;
    let local = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(local["description"], "");
    assert_eq!(local["year"], Value::Null);
    assert_eq!(total_calls(&app).await, 0);

    let refreshed = ada.post("/api/books/works/OL1W/refresh", json!({})).await;
    refreshed.assert_status(200);
    let body = refreshed.json();
    assert_eq!(body["description"], "A plain string description.");
    assert_eq!(body["subjects"], json!(["Mythology", "Greek literature"]));
    assert_eq!(ol_cover(&app, &body).await, Some(555));
    assert_eq!(body["year"], 2018);
    assert_eq!(body["pages"], 393);
    assert_eq!(body["ol_rating"], 4.27);
    assert_eq!(body["ol_rating_count"], 812);
    assert_eq!(body["on_shelf"], "currently_reading");
    assert!(calls(&app, "/works/OL1W.json").await >= 1);
    assert!(calls(&app, "search.json").await >= 1);

    let mut conn = app.db.acquire().await.unwrap();
    let shelf_book = book_out(&mut conn, BookRef::work(work))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(shelf_book.pages, Some(393));

    app.state.catalog.clear_caches();
    let before = total_calls(&app).await;
    let again = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(again["description"], "A plain string description.");
    assert_eq!(again["year"], 2018);
    assert_eq!(total_calls(&app).await, before);
}

/// parity: test_search.py::test_refresh_bypasses_author_cache
#[sqlx::test]
async fn refresh_bypasses_author_cache(db: PgPool) {
    let (_app, fake, ada) = setup(db).await;
    let first = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(first["authors"], "Madeline Miller");
    fake.set_author("M. Miller");
    let refreshed = ada.post("/api/books/works/OL1W/refresh", json!({})).await;
    refreshed.assert_status(200);
    assert_eq!(refreshed.json()["authors"], "M. Miller");
}

/// parity: test_book_details.py::test_work_details_for_unknown_book
/// Adapted: `/work/{id}` is merged into `/works/{id}` (`pages`, `ol_rating`,
/// `ol_rating_count`); `members` became `readers` (others in the audience), `club_pick` became
/// `club_picks`, and the cover is a same-origin `/media/` URL of Open Library cover 123.
#[sqlx::test]
async fn work_details_for_unknown_book(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    FakeOl::mount_with(&app, details_work(), details_doc()).await;
    let ada = app.user("ada").await;
    let res = ada.get("/api/books/works/OL1W").await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["title"], "Circe");
    assert_eq!(body["authors"], "Madeline Miller");
    assert_eq!(body["pages"], 393);
    assert_eq!(body["ol_rating"], 4.27);
    assert_eq!(body["ol_rating_count"], 812);
    assert_eq!(body["description"], "A witch on an island.");
    // Subjects: deduplicated, long and comma-laden entries dropped.
    assert_eq!(
        body["subjects"],
        json!(["Greek mythology", "Fiction", "Witches"])
    );
    assert_eq!(body["on_shelf"], Value::Null);
    assert_eq!(body["readers"], json!([]));
    assert_eq!(body["club_picks"], json!([]));
    assert_eq!(ol_cover(&app, &body).await, Some(123));
    assert!(body["cover_thumb_url"].as_str().unwrap().ends_with("/s"));
    // The work JSON and its search doc; the doc has names, so no author request.
    assert_eq!(total_calls(&app).await, 2);
    // Every documented field is present.
    let keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for key in [
        "id",
        "ol_work_key",
        "title",
        "subtitle",
        "authors",
        "year",
        "cover_url",
        "cover_thumb_url",
        "cover_public_id",
        "description",
        "subjects",
        "pages",
        "ol_rating",
        "ol_rating_count",
        "source",
        "custom",
        "editions_count",
        "on_shelf",
        "shelf_id",
        "edition_id",
        "rating",
        "rating_visibility",
        "take",
        "dnf_reason",
        "progress",
        "readers",
        "club_rating",
        "rating_count",
        "quote_count",
        "favorite_position",
        "club_picks",
        "can_refresh",
    ] {
        assert!(keys.contains(&key), "missing {key}");
    }
    assert_eq!(keys.len(), 32);

    // The first view imports the work; later reads are local even without the RAM caches.
    app.state.catalog.clear_caches();
    ada.get("/api/books/works/OL1W").await.assert_status(200);
    assert_eq!(total_calls(&app).await, 2);
}

/// parity: test_book_details.py::test_work_details_persist_on_known_book_and_list_members
/// Adapted: both readers share a club; the viewer's own row is the top-level shelf state and
/// `readers` lists the others (the merged DTO has no viewer-first `members`); `club_pick` is
/// `club_picks`; the shelf's `pages` is read through the shared `BookOut` loader.
#[sqlx::test]
async fn work_details_persist_on_known_book_and_list_members(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    FakeOl::mount_with(&app, details_work(), details_doc()).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let work = known_ol_work(&app, "OL1W", "Circe", Some(2018), Some(123)).await;
    let ada_entry = shelve(&app, &ada, work, "finished").await;
    app.exec(&format!(
        "UPDATE shelf_entries SET rating = 5, take = 'Loved it', finished_at = now()
         WHERE id = {ada_entry}"
    ))
    .await;
    let grace_entry = shelve(&app, &grace, work, "currently_reading").await;
    app.exec(&format!(
        "UPDATE shelf_entries SET progress = 40 WHERE id = {grace_entry}"
    ))
    .await;
    pick(&app, &club, work).await;

    let body = grace.get("/api/books/works/OL1W").await.json();
    assert_eq!(total_calls(&app).await, 0);
    assert_eq!(
        body["club_picks"],
        json!([{"slug": club.slug, "name": "Readers"}])
    );
    assert_eq!(body["on_shelf"], "currently_reading");
    assert_eq!(body["shelf_id"], grace_entry);
    assert_eq!(body["progress"], 40);
    assert_eq!(
        body["readers"],
        json!([{"username": "ada", "status": "finished", "rating": 5, "progress": null,
                "avatar_url": null}])
    );
    assert_eq!(body["pages"], Value::Null);

    let refreshed = grace.post("/api/books/works/OL1W/refresh", json!({})).await;
    refreshed.assert_status(200);
    assert_eq!(refreshed.json()["pages"], 393);
    let mut conn = app.db.acquire().await.unwrap();
    let shelf_book = book_out(&mut conn, BookRef::work(work))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(shelf_book.pages, Some(393));

    app.state.catalog.clear_caches();
    let before = total_calls(&app).await;
    grace.get("/api/books/works/OL1W").await.assert_status(200);
    assert_eq!(total_calls(&app).await, before);
}

/// parity: test_book_details.py::test_work_details_rejects_bad_ids_and_upstream_failure
/// Adapted: `/work/not-a-key` was 400 on the dropped `/work/` route; `/works/` answers 404.
/// A connect error on the import is 502.
#[sqlx::test]
async fn work_details_rejects_bad_ids_and_upstream_failure(db: PgPool) {
    // Nothing listens on port 9: every Open Library request fails to connect.
    let app = TestApp::with_env(db, &[("OPENLIBRARY_URL", "http://127.0.0.1:9/")]).await;
    let ada = app.user("ada").await;
    ada.get("/api/books/works/not-a-key")
        .await
        .assert_detail(404, "No such book.");
    ada.get("/api/books/works/OL9W")
        .await
        .assert_detail(502, "Could not load the library right now. Try again.");
}

/// parity: test_book_details.py::test_work_details_skips_leading_negative_cover
#[sqlx::test]
async fn work_details_skips_leading_negative_cover(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let mut work = details_work();
    work["covers"] = json!([-1, 0, 555]);
    FakeOl::mount_with(
        &app,
        work,
        json!({"key": "/works/OL1W", "title": "Circe", "author_name": ["Madeline Miller"],
               "cover_i": -1, "first_publish_year": 2018}),
    )
    .await;
    let ada = app.user("ada").await;
    let body = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(ol_cover(&app, &body).await, Some(555));
    assert!(body["cover_url"].as_str().unwrap().starts_with("/media/"));
}

/// parity: test_book_details.py::test_cancelled_work_details_unblocks_coalesced_waiters
/// Reasoned change (decision 16): when the caller running the coalesced fetch is cancelled, the
/// waiting caller runs the fetch itself instead of failing with a 502; either way it never
/// hangs.
#[sqlx::test]
async fn cancelled_work_details_unblock_coalesced_waiters(db: PgPool) {
    let app = TestApp::new(db).await;
    let fake = FakeOl::mount(&app).await;
    fake.delay(Duration::from_millis(400));
    let catalog = app.state.catalog.clone();
    let owner = tokio::spawn({
        let catalog = catalog.clone();
        async move { catalog.ol_work_record("OL1W", false).await }
    });
    for _ in 0..200 {
        if calls(&app, "/works/OL1W.json").await > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let follower = tokio::spawn({
        let catalog = catalog.clone();
        async move { catalog.ol_work_record("OL1W", false).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    owner.abort();
    assert!(owner.await.unwrap_err().is_cancelled());
    let record = tokio::time::timeout(Duration::from_secs(10), follower)
        .await
        .expect("the follower never hangs")
        .unwrap()
        .expect("the follower fetches the work itself");
    assert_eq!(record.title, "Circe");
}

/// parity: test_local_reads.py::test_known_book_reads_never_touch_open_library
/// The catalog half: with a shelf entry, a pick, a diary entry, a quote and a nomination on a
/// known book (inserted directly; their routes belong to SHELF, POSTS and CLUB), the detail (by
/// id, by `OL…W`), its editions and the ISBN lookup of its edition make no outbound request.
/// `/api/books/work/OL1W` is merged into `/works/`.
#[sqlx::test]
async fn known_book_reads_never_touch_open_library(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let work = known_ol_work(&app, "OL1W", "Circe", Some(2018), Some(123)).await;
    let mut conn = app.db.acquire().await.unwrap();
    let edition = ingest::upsert_edition(
        &mut conn,
        work,
        &ingest::EditionRecord {
            isbn13: bookclub_server::catalog::shared::isbn::parse_isbn("9780316556347"),
            ..ingest::EditionRecord::new(Source::OpenLibrary)
        },
        Mode::Refresh,
    )
    .await
    .unwrap()
    .1;
    drop(conn);
    let entry = shelve(&app, &ada, work, "currently_reading").await;
    app.exec(&format!(
        "UPDATE shelf_entries SET progress = 20 WHERE id = {entry}"
    ))
    .await;
    pick(&app, &club, work).await;
    app.exec(&format!(
        "INSERT INTO posts (work_id, author_id, body, spoiler_upto) VALUES ({work}, {}, 'Halfway through.', 40);
         INSERT INTO quotes (user_id, work_id, body) VALUES ({}, {work}, 'But in a story');
         INSERT INTO next_up_votes (club_id) VALUES ({});
         INSERT INTO next_up_nominations (club_id, vote_id, work_id, nominated_by)
             SELECT {}, id, {work}, {} FROM next_up_votes WHERE club_id = {};",
        ada.id(),
        ada.id(),
        club.id,
        club.id,
        ada.id(),
        club.id
    ))
    .await;

    for path in [
        format!("/api/books/works/{work}"),
        "/api/books/works/OL1W".to_owned(),
        format!("/api/books/works/{work}/editions"),
        format!("/api/books/editions/{edition}"),
        "/api/books/isbn/9780316556347".to_owned(),
    ] {
        ada.get(&path).await.assert_status(200);
    }
    let body = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(body["quote_count"], 1);
    assert_eq!(body["editions_count"], 1);
    assert_eq!(total_calls(&app).await, 0);
}

/// parity: test_local_reads.py::test_shelving_persists_search_metadata_for_later_local_reads
/// Adapted: every search hit is persisted, so the search metadata (title, authors, cover 123,
/// year 2018) is stored by the search itself; shelving needs no client metadata. With Open
/// Library failing, the first detail view's enrichment fails and the stored hit is served.
#[sqlx::test]
async fn search_metadata_serves_later_local_reads(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    ada.get("/api/books/search?q=circe")
        .await
        .assert_status(200);
    let work = work_id_of(&app, "OL1W").await;
    shelve(&app, &ada, work, "want_to_read").await;
    fake.status_for("/", 503);

    let res = ada.get("/api/books/works/OL1W").await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["title"], "Circe");
    assert_eq!(body["authors"], "Madeline Miller");
    assert_eq!(ol_cover(&app, &body).await, Some(123));
    assert_eq!(body["year"], 2018);
    assert_eq!(body["on_shelf"], "want_to_read");
    assert_eq!(body["description"], "");
}

/// Opens every trending hit while Open Library answers `status` to everything.
async fn open_trending_during_outage(db: PgPool, status: u16) -> (TestApp, FakeOl, TestClient) {
    let (app, fake, ada) = setup(db).await;
    let trending = ada.get("/api/books/trending?limit=14").await;
    trending.assert_status(200);
    let ids: Vec<i64> = trending.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    fake.status_for("/", status);
    for (id, title) in ids.iter().zip(["Atomic Habits", "No cover"]) {
        let res = ada.get(&format!("/api/books/works/{id}")).await;
        res.assert_status(200);
        assert_eq!(res.json()["title"], title);
        assert_eq!(res.json()["id"], *id);
    }
    (app, fake, ada)
}

/// parity: test_google_books.py::test_opening_discovered_book_does_not_call_google
/// (the open half; KEEP: opening a displayed result never needs an upstream call and works
/// during an outage. Adapted: the stored hit's one-time enrichment is attempted, fails, and the
/// hit is served; a 503 opens no cooldown.)
/// parity: test_google_books.py::test_empty_subject_preview_recovers_with_google_topic_search
/// (the open half, ADAPT: a displayed browse result opens from the stored hit.)
#[sqlx::test]
async fn a_displayed_hit_opens_during_a_503_outage(db: PgPool) {
    let (app, fake, ada) = open_trending_during_outage(db, 503).await;
    // No cooldown: the next request reaches Open Library again.
    fake.clear_status();
    let before = total_calls(&app).await;
    ada.get("/api/books/search?q=dune").await.assert_status(200);
    assert!(total_calls(&app).await > before);
}

/// The same with Open Library answering 429: the hit is still served.
#[sqlx::test]
async fn a_displayed_hit_opens_while_open_library_is_busy(db: PgPool) {
    open_trending_during_outage(db, 429).await;
}

/// parity: test_google_books.py::test_title_hit_opens_even_when_google_isbn_lookup_misses
/// (KEEP: opening a search result does not depend on a second lookup succeeding. The stored hit
/// opens although its work record is gone upstream.)
#[sqlx::test]
async fn a_search_hit_opens_when_the_second_lookup_misses(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    let hits = ada.get("/api/books/search?q=circe").await.json();
    let id = hits["items"][0]["id"].as_i64().unwrap();
    fake.missing("/works/");
    fake.set_doc(Value::Null);
    let res = ada.get(&format!("/api/books/works/{id}")).await;
    res.assert_status(200);
    assert_eq!(res.json()["title"], "Circe");
    assert!(calls(&app, "/works/OL1W.json").await >= 1);
}

/// parity: test_google_books.py::test_discovered_book_opens_during_existing_google_cooldown
/// (ADAPT: with persist-on-search, opening is local; during a cooldown the enrichment fails at
/// once, without a request.)
#[sqlx::test]
async fn a_discovered_book_opens_during_a_cooldown(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    let trending = ada.get("/api/books/trending").await.json();
    let id = trending["items"][0]["id"].as_i64().unwrap();
    fake.status_for("search.json", 429);
    ada.get("/api/books/search?q=uncached")
        .await
        .assert_status(429);
    let before = total_calls(&app).await;
    let res = ada.get(&format!("/api/books/works/{id}")).await;
    res.assert_status(200);
    assert_eq!(res.json()["title"], "Atomic Habits");
    assert_eq!(total_calls(&app).await, before);
}

/// parity: test_google_books.py::test_discovered_book_refresh_still_fetches_fresh_metadata
/// (ADAPT: a refresh always goes to the source and bypasses the caches.)
#[sqlx::test]
async fn refresh_fetches_fresh_metadata(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    let trending = ada.get("/api/books/trending").await.json();
    let id = trending["items"][0]["id"].as_i64().unwrap();
    let mut work = circe_work();
    work["title"] = json!("Atomic Habits");
    work["description"] = json!("Old synopsis.");
    fake.set_work(work.clone());
    let first = ada.get(&format!("/api/books/works/{id}")).await.json();
    assert_eq!(first["description"], "Old synopsis.");
    let fetched = calls(&app, "/works/OL7W.json").await;

    work["description"] = json!("Updated synopsis.");
    fake.set_work(work);
    let local = ada.get(&format!("/api/books/works/{id}")).await.json();
    assert_eq!(local["description"], "Old synopsis.");
    let refreshed = ada
        .post(&format!("/api/books/works/{id}/refresh"), json!({}))
        .await;
    refreshed.assert_status(200);
    assert_eq!(refreshed.json()["description"], "Updated synopsis.");
    assert_eq!(calls(&app, "/works/OL7W.json").await, fetched + 1);
}

/// parity: test_google_books.py::test_isbn_refresh_keeps_description_when_google_misses
/// (KEEP: a refresh miss never wipes the local record: an empty record keeps every field, and
/// a work Open Library no longer has is 404 with the row untouched.)
#[sqlx::test]
async fn a_refresh_miss_never_wipes_the_record(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    let first = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(ol_cover(&app, &first).await, Some(555));

    fake.set_work(json!({"title": "Circe"}));
    fake.set_doc(Value::Null);
    let res = ada.post("/api/books/works/OL1W/refresh", json!({})).await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["description"], "A plain string description.");
    assert_eq!(body["subjects"], json!(["Mythology", "Greek literature"]));
    assert_eq!(body["cover_public_id"], first["cover_public_id"]);
    assert_eq!(body["year"], 2018);
    assert_eq!(body["authors"], "Madeline Miller");

    fake.missing("/works/");
    ada.post("/api/books/works/OL1W/refresh", json!({}))
        .await
        .assert_detail(404, "No such book.");
    let again = ada.get("/api/books/works/OL1W").await.json();
    assert_eq!(again["description"], "A plain string description.");
}

/// parity: test_google_books.py::test_isbn_refresh_without_key_keeps_description
/// (ADAPT: a source record without a description does not wipe the stored one; None is not
/// empty. Its other values still update.)
#[sqlx::test]
async fn a_refresh_without_a_description_keeps_it(db: PgPool) {
    let (_app, fake, ada) = setup(db).await;
    ada.get("/api/books/works/OL1W").await.assert_status(200);
    fake.set_work(json!({"title": "Circe: A Novel", "subjects": ["Witches"]}));
    let body = ada
        .post("/api/books/works/OL1W/refresh", json!({}))
        .await
        .json();
    assert_eq!(body["title"], "Circe: A Novel");
    assert_eq!(body["subjects"], json!(["Witches"]));
    assert_eq!(body["description"], "A plain string description.");
}

/// parity: test_google_books.py::test_isbn_x_and_X_are_the_same_row
/// (the legacy-key half, KEEP: `ISBN…x` and `ISBN…X` are one key, through `legacy_work_keys`.
/// `BC…` and `GB…` keys resolve the same way; a GB key is stored hashed.)
#[sqlx::test]
async fn legacy_keys_resolve_through_the_key_map(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let isbn_work = app.work("The Hobbit").await;
    let bc_work = app.work("My Zine").await;
    let gb_work = app.work("Local Zine").await;
    let gb_key = bookclub_server::catalog::shared::work_ref::legacy_key("GBabcVolumeId1");
    app.exec(&format!(
        "INSERT INTO legacy_work_keys (legacy_key, legacy_book_id, work_id, match) VALUES
             ('/works/ISBN080442957X', 1, {}, 'isbn_unmatched'),
             ('/works/BCdeadbeef01', 2, {}, 'user'),
             ('{gb_key}', 3, {}, 'google_volume')",
        isbn_work.id, bc_work.id, gb_work.id
    ))
    .await;
    for raw in ["ISBN080442957X", "ISBN080442957x", "isbn080442957x"] {
        let res = ada.get(&format!("/api/books/works/{raw}")).await;
        res.assert_status(200);
        assert_eq!(res.json()["id"], isbn_work.id);
        assert_eq!(res.json()["ol_work_key"], Value::Null);
    }
    assert_eq!(
        ada.get("/api/books/works/BCdeadbeef01").await.json()["id"],
        bc_work.id
    );
    assert_eq!(
        ada.get("/api/books/works/GBabcVolumeId1").await.json()["id"],
        gb_work.id
    );
    ada.get("/api/books/works/GBabcvolumeid1")
        .await
        .assert_detail(404, "No such book.");
    assert_eq!(total_calls(&app).await, 0);
}

/// parity: test_google_books.py::test_volume_refresh_429_is_classified
/// (ADAPT: an upstream 429 during a refresh is a 429 "The library is busy…".)
#[sqlx::test]
async fn a_429_during_refresh_is_busy(db: PgPool) {
    let (_app, fake, ada) = setup(db).await;
    ada.get("/api/books/works/OL1W").await.assert_status(200);
    fake.status_for("/works/", 429);
    let res = ada.post("/api/books/works/OL1W/refresh", json!({})).await;
    res.assert_status(429);
    assert!(res.detail().contains("busy"), "{}", res.detail());
}

/// A stub (a stored search hit) is enriched once, on its first detail view: one fetch of its
/// record, then never again; a failed enrichment serves the stub and the next view tries again.
#[sqlx::test]
async fn a_stub_is_enriched_once_on_its_first_view(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    ada.get("/api/books/search?q=circe")
        .await
        .assert_status(200);
    let work = work_id_of(&app, "OL1W").await;
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {work} AND details_fetched_at IS NULL"
        ))
        .await,
        1
    );
    fake.status_for("/works/", 503);
    let stub = ada.get(&format!("/api/books/works/{work}")).await;
    stub.assert_status(200);
    assert_eq!(stub.json()["description"], "");
    fake.clear_status();

    let full = ada.get(&format!("/api/books/works/{work}")).await.json();
    assert_eq!(full["description"], "A plain string description.");
    let fetched = calls(&app, "/works/OL1W.json").await;
    app.state.catalog.clear_caches();
    ada.get(&format!("/api/books/works/{work}"))
        .await
        .assert_status(200);
    assert_eq!(calls(&app, "/works/OL1W.json").await, fetched);
}

/// A DNB work gets one Open Library cover check by ISBN on its first detail view
/// (conventions §7.4): the cover becomes its edition's and the work's, and the check is not
/// repeated. Without an answer the check stays open.
#[sqlx::test]
async fn a_dnb_work_gets_one_cover_check_on_its_first_view(db: PgPool) {
    let (app, fake, ada) = setup(db).await;
    let (work, edition) = app
        .dnb_work(
            "Der Mörder von Whitechapel",
            "Patrice McDonough",
            "9783694050797",
        )
        .await;
    fake.status_for("/isbn/", 503);
    let first = ada.get(&format!("/api/books/works/{}", work.id)).await;
    first.assert_status(200);
    assert_eq!(first.json()["cover_url"], Value::Null);
    assert_eq!(first.json()["can_refresh"], false);
    assert_eq!(calls(&app, "/isbn/9783694050797.json").await, 2);
    fake.clear_status();

    let body = ada
        .get(&format!("/api/books/works/{}", work.id))
        .await
        .json();
    assert_eq!(ol_cover(&app, &body).await, Some(777));
    assert_eq!(body["source"], "dnb");
    let edition_cover = ada
        .get(&format!("/api/books/editions/{}", edition.id))
        .await
        .json();
    assert_eq!(edition_cover["cover_public_id"], body["cover_public_id"]);
    let checks = calls(&app, "/isbn/").await;
    app.state.catalog.clear_caches();
    ada.get(&format!("/api/books/works/{}", work.id))
        .await
        .assert_status(200);
    assert_eq!(calls(&app, "/isbn/").await, checks);
    // No other upstream was asked.
    assert_eq!(calls(&app, "/sru/").await, 0);
    assert_eq!(calls(&app, "/works/").await, 0);
}

/// Refresh is for Open Library works: user-added, DNB and legacy works are 400 with the new
/// string; an unknown `OL…W` id is imported; anything else is 404. 10 refreshes per hour.
#[sqlx::test]
async fn refresh_rules(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let user_work = app.work("My Zine").await;
    let (dnb_work, _) = app
        .dnb_work(
            "Der Mörder von Whitechapel",
            "Patrice McDonough",
            "9783694050797",
        )
        .await;
    let legacy = {
        let mut conn = app.db.acquire().await.unwrap();
        ingest::upsert_work(
            &mut conn,
            &WorkRecord::new(Source::Legacy, "Bare facts"),
            Mode::Refresh,
        )
        .await
        .unwrap()
    };
    for id in [user_work.id, dnb_work.id, legacy] {
        ada.post(&format!("/api/books/works/{id}/refresh"), json!({}))
            .await
            .assert_detail(400, "This book was added by a reader, not Open Library.");
    }
    ada.post("/api/books/works/not-a-key/refresh", json!({}))
        .await
        .assert_detail(404, "No such book.");
    ada.post("/api/books/works/424242/refresh", json!({}))
        .await
        .assert_detail(404, "No such book.");
    assert_eq!(total_calls(&app).await, 0);

    // An unknown Open Library id is imported by its refresh.
    let imported = ada.post("/api/books/works/OL1W/refresh", json!({})).await;
    imported.assert_status(200);
    assert_eq!(imported.json()["title"], "Circe");
    for _ in 0..9 {
        ada.post("/api/books/works/OL1W/refresh", json!({}))
            .await
            .assert_status(200);
    }
    let limited = ada.post("/api/books/works/OL1W/refresh", json!({})).await;
    limited.assert_detail(429, "Slow down a little. Try again later.");
    assert!(limited.header("retry-after").is_some());
    // Another user has their own budget.
    let grace = app.user("grace").await;
    grace
        .post("/api/books/works/OL1W/refresh", json!({}))
        .await
        .assert_status(200);
    // Reading is not limited.
    ada.get("/api/books/works/OL1W").await.assert_status(200);
}

/// A merged-away work answers with the work it was merged into, by id and by its old keys.
#[sqlx::test]
async fn a_merged_work_answers_with_its_target(db: PgPool) {
    let (app, _fake, ada) = setup(db).await;
    let loser = app.work("Circe (bare)").await;
    let winner = known_ol_work(&app, "OL1W", "Circe", Some(2018), None).await;
    app.exec(&format!(
        "UPDATE works SET merged_into_id = {winner} WHERE id = {}",
        loser.id
    ))
    .await;
    let res = ada.get(&format!("/api/books/works/{}", loser.id)).await;
    res.assert_status(200);
    assert_eq!(res.json()["id"], winner);
    assert_eq!(res.json()["title"], "Circe");
    assert_eq!(total_calls(&app).await, 0);
}

/// Anonymous callers get 401 before anything else.
#[sqlx::test]
async fn detail_and_refresh_need_a_session(db: PgPool) {
    let (app, _fake, _ada) = setup(db).await;
    app.anon()
        .get("/api/books/works/OL1W")
        .await
        .assert_detail(401, "Not signed in");
    app.anon()
        .post("/api/books/works/OL1W/refresh", json!({}))
        .await
        .assert_detail(401, "Not signed in");
    assert_eq!(total_calls(&app).await, 0);
}
