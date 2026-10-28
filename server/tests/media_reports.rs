//! Notice and takedown (#43, #54, endpoint-map §6): `POST /api/media/{public_id}/reports`, the
//! admin queue in `media::admin_routes()`, and what a takedown guarantees afterwards.

mod common;

use std::time::Duration;

use bookclub_server::catalog::shared::books::{BookRef, book_out};
use bookclub_server::catalog::shared::ingest::{
    self, CoverTarget, Mode, Source, WorkRecord, ol_cover_media, set_cover,
};
use bookclub_server::media::reports::take_down_locked;
use bookclub_server::media::service::{CoverOutcome, PREFETCH_POLICY};
use common::*;
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage};
use serde_json::{Value, json};
use sqlx::pool::PoolConnection;
use sqlx::{PgPool, Postgres};
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
        json!({"items": [], "page": 1, "has_more": false})
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
}

/// The reports row a direct takedown wrote for its notice, as `(reason, details, created_by,
/// created_at, status, resolved_by, resolved_at, media_id, work_id)`, with the times as epoch
/// seconds.
type NoticeRow = (
    String,
    String,
    Option<i64>,
    i64,
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

async fn notice_rows(app: &TestApp) -> Vec<NoticeRow> {
    sqlx::query_as(
        "SELECT reason, details, created_by, extract(epoch FROM created_at)::int8, status,
                resolved_by, extract(epoch FROM resolved_at)::int8, media_id, work_id
         FROM reports ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
}

fn rfc3339(at: time::OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

/// Acceptance (#54, D2): a takedown with a body records the emailed notice it answers: one
/// `removed` report of the picture and its book, `created_at` = `received_at`, no reporter,
/// resolved by the admin now. Without a body it answers 204 and records nothing.
#[sqlx::test]
async fn a_takedown_with_a_notice_records_it(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let (work, public_id, _) = shown_ol_cover(&app).await;
    let media_id = app
        .scalar_i64(&format!(
            "SELECT id FROM media WHERE public_id = '{public_id}'"
        ))
        .await;
    let received = (time::OffsetDateTime::now_utc() - time::Duration::hours(30))
        .replace_nanosecond(0)
        .unwrap();
    let res = root
        .delete_json(
            &format!("/api/admin/media/{public_id}"),
            json!({
                "details": "  Mail from the publisher: the photo is ours.  ",
                "received_at": rfc3339(received),
                "reason": "copyright"
            }),
        )
        .await;
    res.assert_status(204);
    assert_eq!(cover_of(&app, work).await, None, "the picture is down");
    let rows = notice_rows(&app).await;
    assert_eq!(rows.len(), 1);
    let (reason, details, created_by, created_at, status, resolved_by, resolved_at, media, book) =
        rows[0].clone();
    assert_eq!(
        (
            reason.as_str(),
            details.as_str(),
            created_by,
            status.as_str()
        ),
        (
            "copyright",
            "Mail from the publisher: the photo is ours.",
            None,
            "removed"
        )
    );
    assert_eq!(created_at, received.unix_timestamp());
    assert_eq!(resolved_by, Some(root.id()));
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    assert!((now - resolved_at.unwrap()).abs() < 60);
    assert_eq!((media, book), (Some(media_id), Some(work)));

    // In the history, not in the open queue.
    let history = root.get("/api/admin/reports?status=resolved").await.json();
    let item = &history["items"][0];
    assert_eq!(item["status"], "removed");
    assert_eq!(item["created_by"], Value::Null);
    assert_eq!(item["resolved_by"]["username"], "root");
    assert_eq!(item["book"]["id"], work);
    assert_eq!(item["removed"], true);
    assert_eq!(
        root.get("/api/admin/reports").await.json()["items"],
        json!([])
    );

    // `{}` records a notice that arrived now, reason `other`; no body records nothing.
    let (_, other) = second_picture(&app).await;
    root.delete_json(&format!("/api/admin/media/{other}"), json!({}))
        .await
        .assert_status(204);
    let rows = notice_rows(&app).await;
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[1].0.as_str(), rows[1].1.as_str()), ("other", ""));
    assert!((now - rows[1].3).abs() < 60);
    let (_, third) = third_picture(&app).await;
    root.delete(&format!("/api/admin/media/{third}"))
        .await
        .assert_status(204);
    assert_eq!(notice_rows(&app).await.len(), 2);
}

/// A work showing Open Library cover 222 (never fetched); returns the work and public id.
async fn second_picture(app: &TestApp) -> (i64, String) {
    ol_picture(app, "Ich bin Circe", 222).await
}

async fn third_picture(app: &TestApp) -> (i64, String) {
    ol_picture(app, "Kirke", 333).await
}

async fn ol_picture(app: &TestApp, title: &str, ol_cover_id: i64) -> (i64, String) {
    let work = app.work(title).await;
    let mut conn = app.db.acquire().await.unwrap();
    let media = ol_cover_media(&mut conn, ol_cover_id)
        .await
        .unwrap()
        .unwrap();
    let set = set_cover(
        &mut conn,
        CoverTarget::Work(work.id),
        media,
        Source::OpenLibrary,
        false,
    )
    .await;
    assert!(set.unwrap());
    let public_id = sqlx::query_scalar("SELECT public_id FROM media WHERE id = $1")
        .bind(media)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    (work.id, public_id)
}

/// A notice about a picture that is down already is still recorded: resolved when the picture
/// came down if it arrived before that (and counted as the admin's), or closed on arrival
/// (no `resolved_by`) if it arrived after.
#[sqlx::test]
async fn a_notice_about_a_removed_picture_is_recorded_too(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let (_, public_id) = second_picture(&app).await;
    root.delete(&format!("/api/admin/media/{public_id}"))
        .await
        .assert_status(204);
    app.exec(&format!(
        "UPDATE media SET removed_at = now() - interval '10 hours' WHERE public_id = '{public_id}'"
    ))
    .await;
    let removed_at = app
        .scalar_i64(&format!(
            "SELECT extract(epoch FROM removed_at)::int8 FROM media WHERE public_id = '{public_id}'"
        ))
        .await;
    let now = time::OffsetDateTime::now_utc();
    let before = (now - time::Duration::hours(20))
        .replace_nanosecond(0)
        .unwrap();
    let after = (now - time::Duration::hours(2))
        .replace_nanosecond(0)
        .unwrap();
    for received in [before, after] {
        root.delete_json(
            &format!("/api/admin/media/{public_id}"),
            json!({"received_at": rfc3339(received), "details": "By email."}),
        )
        .await
        .assert_status(204);
    }
    let rows = notice_rows(&app).await;
    assert_eq!(rows.len(), 2);
    let earlier = &rows[0];
    assert_eq!(
        (earlier.3, earlier.4.as_str(), earlier.5, earlier.6),
        (
            before.unix_timestamp(),
            "removed",
            Some(root.id()),
            Some(removed_at)
        )
    );
    let later = &rows[1];
    assert_eq!(
        (later.3, later.4.as_str(), later.5, later.6),
        (
            after.unix_timestamp(),
            "removed",
            None,
            Some(after.unix_timestamp())
        )
    );
    // The history tells the two apart: only the later one was closed on arrival.
    let history = root.get("/api/admin/reports?status=resolved").await.json();
    let closed: Vec<(i64, bool)> = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["resolved_by"]["id"].as_i64().unwrap_or(0),
                item["closed_on_arrival"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(closed, [(0, true), (root.id(), false)]);
}

/// A notice must name a known reason and must not arrive in the future; a refused body takes
/// nothing down.
#[sqlx::test]
async fn a_takedown_notice_is_checked(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let (work, public_id) = second_picture(&app).await;
    let path = format!("/api/admin/media/{public_id}");
    let tomorrow = time::OffsetDateTime::now_utc() + time::Duration::days(1);
    for (body, detail) in [
        (
            json!({"reason": "boring"}),
            "Input should be 'copyright', 'wrong_book', 'offensive' or 'other'",
        ),
        (
            json!({"received_at": rfc3339(tomorrow)}),
            "Input should be in the past",
        ),
        (json!({"received_at": "yesterday"}), "Invalid value"),
        (json!({"details": 5}), "Input should be a valid string"),
        (
            json!([]),
            "Input should be a valid dictionary or object to extract fields from",
        ),
    ] {
        root.delete_json(&path, body)
            .await
            .assert_detail(400, detail);
    }
    assert!(cover_of(&app, work).await.is_some());
    assert!(notice_rows(&app).await.is_empty());

    // Long details are cut to 2000 characters, as a reader's report is.
    root.delete_json(&path, json!({"details": "x".repeat(2100)}))
        .await
        .assert_status(204);
    assert_eq!(notice_rows(&app).await[0].1.chars().count(), 2000);

    // The document declares the body nullable: `null` is no notice, like no body.
    let (third_work, third) = third_picture(&app).await;
    root.delete_json(&format!("/api/admin/media/{third}"), Value::Null)
        .await
        .assert_status(204);
    assert_eq!(
        cover_of(&app, third_work).await,
        None,
        "the picture is down"
    );
    assert_eq!(notice_rows(&app).await.len(), 1, "and nothing recorded");
}

/// Until ADM2 widens `reports.target_kind`, a notice about an avatar cannot be recorded: the
/// takedown is refused before anything changes, so the admin is never told a notice is on
/// record when it is not. Without a notice (no body, or `null`) the avatar comes down.
#[sqlx::test]
async fn a_notice_about_an_avatar_is_refused(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let avatar = "avataravataravataravatar22";
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, uploaded_by)
         VALUES ('{avatar}', 'avatar', 'upload', {})",
        ada.id()
    ))
    .await;
    let removed = format!(
        "SELECT count(*) FROM media WHERE public_id = '{avatar}' AND removed_at IS NOT NULL"
    );
    let path = format!("/api/admin/media/{avatar}");
    for body in [
        json!({}),
        json!({"details": "Mail: an offensive profile picture."}),
    ] {
        root.delete_json(&path, body)
            .await
            .assert_detail(400, "Only notices about covers can be recorded");
    }
    assert_eq!(app.scalar_i64(&removed).await, 0, "nothing was taken down");
    assert!(notice_rows(&app).await.is_empty());

    root.delete_json(&path, Value::Null)
        .await
        .assert_status(204);
    assert_eq!(app.scalar_i64(&removed).await, 1);
    root.delete(&path).await.assert_status(204);
    assert!(notice_rows(&app).await.is_empty());
}

