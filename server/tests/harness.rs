//! The test harness itself (docs/phase1/conventions.md §17.2): fixtures, `TestClient`,
//! `FakeCatalog` and the wiremock upstream. Wave 2 packages build on these, and the harness is
//! frozen after Wave 1.

mod common;

use std::net::IpAddr;
use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use bookclub_server::catalog::shared::ingest::Source;
use bookclub_server::catalog::shared::isbn::parse_isbn;
use bookclub_server::catalog::shared::lookup::{CatalogLookup, LookupError, Resolved};
use bookclub_server::client_ip::ClientIp;
use bookclub_server::extract::ApiMultipart;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

#[sqlx::test]
async fn fixture_users(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user_unverified("grace").await;
    let root = app.admin("root").await;
    let me = ada.get("/api/auth/me").await.json();
    assert_eq!(me["id"], ada.id());
    assert_eq!(me["email"], "ada@example.test");
    assert_eq!(me["email_verified"], true);
    assert_eq!(me["is_admin"], false);
    assert_eq!(ada.username(), "ada");
    assert_eq!(
        grace.get("/api/auth/me").await.json()["email_verified"],
        false
    );
    assert_eq!(root.get("/api/auth/me").await.json()["is_admin"], true);
    // Every fixture user can log in with the shared password.
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": "grace", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
    app.anon()
        .get("/api/auth/me")
        .await
        .assert_detail(401, "Not signed in");
}

#[sqlx::test]
async fn fixture_clubs(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Thursday Readers!").await;
    assert!(club.slug.starts_with("thursday-readers-"), "{}", club.slug);
    assert_eq!(club.slug.len(), "thursday-readers-".len() + 4);
    assert_eq!(club.name, "Thursday Readers!");
    let odd = app.club(&ada, "!!!").await;
    assert!(odd.slug.starts_with("club-"), "{}", odd.slug);
    app.join(&club, &grace, "admin").await;
    let clubs = grace.get("/api/auth/me").await.json()["clubs"].clone();
    assert_eq!(
        clubs,
        json!([{"slug": club.slug, "name": club.name, "role": "admin", "timezone": "UTC", "member_count": 2}])
    );
}

#[sqlx::test]
async fn fixture_works_and_editions(db: PgPool) {
    let app = TestApp::new(db).await;
    let circe = app.work("Circe").await;
    let ol = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
    let (dnb, first) = app
        .dnb_work("Die Wand", "Marlen Haushofer", "978-3-548-60649-1")
        .await;
    let (again, second) = app
        .dnb_work("Die Wand", "Marlen Haushofer", "9783471350263")
        .await;
    assert_eq!(dnb.id, again.id, "DNB editions group by title and author");
    assert_ne!(first.id, second.id);
    assert_eq!(first.isbn13, "9783548606491");
    let edition = app.edition(&circe, "9780316556347").await;
    assert_eq!(edition.work_id, circe.id);
    let rows: Vec<(i64, String, Option<String>, String)> =
        sqlx::query_as("SELECT id, source, ol_work_id, author_names FROM works ORDER BY id")
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![
            (circe.id, "user".into(), None, String::new()),
            (
                ol.id,
                "openlibrary".into(),
                Some("OL1W".into()),
                "Madeline Miller".into()
            ),
            (dnb.id, "dnb".into(), None, "Marlen Haushofer".into()),
        ]
    );
    assert_eq!(app.scalar_i64("SELECT count(*) FROM editions").await, 3);
}

#[sqlx::test]
async fn fake_catalog_creates_programmed_hits_once(db: PgPool) {
    let app = TestApp::new(db).await;
    let isbn = parse_isbn("9780316769488").unwrap();
    app.catalog.on_isbn(
        "9780316769488",
        FakeHit::New {
            title: "The Catcher in the Rye".into(),
            authors: "J. D. Salinger".into(),
            ol_work_id: Some("OL3335245W".into()),
            source: Source::OpenLibrary,
        },
    );
    let lookup = app.state.lookup.clone();
    let mut conn = app.db.acquire().await.unwrap();
    let first = lookup
        .resolve_isbn(&mut conn, &isbn)
        .await
        .unwrap()
        .unwrap();
    assert!(first.edition_id.is_some());
    let second = lookup.resolve_isbn(&mut conn, &isbn).await.unwrap();
    assert_eq!(second, Some(first), "the local step finds it");
    let remote = lookup.resolve_isbn_remote(&mut conn, &isbn).await.unwrap();
    assert_eq!(remote, Some(first), "created once");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM works").await, 1);
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM editions WHERE isbn13 = '9780316769488'")
            .await,
        1
    );
    let unknown = parse_isbn("9780140449136").unwrap();
    assert_eq!(
        lookup.resolve_isbn(&mut conn, &unknown).await.unwrap(),
        None
    );
    assert_eq!(
        app.catalog.calls(),
        vec![
            "isbn:9780316769488",
            "isbn:9780316769488",
            "isbn_remote:9780316769488",
            "isbn:9780140449136",
        ]
    );
}

