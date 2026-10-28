//! Cover photos (#43, endpoint-map §6): `POST /api/books/works/{work_id}/cover`. The first
//! cover of an edition or work is set at once with provenance `user`; any later one becomes a
//! `cover_replace` request for the admin queue.

mod common;

use std::time::Duration;

use bookclub_server::catalog::merge_works;
use common::*;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage};
use serde_json::{Value, json};
use sqlx::PgPool;

fn png(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
    let image = RgbImage::from_pixel(width, height, Rgb(color));
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)
        .unwrap();
    out
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/images/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

async fn upload(
    client: &TestClient,
    work: &str,
    data: Vec<u8>,
    edition: Option<&str>,
) -> TestResponse {
    let mut parts = vec![Part::file("cover.png", "image/png", data)];
    if let Some(edition) = edition {
        parts.push(Part::text("edition_id", edition));
    }
    client
        .post_multipart(&format!("/api/books/works/{work}/cover"), parts)
        .await
}

/// `(cover_media_id, its provenance)` of a work.
async fn work_cover(app: &TestApp, work: i64) -> (Option<i64>, Option<String>) {
    sqlx::query_as(
        "SELECT cover_media_id, field_sources ->> 'cover_media_id' FROM works WHERE id = $1",
    )
    .bind(work)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

async fn open_requests(app: &TestApp) -> Vec<(i64, Option<i64>, Option<i64>, Value, Option<i64>)> {
    sqlx::query_as(
        "SELECT work_id, edition_id, media_id, payload, created_by FROM catalog_edit_requests
         WHERE kind = 'cover_replace' AND status = 'open' ORDER BY id",
    )
    .fetch_all(&app.db)
    .await
    .unwrap()
}

fn public_id(url: &Value) -> String {
    let url = url.as_str().unwrap();
    url.strip_prefix("/media/")
        .and_then(|rest| rest.strip_suffix("/l"))
        .unwrap()
        .to_owned()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// The first photo of a book without a cover is its cover at once, with provenance `user`,
/// served from our origin in both sizes.
#[sqlx::test]
async fn a_first_upload_becomes_the_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = app.work("My Zine").await;
    let data = png(400, 600, [30, 90, 160]);
    let res = upload(&ada, &work.id.to_string(), data.clone(), None).await;
    res.assert_status(200);
    let book = res.json()["book"].clone();
    assert_eq!(book["id"], work.id);
    let public_id = public_id(&book["cover_url"]);
    assert_eq!(book["cover_thumb_url"], format!("/media/{public_id}/s"));
    assert_eq!(book["cover_public_id"], public_id);
    assert!(book["spine_color"].as_str().unwrap().starts_with('#'));

    let (cover, owner) = work_cover(&app, work.id).await;
    assert!(cover.is_some());
    assert_eq!(owner.as_deref(), Some("user"));
    let row: (String, String, Vec<String>, i32, i32, Option<i64>, Vec<u8>) = sqlx::query_as(
        "SELECT kind, source, variants, width, height, uploaded_by, sha256 FROM media WHERE public_id = $1",
    )
    .bind(&public_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!((row.0.as_str(), row.1.as_str()), ("cover", "upload"));
    assert_eq!(row.2, ["l", "s"]);
    assert_eq!((row.3, row.4), (400, 600));
    assert_eq!(row.5, Some(ada.id()));
    assert_eq!(
        row.6,
        bookclub_server::media::pipeline::sha256(&data).to_vec()
    );

    for (variant, size) in [("l", (400, 600)), ("s", (240, 360))] {
        let res = app
            .anon()
            .get(&format!("/media/{public_id}/{variant}"))
            .await;
        res.assert_status(200);
        assert_eq!(res.header("content-type"), Some("image/webp"));
        let image = image::load_from_memory(&res.body).unwrap();
        assert_eq!((image.width(), image.height()), size);
    }
    assert!(app.upstream_requests().await.is_empty());
}

/// A book that has a cover: the photo waits for an admin as a `cover_replace` request, which
/// CAT's approve applies with provenance `admin`.
#[sqlx::test]
async fn a_second_upload_becomes_a_cover_replace_request(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let root = app.admin("root").await;
    let work = app.work("My Zine").await;
    upload(&ada, &work.id.to_string(), png(40, 60, [200, 30, 30]), None)
        .await
        .assert_status(200);
    let (first, _) = work_cover(&app, work.id).await;

    let res = upload(
        &grace,
        &work.id.to_string(),
        png(40, 60, [30, 200, 30]),
        None,
    )
    .await;
    res.assert_status(202);
    let request_id = res.json()["request_id"].as_i64().unwrap();
    let requests = open_requests(&app).await;
    assert_eq!(requests.len(), 1);
    let (work_id, edition_id, media_id, payload, created_by) = requests[0].clone();
    assert_eq!((work_id, edition_id), (work.id, None));
    assert_eq!(payload, json!({}));
    assert_eq!(created_by, Some(grace.id()));
    assert_ne!(media_id, first);
    assert_eq!(work_cover(&app, work.id).await.0, first);

    root.post(
        &format!("/api/admin/edit-requests/{request_id}/approve"),
        json!({}),
    )
    .await
    .assert_status(200);
    let (cover, owner) = work_cover(&app, work.id).await;
    assert_eq!(cover, media_id);
    assert_eq!(owner.as_deref(), Some("admin"));
}

/// With `edition_id` the photo is that edition's first cover (and the work's, which had none);
/// an edition of another book or a malformed id is refused.
#[sqlx::test]
async fn an_edition_photo_is_the_editions_first_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = app.work("My Zine").await;
    let edition = app.edition(&work, "9780316556347").await;
    let other = app.work("Other").await;
    let other_edition = app.edition(&other, "9783694050797").await;
    let work_path = work.id.to_string();

    let res = upload(
        &ada,
        &work_path,
        png(40, 60, [200, 30, 30]),
        Some(&edition.id.to_string()),
    )
    .await;
    res.assert_status(200);
    let book = res.json()["book"].clone();
    let edition_cover: Option<i64> =
        sqlx::query_scalar("SELECT cover_media_id FROM editions WHERE id = $1")
            .bind(edition.id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert!(edition_cover.is_some());
    assert_eq!(work_cover(&app, work.id).await.0, edition_cover);
    let shown: i64 = sqlx::query_scalar("SELECT id FROM media WHERE public_id = $1")
        .bind(public_id(&book["cover_url"]))
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(Some(shown), edition_cover);

    let res = upload(
        &ada,
        &work_path,
        png(40, 60, [30, 200, 30]),
        Some(&edition.id.to_string()),
    )
    .await;
    res.assert_status(202);
    let requests = open_requests(&app).await;
    assert_eq!(requests[0].1, Some(edition.id));

    upload(
        &ada,
        &work_path,
        png(40, 60, [1, 2, 3]),
        Some(&other_edition.id.to_string()),
    )
    .await
    .assert_detail(400, "That edition belongs to another book");
    upload(&ada, &work_path, png(40, 60, [1, 2, 3]), Some("abc"))
        .await
        .assert_detail(
            400,
            "Input should be a valid integer, unable to parse string as an integer",
        );
    assert_eq!(app.scalar_i64("SELECT count(*) FROM media").await, 2);
}

/// Acceptance (#43): a phone photo with GPS EXIF comes back without any metadata, upright.
#[sqlx::test]
async fn gps_exif_is_stripped_and_the_photo_is_upright(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = app.work("My Zine").await;
    let photo = fixture("gps.jpg");
    for needle in [&b"GPS Phone 9000"[..], b"BookclubTestCam", b"SecretEditor"] {
        assert!(contains(&photo, needle));
    }
    let res = ada
        .post_multipart(
            &format!("/api/books/works/{}/cover", work.id),
            vec![Part::file("phone.jpg", "image/jpeg", photo)],
        )
        .await;
    res.assert_status(200);
    let public_id = public_id(&res.json()["book"]["cover_url"]);
    for variant in ["l", "s"] {
        let image = app
            .anon()
            .get(&format!("/media/{public_id}/{variant}"))
            .await;
        image.assert_status(200);
        let webp = &image.body;
        assert_eq!(&webp[..4], b"RIFF");
        let mut offset = 12;
        while offset + 8 <= webp.len() {
            let chunk = &webp[offset..offset + 4];
            assert!(
                matches!(chunk, b"VP8 " | b"VP8L" | b"VP8X" | b"ALPH"),
                "unexpected chunk {:?}",
                String::from_utf8_lossy(chunk)
            );
            let size =
                u32::from_le_bytes(webp[offset + 4..offset + 8].try_into().unwrap()) as usize;
            offset += 8 + size + size % 2;
        }
        for needle in [&b"GPS"[..], b"BookclubTestCam", b"SecretEditor", b"Exif"] {
            assert!(!contains(webp, needle));
        }
        // Stored 120×80 with orientation 6 (rotate clockwise): shown 80×120, the stored top
        // (red) on the right, the stored bottom (blue) on the left.
        let decoded = image::load_from_memory(webp).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (80, 120));
        let left = decoded.get_pixel(20, 60).0;
        let right = decoded.get_pixel(60, 60).0;
        assert!(left[2] > 150 && left[0] < 80, "{left:?}");
        assert!(right[0] > 150 && right[2] < 80, "{right:?}");
    }
}

/// Waits until `n` sessions of this test's database wait for a lock.
async fn until_lock_waits(db: &PgPool, n: i64) {
    for _ in 0..500 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        if waiting >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("fewer than {n} sessions waited for a lock");
}

/// Acceptance (#43): two concurrent first uploads give one applied cover and one
/// `cover_replace` request. Both uploads wait for the work row, which the test holds until
/// both are queued behind it.
#[sqlx::test]
async fn two_concurrent_first_uploads_give_one_cover_and_one_request(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let work = app.work("My Zine").await;
    let path = work.id.to_string();

    let mut blocker = app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM works WHERE id = $1 FOR UPDATE")
        .bind(work.id)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let release = async {
        until_lock_waits(&app.db, 2).await;
        blocker.commit().await.unwrap();
    };
    let (a, b, ()) = tokio::join!(
        upload(&ada, &path, png(40, 60, [200, 30, 30]), None),
        upload(&grace, &path, png(40, 60, [30, 200, 30]), None),
        release
    );
    let mut statuses = [a.status.as_u16(), b.status.as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 202], "{} / {}", a.text(), b.text());

    let (cover, owner) = work_cover(&app, work.id).await;
    assert_eq!(owner.as_deref(), Some("user"));
    let requests = open_requests(&app).await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].2.is_some());
    assert_ne!(requests[0].2, cover);
    let (winner, request) = if a.status == 200 { (&a, &b) } else { (&b, &a) };
    assert_eq!(
        winner.json()["book"]["cover_public_id"],
        json!(
            sqlx::query_scalar::<_, String>("SELECT public_id FROM media WHERE id = $1")
                .bind(cover.unwrap())
                .fetch_one(&app.db)
                .await
                .unwrap()
        )
    );
    assert!(request.json()["request_id"].as_i64().is_some());
}

