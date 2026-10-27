//! User-added books and editions (#41): `POST /api/books/custom`, the editions list, one
//! edition and `POST /api/books/works/{work_id}/editions`. With an ISBN the catalog and the
//! sources are asked first, and a hit is 409 (endpoint-map §4).

mod common;

use std::time::Duration;

use bookclub_server::catalog::merge_works;
use bookclub_server::catalog::shared::ingest::Source;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

const OL_SEARCH_CIRCE: &str = include_str!("fixtures/openlibrary/search_isbn_9780316556347.json");

/// Open Library search answers nothing (remote hits are not what these tests are about).
async fn mount_empty_search(app: &TestApp) {
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"numFound": 0, "docs": []})))
        .mount(&app.upstream)
        .await;
}

/// Open Library's search answers Circe for `q` (like the Python fake did for any text query),
/// ahead of the generic mocks.
async fn mount_circe_search(app: &TestApp, q: &str) {
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", q))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "numFound": 1,
            "docs": [{
                "key": "/works/OL1W",
                "title": "Circe",
                "author_name": ["Madeline Miller"],
                "first_publish_year": 2018
            }]
        })))
        .with_priority(1)
        .mount(&app.upstream)
        .await;
}

/// A 409 `That ISBN is already in the catalog`; its body (with `work_id` and `edition_id`).
fn assert_isbn_conflict(res: &TestResponse) -> Value {
    res.assert_status(409);
    assert_eq!(res.detail(), "That ISBN is already in the catalog");
    res.json()
}

async fn total_calls(app: &TestApp) -> usize {
    app.upstream_requests().await.len()
}

async fn zine(client: &TestClient) -> Value {
    let res = client
        .post(
            "/api/books/custom",
            json!({"title": "My Zine", "authors": "Ada", "year": 2024}),
        )
        .await;
    res.assert_status(201);
    res.json()
}

/// parity: test_search.py::test_local_catalog_is_searched_without_open_library_for_custom_books
/// Adapted: a user-added work has a numeric id and no `ol_work_key` (the legacy `/works/BC…`
/// key is gone; old BC links resolve through `legacy_work_keys`). Open Library answers the
/// search with Circe, as the Python fake did, and the catalog's own book comes first.
#[sqlx::test]
async fn local_catalog_is_searched_without_open_library_for_custom_books(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_empty_search(&app).await;
    mount_circe_search(&app, "zine").await;
    let ada = app.user("ada").await;
    let body = zine(&ada).await;
    assert_eq!(body["custom"], true);
    assert_eq!(body["source"], "user");
    assert_eq!(body["title"], "My Zine");
    assert_eq!(body["authors"], "Ada");
    assert_eq!(body["year"], 2024);
    assert_eq!(body["ol_work_key"], Value::Null);
    assert_eq!(body["can_refresh"], false);
    let id = body["id"].as_i64().unwrap();
    assert_eq!(total_calls(&app).await, 0);

    let local = ada.get(&format!("/api/books/works/{id}")).await;
    local.assert_status(200);
    assert_eq!(local.json()["custom"], true);
    assert_eq!(total_calls(&app).await, 0);

    let search = ada.get("/api/books/search?q=zine").await.json();
    assert_eq!(search["items"][0]["id"], id);
    assert_eq!(search["items"][0]["custom"], true);
    assert_eq!(search["items"][1]["ol_work_key"], "/works/OL1W");
    assert_eq!(search["items"].as_array().unwrap().len(), 2);

    let refresh = ada
        .post(&format!("/api/books/works/{id}/refresh"), json!({}))
        .await;
    refresh.assert_status(400);
    assert!(refresh.detail().contains("Open Library"));
}

/// parity: test_search.py::test_custom_book_requires_a_title
#[sqlx::test]
async fn custom_book_requires_a_title(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.post(
        "/api/books/custom",
        json!({"title": "   ", "authors": "Ada"}),
    )
    .await
    .assert_detail(400, "A title is required");
    ada.post("/api/books/custom", json!({"authors": "Ada"}))
        .await
        .assert_detail(400, "Field required");
    ada.post("/api/books/custom", json!({"title": 5}))
        .await
        .assert_detail(400, "Input should be a valid string");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM works").await, 0);
}

