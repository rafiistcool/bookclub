//! Profile pictures (#43): `PUT`/`DELETE /api/auth/me/avatar` and their `/media/{public_id}/a`
//! URLs (endpoint-map §2 and §6, docs/phase1/spec/auth.md §1.6–1.7).

mod common;

use axum::http::Method;
use common::*;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage};
use serde_json::{Value, json};
use sqlx::PgPool;

const PICTURE_TOO_LARGE: &str = "That picture is too large (50 MB max)";
const PICTURE_BAD_TYPE: &str = "Use a JPEG, PNG, WebP, or GIF";
const PICTURE_NOT_AN_IMAGE: &str = "That file is not a picture";
const PICTURE_TOO_MANY_PIXELS: &str = "That picture has too many pixels";
const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

fn png(size: u32, color: [u8; 3]) -> Vec<u8> {
    let image = RgbImage::from_pixel(size, size, Rgb(color));
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(image.as_raw(), size, size, ExtendedColorType::Rgb8)
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
    data: Vec<u8>,
    filename: &str,
    content_type: &str,
) -> TestResponse {
    client
        .put_multipart(
            "/api/auth/me/avatar",
            vec![Part::file(filename, content_type, data)],
        )
        .await
}

async fn avatar_url(client: &TestClient) -> Value {
    client.get("/api/auth/me").await.json()["avatar_url"].clone()
}

/// The public id in `/media/{public_id}/a`.
fn public_id(url: &str) -> &str {
    url.strip_prefix("/media/")
        .and_then(|rest| rest.strip_suffix("/a"))
        .unwrap_or_else(|| panic!("not an avatar URL: {url}"))
}

/// parity: test_avatars.py::test_upload_serves_and_deletes_avatar
#[sqlx::test]
async fn upload_serves_and_deletes_avatar(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = upload(&ada, png(48, [180, 40, 40]), "me.png", "image/png").await;
    res.assert_status(200);
    let body = res.json();
    assert_eq!(body["username"], "ada");
    assert!(body.get("avatar").is_none());
    let url = body["avatar_url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/media/") && url.ends_with("/a"), "{url}");

    let image = app.anon().get(&url).await;
    image.assert_status(200);
    assert_eq!(image.header("content-type"), Some("image/webp"));
    assert_eq!(&image.body[..4], b"RIFF");
    let decoded = image::load_from_memory(&image.body).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (256, 256));
    let centre = decoded.to_rgb8().get_pixel(128, 128).0;
    assert!(centre[0] > 150 && centre[1] < 80, "{centre:?}");

    assert_eq!(avatar_url(&ada).await, json!(url));

    let removed = ada.delete("/api/auth/me/avatar").await;
    removed.assert_status(200);
    assert_eq!(removed.json()["avatar_url"], Value::Null);
    let gone = app.anon().get(&url).await;
    gone.assert_detail(404, "Not found");
    assert_eq!(gone.header("cache-control"), Some("no-store"));
    assert_eq!(avatar_url(&ada).await, Value::Null);
    assert!(app.media.keys().is_empty(), "{:?}", app.media.keys());
    assert_eq!(app.scalar_i64("SELECT count(*) FROM media").await, 0);
    // Deleting again is fine.
    ada.delete("/api/auth/me/avatar").await.assert_status(200);
}

