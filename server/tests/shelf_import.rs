//! The Goodreads import: the parser, the background job, its skip codes, ratings, reading dates
//! and editions, throttled lookups and the resume at startup (endpoint-map §7,
//! docs/phase1/spec/shelf.md §1.3, §2.7; #44).

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bookclub_server::catalog::shared::ingest::Source;
use bookclub_server::catalog::shared::isbn::Isbn13;
use bookclub_server::catalog::shared::lookup::{CatalogLookup, LookupError, Resolved};
use bookclub_server::dto::ShelfStatus;
use bookclub_server::shelf::goodreads::{
    self, GoodreadsRow, ImportSkipCode, ImportSkipOut, MAX_IMPORT_BYTES, clean_isbn,
    parse_goodreads_csv,
};
use common::*;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

/// `backend/tests/fixtures/goodreads.csv`, copied byte for byte.
const FIXTURE: &[u8] = include_bytes!("fixtures/goodreads/goodreads.csv");

const CIRCE_ISBN: &str = "9780316556347";

fn circe_hit() -> FakeHit {
    FakeHit::New {
        title: "Circe".into(),
        authors: "Madeline Miller".into(),
        ol_work_id: Some("OL1W".into()),
        source: Source::OpenLibrary,
    }
}

fn achilles_hit() -> FakeHit {
    FakeHit::New {
        title: "The Song of Achilles".into(),
        authors: "Madeline Miller".into(),
        ol_work_id: Some("OL2W".into()),
        source: Source::OpenLibrary,
    }
}

/// The fake catalog of the legacy tests: Circe by ISBN, Achilles by title and author, nothing
/// else.
fn program_fixture(app: &TestApp) {
    app.catalog.on_isbn(CIRCE_ISBN, circe_hit());
    app.catalog
        .on_text("The Song of Achilles", "Madeline Miller", achilles_hit());
}

/// Uploads `raw` as the `file` part.
async fn upload(client: &TestClient, raw: &[u8]) -> TestResponse {
    client
        .post_multipart(
            "/api/shelf/import",
            vec![Part::file("goodreads.csv", "text/csv", raw.to_vec())],
        )
        .await
}

/// Uploads `raw`, asserts 202, waits for the job and returns its final state.
async fn import(app: &TestApp, client: &TestClient, raw: &[u8]) -> Value {
    let res = upload(client, raw).await;
    res.assert_status(202);
    app.settle().await;
    let job = client
        .get(&format!("/api/shelf/import/{}", res.json()["id"]))
        .await;
    job.assert_status(200);
    job.json()
}

async fn items(client: &TestClient) -> Vec<Value> {
    let res = client.get("/api/shelf").await;
    res.assert_status(200);
    res.json()["items"].as_array().unwrap().clone()
}

fn by_title<'a>(items: &'a [Value], title: &str) -> &'a Value {
    items
        .iter()
        .find(|item| item["book"]["title"] == title)
        .unwrap_or_else(|| panic!("{title} is not on the shelf"))
}

fn skip(title: &str, code: &str, reason: &str) -> Value {
    json!({"title": title, "code": code, "reason": reason})
}

fn csv(rows: &[&str]) -> Vec<u8> {
    let mut text = String::from("Title,Author,ISBN13,My Rating,Date Read,Exclusive Shelf\n");
    for row in rows {
        text.push_str(row);
        text.push('\n');
    }
    text.into_bytes()
}

/// parity: test_import.py::test_clean_isbn_unwraps_goodreads_formula
#[test]
fn clean_isbn_unwraps_goodreads_formula() {
    assert_eq!(clean_isbn("=\"9780316556347\""), "9780316556347");
    assert_eq!(clean_isbn("=\"0316556343\""), "0316556343");
    assert_eq!(clean_isbn(""), "");
    assert_eq!(clean_isbn("=\"\""), "");
}