/// parity: test_google_books.py::test_club_custom_book_still_skips_catalogs
/// (KEEP: user-added books never call a catalog, and refreshing them is rejected with the new
/// string.)
#[sqlx::test]
async fn custom_books_skip_the_catalogs(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    let ada = app.user("ada").await;
    let id = zine(&ada).await["id"].as_i64().unwrap();
    let detail = ada.get(&format!("/api/books/works/{id}")).await;
    detail.assert_status(200);
    assert_eq!(detail.json()["custom"], true);
    ada.post(&format!("/api/books/works/{id}/refresh"), json!({}))
        .await
        .assert_detail(400, "This book was added by a reader, not Open Library.");
    assert_eq!(total_calls(&app).await, 0);
}

/// Every entered field is stored with provenance `user` and the creator; edition facts make an
/// edition. Values are trimmed, long text is cut (parity), numbers are range-checked with the
/// pydantic wording, and the book is visible to everyone at once.
#[sqlx::test]
async fn custom_book_fields_and_provenance(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    mount_empty_search(&app).await;
    let ada = app.user("ada").await;
    let long_title = format!("  {}  ", "T".repeat(600));
    let res = ada
        .post(
            "/api/books/custom",
            json!({"title": long_title, "authors": "  Ada Lovelace ", "year": 1843,
                   "description": "  Notes on the engine.  ", "publisher": " Taylor ",
                   "pages": 66, "language": " ENG "}),
        )
        .await;
    res.assert_status(201);
    let body = res.json();
    assert_eq!(body["title"].as_str().unwrap().chars().count(), 500);
    assert_eq!(body["authors"], "Ada Lovelace");
    assert_eq!(body["description"], "Notes on the engine.");
    assert_eq!(body["year"], 1843);
    assert_eq!(body["pages"], 66);
    assert_eq!(body["editions_count"], 1);
    let id = body["id"].as_i64().unwrap();
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {id} AND created_by = {}
               AND source = 'user' AND field_sources ->> 'title' = 'user'
               AND field_sources ->> 'description' = 'user'",
            ada.id()
        ))
        .await,
        1
    );
    let editions = ada
        .get(&format!("/api/books/works/{id}/editions"))
        .await
        .json();
    let edition = &editions["items"][0];
    assert_eq!(edition["publisher"], "Taylor");
    assert_eq!(edition["language"], "eng");
    assert_eq!(edition["pages"], 66);
    assert_eq!(edition["isbn13"], Value::Null);
    assert_eq!(edition["source"], "user");

    // Without edition facts, no edition.
    let bare = zine(&ada).await;
    assert_eq!(bare["editions_count"], 0);

    // Everyone finds it at once, ahead of Open Library's hits.
    mount_circe_search(&app, "lovelace").await;
    let grace = app.user("grace").await;
    let found = grace.get("/api/books/search?q=lovelace").await.json();
    assert_eq!(found["items"][0]["id"], id);
    assert_eq!(found["items"][1]["ol_work_key"], "/works/OL1W");

    for (body, detail) in [
        (
            json!({"title": "X", "year": 3001}),
            "Input should be less than or equal to 3000",
        ),
        (
            json!({"title": "X", "year": 0}),
            "Input should be greater than or equal to 1",
        ),
        (
            json!({"title": "X", "pages": 0}),
            "Input should be greater than or equal to 1",
        ),
        (
            json!({"title": "X", "language": "german"}),
            "String should match pattern '^[a-z]{3}$'",
        ),
        (
            json!({"title": "X", "isbn": "12345"}),
            "That is not a valid ISBN-10 or ISBN-13",
        ),
        (
            json!({"title": " ", "year": 0}),
            "A title is required; Input should be greater than or equal to 1",
        ),
    ] {
        ada.post("/api/books/custom", body)
            .await
            .assert_detail(400, detail);
    }
}