/// Acceptance (#54): a repeat report of a picture by the same reader answers with the open
/// report it already has: same id, no second row, no second hit on the limit. Another reader's
/// report is a report of its own.
#[sqlx::test]
async fn a_repeat_report_answers_with_the_open_report(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let root = app.admin("root").await;
    let (_, public_id, _) = shown_ol_cover(&app).await;
    let first = report(&ada, &public_id, json!({"reason": "offensive"})).await;
    first.assert_status(201);
    let id = first.json()["id"].as_i64().unwrap();
    assert_eq!(first.json(), json!({"id": id, "status": "open"}));
    // Even with another reason and details, and past the daily limit of new reports.
    for _ in 0..25 {
        let again = report(
            &ada,
            &public_id,
            json!({"reason": "copyright", "details": "Mine."}),
        )
        .await;
        again.assert_status(201);
        assert_eq!(again.json(), json!({"id": id, "status": "open"}));
    }
    let other = report(&grace, &public_id, json!({"reason": "other"})).await;
    other.assert_status(201);
    assert_ne!(other.json()["id"], id);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 2);
    let stored: String = sqlx::query_scalar("SELECT reason FROM reports WHERE id = $1")
        .bind(id)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored, "offensive", "a repeat changes nothing");
    let items = root.get("/api/admin/reports").await.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 2);
    // Every request counts against the per-minute limit (30), the repeats too; a new minute.
    app.state
        .limits
        .report_request_user
        .clear(&format!("u:{}", ada.id()));
    // Ada's one report counted once against the daily limit: 19 more pictures, then the limit.
    for n in 0..20 {
        let other_id = format!("pic{n:0>23}");
        app.exec(&format!(
            "INSERT INTO media (public_id, kind, source, uploaded_by)
             VALUES ('{other_id}', 'cover', 'upload', {})",
            grace.id()
        ))
        .await;
        let res = report(&ada, &other_id, json!({"reason": "other"})).await;
        if n < 19 {
            res.assert_status(201);
        } else {
            res.assert_detail(429, "Slow down a little. Try again later.");
        }
    }
    // A repeat still answers at the limit.
    let again = report(&ada, &public_id, json!({"reason": "other"})).await;
    assert_eq!(again.json(), json!({"id": id, "status": "open"}));
}