#[sqlx::test]
async fn fake_catalog_existing_hits_texts_and_failures(db: PgPool) {
    let app = TestApp::new(db).await;
    let circe = app.work("Circe").await;
    let isbn = parse_isbn("9780316556347").unwrap();
    app.catalog.on_isbn(
        "9780316556347",
        FakeHit::Existing {
            work_id: circe.id,
            edition_id: None,
        },
    );
    app.catalog.on_text(
        "Die Wand",
        "Marlen Haushofer",
        FakeHit::New {
            title: "Die Wand".into(),
            authors: "Marlen Haushofer".into(),
            ol_work_id: None,
            source: Source::Dnb,
        },
    );
    let lookup: Arc<dyn CatalogLookup> = app.state.lookup.clone();
    let mut conn = app.db.acquire().await.unwrap();

    app.catalog
        .fail_isbn("9780316556347", FakeError::Throttled, Some(2));
    for _ in 0..2 {
        assert!(matches!(
            lookup.resolve_isbn(&mut conn, &isbn).await,
            Err(LookupError::Throttled)
        ));
    }
    assert_eq!(
        lookup.resolve_isbn(&mut conn, &isbn).await.unwrap(),
        Some(Resolved {
            work_id: circe.id,
            edition_id: None
        })
    );
    let wall = lookup
        .resolve_text(&mut conn, "Die Wand", "Marlen Haushofer")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wall.edition_id, None);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM works WHERE id = {} AND source = 'dnb'",
            wall.work_id
        ))
        .await,
        1
    );
    assert_eq!(
        lookup.resolve_text(&mut conn, "Circe", "").await.unwrap(),
        None
    );

    app.catalog.fail_all(FakeError::Unavailable);
    assert!(matches!(
        lookup
            .resolve_text(&mut conn, "Die Wand", "Marlen Haushofer")
            .await,
        Err(LookupError::Unavailable)
    ));
    assert!(matches!(
        lookup.resolve_isbn_remote(&mut conn, &isbn).await,
        Err(LookupError::Unavailable)
    ));
    assert_eq!(
        app.catalog.calls().last().unwrap(),
        "isbn_remote:9780316556347"
    );
    assert!(
        app.catalog
            .calls()
            .contains(&"text:Die Wand|Marlen Haushofer".to_owned())
    );
}

#[sqlx::test]
async fn upstream_and_lookup_wiring(db: PgPool) {
    let app = TestApp::new(db).await;
    let uri = app.upstream.uri();
    assert_eq!(
        app.config().openlibrary_url.as_str().trim_end_matches('/'),
        uri
    );
    assert_eq!(app.config().dnb_sru_url.as_str(), format!("{uri}/sru/dnb"));
    assert!(app.upstream_requests().await.is_empty());
    let res = app
        .state
        .outbound
        .client
        .get(format!("{uri}/works/OL1W.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 404, "unmatched requests are 404");
    let seen = app.upstream_requests().await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url.path(), "/works/OL1W.json");
    let ua = seen[0].headers.get("user-agent").unwrap().to_str().unwrap();
    assert!(ua.starts_with("Bookclub/1.0"), "{ua}");

    let fake = Arc::as_ptr(&app.state.lookup) as *const ();
    assert_eq!(fake, Arc::as_ptr(&app.catalog) as *const ());
    let real = TestApp::with_real_lookup(app.db.clone()).await;
    assert_eq!(
        Arc::as_ptr(&real.state.lookup) as *const (),
        Arc::as_ptr(&real.state.catalog) as *const ()
    );
}

#[tokio::test]
async fn multipart_bodies_and_client_settings() {
    async fn upload(ApiMultipart(multipart): ApiMultipart) -> axum::Json<Value> {
        let (file, fields) = ApiMultipart(multipart)
            .file(1024, "bad upload")
            .await
            .unwrap();
        axum::Json(json!({
            "filename": file.filename,
            "content_type": file.content_type,
            "bytes": file.bytes.to_vec(),
            "fields": fields.0,
        }))
    }
    async fn whoami(ClientIp(ip): ClientIp, headers: axum::http::HeaderMap) -> String {
        let probe = headers
            .get("x-probe")
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        format!("{ip} {probe}")
    }
    let state = test_state(unreachable_db());
    let router = Router::new()
        .route("/upload", post(upload).put(upload))
        .route("/whoami", get(whoami))
        .with_state(state);
    let client = TestClient::for_router(router);
    for res in [
        client
            .post_multipart(
                "/upload",
                vec![
                    Part::text("edition_id", "42"),
                    Part::file("a.png", "image/png", vec![0, 159, 146, 150]),
                ],
            )
            .await,
        client
            .put_multipart(
                "/upload",
                vec![
                    Part::file("a.png", "image/png", vec![0, 159, 146, 150]),
                    Part::text("edition_id", "42"),
                ],
            )
            .await,
    ] {
        res.assert_status(200);
        assert_eq!(
            res.json(),
            json!({
                "filename": "a.png",
                "content_type": "image/png",
                "bytes": [0, 159, 146, 150],
                "fields": {"edition_id": "42"},
            })
        );
    }
    let ip: IpAddr = "2001:db8::7".parse().unwrap();
    let res = client
        .clone()
        .with_ip(ip)
        .with_header("x-probe", "yes")
        .get("/whoami")
        .await;
    assert_eq!(res.text(), "2001:db8::7 yes");
    assert_eq!(client.get("/whoami").await.text(), "0.0.0.0 ");
}

/// `settle` returns at once while nothing runs in the background (requests leave no reference
/// to the state behind), and waits for a task that holds a clone of the state.
#[sqlx::test]
async fn settle_waits_for_background_tasks(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.get("/api/auth/me").await.assert_status(200);
    app.anon()
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": "x"}),
        )
        .await
        .assert_status(401);
    tokio::time::timeout(std::time::Duration::from_millis(100), app.settle())
        .await
        .expect("an idle app settles at once");

    let state = app.state.clone();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = done.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(state);
    });
    app.settle().await;
    assert!(done.load(std::sync::atomic::Ordering::SeqCst));
}