/// 30 uploads per user and day; the 31st stores nothing.
#[sqlx::test]
async fn uploads_are_limited_to_30_a_day(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let work = app.work("My Zine").await;
    for _ in 0..30 {
        app.state.limits.upload_user.hit(&format!("u:{}", ada.id()));
    }
    let res = upload(&ada, &work.id.to_string(), png(40, 60, [1, 2, 3]), None).await;
    res.assert_detail(429, "Slow down a little. Try again later.");
    assert!(res.header("retry-after").is_some());
    assert_eq!(app.scalar_i64("SELECT count(*) FROM media").await, 0);
    assert!(app.media.keys().is_empty());
    // Per user.
    upload(&grace, &work.id.to_string(), png(40, 60, [1, 2, 3]), None)
        .await
        .assert_status(200);
}

/// The same file as a taken-down picture is refused, and so is either WebP another reader saved
/// from its URLs; the book keeps no cover and no request is filed.
#[sqlx::test]
async fn an_upload_of_a_removed_picture_is_refused(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let root = app.admin("root").await;
    let work = app.work("My Zine").await;
    let data = png(40, 60, [200, 30, 30]);
    let res = upload(&ada, &work.id.to_string(), data.clone(), None).await;
    let public_id = public_id(&res.json()["book"]["cover_url"]);
    // What bob has: the served WebP variants, not ada's PNG.
    let mut saved = Vec::new();
    for variant in ["l", "s"] {
        let res = app
            .anon()
            .get(&format!("/media/{public_id}/{variant}"))
            .await;
        res.assert_status(200);
        assert_ne!(res.body.to_vec(), data);
        saved.push(res.body.to_vec());
    }
    root.delete(&format!("/api/admin/media/{public_id}"))
        .await
        .assert_status(204);
    assert_eq!(work_cover(&app, work.id).await, (None, None));
    upload(&ada, &work.id.to_string(), data, None)
        .await
        .assert_detail(400, "This picture was removed");
    for bytes in saved {
        upload(&bob, &work.id.to_string(), bytes, None)
            .await
            .assert_detail(400, "This picture was removed");
    }
    assert_eq!(work_cover(&app, work.id).await, (None, None));
    assert!(open_requests(&app).await.is_empty());
    assert_eq!(app.scalar_i64("SELECT count(*) FROM media").await, 1);
    assert!(app.media.keys().is_empty());
}