/// With an ISBN, the catalog and then the remote route are asked first: a local edition or a
/// remote hit is 409 with `work_id` and `edition_id`; a lookup without a definite answer is
/// 429 or 502 and adds nothing; a miss adds the book with that ISBN.
#[sqlx::test]
async fn custom_book_with_an_isbn_asks_the_sources_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (known, edition) = app
        .dnb_work("Der Hobbit", "J. R. R. Tolkien", "9783608938289")
        .await;
    let local = ada
        .post(
            "/api/books/custom",
            json!({"title": "Hobbit", "isbn": "978-3-608-93828-9"}),
        )
        .await;
    assert_isbn_conflict(&local);
    assert_eq!(local.json()["work_id"], known.id);
    assert_eq!(local.json()["edition_id"], edition.id);
    assert!(app.catalog.calls().is_empty());

    app.catalog.on_isbn(
        "9780316556347",
        FakeHit::New {
            title: "Circe".into(),
            authors: "Madeline Miller".into(),
            ol_work_id: Some("OL1W".into()),
            source: Source::OpenLibrary,
        },
    );
    let remote = ada
        .post(
            "/api/books/custom",
            json!({"title": "Circe (my copy)", "isbn": "0316556343"}),
        )
        .await;
    assert_isbn_conflict(&remote);
    let circe = remote.json()["work_id"].as_i64().unwrap();
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {circe} AND ol_work_id = 'OL1W'"
        ))
        .await,
        1
    );
    assert!(remote.json()["edition_id"].is_i64());
    assert_eq!(app.catalog.calls(), ["isbn_remote:9780316556347"]);

    app.catalog
        .fail_isbn("9780062060624", FakeError::Throttled, Some(1));
    ada.post(
        "/api/books/custom",
        json!({"title": "Achilles", "isbn": "9780062060624"}),
    )
    .await
    .assert_detail(
        429,
        "The library is busy. Wait a few seconds and try again.",
    );
    app.catalog
        .fail_isbn("9780062060624", FakeError::Unavailable, Some(1));
    ada.post(
        "/api/books/custom",
        json!({"title": "Achilles", "isbn": "9780062060624"}),
    )
    .await
    .assert_detail(502, "Could not look that ISBN up right now. Try again.");
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM works WHERE title = 'Achilles'")
            .await,
        0
    );

    // A miss: the reader's ISBN is stored on a user edition.
    let added = ada
        .post(
            "/api/books/custom",
            json!({"title": "Achilles", "isbn": "9780062060624"}),
        )
        .await;
    added.assert_status(201);
    let id = added.json()["id"].as_i64().unwrap();
    let editions = ada
        .get(&format!("/api/books/works/{id}/editions"))
        .await
        .json();
    assert_eq!(editions["items"][0]["isbn13"], "9780062060624");
    assert_eq!(editions["items"][0]["isbn10"], "0062060627");
    assert_eq!(editions["items"][0]["source"], "user");
    // Now the catalog knows it.
    ada.post(
        "/api/books/custom",
        json!({"title": "Achilles again", "isbn": "9780062060624"}),
    )
    .await
    .assert_status(409);
}

/// The real chain: an English ISBN Open Library knows is 409 (and stored), without asking the
/// DNB.
#[sqlx::test]
async fn an_isbn_open_library_knows_cannot_be_claimed(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "isbn:9780316556347"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(OL_SEARCH_CIRCE.as_bytes().to_vec(), "application/json"),
        )
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    let res = ada
        .post(
            "/api/books/custom",
            json!({"title": "Circe", "isbn": "9780316556347"}),
        )
        .await;
    assert_isbn_conflict(&res);
    let work = res.json()["work_id"].as_i64().unwrap();
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {work} AND source = 'openlibrary'"
        ))
        .await,
        1
    );
    assert!(
        app.upstream_requests()
            .await
            .iter()
            .all(|r| !r.url.path().contains("/sru/"))
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM works WHERE source = 'user'")
            .await,
        0
    );
}

/// 30 books and editions per day, shared; another user has their own budget.
#[sqlx::test]
async fn custom_books_and_editions_share_a_daily_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for i in 0..29 {
        ada.post("/api/books/custom", json!({"title": format!("Zine {i}")}))
            .await
            .assert_status(201);
    }
    let work = app.work("A book").await;
    ada.post(
        &format!("/api/books/works/{}/editions", work.id),
        json!({"publisher": "Self"}),
    )
    .await
    .assert_status(201);
    let limited = ada
        .post("/api/books/custom", json!({"title": "One more"}))
        .await;
    limited.assert_detail(429, "Slow down a little. Try again later.");
    assert!(limited.header("retry-after").is_some());
    ada.post(
        &format!("/api/books/works/{}/editions", work.id),
        json!({"publisher": "Self"}),
    )
    .await
    .assert_status(429);
    let grace = app.user("grace").await;
    grace
        .post("/api/books/custom", json!({"title": "Hers"}))
        .await
        .assert_status(201);
    // Invalid requests are not counted.
    let eve = app.user("eve").await;
    for _ in 0..31 {
        eve.post("/api/books/custom", json!({"title": " "}))
            .await
            .assert_status(400);
    }
    eve.post("/api/books/custom", json!({"title": "Valid"}))
        .await
        .assert_status(201);
}

