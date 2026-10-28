//! Notice and takedown (#43, endpoint-map §6): `POST /api/media/{public_id}/reports`, the admin
//! queue in `media::admin_routes()`, and what a takedown guarantees afterwards.

mod common;

use std::time::Duration;

use bookclub_server::catalog::shared::books::{BookRef, book_out};
use bookclub_server::catalog::shared::ingest::{
    self, CoverTarget, Mode, Source, WorkRecord, ol_cover_media, set_cover,
};
use bookclub_server::media::service::{CoverOutcome, PREFETCH_POLICY};
use common::*;
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage};
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

fn jpeg(color: [u8; 3]) -> Vec<u8> {
    let image = RgbImage::from_pixel(60, 90, Rgb(color));
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .write_image(image.as_raw(), 60, 90, ExtendedColorType::Rgb8)
        .unwrap();
    out
}

/// Open Library work OL1W "Circe" with cover 111, shown once so its bytes are stored. Returns
/// the work id, the cover's public id and the bytes Open Library served.
async fn shown_ol_cover(app: &TestApp) -> (i64, String, Vec<u8>) {
    let bytes = jpeg([150, 40, 40]);
    Mock::given(method("GET"))
        .and(path("/b/id/111-L.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
        .mount(&app.upstream)
        .await;
    let mut conn = app.db.acquire().await.unwrap();
    let record = WorkRecord {
        ol_work_id: Some("OL1W".into()),
        ol_cover_id: Some(111),
        author_names: "Madeline Miller".into(),
        full_details: true,
        ..WorkRecord::new(Source::OpenLibrary, "Circe")
    };
    let work = ingest::upsert_work(&mut conn, &record, Mode::Refresh)
        .await
        .unwrap();
    let public_id = book_out(&mut conn, BookRef::work(work))
        .await
        .unwrap()
        .unwrap()
        .cover_public_id
        .unwrap();
    app.anon()
        .get(&format!("/media/{public_id}/l"))
        .await
        .assert_status(200);
    (work, public_id, bytes)
}

async fn cover_of(app: &TestApp, work: i64) -> Option<i64> {
    sqlx::query_scalar("SELECT cover_media_id FROM works WHERE id = $1")
        .bind(work)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

async fn report(client: &TestClient, public_id: &str, body: Value) -> TestResponse {
    client
        .post(&format!("/api/media/{public_id}/reports"), body)
        .await
}

async fn cover_requests(app: &TestApp) -> usize {
    app.upstream_requests()
        .await
        .iter()
        .filter(|request| request.url.path().starts_with("/b/id/"))
        .count()
}

/// Acceptance (#43): reporting creates a queue item, and an admin can remove the cover. The
/// cover is gone everywhere: the book shows none, its URL answers 410 and its bytes are
/// deleted.
#[sqlx::test]
async fn reporting_creates_a_queue_item_and_an_admin_removes_the_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, public_id, _) = shown_ol_cover(&app).await;
    assert!(!app.media.keys().is_empty());

    let res = report(
        &ada,
        &public_id,
        json!({"reason": "copyright", "details": "  This is my photo.  "}),
    )
    .await;
    res.assert_status(201);
    let report_id = res.json()["id"].as_i64().unwrap();

    let queue = root.get("/api/admin/reports").await;
    queue.assert_status(200);
    let items = queue.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 1);
    let item = &items[0];
    assert_eq!(item["id"], report_id);
    assert_eq!(item["reason"], "copyright");
    assert_eq!(item["details"], "This is my photo.");
    assert_eq!(item["status"], "open");
    assert_eq!(item["media_public_id"], public_id);
    assert_eq!(item["removed"], false);
    assert_eq!(item["cover_url"], format!("/media/{public_id}/l"));
    assert_eq!(item["book"]["id"], work);
    assert_eq!(item["created_by"]["username"], "ada");
    assert_eq!(item["resolved_by"], Value::Null);

    let res = root
        .post(
            &format!("/api/admin/reports/{report_id}/resolve"),
            json!({"action": "remove"}),
        )
        .await;
    res.assert_status(200);
    let resolved = res.json();
    assert_eq!(resolved["status"], "removed");
    assert_eq!(resolved["removed"], true);
    assert_eq!(resolved["cover_url"], Value::Null);
    assert_eq!(resolved["resolved_by"]["username"], "root");
    assert!(resolved["resolved_at"].is_string());

    assert_eq!(cover_of(&app, work).await, None);
    let mut conn = app.db.acquire().await.unwrap();
    let book = book_out(&mut conn, BookRef::work(work))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(book.cover_url, None);
    let gone = app.anon().get(&format!("/media/{public_id}/l")).await;
    gone.assert_detail(410, "This picture was removed");
    assert!(app.media.keys().is_empty(), "{:?}", app.media.keys());
    assert_eq!(
        root.get("/api/admin/reports").await.json(),
        json!({"items": []})
    );
    root.post(
        &format!("/api/admin/reports/{report_id}/resolve"),
        json!({"action": "dismiss"}),
    )
    .await
    .assert_detail(409, "This report was already resolved");
}