/// parity: test_avatars.py::test_other_members_see_avatar_url (the members list moved under the
/// club; the shelf's owner block is unchanged)
#[sqlx::test]
async fn other_members_see_avatar_url(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    upload(&ada, png(48, [180, 40, 40]), "me.png", "image/png")
        .await
        .assert_status(200);
    let ada_url = avatar_url(&ada).await;

    let members = grace
        .get(&format!("/api/clubs/{}/members", club.slug))
        .await
        .json();
    let row = members["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["username"] == "ada")
        .unwrap()
        .clone();
    assert_eq!(row["avatar_url"], ada_url);
    grace
        .get(ada_url.as_str().unwrap())
        .await
        .assert_status(200);

    let shelf = grace.get("/api/shelf?username=ada").await;
    shelf.assert_status(200);
    assert_eq!(shelf.json()["user"]["avatar_url"], ada_url);
}

/// parity: test_avatars.py::test_rejects_non_image_and_oversize
#[sqlx::test]
async fn rejects_non_image_and_oversize(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    upload(
        &ada,
        b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".to_vec(),
        "x.svg",
        "image/svg+xml",
    )
    .await
    .assert_detail(400, PICTURE_BAD_TYPE);
    let mut huge = b"\xff\xd8\xff".to_vec();
    huge.resize(MAX_UPLOAD_BYTES + 3, b'x');
    upload(&ada, huge, "big.jpg", "image/jpeg")
        .await
        .assert_detail(400, PICTURE_TOO_LARGE);
    assert_eq!(avatar_url(&ada).await, Value::Null);
    assert!(app.media.keys().is_empty());
}

/// parity: test_avatars.py::test_rejects_small_file_with_too_many_pixels
#[sqlx::test]
async fn rejects_small_file_with_too_many_pixels(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bomb = fixture("too_many_pixels.png");
    assert!(bomb.len() < MAX_UPLOAD_BYTES);
    let size = image::ImageReader::new(std::io::Cursor::new(&bomb))
        .with_guessed_format()
        .unwrap()
        .into_dimensions()
        .unwrap();
    assert_eq!(size, (6000, 6000));
    upload(&ada, bomb, "bomb.png", "image/png")
        .await
        .assert_detail(400, PICTURE_TOO_MANY_PIXELS);
    assert_eq!(avatar_url(&ada).await, Value::Null);
}

/// parity: test_avatars.py::test_oversize_content_length_is_rejected_before_read (the HTTP half;
/// the unit half is `media::upload::tests`)
#[sqlx::test]
async fn oversize_content_length_is_rejected_before_read(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let res = ada
        .clone()
        .with_header("content-length", &(MAX_UPLOAD_BYTES + 4096 + 1).to_string())
        .request(
            Method::PUT,
            "/api/auth/me/avatar",
            Some("multipart/form-data; boundary=x"),
            b"ignored".to_vec(),
        )
        .await;
    res.assert_detail(413, PICTURE_TOO_LARGE);
    // Exactly the limit plus slack still reads the body (which is no picture form here).
    let res = ada
        .clone()
        .with_header("content-length", &(MAX_UPLOAD_BYTES + 4096).to_string())
        .request(
            Method::PUT,
            "/api/auth/me/avatar",
            Some("multipart/form-data; boundary=x"),
            b"ignored".to_vec(),
        )
        .await;
    res.assert_detail(400, PICTURE_BAD_TYPE);
}

/// parity: test_avatars.py::test_avatar_etag_and_nosniff (reasoned change: the URL is immutable,
/// so instead of an ETag and 304 the picture is cached for a year; a new upload gets a new URL)
#[sqlx::test]
async fn avatar_urls_are_immutable_long_cached_and_nosniff(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = upload(&ada, png(48, [180, 40, 40]), "me.png", "image/png")
        .await
        .json()["avatar_url"]
        .as_str()
        .unwrap()
        .to_owned();
    let image = app.anon().get(&first).await;
    image.assert_status(200);
    assert_eq!(image.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        image.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    let second = upload(&ada, png(48, [40, 40, 180]), "me.png", "image/png")
        .await
        .json()["avatar_url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(first, second);
    app.anon().get(&second).await.assert_status(200);
}

/// parity: test_avatars.py::test_avatar_requires_session (reasoned change: the picture URL is
/// public and unguessable, like every `/media` URL; only the writes need a session)
#[sqlx::test]
async fn avatar_writes_require_a_session_and_the_url_is_public(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    upload(&ada, png(48, [180, 40, 40]), "me.png", "image/png")
        .await
        .assert_status(200);
    let url = avatar_url(&ada).await.as_str().unwrap().to_owned();
    let anon = app.anon();
    upload(&anon, png(48, [1, 2, 3]), "me.png", "image/png")
        .await
        .assert_detail(401, "Not signed in");
    anon.delete("/api/auth/me/avatar")
        .await
        .assert_detail(401, "Not signed in");
    anon.get(&url).await.assert_status(200);
}

/// A replaced picture is gone: its row and its bytes. Rows a crash left behind go too; other
/// users' pictures stay.
#[sqlx::test]
async fn a_new_avatar_deletes_every_previous_one(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    upload(&grace, png(48, [9, 9, 9]), "g.png", "image/png")
        .await
        .assert_status(200);
    let grace_id = public_id(avatar_url(&grace).await.as_str().unwrap()).to_owned();
    upload(&ada, png(48, [180, 40, 40]), "me.png", "image/png")
        .await
        .assert_status(200);
    let first = avatar_url(&ada).await.as_str().unwrap().to_owned();
    // A row a crash left behind: uploaded by ada, not her current avatar.
    app.exec(&format!(
        "INSERT INTO media (public_id, kind, source, storage_key, content_type, variants, uploaded_by)
         VALUES ('leftoverleftoverleftover22', 'avatar', 'upload',
                 'avatars/le/leftoverleftoverleftover22', 'image/webp', '{{a}}', {})",
        ada.id()
    ))
    .await;
    upload(&ada, png(48, [40, 40, 180]), "me.png", "image/png")
        .await
        .assert_status(200);
    let current = avatar_url(&ada).await.as_str().unwrap().to_owned();
    assert_ne!(current, first);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE uploaded_by = {}",
            ada.id()
        ))
        .await,
        1
    );
    app.anon().get(&first).await.assert_status(404);
    let current_id = public_id(&current);
    assert_eq!(app.media.keys(), {
        let mut keys = vec![
            format!("avatars/{}/{current_id}/a", &current_id[..2]),
            format!("avatars/{}/{grace_id}/a", &grace_id[..2]),
        ];
        keys.sort();
        keys
    });
}