/// The editions of a work, oldest first; one edition; adding one with every field.
#[sqlx::test]
async fn editions_list_add_and_get(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (work, first) = app
        .dnb_work("Der Hobbit", "J. R. R. Tolkien", "9783608938289")
        .await;
    let res = ada
        .post(
            &format!("/api/books/works/{}/editions", work.id),
            json!({"isbn": "978-3-423-71668-0", "title": "Der kleine Hobbit",
                   "subtitle": " Taschenbuch ", "publisher": "dtv", "language": "ger",
                   "format": "Paperback", "pages": 384, "publication_year": 2016}),
        )
        .await;
    res.assert_status(201);
    let added = res.json();
    assert_eq!(added["work_id"], work.id);
    assert_eq!(added["isbn13"], "9783423716680");
    assert_eq!(added["isbn10"], "3423716681");
    assert_eq!(added["title"], "Der kleine Hobbit");
    assert_eq!(added["subtitle"], "Taschenbuch");
    assert_eq!(added["publisher"], "dtv");
    assert_eq!(added["language"], "ger");
    assert_eq!(added["format"], "paperback");
    assert_eq!(added["pages"], 384);
    assert_eq!(added["publication_year"], 2016);
    assert_eq!(added["planned_publication_date"], Value::Null);
    assert_eq!(added["contributors"], json!([]));
    assert_eq!(added["source"], "user");
    assert_eq!(app.catalog.calls(), ["isbn_remote:9783423716680"]);
    let id = added["id"].as_i64().unwrap();
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE id = {id} AND created_by = {}
               AND field_sources ->> 'publisher' = 'user'",
            ada.id()
        ))
        .await,
        1
    );

    let list = ada
        .get(&format!("/api/books/works/{}/editions", work.id))
        .await;
    list.assert_status(200);
    let ids: Vec<i64> = list.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [first.id, id]);
    let one = ada.get(&format!("/api/books/editions/{id}")).await;
    one.assert_status(200);
    assert_eq!(one.json(), added);
    // The new edition title is searchable (works.search_extra).
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {} AND search_extra LIKE '%kleine Hobbit%'",
            work.id
        ))
        .await,
        1
    );

    // An edition titled like its work stores no own title.
    let same = ada
        .post(
            &format!("/api/books/works/{}/editions", work.id),
            json!({"title": "der hobbit", "format": "ebook"}),
        )
        .await;
    same.assert_status(201);
    assert_eq!(same.json()["title"], Value::Null);

    ada.get("/api/books/editions/424242")
        .await
        .assert_detail(404, "No such book.");
    ada.get("/api/books/editions/abc").await.assert_status(400);
    ada.get("/api/books/works/424242/editions")
        .await
        .assert_detail(404, "No such book.");
    ada.get("/api/books/works/not-a-key/editions")
        .await
        .assert_detail(404, "No such book.");
}