/// Acceptance (#43): after a takedown, a refresh, a first-view enrichment and a re-upload of the
/// same file leave `cover_media_id` NULL and request no cover; nothing fetches the picture
/// again. "The same file" is both the file Open Library sent and the WebP files readers saved
/// from the cover's URLs (all they ever had).
#[sqlx::test]
async fn after_a_takedown_nothing_brings_the_cover_back(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, public_id, ol_bytes) = shown_ol_cover(&app).await;
    ada.post("/api/shelf", json!({"work_id": work}))
        .await
        .assert_status(201);
    let mut saved = Vec::new();
    for variant in ["l", "s"] {
        let res = app
            .anon()
            .get(&format!("/media/{public_id}/{variant}"))
            .await;
        res.assert_status(200);
        saved.push(res.body.to_vec());
    }
    // Open Library still lists cover 111 for the work.
    Mock::given(method("GET"))
        .and(path("/works/OL1W.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "title": "Circe", "covers": [111]
        })))
        .mount(&app.upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/search.json"))
        .and(query_param("q", "key:/works/OL1W"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "numFound": 1,
            "docs": [{"key": "/works/OL1W", "title": "Circe", "author_name": ["Madeline Miller"],
                      "cover_i": 111}]
        })))
        .mount(&app.upstream)
        .await;
    root.delete(&format!("/api/admin/media/{public_id}"))
        .await
        .assert_status(204);
    assert_eq!(cover_requests(&app).await, 1);

    // A refresh.
    ada.post(&format!("/api/books/works/{work}/refresh"), json!({}))
        .await
        .assert_status(200);
    assert_eq!(cover_of(&app, work).await, None);
    // The one-time enrichment of a first detail view.
    app.exec(&format!(
        "UPDATE works SET details_fetched_at = NULL WHERE id = {work}"
    ))
    .await;
    let detail = ada.get(&format!("/api/books/works/{work}")).await;
    detail.assert_status(200);
    assert_eq!(detail.json()["cover_url"], Value::Null);
    assert_eq!(cover_of(&app, work).await, None);
    let enriched: bool =
        sqlx::query_scalar("SELECT details_fetched_at IS NOT NULL FROM works WHERE id = $1")
            .bind(work)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(enriched, "the enrichment ran");
    // A re-upload of the same file: Open Library's JPEG, or the large or small WebP a reader
    // saved. The work has no cover, so an accepted upload would be its cover at once.
    for (name, bytes) in [
        ("circe.jpg", ol_bytes),
        ("l.webp", saved[0].clone()),
        ("s.webp", saved[1].clone()),
    ] {
        ada.post_multipart(
            &format!("/api/books/works/{work}/cover"),
            vec![Part::file(name, "image/jpeg", bytes)],
        )
        .await
        .assert_detail(400, "This picture was removed");
    }
    assert_eq!(cover_of(&app, work).await, None);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM catalog_edit_requests")
            .await,
        0
    );
    // Neither the picture's URL, the spine backfill nor the prefetch worker's fetch asks for it.
    app.anon()
        .get(&format!("/media/{public_id}/s"))
        .await
        .assert_status(410);
    let colors = ada.post("/api/shelf/spine-colors", json!({})).await;
    assert_eq!(colors.json(), json!({"colors": {}, "remaining": 0}));
    let media_id = app
        .scalar_i64(&format!(
            "SELECT id FROM media WHERE public_id = '{public_id}'"
        ))
        .await;
    let prefetched = app
        .state
        .media_service
        .ensure_cover(&app.db, media_id, PREFETCH_POLICY)
        .await;
    assert_eq!(prefetched, CoverOutcome::Removed);
    assert_eq!(cover_requests(&app).await, 1);
    let ol_rows: i64 = app
        .scalar_i64("SELECT count(*) FROM media WHERE ol_cover_id = 111 AND removed_at IS NULL")
        .await;
    assert_eq!(ol_rows, 0);
}