/// parity: test_import.py::test_parse_fixture_maps_shelves_and_skips
/// The rows now carry the ISBN as a validated ISBN-13 (`None` for none) and, for `read`
/// rows, `My Rating` and `Date Read`.
#[test]
fn parse_fixture_maps_shelves_and_skips() {
    let parsed = parse_goodreads_csv(FIXTURE).unwrap();
    let rows: Vec<_> = parsed
        .rows
        .iter()
        .map(|row| (row.title.as_str(), row.status, row.isbn13.as_deref()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Circe", ShelfStatus::WantToRead, Some(CIRCE_ISBN)),
            ("The Song of Achilles", ShelfStatus::CurrentlyReading, None),
            (
                "Unmatched Novel",
                ShelfStatus::Finished,
                Some("9780000000002")
            ),
        ]
    );
    assert_eq!(
        parsed.rows[2],
        GoodreadsRow {
            title: "Unmatched Novel".into(),
            authors: "Nobody".into(),
            isbn13: Some("9780000000002".into()),
            status: ShelfStatus::Finished,
            rating: Some(5),
            read_on: Some(time::macros::date!(2020 - 01 - 03)),
        }
    );
    assert_eq!(
        parsed.skips,
        vec![
            ImportSkipOut::new("Custom Shelf Book", ImportSkipCode::UnknownShelf),
            ImportSkipOut::new("Untitled", ImportSkipCode::NoTitle),
        ]
    );
}

/// parity: test_import.py::test_import_requires_auth
#[sqlx::test]
async fn import_requires_auth(db: PgPool) {
    let app = TestApp::new(db).await;
    upload(&app.anon(), FIXTURE)
        .await
        .assert_detail(401, "Not signed in");
    app.anon()
        .get("/api/shelf/import/1")
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_import.py::test_import_matches_isbn_and_title
/// The import is a background job now: 202, then the job's final state. Skips carry `code`.
#[sqlx::test]
async fn import_matches_isbn_and_title(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    program_fixture(&app);

    let res = upload(&ada, FIXTURE).await;
    res.assert_status(202);
    let queued = res.json();
    assert_eq!(queued["status"], "queued");
    assert_eq!(queued["total"], 3);
    assert_eq!(queued["processed"], 0);
    assert_eq!(queued["imported"], 0);
    assert_eq!(queued["skipped"], 2);
    assert_eq!(queued["error"], Value::Null);
    app.settle().await;

    let job = ada
        .get(&format!("/api/shelf/import/{}", queued["id"]))
        .await
        .json();
    assert_eq!(
        job,
        json!({
            "id": queued["id"],
            "status": "done",
            "total": 3,
            "processed": 3,
            "imported": 2,
            "skipped": 3,
            "skips": [
                skip("Custom Shelf Book", "unknown_shelf", "Unknown shelf"),
                skip("Untitled", "no_title", "No title"),
                skip("Unmatched Novel", "no_match", "No match"),
            ],
            "error": null
        })
    );
    let calls = app.catalog.calls();
    assert!(calls.contains(&format!("isbn:{CIRCE_ISBN}")), "{calls:?}");
    assert!(
        calls.contains(&"text:The Song of Achilles|Madeline Miller".to_owned()),
        "{calls:?}"
    );

    let shelf = items(&ada).await;
    assert_eq!(shelf.len(), 2);
    let circe = by_title(&shelf, "Circe");
    assert_eq!(circe["status"], "want_to_read");
    assert_eq!(circe["book"]["ol_work_key"], "/works/OL1W");
    let achilles = by_title(&shelf, "The Song of Achilles");
    assert_eq!(achilles["status"], "currently_reading");
    assert_eq!(achilles["book"]["ol_work_key"], "/works/OL2W");
    // The parsed rows are dropped once the job is done.
    assert_eq!(
        app.scalar_i64("SELECT jsonb_array_length(rows)::int8 FROM import_jobs")
            .await,
        0
    );
}

/// parity: test_import.py::test_import_skips_books_already_on_shelf
#[sqlx::test]
async fn import_skips_books_already_on_shelf(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    ada.post(
        "/api/shelf",
        json!({"work_id": circe.id, "status": "finished"}),
    )
    .await
    .assert_status(201);
    app.catalog.on_isbn(
        CIRCE_ISBN,
        FakeHit::Existing {
            work_id: circe.id,
            edition_id: None,
        },
    );
    app.catalog
        .on_text("The Song of Achilles", "Madeline Miller", achilles_hit());

    let job = import(&app, &ada, FIXTURE).await;
    assert_eq!(job["imported"], 1);
    let already: Vec<&Value> = job["skips"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|skip| skip["title"] == "Circe")
        .collect();
    assert_eq!(
        already,
        vec![&skip("Circe", "already_on_shelf", "Already on your shelf")]
    );
    assert_eq!(by_title(&items(&ada).await, "Circe")["status"], "finished");
}

/// parity: test_import.py::test_import_stays_on_signed_in_shelf
#[sqlx::test]
async fn import_stays_on_signed_in_shelf(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    program_fixture(&app);

    assert_eq!(import(&app, &grace, FIXTURE).await["imported"], 2);
    assert_eq!(items(&grace).await.len(), 2);
    let res = grace.get("/api/shelf?username=ada").await;
    res.assert_status(200);
    assert_eq!(res.json()["items"], json!([]));
}

/// parity: test_import.py::test_import_rejects_non_goodreads_csv
#[sqlx::test]
async fn import_rejects_non_goodreads_csv(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    upload(&ada, b"hello,world\n1,2\n")
        .await
        .assert_detail(400, "That does not look like a Goodreads export");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM import_jobs").await, 0);
}

/// parity: test_import.py::test_import_rejects_oversized_file
#[sqlx::test]
async fn import_rejects_oversized_file(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut raw = b"Title,Exclusive Shelf\n".to_vec();
    raw.extend(std::iter::repeat_n(b'x', MAX_IMPORT_BYTES + 1));
    upload(&ada, &raw)
        .await
        .assert_detail(400, "That file is too large. Keep the export under 1.5 MB.");
    // A body over the route's 2 MiB limit gets the same answer, not a 413.
    let raw = vec![b'x'; 3 * 1024 * 1024];
    upload(&ada, &raw)
        .await
        .assert_detail(400, "That file is too large. Keep the export under 1.5 MB.");
}

/// A file between the global 1 MiB body limit and 1.5 MB is accepted: the route raises its
/// own limit to 2 MiB (conventions §4.6).
#[sqlx::test]
async fn import_accepts_a_file_over_the_global_body_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut raw = b"Title,Exclusive Shelf,Notes\nDune,to-read,\n".to_vec();
    let padding = "x".repeat(100_000);
    for n in 0..14 {
        raw.extend_from_slice(format!("Book {n},dnf,{padding}\n").as_bytes());
    }
    assert!(raw.len() > 1024 * 1024 + 300_000 && raw.len() < MAX_IMPORT_BYTES);
    let res = upload(&ada, &raw).await;
    res.assert_status(202);
    assert_eq!(res.json()["total"], 1);
    assert_eq!(res.json()["skipped"], 14);
    app.settle().await;
}