/// Adding an edition follows the ISBN rule of user-added books; an empty edition or bad values
/// are 400; an unknown work is 404 (nothing is imported for a write).
#[sqlx::test]
async fn adding_an_edition_follows_the_isbn_rule(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = app.work("My Zine").await;
    let other = app.work("Other").await;
    let taken = app.edition(&other, "9780316556347").await;
    let path = format!("/api/books/works/{}/editions", work.id);

    let conflict = ada.post(&path, json!({"isbn": "0316556343"})).await;
    assert_isbn_conflict(&conflict);
    assert_eq!(conflict.json()["work_id"], other.id);
    assert_eq!(conflict.json()["edition_id"], taken.id);

    app.catalog.on_isbn(
        "9780062060624",
        FakeHit::New {
            title: "The Song of Achilles".into(),
            authors: "Madeline Miller".into(),
            ol_work_id: Some("OL9W".into()),
            source: Source::OpenLibrary,
        },
    );
    assert_isbn_conflict(&ada.post(&path, json!({"isbn": "9780062060624"})).await);
    app.catalog.fail_all(FakeError::Unavailable);
    ada.post(&path, json!({"isbn": "9783423716680"}))
        .await
        .assert_detail(502, "Could not look that ISBN up right now. Try again.");

    for (body, detail) in [
        (json!({}), "Field required"),
        (json!({"title": "  "}), "Field required"),
        (
            json!({"format": "scroll"}),
            "Input should be 'hardcover', 'paperback', 'ebook', 'audiobook' or 'other'",
        ),
        (
            json!({"pages": 100001}),
            "Input should be less than or equal to 100000",
        ),
        (
            json!({"publication_year": 3001}),
            "Input should be less than or equal to 3000",
        ),
        (
            json!({"isbn": "9780316556348"}),
            "That is not a valid ISBN-10 or ISBN-13",
        ),
        (
            json!({"language": "de"}),
            "String should match pattern '^[a-z]{3}$'",
        ),
    ] {
        ada.post(&path, body).await.assert_detail(400, detail);
    }
    ada.post("/api/books/works/OL5W/editions", json!({"publisher": "X"}))
        .await
        .assert_detail(404, "No such book.");
    // Writes never import an unknown Open Library work: nobody was asked.
    ada.post(
        "/api/books/works/OL5W/edit-requests",
        json!({"fields": {"subtitle": "Roman"}}),
    )
    .await
    .assert_detail(404, "No such book.");
    ada.post(
        "/api/books/works/OL5W/merge-requests",
        json!({"into_work_id": work.id}),
    )
    .await
    .assert_detail(404, "No such book.");
    app.admin("root")
        .await
        .post(
            "/api/admin/works/OL5W/merge",
            json!({"into_work_id": work.id}),
        )
        .await
        .assert_detail(404, "No such book.");
    assert!(app.upstream_requests().await.is_empty());
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE work_id = {}",
            work.id
        ))
        .await,
        0
    );
}

/// An edition added while an admin merges its work away lands on the surviving work (ingest
/// follows the merge); that is no ISBN conflict. The merge commits while the ISBN check waits
/// for Open Library.
#[sqlx::test]
async fn an_edition_added_during_a_merge_lands_on_the_survivor(db: PgPool) {
    let app = TestApp::with_real_lookup(db).await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"numFound": 0, "docs": []}))
                .set_delay(Duration::from_millis(1500)),
        )
        .mount(&app.upstream)
        .await;
    let ada = app.user("ada").await;
    let work = app.work("My Zine").await;
    let survivor = app.work("My Zine (kept)").await;
    let merge = async {
        for _ in 0..250 {
            if !app.upstream_requests().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut conn = app.db.acquire().await.unwrap();
        merge_works(&mut conn, work.id, survivor.id).await.unwrap();
    };
    let path = format!("/api/books/works/{}/editions", work.id);
    let (res, ()) = tokio::join!(
        ada.post(
            &path,
            json!({"isbn": "9780316556347", "publisher": "Little, Brown"}),
        ),
        merge
    );
    res.assert_status(201);
    assert_eq!(res.json()["work_id"], survivor.id);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE work_id = {} AND isbn13 = '9780316556347'",
            survivor.id
        ))
        .await,
        1
    );
}

/// Waits until a session of this test's database waits for a row lock that the test holds.
async fn until_a_lock_wait(db: &PgPool) {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("nothing waited for a lock");
}

/// A merge that is already moving the loser's editions cannot see an edition added at the same
/// moment. The edition's insert waits for the merge on the work's row, sees it merged away and
/// starts over on the surviving work, so no edition stays behind on the merged-away work.
#[sqlx::test]
async fn an_edition_added_while_a_merge_runs_is_not_left_behind(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = app.work("My Zine").await;
    let survivor = app.work("My Zine (kept)").await;
    let mut merge = app.db.begin().await.unwrap();
    merge_works(&mut merge, work.id, survivor.id).await.unwrap();
    let finish_merge = async {
        until_a_lock_wait(&app.db).await;
        merge.commit().await.unwrap();
    };
    let path = format!("/api/books/works/{}/editions", work.id);
    let (res, ()) = tokio::join!(
        ada.post(&path, json!({"publisher": "Zine Press"})),
        finish_merge
    );
    res.assert_status(201);
    assert_eq!(res.json()["work_id"], survivor.id);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM editions WHERE work_id = {}",
            work.id
        ))
        .await,
        0
    );
}
