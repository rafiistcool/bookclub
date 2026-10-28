//! `POST /api/shelf/spine-colors` (#43, docs/phase1/spec/shelf.md §1.6 and §2.8): the spine
//! colour backfill of the caller's shelf, and the SSRF guard of the cover fetcher.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use bookclub_server::catalog::shared::ingest::{
    self, CoverTarget, Mode, Source, WorkRecord, ol_cover_media, set_cover,
};
use bookclub_server::media::service::{CoverFetch, fetch_ol_cover};
use bookclub_server::outbound::{HostThrottle, Outbound, RetryPolicy};
use common::*;
use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage};
use serde_json::{Value, json};
use sqlx::PgPool;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The legacy test helper: a 120×180 JPEG of one colour.
fn jpeg(color: [u8; 3]) -> Vec<u8> {
    let image = RgbImage::from_pixel(120, 180, Rgb(color));
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 92)
        .write_image(image.as_raw(), 120, 180, ExtendedColorType::Rgb8)
        .unwrap();
    out
}

fn rgb(value: &str) -> [i32; 3] {
    [1, 3, 5].map(|i| i32::from_str_radix(&value[i..i + 2], 16).unwrap())
}

/// An Open Library work with Open Library cover `cover_id` (none for 0) on `who`'s shelf.
async fn shelved(app: &TestApp, who: &TestClient, n: i64, cover_id: i64) -> i64 {
    let mut conn = app.db.acquire().await.unwrap();
    let record = WorkRecord {
        ol_work_id: Some(format!("OL{n}W")),
        ol_cover_id: (cover_id > 0).then_some(cover_id),
        full_details: true,
        ..WorkRecord::new(Source::OpenLibrary, format!("Book {n}"))
    };
    let work_id = ingest::upsert_work(&mut conn, &record, Mode::Refresh)
        .await
        .unwrap();
    who.post("/api/shelf", json!({"work_id": work_id}))
        .await
        .assert_status(201);
    work_id
}

async fn serve_cover(app: &TestApp, cover_id: i64, template: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(format!("/b/id/{cover_id}-L.jpg")))
        .respond_with(template)
        .mount(&app.upstream)
        .await;
}

fn image(color: [u8; 3]) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_bytes(jpeg(color))
}

async fn spine_colors(client: &TestClient) -> Value {
    let res = client.post("/api/shelf/spine-colors", json!({})).await;
    res.assert_status(200);
    res.json()
}

/// `(work id, spine_color)` of every shelf book, by work id.
async fn shelf_spines(client: &TestClient) -> Vec<(i64, Value)> {
    let shelf = client.get("/api/shelf").await.json();
    let mut spines: Vec<(i64, Value)> = shelf["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["book"]["id"].as_i64().unwrap(),
                item["book"]["spine_color"].clone(),
            )
        })
        .collect();
    spines.sort_by_key(|(id, _)| *id);
    spines
}