#[sqlx::test]
async fn bad_uploads_are_400(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.post_multipart("/api/shelf/import", vec![Part::text("other", "x")])
        .await
        .assert_detail(400, "Field required");
    ada.post("/api/shelf/import", json!({}))
        .await
        .assert_detail(400, "Field required");
    upload(&ada, b"Title,Exclusive Shelf\n\xff,read\n")
        .await
        .assert_detail(400, "The file must be a UTF-8 Goodreads CSV export");
    upload(&ada, b"")
        .await
        .assert_detail(400, "That does not look like a Goodreads export");
    // A byte order mark is fine.
    let res = upload(&ada, b"\xef\xbb\xbfTitle,Exclusive Shelf\nDune,dnf\n").await;
    res.assert_status(202);
    app.settle().await;
}

/// New: `My Rating` and `Date Read` of `read` rows are imported (the date as noon in the
/// user's zone), and an ISBN that resolves names the entry's edition.
#[sqlx::test]
async fn ratings_reading_dates_and_editions_are_imported(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.exec("UPDATE users SET timezone = 'Pacific/Kiritimati' WHERE username = 'ada'")
        .await;
    program_fixture(&app);
    let raw = csv(&[
        "Circe,Madeline Miller,=\"9780316556347\",4,2024/02/03,read",
        "The Song of Achilles,Madeline Miller,,5,2999/01/01,read",
    ]);
    let job = import(&app, &ada, &raw).await;
    assert_eq!(job["imported"], 2, "{job}");

    let shelf = items(&ada).await;
    let circe = by_title(&shelf, "Circe");
    assert_eq!(circe["status"], "finished");
    assert_eq!(circe["rating"], 4);
    // Noon on 3 February at UTC+14 is 22:00 UTC the day before; the start is clamped to it.
    assert_eq!(circe["finished_at"], "2024-02-02T22:00:00Z");
    assert_eq!(circe["started_at"], "2024-02-02T22:00:00Z");
    let edition = app
        .scalar_i64(&format!(
            "SELECT id FROM editions WHERE isbn13 = '{CIRCE_ISBN}'"
        ))
        .await;
    assert_eq!(circe["edition"]["id"], edition);
    assert_eq!(circe["edition"]["isbn13"], CIRCE_ISBN);

    // A text match names no edition, and a date in the future is ignored: the import time.
    let achilles = by_title(&shelf, "The Song of Achilles");
    assert_eq!(achilles["rating"], 5);
    assert_eq!(achilles["edition"], Value::Null);
    let finished = time::OffsetDateTime::parse(
        achilles["finished_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert!(time::OffsetDateTime::now_utc() - finished < time::Duration::minutes(5));
    assert_eq!(achilles["started_at"], achilles["finished_at"]);
}

/// A throttled or unavailable catalog skips the row with `catalog_unavailable`, and the job
/// goes on; a later lookup that answers still imports.
#[sqlx::test]
async fn throttled_lookups_skip_with_catalog_unavailable(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    program_fixture(&app);
    app.catalog.on_text("Circe", "Madeline Miller", circe_hit());
    // The ISBN lookup is throttled, but title and author still find Circe.
    app.catalog
        .fail_isbn(CIRCE_ISBN, FakeError::Throttled, None);
    // Unmatched Novel: the ISBN is unavailable and the text misses: no definite answer.
    app.catalog
        .fail_isbn("9780000000002", FakeError::Unavailable, None);
    let job = import(&app, &ada, FIXTURE).await;
    assert_eq!(job["status"], "done");
    assert_eq!(job["imported"], 2);
    assert_eq!(
        job["skips"][2],
        skip(
            "Unmatched Novel",
            "catalog_unavailable",
            "Catalog unavailable"
        )
    );
    assert_eq!(
        by_title(&items(&ada).await, "Circe")["edition"],
        Value::Null
    );

    // Everything throttled, no cooldown running (the throttle's queue was full): each row is
    // tried three times, then skipped; none fails the job.
    let grace = app.user("grace").await;
    app.catalog.fail_all(FakeError::Throttled);
    let before = app.catalog.calls().len();
    let job = import(&app, &grace, FIXTURE).await;
    let calls = app.catalog.calls()[before..].to_vec();
    let achilles = "text:The Song of Achilles|Madeline Miller".to_owned();
    assert_eq!(
        calls.iter().filter(|call| **call == achilles).count(),
        3,
        "{calls:?}"
    );
    assert_eq!(calls.len(), 3 * 5, "{calls:?}");
    assert_eq!(job["status"], "done");
    assert_eq!(job["imported"], 0);
    assert_eq!(job["skipped"], 5);
    let codes: Vec<&Value> = job["skips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skip| &skip["code"])
        .collect();
    assert_eq!(
        codes,
        vec![
            "unknown_shelf",
            "no_title",
            "catalog_unavailable",
            "catalog_unavailable",
            "catalog_unavailable"
        ]
    );
}

/// A row throttled during Open Library's cooldown waits for its end and is tried again.
#[sqlx::test]
async fn throttled_rows_wait_for_the_cooldown(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.catalog.on_isbn(CIRCE_ISBN, circe_hit());
    app.catalog
        .fail_isbn(CIRCE_ISBN, FakeError::Throttled, Some(1));
    let started = Instant::now();
    app.state
        .outbound
        .ol_api
        .cool_down(Duration::from_millis(700));
    let job = import(
        &app,
        &ada,
        &csv(&["Circe,Madeline Miller,=\"9780316556347\",,,to-read"]),
    )
    .await;
    // The short queue pause alone would have retried after 250 ms.
    assert!(started.elapsed() >= Duration::from_millis(700));
    assert_eq!(job["imported"], 1, "{job}");
    assert_eq!(job["skips"], json!([]));
    let isbn = format!("isbn:{CIRCE_ISBN}");
    let calls = app.catalog.calls();
    assert_eq!(
        calls.iter().filter(|call| **call == isbn).count(),
        2,
        "{calls:?}"
    );
    assert_eq!(
        by_title(&items(&ada).await, "Circe")["edition"]["isbn13"],
        CIRCE_ISBN
    );
}

/// A cooldown longer than a row's 60 s budget is not waited for: the rows are skipped at once.
#[sqlx::test]
async fn a_long_cooldown_skips_at_once(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.catalog.fail_all(FakeError::Throttled);
    app.state
        .outbound
        .ol_api
        .cool_down(Duration::from_secs(3600));
    let started = Instant::now();
    let job = import(&app, &ada, FIXTURE).await;
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(job["status"], "done");
    assert_eq!(job["imported"], 0);
    assert_eq!(job["skips"][2]["code"], "catalog_unavailable");
    // One try per row: two ISBNs and three titles.
    assert_eq!(app.catalog.calls().len(), 5, "{:?}", app.catalog.calls());
}

/// A lookup that takes a while and records how many calls run at once.
#[derive(Default)]
struct SlowLookup {
    running: AtomicUsize,
    most: AtomicUsize,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl CatalogLookup for SlowLookup {
    async fn resolve_isbn(
        &self,
        _conn: &mut PgConnection,
        _isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        Ok(None)
    }

    async fn resolve_isbn_remote(
        &self,
        _conn: &mut PgConnection,
        _isbn: &Isbn13,
    ) -> Result<Option<Resolved>, LookupError> {
        Ok(None)
    }

    async fn resolve_text(
        &self,
        _conn: &mut PgConnection,
        _title: &str,
        _authors: &str,
    ) -> Result<Option<Resolved>, LookupError> {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(40)).await;
        self.running.fetch_sub(1, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
}

/// The jobs of many users share two lookup slots, so they never hold more than two pool
/// connections through remote calls.
#[sqlx::test]
async fn jobs_share_two_lookup_slots(db: PgPool) {
    let app = TestApp::new(db).await;
    let rows = parse_goodreads_csv(&csv(&["Dune,,,,,to-read", "Emma,,,,,to-read"])).unwrap();
    let rows = serde_json::to_string(&rows.rows).unwrap();
    for name in ["ada", "grace", "alan", "edsger", "barbara"] {
        let user = app.user(name).await;
        app.exec(&format!(
            "INSERT INTO import_jobs (user_id, rows, total) VALUES ({}, '{rows}', 2)",
            user.id()
        ))
        .await;
    }
    let lookup = Arc::new(SlowLookup::default());
    let mut state = app.state.clone();
    state.lookup = lookup.clone();
    goodreads::resume(state).await;
    app.settle().await;
    assert_eq!(lookup.calls.load(Ordering::SeqCst), 10);
    let most = lookup.most.load(Ordering::SeqCst);
    assert!((1..=2).contains(&most), "{most} lookups ran at once");
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM import_jobs WHERE status = 'done' AND processed = 2")
            .await,
        5
    );
}

/// Each import goes to the top of its column, so the file's order is reversed per status; a
/// second row of the same book is `already_on_shelf`; the import records no events.
#[sqlx::test]
async fn rows_go_to_the_top_of_their_column_without_events(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for title in ["A", "B", "C"] {
        let work = app.work(title).await;
        app.catalog.on_text(
            title,
            "",
            FakeHit::Existing {
                work_id: work.id,
                edition_id: None,
            },
        );
    }
    let job = import(
        &app,
        &ada,
        &csv(&[
            ",,,,,",
            "A,,,,,to-read",
            "B,,,,,to-read",
            "C,,,,,to-read",
            "A,,,,,read",
        ]),
    )
    .await;
    assert_eq!(job["imported"], 3);
    assert_eq!(
        job["skips"],
        json!([
            skip("Untitled", "no_title", "No title"),
            skip("A", "already_on_shelf", "Already on your shelf")
        ])
    );
    let shelf = items(&ada).await;
    let order: Vec<(&str, i64)> = shelf
        .iter()
        .map(|item| {
            (
                item["book"]["title"].as_str().unwrap(),
                item["position"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(order, vec![("C", 0), ("B", 1), ("A", 2)]);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM events").await, 0);
}

/// `skipped` counts every skip; `skips` lists the first 25.
#[sqlx::test]
async fn the_skip_list_stops_at_25(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let rows: Vec<String> = (0..30).map(|n| format!("Book {n},,,,,dnf")).collect();
    let mut raw = csv(&rows.iter().map(String::as_str).collect::<Vec<_>>());
    raw.extend_from_slice(b"Lost,,,,,to-read\n");
    let job = import(&app, &ada, &raw).await;
    assert_eq!(job["skipped"], 31);
    let skips = job["skips"].as_array().unwrap();
    assert_eq!(skips.len(), 25);
    assert_eq!(skips[0], skip("Book 0", "unknown_shelf", "Unknown shelf"));
    assert_eq!(skips[24]["title"], "Book 24");
}

/// One active import per user: 409 with that import as `job`. Another user may import.
#[sqlx::test]
async fn one_active_import_per_user(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    app.exec(&format!(
        "INSERT INTO import_jobs (id, user_id, status, total) VALUES (77, {}, 'running', 4)",
        ada.id()
    ))
    .await;
    let res = upload(&ada, FIXTURE).await;
    assert_eq!(res.status.as_u16(), 409, "{}", res.text());
    let body = res.json();
    assert_eq!(body["detail"], "An import is already running");
    assert_eq!(body["job"]["id"], 77);
    assert_eq!(body["job"]["status"], "running");
    assert_eq!(body["job"]["total"], 4);

    import(&app, &grace, FIXTURE).await;
    app.exec("UPDATE import_jobs SET status = 'failed' WHERE id = 77")
        .await;
    import(&app, &ada, FIXTURE).await;
}

/// A job is its owner's: anyone else gets the same 404 as for a missing one.
#[sqlx::test]
async fn jobs_are_private(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let job = import(&app, &ada, FIXTURE).await;
    let path = format!("/api/shelf/import/{}", job["id"]);
    grace.get(&path).await.assert_detail(404, "No such import");
    ada.get("/api/shelf/import/999999")
        .await
        .assert_detail(404, "No such import");
    ada.get("/api/shelf/import/x").await.assert_detail(
        400,
        "Input should be a valid integer, unable to parse string as an integer",
    );
    ada.get(&path).await.assert_status(200);
}

/// `resume` continues queued and running jobs at `processed` after a restart; finished jobs
/// stay as they are.
#[sqlx::test]
async fn resume_continues_at_processed(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    program_fixture(&app);
    let parsed = parse_goodreads_csv(FIXTURE).unwrap();
    let rows = serde_json::to_string(&parsed.rows).unwrap();
    // ada's job had done the first row (Circe) when the server stopped.
    app.exec(&format!(
        "INSERT INTO import_jobs (id, user_id, status, rows, total, processed, imported, skipped, skips)
         VALUES (10, {}, 'running', '{rows}', 3, 1, 1, 2,
                 '[{{\"title\": \"Custom Shelf Book\", \"code\": \"unknown_shelf\", \"reason\": \"Unknown shelf\"}}]')",
        ada.id()
    ))
    .await;
    // grace's job was queued and never started. (Its book is not in ada's job: the two jobs
    // run at the same time.)
    let other = parse_goodreads_csv(&csv(&["Dune,Frank Herbert,,,,to-read"])).unwrap();
    let other = serde_json::to_string(&other.rows).unwrap();
    app.catalog.on_text(
        "Dune",
        "Frank Herbert",
        FakeHit::New {
            title: "Dune".into(),
            authors: "Frank Herbert".into(),
            ol_work_id: Some("OL3W".into()),
            source: Source::OpenLibrary,
        },
    );
    app.exec(&format!(
        "INSERT INTO import_jobs (id, user_id, rows, total) VALUES (11, {}, '{other}', 1)",
        grace.id()
    ))
    .await;
    // A finished job is not touched.
    app.exec(&format!(
        "INSERT INTO import_jobs (id, user_id, status, rows, total) VALUES (12, {}, 'done', '{rows}', 3)",
        ada.id()
    ))
    .await;

    goodreads::resume(app.state.clone()).await;
    app.settle().await;

    let ada_job = ada.get("/api/shelf/import/10").await.json();
    assert_eq!(ada_job["status"], "done");
    assert_eq!(ada_job["processed"], 3);
    assert_eq!(ada_job["imported"], 2);
    assert_eq!(ada_job["skipped"], 3);
    assert_eq!(ada_job["skips"][1]["title"], "Unmatched Novel");
    // Circe (the row done before the stop) was not looked up again.
    let calls = app.catalog.calls();
    assert!(!calls.contains(&format!("isbn:{CIRCE_ISBN}")), "{calls:?}");
    let shelf = items(&ada).await;
    let titles: Vec<&str> = shelf
        .iter()
        .map(|item| item["book"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["The Song of Achilles"]);

    let grace_job = grace.get("/api/shelf/import/11").await.json();
    assert_eq!(grace_job["status"], "done");
    assert_eq!(grace_job["imported"], 1);
    assert_eq!(
        by_title(&items(&grace).await, "Dune")["status"],
        "want_to_read"
    );

    let done = ada.get("/api/shelf/import/12").await.json();
    assert_eq!(done["processed"], 0);
    assert_eq!(
        app.scalar_i64("SELECT jsonb_array_length(rows)::int8 FROM import_jobs WHERE id = 12")
            .await,
        3
    );
}

/// Two runs of one job (a resume racing a fresh spawn) handle each row once: a run that finds
/// `processed` past its row stops.
#[sqlx::test]
async fn two_runs_of_one_job_handle_each_row_once(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    program_fixture(&app);
    let parsed = parse_goodreads_csv(FIXTURE).unwrap();
    let rows = serde_json::to_string(&parsed.rows).unwrap();
    let skips = serde_json::to_string(&parsed.skips).unwrap();
    app.exec(&format!(
        "INSERT INTO import_jobs (id, user_id, rows, total, skipped, skips)
         VALUES (20, {}, '{rows}', 3, 2, '{skips}')",
        ada.id()
    ))
    .await;
    goodreads::resume(app.state.clone()).await;
    goodreads::resume(app.state.clone()).await;
    app.settle().await;

    let job = ada.get("/api/shelf/import/20").await.json();
    assert_eq!(job["status"], "done");
    assert_eq!(job["total"], 3);
    assert_eq!(job["processed"], 3);
    assert_eq!(job["imported"], 2);
    assert_eq!(job["skipped"], 3);
    assert_eq!(
        job["skips"],
        json!([
            skip("Custom Shelf Book", "unknown_shelf", "Unknown shelf"),
            skip("Untitled", "no_title", "No title"),
            skip("Unmatched Novel", "no_match", "No match"),
        ])
    );
    assert_eq!(items(&ada).await.len(), 2);
}

/// A job that cannot go on (here: rows this version cannot read) is marked failed with the
/// documented error, its rows are dropped, and it no longer blocks a new import.
#[sqlx::test]
async fn a_job_that_breaks_is_marked_failed(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    app.exec(&format!(
        "INSERT INTO import_jobs (id, user_id, rows, total) VALUES (5, {}, '[{{\"bad\": 1}}]', 1)",
        ada.id()
    ))
    .await;
    goodreads::resume(app.state.clone()).await;
    app.settle().await;
    let job = ada.get("/api/shelf/import/5").await.json();
    assert_eq!(job["status"], "failed");
    assert_eq!(job["error"], "The import stopped unexpectedly. Try again.");
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM import_jobs
             WHERE id = 5 AND jsonb_array_length(rows) = 0 AND finished_at IS NOT NULL"
        )
        .await,
        1
    );
    upload(&ada, FIXTURE).await.assert_status(202);
    app.settle().await;
}

/// A database error in the middle of a job fails it there: the rows committed before stay on
/// the shelf, the failed row is rolled back, and the parsed rows are dropped.
#[sqlx::test]
async fn a_job_failing_midway_keeps_the_rows_before(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut ids = Vec::new();
    for title in ["A", "B", "C"] {
        let work = app.work(title).await;
        app.catalog.on_text(
            title,
            "",
            FakeHit::Existing {
                work_id: work.id,
                edition_id: None,
            },
        );
        ids.push(work.id);
    }
    app.exec(&format!(
        "CREATE FUNCTION refuse_b() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF NEW.work_id = {} THEN RAISE EXCEPTION 'no B'; END IF;
             RETURN NEW;
         END $$;
         CREATE TRIGGER refuse_b BEFORE INSERT ON shelf_entries
             FOR EACH ROW EXECUTE FUNCTION refuse_b();",
        ids[1]
    ))
    .await;
    let job = import(
        &app,
        &ada,
        &csv(&["A,,,,,to-read", "B,,,,,to-read", "C,,,,,to-read"]),
    )
    .await;
    assert_eq!(job["status"], "failed");
    assert_eq!(job["error"], "The import stopped unexpectedly. Try again.");
    assert_eq!(job["processed"], 1);
    assert_eq!(job["imported"], 1);
    assert_eq!(job["skipped"], 0);
    let shelf = items(&ada).await;
    assert_eq!(shelf.len(), 1);
    assert_eq!(shelf[0]["book"]["id"], ids[0]);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM import_jobs
             WHERE jsonb_array_length(rows) = 0 AND finished_at IS NOT NULL"
        )
        .await,
        1
    );
    // C was never looked up.
    assert!(!app.catalog.calls().contains(&"text:C|".to_owned()));
}

/// A lookup that resolves to a merged-away work shelves the work it was merged into.
#[sqlx::test]
async fn merged_works_resolve_to_their_target(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let old = app.work("Old Circe").await;
    let target = app.work("Circe").await;
    app.exec(&format!(
        "UPDATE works SET merged_into_id = {} WHERE id = {}",
        target.id, old.id
    ))
    .await;
    app.catalog.on_text(
        "Circe",
        "",
        FakeHit::Existing {
            work_id: old.id,
            edition_id: None,
        },
    );
    let job = import(&app, &ada, &csv(&["Circe,,,,,to-read"])).await;
    assert_eq!(job["imported"], 1);
    assert_eq!(items(&ada).await[0]["book"]["id"], target.id);
}

/// Whether a query on `import_jobs` waits for a lock right now.
async fn import_insert_waits(app: &TestApp) -> bool {
    app.scalar_i64(
        "SELECT count(*) FROM pg_stat_activity
         WHERE datname = current_database() AND wait_event_type = 'Lock'
           AND query LIKE '%INSERT INTO import_jobs%'",
    )
    .await
        > 0
}

/// An upload whose client goes away while the job row is written still runs its job: the row
/// and its task start together, so no job is left `queued` without a task (which would answer
/// every later upload with 409 until a restart).
#[sqlx::test]
async fn a_cancelled_upload_never_strands_its_job(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    program_fixture(&app);
    // Hold ada's row, so the job's INSERT waits in its foreign key check.
    let mut blocker = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(ada.id())
        .execute(&mut *blocker)
        .await
        .unwrap();
    {
        let request = upload(&ada, FIXTURE);
        tokio::pin!(request);
        let mut polls = 0;
        while !import_insert_waits(&app).await {
            tokio::select! {
                res = &mut request => panic!("the upload answered: {}", res.text()),
                () = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            polls += 1;
            assert!(polls < 500, "the job's INSERT never waited for the lock");
        }
        // The client goes away: the request (and the handler) is dropped here.
    }
    blocker.rollback().await.unwrap();
    // The INSERT commits once the lock is free.
    for _ in 0..500 {
        if app.scalar_i64("SELECT count(*) FROM import_jobs").await > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    app.settle().await;
    let status: String = sqlx::query_scalar("SELECT status FROM import_jobs")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "done");
    assert_eq!(items(&ada).await.len(), 2);
}

/// Two uploads at once: one job, one 409 (the `import_jobs_one_active` index decides).
#[sqlx::test]
async fn concurrent_uploads_start_one_job(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    // Hold ada's row lock, so the first job cannot finish before the second upload lands
    // (NO KEY UPDATE: the job's FOR UPDATE waits, the uploads' foreign key checks do not).
    let mut blocker = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(ada.id())
        .execute(&mut *blocker)
        .await
        .unwrap();
    let (first, second) = tokio::join!(upload(&ada, FIXTURE), upload(&ada, FIXTURE));
    let mut statuses = [first.status.as_u16(), second.status.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [202, 409], "{} / {}", first.text(), second.text());
    blocker.rollback().await.unwrap();
    app.settle().await;
    assert_eq!(app.scalar_i64("SELECT count(*) FROM import_jobs").await, 1);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM import_jobs WHERE status = 'done'")
            .await,
        1
    );
}