/// A photo with GPS EXIF comes back as a WebP with no metadata at all.
#[sqlx::test]
async fn gps_exif_is_stripped(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let photo = fixture("gps.jpg");
    assert!(contains(&photo, b"GPS Phone 9000"));
    assert!(contains(&photo, b"Exif\0\0"));
    upload(&ada, photo, "phone.jpg", "image/jpeg")
        .await
        .assert_status(200);
    let url = avatar_url(&ada).await.as_str().unwrap().to_owned();
    let image = app.anon().get(&url).await;
    image.assert_status(200);
    assert_no_metadata(&image.body);
}

/// The parts of a stored WebP: only the image itself (`VP8 `/`VP8L`, `VP8X` and `ALPH` at
/// most), never `EXIF`, `XMP ` or `ICCP`, and none of the original's metadata bytes.
fn assert_no_metadata(webp: &[u8]) {
    assert_eq!(&webp[..4], b"RIFF");
    assert_eq!(&webp[8..12], b"WEBP");
    let mut offset = 12;
    while offset + 8 <= webp.len() {
        let chunk = &webp[offset..offset + 4];
        assert!(
            matches!(chunk, b"VP8 " | b"VP8L" | b"VP8X" | b"ALPH"),
            "unexpected chunk {:?}",
            String::from_utf8_lossy(chunk)
        );
        let size = u32::from_le_bytes(webp[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8 + size + size % 2;
    }
    for needle in [&b"GPS"[..], b"BookclubTestCam", b"SecretEditor", b"Exif"] {
        assert!(
            !contains(webp, needle),
            "{}",
            String::from_utf8_lossy(needle)
        );
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Transparency becomes white, an animated GIF keeps its first frame, a damaged picture is no
/// picture, and a form without a file is the wrong type.
#[sqlx::test]
async fn pictures_are_flattened_and_checked(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;

    upload(&ada, fixture("transparent.png"), "t.png", "image/png")
        .await
        .assert_status(200);
    let corner = served_pixel(&app, &ada, 4, 4).await;
    assert!(corner.iter().all(|c| *c > 240), "{corner:?}");
    let centre = served_pixel(&app, &ada, 128, 128).await;
    assert!(centre[1] > 120 && centre[0] < 80, "{centre:?}");

    upload(&ada, fixture("animated.gif"), "a.gif", "image/gif")
        .await
        .assert_status(200);
    let first_frame = served_pixel(&app, &ada, 128, 128).await;
    assert!(
        first_frame[0] > 180 && first_frame[2] < 80,
        "{first_frame:?}"
    );

    upload(&ada, fixture("truncated.png"), "t.png", "image/png")
        .await
        .assert_detail(400, PICTURE_NOT_AN_IMAGE);
    ada.put_multipart("/api/auth/me/avatar", vec![Part::text("file", "hello")])
        .await
        .assert_detail(400, PICTURE_BAD_TYPE);
    ada.put("/api/auth/me/avatar", json!({"file": "x"}))
        .await
        .assert_detail(400, PICTURE_BAD_TYPE);
}

async fn served_pixel(app: &TestApp, client: &TestClient, x: u32, y: u32) -> [u8; 3] {
    let url = avatar_url(client).await.as_str().unwrap().to_owned();
    let image = app.anon().get(&url).await;
    image.assert_status(200);
    image::load_from_memory(&image.body)
        .unwrap()
        .to_rgb8()
        .get_pixel(x, y)
        .0
}

/// An avatar an admin took down disappears from the account, its URL answers 410, and neither
/// the uploaded file nor the WebP a reader saved from its URL can be uploaded again; another
/// picture can, and becomes the avatar.
#[sqlx::test]
async fn a_taken_down_avatar_cannot_come_back(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let root = app.admin("root").await;
    let picture = png(48, [180, 40, 40]);
    upload(&ada, picture.clone(), "me.png", "image/png")
        .await
        .assert_status(200);
    let url = avatar_url(&ada).await.as_str().unwrap().to_owned();
    // What every reader has: the served WebP, not ada's PNG.
    let served = app.anon().get(&url).await;
    served.assert_status(200);
    let served = served.body.to_vec();
    assert_ne!(served, picture);
    root.delete(&format!("/api/admin/media/{}", public_id(&url)))
        .await
        .assert_status(204);
    assert_eq!(avatar_url(&ada).await, Value::Null);
    let gone = app.anon().get(&url).await;
    gone.assert_detail(410, "This picture was removed");
    assert_eq!(gone.header("cache-control"), Some("no-store"));
    assert!(app.media.keys().is_empty());
    upload(&ada, picture, "again.png", "image/png")
        .await
        .assert_detail(400, "This picture was removed");
    upload(&bob, served.clone(), "saved.webp", "image/webp")
        .await
        .assert_detail(400, "This picture was removed");
    upload(&ada, served, "saved.webp", "image/webp")
        .await
        .assert_detail(400, "This picture was removed");
    assert_eq!(avatar_url(&bob).await, Value::Null);
    assert!(app.media.keys().is_empty(), "{:?}", app.media.keys());

    upload(&ada, png(48, [1, 2, 3]), "other.png", "image/png")
        .await
        .assert_status(200);
    // The new picture is the avatar; the removed row stays (with its hash).
    let new_url = avatar_url(&ada).await.as_str().unwrap().to_owned();
    assert_ne!(new_url, url);
    app.anon().get(&new_url).await.assert_status(200);
    let new_id = app
        .scalar_i64(&format!(
            "SELECT id FROM media WHERE public_id = '{}'",
            public_id(&new_url)
        ))
        .await;
    assert_eq!(
        app.scalar_i64("SELECT avatar_media_id FROM users WHERE username = 'ada'")
            .await,
        new_id
    );
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM media WHERE removed_at IS NOT NULL")
            .await,
        1
    );
}

/// The hashes of a taken-down avatar outlive its row: account deletion deletes the row, and
/// neither file can come back under a new account.
#[sqlx::test]
async fn a_taken_down_avatar_stays_out_after_the_account_is_deleted(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let picture = png(48, [180, 40, 40]);
    upload(&ada, picture.clone(), "me.png", "image/png")
        .await
        .assert_status(200);
    let url = avatar_url(&ada).await.as_str().unwrap().to_owned();
    let served = app.anon().get(&url).await.body.to_vec();
    root.delete(&format!("/api/admin/media/{}", public_id(&url)))
        .await
        .assert_status(204);
    ada.delete_json("/api/auth/me", json!({"password": PASSWORD}))
        .await
        .assert_status(204);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM media").await, 0);
    assert_eq!(
        app.scalar_i64(
            "SELECT count(*) FROM media_removed_hashes WHERE media_id IS NULL AND variant IS NULL"
        )
        .await,
        1
    );

    let carol = app.user("carol").await;
    upload(&carol, picture, "me.png", "image/png")
        .await
        .assert_detail(400, "This picture was removed");
    upload(&carol, served, "saved.webp", "image/webp")
        .await
        .assert_detail(400, "This picture was removed");
    assert_eq!(avatar_url(&carol).await, Value::Null);
}

/// Security review 2026-10-28 #4: ten avatar uploads per account an hour. Each one is a full
/// decode in the pipeline every picture of the instance shares.
#[sqlx::test]
async fn avatar_uploads_are_limited_per_account(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for n in 0..10u8 {
        upload(&ada, png(32, [n, 0, 0]), "a.png", "image/png")
            .await
            .assert_status(200);
    }
    let refused = upload(&ada, png(32, [9, 9, 9]), "a.png", "image/png").await;
    refused.assert_detail(429, "Slow down a little. Try again later.");
    assert!(refused.header("retry-after").is_some());
    let grace = app.user("grace").await;
    upload(&grace, png(32, [1, 1, 1]), "g.png", "image/png")
        .await
        .assert_status(200);
}

/// One picture upload per account at a time: while one runs, another (avatar or cover) is
/// refused before its body is read, so one account cannot hold many 50 MiB bodies at once.
#[sqlx::test]
async fn a_second_upload_while_one_runs_is_refused(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = app.work("Circe").await;
    let running = app
        .state
        .limits
        .upload_gate
        .try_enter(&format!("u:{}", ada.id()))
        .unwrap();
    upload(&ada, png(32, [1, 2, 3]), "a.png", "image/png")
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
    ada.post_multipart(
        &format!("/api/books/works/{}/cover", circe.id),
        vec![Part::file("c.png", "image/png", png(64, [4, 5, 6]))],
    )
    .await
    .assert_detail(429, "Slow down a little. Try again later.");
    let grace = app.user("grace").await;
    upload(&grace, png(32, [1, 1, 1]), "g.png", "image/png")
        .await
        .assert_status(200);
    drop(running);
    upload(&ada, png(32, [1, 2, 3]), "a.png", "image/png")
        .await
        .assert_status(200);
    assert_eq!(app.state.limits.upload_gate.keys_in_use(), 0);
}