/// Acceptance (#54): a report of a picture that was already taken down never enters the open
/// queue. It is recorded as resolved (`removed`, by nobody), answered with `status` `removed`,
/// and a repeat names the same record; a reader whose report took the picture down gets that
/// report back.
#[sqlx::test]
async fn reports_of_a_removed_picture_never_enter_the_queue(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let lin = app.user("lin").await;
    let root = app.admin("root").await;
    let (_, public_id, _) = shown_ol_cover(&app).await;
    let ada_report = report(&ada, &public_id, json!({"reason": "copyright"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    root.delete(&format!("/api/admin/media/{public_id}"))
        .await
        .assert_status(204);

    let again = report(&ada, &public_id, json!({"reason": "other"})).await;
    again.assert_status(201);
    assert_eq!(again.json(), json!({"id": ada_report, "status": "removed"}));
    let late = report(
        &lin,
        &public_id,
        json!({"reason": "offensive", "details": "Gone?"}),
    )
    .await;
    late.assert_status(201);
    let late_id = late.json()["id"].as_i64().unwrap();
    assert_ne!(late_id, ada_report);
    assert_eq!(late.json()["status"], "removed");
    let repeat = report(&lin, &public_id, json!({"reason": "other"})).await;
    assert_eq!(repeat.json(), json!({"id": late_id, "status": "removed"}));

    assert_eq!(
        root.get("/api/admin/reports").await.json(),
        json!({"items": [], "page": 1, "has_more": false})
    );
    let row = sqlx::query_as::<_, (String, Option<i64>, bool, String)>(
        "SELECT status, resolved_by, resolved_at IS NOT NULL, details FROM reports WHERE id = $1",
    )
    .bind(late_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(row, ("removed".into(), None, true, "Gone?".into()));
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 2);
    // Lin's was closed on arrival; Ada's was decided by root.
    let history = root.get("/api/admin/reports?status=resolved").await.json();
    let closed: Vec<(i64, bool)> = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["id"].as_i64().unwrap(),
                item["closed_on_arrival"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(closed, [(late_id, true), (ada_report, false)]);
}

/// Acceptance (#54): "dismiss" closes every open report of the picture, not only the one the
/// admin chose; a report of another picture stays open, and a later report opens a new one.
#[sqlx::test]
async fn dismissing_closes_every_open_report_of_the_picture(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let root = app.admin("root").await;
    let (work, public_id, _) = shown_ol_cover(&app).await;
    let mut ids = Vec::new();
    for who in [&ada, &grace] {
        let res = report(who, &public_id, json!({"reason": "wrong_book"})).await;
        ids.push(res.json()["id"].as_i64().unwrap());
    }
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, uploaded_by)
         VALUES ('otherotherotherotherother1', 'cover', 'upload', {})",
        grace.id()
    ))
    .await;
    let elsewhere = report(
        &ada,
        "otherotherotherotherother1",
        json!({"reason": "other"}),
    )
    .await
    .json()["id"]
        .as_i64()
        .unwrap();

    let res = root
        .post(
            &format!("/api/admin/reports/{}/resolve", ids[1]),
            json!({"action": "dismiss"}),
        )
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["status"], "dismissed");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM reports
             WHERE id IN ({}, {}) AND status = 'dismissed' AND resolved_by = {}",
            ids[0],
            ids[1],
            root.id()
        ))
        .await,
        2
    );
    let items = root.get("/api/admin/reports").await.json()["items"].clone();
    assert_eq!(items.as_array().unwrap().len(), 1);
    assert_eq!(items[0]["id"], elsewhere);
    assert!(cover_of(&app, work).await.is_some(), "the picture stays");
    root.post(
        &format!("/api/admin/reports/{}/resolve", ids[0]),
        json!({"action": "remove"}),
    )
    .await
    .assert_detail(409, "This report was already resolved");
    // Dismissed is not open: a new report of the picture is a new item.
    let fresh = report(&ada, &public_id, json!({"reason": "offensive"})).await;
    fresh.assert_status(201);
    assert!(!ids.contains(&fresh.json()["id"].as_i64().unwrap()));
    assert_eq!(fresh.json()["status"], "open");
}