/// Dismissing keeps the picture; a report cannot be resolved twice.
#[sqlx::test]
async fn dismissing_keeps_the_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, public_id, _) = shown_ol_cover(&app).await;
    let id = report(&ada, &public_id, json!({"reason": "wrong_book"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let res = root
        .post(
            &format!("/api/admin/reports/{id}/resolve"),
            json!({"action": "dismiss"}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "dismissed");
    assert_eq!(res.json()["details"], "");
    assert!(cover_of(&app, work).await.is_some());
    app.anon()
        .get(&format!("/media/{public_id}/l"))
        .await
        .assert_status(200);
    root.post(
        &format!("/api/admin/reports/{id}/resolve"),
        json!({"action": "remove"}),
    )
    .await
    .assert_detail(409, "This report was already resolved");
    assert!(cover_of(&app, work).await.is_some());
    root.post(
        "/api/admin/reports/999999/resolve",
        json!({"action": "remove"}),
    )
    .await
    .assert_detail(404, "Not found");
    root.post(
        &format!("/api/admin/reports/{id}/resolve"),
        json!({"action": "burn"}),
    )
    .await
    .assert_detail(400, "Input should be 'remove' or 'dismiss'");
}

/// A takedown resolves every open report of the picture; taking it down again changes nothing.
#[sqlx::test]
async fn a_takedown_resolves_every_open_report_of_the_picture(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let root = app.admin("root").await;
    let (_, public_id, _) = shown_ol_cover(&app).await;
    report(&ada, &public_id, json!({"reason": "offensive"}))
        .await
        .assert_status(201);
    report(
        &grace,
        &public_id,
        json!({"reason": "other", "details": "?"}),
    )
    .await
    .assert_status(201);
    root.delete(&format!("/api/admin/media/{public_id}"))
        .await
        .assert_status(204);
    root.delete(&format!("/api/admin/media/{public_id}"))
        .await
        .assert_status(204);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM reports WHERE status = 'removed' AND resolved_by = {}",
            root.id()
        ))
        .await,
        2
    );
    root.delete("/api/admin/media/abcdefghijklmnopqrstuvwxyz")
        .await
        .assert_detail(404, "Not found");
    // A removed picture can still be reported (the queue shows it removed); it stays removed.
    report(&ada, &public_id, json!({"reason": "other"}))
        .await
        .assert_status(201);
    let items = root.get("/api/admin/reports").await.json()["items"].clone();
    assert_eq!(items[0]["removed"], true);
    assert_eq!(items[0]["cover_url"], Value::Null);
}

