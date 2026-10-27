//! `GET /media/{public_id}/{variant}` (#43, endpoint-map §6): stored pictures, the lazy and
//! coalesced Open Library cover fetch, the no-store rule for every non-200, and the prefetch
//! worker.

mod common;

use std::time::Duration;

use bookclub_server::catalog::shared::books::{BookRef, book_out};
use bookclub_server::catalog::shared::ingest::{self, Mode, Source, WorkRecord};
use bookclub_server::media::service::run_prefetch;
use bookclub_server::media::shared::MediaStore;
use common::*;
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

fn jpeg(color: [u8; 3], width: u32, height: u32) -> Vec<u8> {
    let image = RgbImage::from_pixel(width, height, Rgb(color));
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 90)
        .write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)
        .unwrap();
    out
}

/// An Open Library work whose cover is Open Library cover `cover_id`; returns the work id and
/// the cover's media id and public id.
async fn ol_cover_work(app: &TestApp, ol_id: &str, cover_id: i64) -> (i64, i64, String) {
    let mut conn = app.db.acquire().await.unwrap();
    let record = WorkRecord {
        ol_work_id: Some(ol_id.to_owned()),
        ol_cover_id: Some(cover_id),
        ..WorkRecord::new(Source::OpenLibrary, "Circe")
    };
    let work_id = ingest::upsert_work(&mut conn, &record, Mode::Refresh)
        .await
        .unwrap();
    let (media_id, public_id): (i64, String) = sqlx::query_as(
        "SELECT m.id, m.public_id FROM works w JOIN media m ON m.id = w.cover_media_id
         WHERE w.id = $1",
    )
    .bind(work_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    (work_id, media_id, public_id)
}

/// Open Library serves cover `cover_id` as `body` (status 200).
async fn serve_cover(app: &TestApp, cover_id: i64, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path(format!("/b/id/{cover_id}-L.jpg")))
        .and(query_param("default", "false"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/jpeg")
                .set_body_bytes(body),
        )
        .mount(&app.upstream)
        .await;
}

async fn cover_requests(app: &TestApp) -> usize {
    app.upstream_requests()
        .await
        .iter()
        .filter(|request| request.url.path().starts_with("/b/id/"))
        .count()
}

fn assert_no_store(res: &TestResponse) {
    assert_eq!(
        res.header("cache-control"),
        Some("no-store"),
        "{}",
        res.text()
    );
}

/// Acceptance (#43): a cover renders with no client request to Open Library. The book's URL is
/// same-origin, and the bytes come from the covers host exactly once, however often and in
/// whichever size they are shown.
#[sqlx::test]
async fn a_cover_is_fetched_once_and_served_from_our_origin(db: PgPool) {
    let app = TestApp::new(db).await;
    let (work_id, _, public_id) = ol_cover_work(&app, "OL1W", 111).await;
    serve_cover(&app, 111, jpeg([30, 90, 160], 300, 450)).await;
    let mut conn = app.db.acquire().await.unwrap();
    let book = book_out(&mut conn, BookRef::work(work_id))
        .await
        .unwrap()
        .unwrap();
    let large = book.cover_url.unwrap();
    let small = book.cover_thumb_url.unwrap();
    assert_eq!(large, format!("/media/{public_id}/l"));
    assert_eq!(small, format!("/media/{public_id}/s"));

    let anon = app.anon();
    let res = anon.get(&large).await;
    res.assert_status(200);
    assert_eq!(res.header("content-type"), Some("image/webp"));
    assert_eq!(
        res.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(res.header("x-content-type-options"), Some("nosniff"));
    let image = image::load_from_memory(&res.body).unwrap();
    assert_eq!((image.width(), image.height()), (300, 450));
    let thumb = anon.get(&small).await;
    thumb.assert_status(200);
    let thumb = image::load_from_memory(&thumb.body).unwrap();
    assert_eq!((thumb.width(), thumb.height()), (240, 360));
    anon.get(&large).await.assert_status(200);

    let requests = app.upstream_requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/b/id/111-L.jpg");
    assert_eq!(requests[0].url.query(), Some("default=false"));
    let row: (String, Vec<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT content_type, variants, dominant_color, octet_length(sha256)::int8
         FROM media WHERE public_id = $1",
    )
    .bind(&public_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(row.0, "image/webp");
    assert_eq!(row.1, ["l", "s"]);
    assert!(row.2.unwrap().starts_with('#'));
    assert_eq!(row.3, 32);
}

/// Concurrent requests for one cover share one fetch.
#[sqlx::test]
async fn concurrent_requests_share_one_fetch(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, _, public_id) = ol_cover_work(&app, "OL1W", 111).await;
    Mock::given(method("GET"))
        .and(path("/b/id/111-L.jpg"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(jpeg([200, 30, 30], 100, 150))
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&app.upstream)
        .await;
    let anon = app.anon();
    let large = format!("/media/{public_id}/l");
    let small = format!("/media/{public_id}/s");
    let (a, b, c) = tokio::join!(anon.get(&large), anon.get(&small), anon.get(&large));
    for res in [a, b, c] {
        res.assert_status(200);
    }
    assert_eq!(cover_requests(&app).await, 1);
}

/// Unknown ids, malformed ids and variants a picture does not have are 404, never cached.
#[sqlx::test]
async fn unknown_pictures_and_variants_are_404(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, _, public_id) = ol_cover_work(&app, "OL1W", 111).await;
    let anon = app.anon();
    for path in [
        "/media/abcdefghijklmnopqrstuvwxyz/l".to_owned(),
        "/media/x/l".to_owned(),
        "/media/ABCDEFGHIJKLMNOPQRSTUVWXYZ/l".to_owned(),
        "/media/abc%00defghijklmnopqrstuvwx/l".to_owned(),
        format!("/media/{public_id}/a"),
        format!("/media/{public_id}/xl"),
    ] {
        let res = anon.get(&path).await;
        res.assert_detail(404, "Not found");
        assert_no_store(&res);
    }
    assert_eq!(cover_requests(&app).await, 0);
}

/// Open Library has no such cover: 404, and the cover is not asked for again within a week.
#[sqlx::test]
async fn a_missing_open_library_cover_is_not_asked_for_again_for_a_week(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, media_id, public_id) = ol_cover_work(&app, "OL1W", 111).await;
    Mock::given(method("GET"))
        .and(path("/b/id/111-L.jpg"))
        .respond_with(ResponseTemplate::new(404))
        .up_to_n_times(1)
        .mount(&app.upstream)
        .await;
    let url = format!("/media/{public_id}/l");
    let res = app.anon().get(&url).await;
    res.assert_detail(404, "Not found");
    assert_no_store(&res);
    let (failed, color): (bool, Option<String>) = sqlx::query_as(
        "SELECT fetch_failed_at IS NOT NULL, dominant_color FROM media WHERE id = $1",
    )
    .bind(media_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(failed);
    assert_eq!(color.as_deref(), Some(""));
    app.anon().get(&url).await.assert_status(404);
    assert_eq!(cover_requests(&app).await, 1);

    // A week later it is tried again, and the picture Open Library has now is stored.
    app.exec(&format!(
        "UPDATE media SET fetch_failed_at = now() - interval '8 days' WHERE id = {media_id}"
    ))
    .await;
    serve_cover(&app, 111, jpeg([30, 160, 60], 100, 150)).await;
    app.anon().get(&url).await.assert_status(200);
    assert_eq!(cover_requests(&app).await, 2);
    let (failed, color): (bool, Option<String>) = sqlx::query_as(
        "SELECT fetch_failed_at IS NOT NULL, dominant_color FROM media WHERE id = $1",
    )
    .bind(media_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(!failed);
    assert!(color.unwrap().starts_with('#'));
}

/// An outage records nothing: 502 without caching, and the next view tries again.
#[sqlx::test]
async fn an_open_library_outage_is_retried_on_the_next_view(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, media_id, public_id) = ol_cover_work(&app, "OL1W", 111).await;
    // The first attempt and its one retry fail.
    Mock::given(method("GET"))
        .and(path("/b/id/111-L.jpg"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .mount(&app.upstream)
        .await;
    serve_cover(&app, 111, jpeg([30, 160, 60], 100, 150)).await;
    let url = format!("/media/{public_id}/s");
    let res = app.anon().get(&url).await;
    res.assert_detail(502, "Could not load the cover right now. Try again.");
    assert_no_store(&res);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE id = {media_id}
               AND fetch_failed_at IS NULL AND dominant_color IS NULL AND storage_key IS NULL"
        ))
        .await,
        1
    );
    app.anon().get(&url).await.assert_status(200);
    assert_eq!(cover_requests(&app).await, 3);
}

/// While Open Library cools down, a cover without bytes is 429, without a request.
#[sqlx::test]
async fn a_throttled_open_library_is_429(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, _, public_id) = ol_cover_work(&app, "OL1W", 111).await;
    app.state
        .outbound
        .ol_covers
        .cool_down(Duration::from_secs(30));
    let res = app.anon().get(&format!("/media/{public_id}/l")).await;
    res.assert_detail(
        429,
        "The library is busy. Wait a few seconds and try again.",
    );
    assert_no_store(&res);
    let retry: u64 = res.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=30).contains(&retry), "{retry}");
    assert_eq!(cover_requests(&app).await, 0);
}