/// Acceptance (#54): a takedown rejects the open `cover_replace` requests that propose the
/// picture, with a note, so none of them waits for an approval that could only fail. Requests
/// for other pictures stay open.
#[sqlx::test]
async fn a_takedown_rejects_the_cover_requests_for_the_picture(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let lin = app.user("lin").await;
    let root = app.admin("root").await;
    let work = app.work("My Zine").await;
    for (who, color) in [
        (&ada, [200, 0, 0]),
        (&grace, [0, 200, 0]),
        (&lin, [0, 0, 200]),
    ] {
        let res = who
            .post_multipart(
                &format!("/api/books/works/{}/cover", work.id),
                vec![Part::file("c.jpg", "image/jpeg", jpeg(color))],
            )
            .await;
        assert!(res.status.is_success(), "{}", res.text());
    }
    let requests: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT r.id, m.public_id, r.created_by FROM catalog_edit_requests r
         JOIN media m ON m.id = r.media_id ORDER BY r.id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(requests.len(), 2, "grace's and lin's photos wait");
    let (graces, picture, _) = requests[0].clone();
    let (lins, _, _) = requests[1].clone();

    // Through a report's "remove", like the direct takedown (both use take_down_locked).
    let reported = report(&ada, &picture, json!({"reason": "offensive"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    root.post(
        &format!("/api/admin/reports/{reported}/resolve"),
        json!({"action": "remove"}),
    )
    .await
    .assert_status(200);
    let decided = sqlx::query_as::<_, (String, Option<i64>, bool, String)>(
        "SELECT status, decided_by, decided_at IS NOT NULL, decision_note
         FROM catalog_edit_requests WHERE id = $1",
    )
    .bind(graces)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        decided,
        (
            "rejected".into(),
            Some(root.id()),
            true,
            "The picture was taken down".into()
        )
    );
    let queue = root.get("/api/admin/edit-requests").await.json();
    let open: Vec<i64> = queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect();
    assert_eq!(open, [lins]);
    root.post(
        &format!("/api/admin/edit-requests/{graces}/approve"),
        json!({}),
    )
    .await
    .assert_detail(409, "This request was already decided");
}

/// Reports name a cover by its public id: unknown ids and avatars are 404; the reason is one
/// of four; details are trimmed and cut at 2000 characters; 20 new reports per user and day.
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
    // 20 new reports a day (a repeat of an open report is no new report): 19 more pictures.
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, uploaded_by)
         SELECT 'limit' || lpad(n::text, 21, '0'), 'cover', 'upload', {}
         FROM generate_series(1, 20) AS n",
        ada.id()
    ))
    .await;
    let picture = |n: usize| format!("limit{n:0>21}");
    for n in 1..20 {
        report(&ada, &picture(n), json!({"reason": "other"}))
            .await
            .assert_status(201);
    }
    let limited = report(&ada, &picture(20), json!({"reason": "other"})).await;
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
    until_lock_waits(db, 1).await;
}