/// Unknown books are 404, signed-out callers 401; `OL…W` keys work like ids.
#[sqlx::test]
async fn the_book_must_exist(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for path in ["999999", "OL999W", "not-a-key"] {
        upload(&ada, path, png(40, 60, [1, 2, 3]), None)
            .await
            .assert_detail(404, "No such book.");
    }
    let work = app.ol_work("OL7W", "Circe", "Madeline Miller").await;
    upload(&app.anon(), "OL7W", png(40, 60, [1, 2, 3]), None)
        .await
        .assert_detail(401, "Not signed in");
    let res = upload(&ada, "OL7W", png(40, 60, [1, 2, 3]), None).await;
    res.assert_status(200);
    assert_eq!(res.json()["book"]["id"], work.id);
    // A picture that is no picture is refused before anything is stored.
    upload(
        &ada,
        "OL7W",
        b"<svg/> padding padding padding".to_vec(),
        None,
    )
    .await
    .assert_detail(400, "Use a JPEG, PNG, WebP, or GIF");
    assert!(app.upstream_requests().await.is_empty());
}

/// An upload for a work merged away lands on the surviving work.
#[sqlx::test]
async fn an_upload_to_a_merged_work_lands_on_the_survivor(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let loser = app.work("Zine").await;
    let winner = app.work("My Zine").await;
    {
        let mut tx = app.db.begin().await.unwrap();
        merge_works(&mut tx, loser.id, winner.id).await.unwrap();
        tx.commit().await.unwrap();
    }
    let res = upload(&ada, &loser.id.to_string(), png(40, 60, [1, 2, 3]), None).await;
    res.assert_status(200);
    assert_eq!(res.json()["book"]["id"], winner.id);
    assert!(work_cover(&app, winner.id).await.0.is_some());
    assert_eq!(work_cover(&app, loser.id).await.0, None);
}