/// A removed picture is 410 on every variant and never fetched again.
#[sqlx::test]
async fn a_removed_cover_is_410_and_never_fetched(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, media_id, public_id) = ol_cover_work(&app, "OL1W", 111).await;
    serve_cover(&app, 111, jpeg([30, 160, 60], 100, 150)).await;
    app.exec(&format!(
        "UPDATE media SET removed_at = now() WHERE id = {media_id}"
    ))
    .await;
    for variant in ["l", "s", "a", "zz"] {
        let res = app
            .anon()
            .get(&format!("/media/{public_id}/{variant}"))
            .await;
        res.assert_detail(410, "This picture was removed");
        assert_no_store(&res);
    }
    assert_eq!(cover_requests(&app).await, 0);
}

/// An imported avatar is the legacy JPEG: it is served with the row's content type.
#[sqlx::test]
async fn imported_jpeg_avatars_keep_their_content_type(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bytes = jpeg([10, 20, 30], 256, 256);
    let public_id = "legacylegacylegacylegacy22";
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, storage_key, content_type, variants, uploaded_by)
         VALUES ('{public_id}', 'avatar', 'legacy', 'avatars/le/{public_id}', 'image/jpeg', '{{a}}', {})",
        ada.id()
    ))
    .await;
    app.media
        .put(
            &format!("avatars/le/{public_id}/a"),
            bytes.clone().into(),
            "image/jpeg",
        )
        .await
        .unwrap();
    let res = app.anon().get(&format!("/media/{public_id}/a")).await;
    res.assert_status(200);
    assert_eq!(res.header("content-type"), Some("image/jpeg"));
    assert_eq!(res.body.as_ref(), bytes.as_slice());
}