/// Reports name a cover by its public id: unknown ids and avatars are 404; the reason is one
/// of four; details are trimmed and cut at 2000 characters; 20 per user and day.
#[sqlx::test]
async fn reports_are_checked_and_limited(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (_, public_id, _) = shown_ol_cover(&app).await;
    report(
        &ada,
        "abcdefghijklmnopqrstuvwxyz",
        json!({"reason": "other"}),
    )
    .await
    .assert_detail(404, "Not found");
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, uploaded_by)
         VALUES ('avataravataravataravatar22', 'avatar', 'upload', {})",
        ada.id()
    ))
    .await;
    report(
        &ada,
        "avataravataravataravatar22",
        json!({"reason": "offensive"}),
    )
    .await
    .assert_detail(404, "Not found");
    report(&ada, &public_id, json!({"reason": "ugly"}))
        .await
        .assert_detail(
            400,
            "Input should be 'copyright', 'wrong_book', 'offensive' or 'other'",
        );
    report(&ada, &public_id, json!({}))
        .await
        .assert_detail(400, "Field required");
    app.anon()
        .post(
            &format!("/api/media/{public_id}/reports"),
            json!({"reason": "other"}),
        )
        .await
        .assert_detail(401, "Not signed in");

    let long = "x".repeat(2500);
    report(
        &ada,
        &public_id,
        json!({"reason": "other", "details": long}),
    )
    .await
    .assert_status(201);
    let stored: i64 = app
        .scalar_i64("SELECT max(char_length(details))::int8 FROM reports")
        .await;
    assert_eq!(stored, 2000);
    for _ in 1..20 {
        report(&ada, &public_id, json!({"reason": "other"}))
            .await
            .assert_status(201);
    }
    let limited = report(&ada, &public_id, json!({"reason": "other"})).await;
    limited.assert_detail(429, "Slow down a little. Try again later.");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 20);
}

/// A report of a picture that waits in a `cover_replace` request names the book it was
/// proposed for.
#[sqlx::test]
async fn a_report_of_a_proposed_cover_names_its_book(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let root = app.admin("root").await;
    let work = app.work("My Zine").await;
    let photo = |color| {
        let image = RgbImage::from_pixel(40, 60, Rgb(color));
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 90)
            .write_image(image.as_raw(), 40, 60, ExtendedColorType::Rgb8)
            .unwrap();
        out
    };
    for (who, color) in [(&ada, [200, 0, 0]), (&grace, [0, 200, 0])] {
        who.post_multipart(
            &format!("/api/books/works/{}/cover", work.id),
            vec![Part::file("c.jpg", "image/jpeg", photo(color))],
        )
        .await;
    }
    let proposed: String = sqlx::query_scalar(
        "SELECT m.public_id FROM catalog_edit_requests r JOIN media m ON m.id = r.media_id",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    report(&ada, &proposed, json!({"reason": "offensive"}))
        .await
        .assert_status(201);
    let items = root.get("/api/admin/reports").await.json()["items"].clone();
    assert_eq!(items[0]["book"]["id"], work.id);
}

/// Waits until a session of this test's database waits for a lock.
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

/// A takedown that meets an ingest attaching the picture waits for it, then clears the new
/// reference too (conventions §7.2 "Covers and takedowns").
#[sqlx::test]
async fn a_takedown_waits_for_an_ingest_attaching_the_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let work = app.work("My Zine").await;
    let (media, public_id): (i64, String) = {
        let mut conn = app.db.acquire().await.unwrap();
        let media = ol_cover_media(&mut conn, 777).await.unwrap().unwrap();
        let public_id = sqlx::query_scalar("SELECT public_id FROM media WHERE id = $1")
            .bind(media)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        (media, public_id)
    };
    let mut ingest_tx = app.db.begin().await.unwrap();
    assert!(
        set_cover(
            &mut ingest_tx,
            CoverTarget::Work(work.id),
            media,
            Source::OpenLibrary,
            false
        )
        .await
        .unwrap()
    );
    let finish_ingest = async {
        until_a_lock_wait(&app.db).await;
        ingest_tx.commit().await.unwrap();
    };
    let take_down = format!("/api/admin/media/{public_id}");
    let (res, ()) = tokio::join!(root.delete(&take_down), finish_ingest);
    res.assert_status(204);
    assert_eq!(cover_of(&app, work.id).await, None);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE id = {media} AND removed_at IS NOT NULL"
        ))
        .await,
        1
    );
}