/// Waits until `sessions` sessions of this test's database wait for a lock at once.
async fn until_lock_waits(db: &PgPool, sessions: i64) {
    assert!(
        lock_waits(db, sessions).await,
        "fewer than {sessions} sessions waited for a lock"
    );
}

/// Polls for up to 10 s until `sessions` sessions of this test's database wait for a lock at
/// once; whether they did. A test that releases a lock after this asserts the outcome itself,
/// so a missing lock shows as a wrong result rather than a timeout.
async fn lock_waits(db: &PgPool, sessions: i64) -> bool {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting >= sessions {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn media_id(app: &TestApp, public_id: &str) -> i64 {
    app.scalar_i64(&format!(
        "SELECT id FROM media WHERE public_id = '{public_id}'"
    ))
    .await
}

/// The lock order (conventions §7.2): a report locks the picture `FOR SHARE` before it reads
/// whether the picture is down, so a report filed while a takedown runs waits for it and is
/// recorded as closed. Without that lock it would read the picture as live and leave an open
/// report of a removed picture in the queue.
#[sqlx::test]
async fn a_report_filed_during_a_takedown_sees_the_removal(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, public_id) = second_picture(&app).await;
    let media = media_id(&app, &public_id).await;

    // A takedown that holds the picture while the report starts.
    let mut takedown = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM media WHERE id = $1 FOR UPDATE")
        .bind(media)
        .execute(&mut *takedown)
        .await
        .unwrap();
    let finish_takedown = async {
        until_a_lock_wait(&app.db).await;
        take_down_locked(&mut takedown, app.media.as_ref(), media, root.id())
            .await
            .unwrap();
        takedown.commit().await.unwrap();
    };
    let filing = report(&ada, &public_id, json!({"reason": "offensive"}));
    let (res, ()) = tokio::join!(filing, finish_takedown);
    res.assert_status(201);
    let id = res.json()["id"].as_i64().unwrap();
    assert_eq!(res.json(), json!({"id": id, "status": "removed"}));
    assert_eq!(
        root.get("/api/admin/reports").await.json(),
        json!({"items": [], "page": 1, "has_more": false})
    );
    let row = sqlx::query_as::<_, (String, Option<i64>, bool)>(
        "SELECT status, resolved_by, resolved_at = created_at FROM reports WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(row, ("removed".into(), None, true), "closed on arrival");
    assert_eq!(cover_of(&app, work).await, None);
}

/// The lock order (conventions §7.2): a dismissal locks the picture before it closes the
/// picture's open reports. A report that is being filed holds the picture `FOR SHARE`, so the
/// dismissal waits for it and then closes it with the others. Here something else holds the
/// picture first, so the report and then the dismissal queue behind it in that order.
#[sqlx::test]
async fn a_dismissal_waits_for_a_report_being_filed_and_closes_it_too(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let root = app.admin("root").await;
    let (work, public_id) = second_picture(&app).await;
    let media = media_id(&app, &public_id).await;
    let graces = report(&grace, &public_id, json!({"reason": "wrong_book"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();

    let mut holder = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM media WHERE id = $1 FOR UPDATE")
        .bind(media)
        .execute(&mut *holder)
        .await
        .unwrap();
    let filing = report(&ada, &public_id, json!({"reason": "offensive"}));
    let dismissing = async {
        until_lock_waits(&app.db, 1).await;
        root.post(
            &format!("/api/admin/reports/{graces}/resolve"),
            json!({"action": "dismiss"}),
        )
        .await
    };
    let release = async {
        lock_waits(&app.db, 2).await;
        holder.commit().await.unwrap();
    };
    let (filed, dismissed, ()) = tokio::join!(filing, dismissing, release);
    filed.assert_status(201);
    assert_eq!(filed.json()["status"], "open");
    dismissed.assert_status(200);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM reports WHERE status = 'dismissed' AND resolved_by = {}",
            root.id()
        ))
        .await,
        2,
        "Ada's report, filed while the dismissal waited, was dismissed too"
    );
    assert_eq!(
        root.get("/api/admin/reports").await.json()["items"],
        json!([])
    );
    assert!(cover_of(&app, work).await.is_some(), "the picture stays");
}

/// Holds the server's advisory lock for `reader`'s reports of `media` (space 0x6263_0200, key
/// hashtext('<user id>:<media id>')) on a connection of its own, as a request of the reader that
/// has its turn would, until `pg_advisory_unlock_all()` on that connection.
async fn hold_reader_turn(app: &TestApp, reader: i64, media: i64) -> PoolConnection<Postgres> {
    let mut holder = app.db.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1, hashtext($2))")
        .bind(0x6263_0200_i32)
        .bind(format!("{reader}:{media}"))
        .execute(&mut *holder)
        .await
        .unwrap();
    holder
}

/// Security review, Phase 2 Wave 1 #1: a repeat report is answered from a plain read and takes
/// no lock. A repeat that locked the picture `FOR SHARE` and then waited for the reader's turn
/// let one account keep the picture share-locked without a break, by sending repeats (which
/// the daily limit never counted) faster than they ended, and a takedown waited until the
/// request deadline. Here repeats answer at once while another request of the reader has its
/// turn, and while a takedown holds the picture.
#[sqlx::test]
async fn repeat_reports_take_no_lock(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (_, public_id) = second_picture(&app).await;
    let media = media_id(&app, &public_id).await;
    let id = report(&ada, &public_id, json!({"reason": "offensive"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let repeats = |n: usize| {
        futures::future::join_all(
            (0..n).map(|_| report(&ada, &public_id, json!({"reason": "other"}))),
        )
    };

    let mut holder = hold_reader_turn(&app, ada.id(), media).await;
    let answered = tokio::time::timeout(Duration::from_secs(5), repeats(3))
        .await
        .expect("repeats waited for the reader's turn");
    for res in answered {
        assert_eq!(res.json(), json!({"id": id, "status": "open"}));
    }
    sqlx::query("SELECT pg_advisory_unlock_all()")
        .execute(&mut *holder)
        .await
        .unwrap();

    let mut takedown = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM media WHERE id = $1 FOR UPDATE")
        .bind(media)
        .execute(&mut *takedown)
        .await
        .unwrap();
    let answered = tokio::time::timeout(Duration::from_secs(5), repeats(3))
        .await
        .expect("repeats waited for the takedown's lock on the picture");
    for res in answered {
        assert_eq!(res.json(), json!({"id": id, "status": "open"}));
    }
    takedown.rollback().await.unwrap();
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 1);
}

/// Security review, Phase 2 Wave 1 #1: a new report waits for its reader's turn before it locks
/// the picture, so only the request whose turn it is holds the picture, and one that waits
/// holds nothing a takedown needs. Locked the other way round, every waiting request held the
/// picture `FOR SHARE`, and the takedown below waited for the reader's turn to come.
#[sqlx::test]
async fn a_report_waiting_for_its_turn_does_not_hold_off_a_takedown(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let (work, public_id) = second_picture(&app).await;
    let media = media_id(&app, &public_id).await;

    let mut holder = hold_reader_turn(&app, ada.id(), media).await;
    let filing = report(&ada, &public_id, json!({"reason": "offensive"}));
    let take_down = async {
        until_a_lock_wait(&app.db).await;
        let taken = tokio::time::timeout(
            Duration::from_secs(5),
            root.delete(&format!("/api/admin/media/{public_id}")),
        )
        .await;
        sqlx::query("SELECT pg_advisory_unlock_all()")
            .execute(&mut *holder)
            .await
            .unwrap();
        taken
    };
    let (filed, taken) = tokio::join!(filing, take_down);
    taken
        .expect("the takedown waited for a report that waited for its turn")
        .assert_status(204);
    assert_eq!(cover_of(&app, work).await, None);
    // The report went on after the takedown and saw it.
    filed.assert_status(201);
    let id = filed.json()["id"].as_i64().unwrap();
    assert_eq!(filed.json(), json!({"id": id, "status": "removed"}));
    assert_eq!(
        root.get("/api/admin/reports").await.json()["items"],
        json!([])
    );
}

/// Security review, Phase 2 Wave 1 #1: every report request counts against a per-account limit
/// of 30 a minute, repeats and unknown pictures included, so repeats, which the daily limit of
/// new reports never counts, cannot be sent without end. Another reader is not affected.
#[sqlx::test]
async fn every_report_request_counts_against_a_minute_limit(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let (_, public_id) = second_picture(&app).await;
    let id = report(&ada, &public_id, json!({"reason": "offensive"}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    report(
        &ada,
        "abcdefghijklmnopqrstuvwxyz",
        json!({"reason": "other"}),
    )
    .await
    .assert_detail(404, "Not found");
    for _ in 0..28 {
        let again = report(&ada, &public_id, json!({"reason": "other"})).await;
        assert_eq!(again.json(), json!({"id": id, "status": "open"}));
    }
    let limited = report(&ada, &public_id, json!({"reason": "other"})).await;
    limited.assert_detail(429, "Slow down a little. Try again later.");
    let wait: u64 = limited.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=60).contains(&wait), "{wait}");
    report(&grace, &public_id, json!({"reason": "other"}))
        .await
        .assert_status(201);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 2);
}

/// Two identical reports by one reader at once are one report: the advisory lock on (reader,
/// picture) makes the second wait until the first is committed, and then it finds that one.
/// Without the lock both would find no earlier report and both would insert.
#[sqlx::test]
async fn two_reports_at_once_by_one_reader_are_one_report(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let (_, public_id) = second_picture(&app).await;
    let media = media_id(&app, &public_id).await;

    // The server's lock: space 0x6263_0200, key hashtext('<user id>:<media id>').
    let key = format!("{}:{media}", ada.id());
    let mut holder = app.db.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock($1, hashtext($2))")
        .bind(0x6263_0200_i32)
        .bind(&key)
        .execute(&mut *holder)
        .await
        .unwrap();
    let release = async {
        lock_waits(&app.db, 2).await;
        sqlx::query("SELECT pg_advisory_unlock($1, hashtext($2))")
            .bind(0x6263_0200_i32)
            .bind(&key)
            .execute(&mut *holder)
            .await
            .unwrap();
    };
    let body = json!({"reason": "offensive"});
    let (first, second, ()) = tokio::join!(
        report(&ada, &public_id, body.clone()),
        report(&ada, &public_id, body.clone()),
        release
    );
    first.assert_status(201);
    second.assert_status(201);
    assert_eq!(first.json()["status"], "open");
    assert_eq!(first.json(), second.json(), "the same report");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM reports").await, 1);
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