/// A stored row whose bytes are missing answers 404 (not cached) instead of an error.
#[sqlx::test]
async fn a_stored_picture_without_bytes_is_404(db: PgPool) {
    let app = TestApp::new(db).await;
    let public_id = "nobytesnobytesnobytesnob22";
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, storage_key, content_type, variants)
         VALUES ('{public_id}', 'cover', 'upload', 'covers/no/{public_id}', 'image/webp', '{{l,s}}')"
    ))
    .await;
    let res = app.anon().get(&format!("/media/{public_id}/l")).await;
    res.assert_detail(404, "Not found");
    assert_no_store(&res);
}

/// The prefetch worker fetches queued covers before anyone asks; removed and unknown rows are
/// skipped, and it stops when told to.
#[sqlx::test]
async fn the_prefetch_worker_stores_queued_covers(db: PgPool) {
    let app = TestApp::new(db).await;
    let (_, first, first_public) = ol_cover_work(&app, "OL1W", 111).await;
    let (_, second, _) = ol_cover_work(&app, "OL2W", 222).await;
    let (_, removed, _) = ol_cover_work(&app, "OL3W", 333).await;
    app.exec(&format!(
        "UPDATE media SET removed_at = now() WHERE id = {removed}"
    ))
    .await;
    for id in [111, 222, 333] {
        serve_cover(&app, id, jpeg([100, 50, 25], 60, 90)).await;
    }
    app.state
        .media_service
        .prefetch(&[first, removed, second, 999_999, first]);
    let shutdown = CancellationToken::new();
    let worker = tokio::spawn(run_prefetch(app.state.clone(), shutdown.clone()));
    for _ in 0..500 {
        let stored = app
            .scalar_i64(&format!(
                "SELECT count(*) FROM media WHERE id IN ({first}, {second}) AND storage_key IS NOT NULL"
            ))
            .await;
        if stored == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(10), worker)
        .await
        .expect("the worker stops")
        .unwrap();
    let paths: Vec<String> = app
        .upstream_requests()
        .await
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();
    assert_eq!(paths, ["/b/id/111-L.jpg", "/b/id/222-L.jpg"]);
    app.anon()
        .get(&format!("/media/{first_public}/l"))
        .await
        .assert_status(200);
    assert_eq!(cover_requests(&app).await, 2);

    // The worker can be started again (the queue is handed back).
    app.state.media_service.prefetch(&[second]);
    let shutdown = CancellationToken::new();
    let worker = tokio::spawn(run_prefetch(app.state.clone(), shutdown.clone()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    shutdown.cancel();
    worker.await.unwrap();
    assert_eq!(cover_requests(&app).await, 2);
}

/// A takedown while the fetch waits for Open Library: the fetched bytes are not kept, and the
/// waiting request answers 410.
#[sqlx::test]
async fn a_takedown_during_a_fetch_leaves_no_bytes(db: PgPool) {
    let app = TestApp::new(db).await;
    let root = app.admin("root").await;
    let (_, media_id, public_id) = ol_cover_work(&app, "OL1W", 111).await;
    Mock::given(method("GET"))
        .and(path("/b/id/111-L.jpg"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(jpeg([200, 30, 30], 100, 150))
                .set_delay(Duration::from_secs(1)),
        )
        .mount(&app.upstream)
        .await;
    let anon = app.anon();
    let url = format!("/media/{public_id}/l");
    let take_down = async {
        for _ in 0..200 {
            if cover_requests(&app).await > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        root.delete(&format!("/api/admin/media/{public_id}"))
            .await
            .assert_status(204);
    };
    let (res, ()) = tokio::join!(anon.get(&url), take_down);
    res.assert_detail(410, "This picture was removed");
    assert!(app.media.keys().is_empty(), "{:?}", app.media.keys());
    let row: (Option<String>, bool) =
        sqlx::query_as("SELECT storage_key, removed_at IS NOT NULL FROM media WHERE id = $1")
            .bind(media_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(row, (None, true));
}