async fn color_of_cover(app: &TestApp, work_id: i64) -> Option<String> {
    sqlx::query_scalar(
        "SELECT m.dominant_color FROM works w JOIN media m ON m.id = w.cover_media_id
         WHERE w.id = $1",
    )
    .bind(work_id)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// parity: test_spine_color.py::test_spine_colors_are_sampled_once_and_served_with_the_book
#[sqlx::test]
async fn spine_colors_are_sampled_once_and_served_with_the_book(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let covered = shelved(&app, &ada, 1, 111).await;
    let bare = shelved(&app, &ada, 2, 0).await;
    serve_cover(&app, 111, image([150, 40, 40])).await;
    assert_eq!(
        shelf_spines(&ada).await,
        [(covered, Value::Null), (bare, Value::Null)]
    );

    let result = spine_colors(&ada).await;
    assert_eq!(result["remaining"], 0);
    let colors = result["colors"].as_object().unwrap();
    assert_eq!(colors.keys().collect::<Vec<_>>(), [&covered.to_string()]);
    let requests = app.upstream_requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/b/id/111-L.jpg");
    assert_eq!(requests[0].url.query(), Some("default=false"));

    let spines = shelf_spines(&ada).await;
    assert_eq!(spines[0], (covered, colors[&covered.to_string()].clone()));
    // A book without a cover has no colour, and nothing is fetched twice.
    assert_eq!(spines[1], (bare, Value::Null));
    assert_eq!(
        spine_colors(&ada).await,
        json!({"colors": {}, "remaining": 0})
    );
    assert_eq!(app.upstream_requests().await.len(), 1);
}

/// parity: test_spine_color.py::test_spine_colors_retry_after_network_trouble_but_not_after_a_missing_cover
#[sqlx::test]
async fn spine_colors_retry_after_network_trouble_but_not_after_a_missing_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = shelved(&app, &ada, 1, 111).await;
    // Down: the attempt and its one retry fail. Then Open Library says there is no cover.
    Mock::given(method("GET"))
        .and(path("/b/id/111-L.jpg"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .mount(&app.upstream)
        .await;
    serve_cover(&app, 111, ResponseTemplate::new(404)).await;

    assert_eq!(
        spine_colors(&ada).await,
        json!({"colors": {}, "remaining": 0})
    );
    assert_eq!(color_of_cover(&app, work).await, None);

    assert_eq!(
        spine_colors(&ada).await,
        json!({"colors": {}, "remaining": 0})
    );
    assert_eq!(color_of_cover(&app, work).await.as_deref(), Some(""));
    assert_eq!(app.upstream_requests().await.len(), 3);
}

/// parity: test_spine_color.py::test_a_new_cover_is_sampled_again (the colour belongs to the
/// picture now, so a new cover is a new, unsampled picture)
#[sqlx::test]
async fn a_new_cover_is_sampled_again(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = shelved(&app, &ada, 1, 111).await;
    serve_cover(&app, 111, image([150, 40, 40])).await;
    serve_cover(&app, 222, image([40, 120, 60])).await;
    let first = spine_colors(&ada).await["colors"].clone();
    assert_eq!(first.as_object().unwrap().len(), 1);

    let mut conn = app.db.acquire().await.unwrap();
    let media = ol_cover_media(&mut conn, 222).await.unwrap().unwrap();
    assert!(
        set_cover(
            &mut conn,
            CoverTarget::Work(work),
            media,
            Source::OpenLibrary,
            false
        )
        .await
        .unwrap()
    );
    let second = spine_colors(&ada).await["colors"].clone();
    assert_eq!(second.as_object().unwrap().len(), 1);
    assert_ne!(second, first);
}

/// parity: test_spine_color.py::test_spine_colors_only_cover_the_callers_shelf_and_need_a_login
#[sqlx::test]
async fn spine_colors_only_cover_the_callers_shelf_and_need_a_login(db: PgPool) {
    let app = TestApp::new(db).await;
    app.anon()
        .post("/api/shelf/spine-colors", json!({}))
        .await
        .assert_detail(401, "Not signed in");
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    shelved(&app, &ada, 1, 111).await;
    serve_cover(&app, 111, image([150, 40, 40])).await;
    assert_eq!(
        spine_colors(&grace).await,
        json!({"colors": {}, "remaining": 0})
    );
    assert!(app.upstream_requests().await.is_empty());
}

/// parity: test_spine_color.py::test_one_broken_cover_does_not_block_the_others
#[sqlx::test]
async fn one_broken_cover_does_not_block_the_others(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let broken = shelved(&app, &ada, 1, 111).await;
    let fine = shelved(&app, &ada, 2, 222).await;
    serve_cover(
        &app,
        111,
        ResponseTemplate::new(200).set_body_bytes(b"\xff\xd8\xff not really a JPEG".to_vec()),
    )
    .await;
    serve_cover(&app, 222, image([40, 120, 60])).await;
    let result = spine_colors(&ada).await;
    let colors = result["colors"].as_object().unwrap();
    assert_eq!(colors.keys().collect::<Vec<_>>(), [&fine.to_string()]);
    // The broken one is settled, so it is not retried on every visit.
    assert_eq!(
        spine_colors(&ada).await,
        json!({"colors": {}, "remaining": 0})
    );
    assert_eq!(color_of_cover(&app, broken).await.as_deref(), Some(""));
    assert_eq!(app.upstream_requests().await.len(), 2);
}

/// A batch is 12 books, by work id; `remaining` counts the rest.
#[sqlx::test]
async fn spine_colors_come_in_batches_of_twelve(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut works = Vec::new();
    for n in 1..=13 {
        works.push(shelved(&app, &ada, n, 100 + n).await);
        serve_cover(&app, 100 + n, image([150, 40, 40])).await;
    }
    let first = spine_colors(&ada).await;
    assert_eq!(first["remaining"], 1);
    assert_eq!(first["colors"].as_object().unwrap().len(), 12);
    let second = spine_colors(&ada).await;
    assert_eq!(second["remaining"], 0);
    let keys: Vec<String> = second["colors"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let last = works.iter().max().unwrap().to_string();
    assert_eq!(keys, [last]);
}

/// A shelf entry with an edition shows the edition's cover, so that is the one sampled.
#[sqlx::test]
async fn the_entrys_edition_cover_is_the_one_sampled(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = shelved(&app, &ada, 1, 111).await;
    let edition = app
        .edition(
            &TestWork {
                id: work,
                title: "Book 1".into(),
            },
            "9780316556347",
        )
        .await;
    let mut conn = app.db.acquire().await.unwrap();
    let media = ol_cover_media(&mut conn, 333).await.unwrap().unwrap();
    set_cover(
        &mut conn,
        CoverTarget::Edition(edition.id),
        media,
        Source::OpenLibrary,
        false,
    )
    .await
    .unwrap();
    let entry = ada.get("/api/shelf").await.json()["items"][0]["id"].clone();
    ada.patch(
        &format!("/api/shelf/{entry}"),
        json!({"edition_id": edition.id}),
    )
    .await
    .assert_status(200);
    serve_cover(&app, 333, image([40, 120, 60])).await;
    let result = spine_colors(&ada).await;
    let color = result["colors"][work.to_string()]
        .as_str()
        .unwrap()
        .to_owned();
    let [red, green, _] = rgb(&color);
    assert!(green > red, "{color}");
    let paths: Vec<String> = app
        .upstream_requests()
        .await
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(paths, ["/b/id/333-L.jpg"]);
    assert_eq!(shelf_spines(&ada).await, [(work, json!(color))]);
}

/// A taken-down cover is never pending and never fetched.
#[sqlx::test]
async fn removed_covers_are_never_fetched(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let root = app.admin("root").await;
    let work = shelved(&app, &ada, 1, 111).await;
    serve_cover(&app, 111, image([150, 40, 40])).await;
    let public_id: String = sqlx::query_scalar(
        "SELECT m.public_id FROM works w JOIN media m ON m.id = w.cover_media_id WHERE w.id = $1",
    )
    .bind(work)
    .fetch_one(&app.db)
    .await
    .unwrap();
    root.delete(&format!("/api/admin/media/{public_id}"))
        .await
        .assert_status(204);
    assert_eq!(
        spine_colors(&ada).await,
        json!({"colors": {}, "remaining": 0})
    );
    assert!(app.upstream_requests().await.is_empty());
}

/// A stored picture without a colour (an imported one) is sampled from its bytes, without a
/// request.
#[sqlx::test]
async fn stored_pictures_are_sampled_from_their_bytes(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = shelved(&app, &ada, 1, 111).await;
    serve_cover(&app, 111, image([40, 40, 160])).await;
    // Store the bytes through /media, then forget the colour.
    let public_id: String = sqlx::query_scalar(
        "SELECT m.public_id FROM works w JOIN media m ON m.id = w.cover_media_id WHERE w.id = $1",
    )
    .bind(work)
    .fetch_one(&app.db)
    .await
    .unwrap();
    app.anon()
        .get(&format!("/media/{public_id}/s"))
        .await
        .assert_status(200);
    app.exec("UPDATE media SET dominant_color = NULL").await;
    let result = spine_colors(&ada).await;
    let [red, _, blue] = rgb(result["colors"][work.to_string()].as_str().unwrap());
    assert!(blue > red + 50);
    assert_eq!(app.upstream_requests().await.len(), 1);
}

/// parity: test_spine_color.py::test_only_cover_hosts_are_ever_requested (applied to the cover
/// fetcher; Google hosts removed). Every hop of a redirect is checked before it is requested:
/// the configured cover host and archive.org (and its subdomains) pass; plain http, a port,
/// userinfo, an IP address and look-alike domains settle as "no cover" without a request.
#[tokio::test]
async fn only_cover_hosts_are_ever_requested() {
    // The first request goes to the mock server (the configured cover host), which answers
    // with the redirect under test.
    let server = MockServer::start().await;
    let config = test_config(&[("OPENLIBRARY_COVERS_URL", &server.uri())]);
    let mut outbound = Outbound::new(&config).unwrap();
    outbound.ol_covers = HostThrottle::per_second(1000.0);
    // Every host a redirect names resolves to this machine (reqwest then uses the scheme's
    // port): an allowed hop is attempted and fails here as a transport error, a refused one is
    // never attempted. No test request leaves the machine.
    let local = SocketAddr::from(([127, 0, 0, 1], 443));
    let mut client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2));
    for host in [
        "covers.openlibrary.org",
        "ia800100.us.archive.org",
        "archive.org",
        "example.com",
        "archive.org.example.com",
    ] {
        client = client.resolve(host, local);
    }
    outbound.client = client.build().unwrap();
    let policy = RetryPolicy {
        max_wait: Duration::from_secs(2),
        retry_5xx_once_after: None,
        attempt_timeout: Duration::from_secs(5),
        ..RetryPolicy::MEDIA
    };

    // covers.openlibrary.org passes as a subdomain of the configured openlibrary.org (in
    // production it is the configured cover host itself).
    let allowed = [
        "https://covers.openlibrary.org/b/id/1-M.jpg",
        "https://ia800100.us.archive.org/1/items/x/y.jpg",
        "https://archive.org/download/x/y.jpg",
    ];
    let refused = [
        "http://covers.openlibrary.org/b/id/1-M.jpg",
        "https://covers.openlibrary.org:8443/b/id/1-M.jpg",
        "https://user@covers.openlibrary.org/b/id/1-M.jpg",
        "https://127.0.0.1/b/id/1-M.jpg",
        "https://example.com/cover.jpg",
        "https://archive.org.example.com/cover.jpg",
    ];
    for (n, target) in allowed.iter().chain(refused.iter()).enumerate() {
        Mock::given(method("GET"))
            .and(path(format!("/b/id/{n}-L.jpg")))
            .respond_with(ResponseTemplate::new(302).insert_header("location", *target))
            .mount(&server)
            .await;
    }
    for (n, target) in allowed.iter().enumerate() {
        let fetched = fetch_ol_cover(&outbound, n as i64, &policy).await;
        // Followed: the hop was requested (it cannot connect here: "try again later").
        assert!(
            matches!(fetched, CoverFetch::Failed),
            "{target}: {fetched:?}"
        );
    }
    for (n, target) in refused.iter().enumerate() {
        let n = (allowed.len() + n) as i64;
        let fetched = fetch_ol_cover(&outbound, n, &policy).await;
        assert!(
            matches!(fetched, CoverFetch::NoCover),
            "{target}: {fetched:?}"
        );
    }

    // Redirects inside the configured origin are followed, at most three of them:
    // /hop/1 → /hop/2 → /hop/3 → the image.
    let image_path = "/covers/real.jpg";
    Mock::given(method("GET"))
        .and(path(image_path))
        .respond_with(image([10, 20, 30]))
        .mount(&server)
        .await;
    for hop in 1..=3 {
        let to = if hop == 3 {
            image_path.to_owned()
        } else {
            format!("/hop/{}", hop + 1)
        };
        Mock::given(method("GET"))
            .and(path(format!("/hop/{hop}")))
            .respond_with(ResponseTemplate::new(302).insert_header("location", to.as_str()))
            .mount(&server)
            .await;
    }
    // Three redirects: the cover. Four: refused, no cover.
    for (cover, location, image_expected) in [(100, "/hop/2", true), (101, "/hop/1", false)] {
        Mock::given(method("GET"))
            .and(path(format!("/b/id/{cover}-L.jpg")))
            .respond_with(ResponseTemplate::new(302).insert_header("location", location))
            .mount(&server)
            .await;
        let fetched = fetch_ol_cover(&outbound, cover, &policy).await;
        if image_expected {
            assert!(matches!(fetched, CoverFetch::Image(_)), "{fetched:?}");
        } else {
            assert!(matches!(fetched, CoverFetch::NoCover), "{fetched:?}");
        }
    }
}

/// A cover over 1.5 MB is settled as "no cover" while it streams.
#[sqlx::test]
async fn an_oversized_cover_is_no_cover(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let work = shelved(&app, &ada, 1, 111).await;
    serve_cover(
        &app,
        111,
        ResponseTemplate::new(200).set_body_bytes(vec![0xff; 1_500_001]),
    )
    .await;
    assert_eq!(
        spine_colors(&ada).await,
        json!({"colors": {}, "remaining": 0})
    );
    assert_eq!(color_of_cover(&app, work).await.as_deref(), Some(""));
}
