//! Router-wide behaviour: JSON fallbacks, the SPA, the origin check, CORS and response headers
//! (docs/phase1/conventions.md §3, §5.8).

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use bookclub_server::error::is_unique_violation;
use bookclub_server::{AppState, app};
use common::{get, send, test_config, test_state, test_state_with, unreachable_db};
use sqlx::PgPool;

fn no_db_state() -> AppState {
    test_state(unreachable_db())
}

fn request(method: Method, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn unknown_api_paths_are_json_404s() {
    for path in [
        "/api/nope",
        "/api/",
        "/api",
        "/api/clubs/x/nothing",
        "/media/abc/l",
    ] {
        get(app(no_db_state()), path)
            .await
            .assert_detail(404, "Not found");
    }
    let res = send(app(no_db_state()), request(Method::POST, "/api/nope")).await;
    res.assert_detail(404, "Not found");
}

#[tokio::test]
async fn a_wrong_method_is_a_json_405() {
    for method in [Method::POST, Method::DELETE, Method::PUT] {
        let res = send(app(no_db_state()), request(method, "/api/health")).await;
        res.assert_detail(405, "Method Not Allowed");
    }
}

#[tokio::test]
async fn without_static_dir_other_paths_are_json_404s() {
    for path in ["/", "/discover", "/assets/app.js"] {
        get(app(no_db_state()), path)
            .await
            .assert_detail(404, "Not found");
    }
}

#[tokio::test]
async fn every_response_has_referrer_policy_and_a_request_id() {
    let res = get(app(no_db_state()), "/api/nope").await;
    assert_eq!(res.header("referrer-policy"), Some("same-origin"));
    let id = res.header("x-request-id").expect("request id");
    assert!(!id.is_empty());

    let given = Request::get("/api/health")
        .header("x-request-id", "abc-123")
        .body(Body::empty())
        .unwrap();
    let res = send(app(no_db_state()), given).await;
    assert_eq!(res.header("x-request-id"), Some("abc-123"));
    assert_eq!(res.header("referrer-policy"), Some("same-origin"));

    // Answers of the outer layers: the origin check and a CORS preflight.
    let cross_site = Request::post("/api/health")
        .header("cookie", "bookclub_session=abc")
        .header("origin", "https://evil.example")
        .body(Body::empty())
        .unwrap();
    let res = send(app(no_db_state()), cross_site).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.header("referrer-policy"), Some("same-origin"));
    assert!(res.header("x-request-id").is_some());

    let res = send(app(no_db_state()), preflight("http://localhost:5173")).await;
    assert_eq!(
        res.header("access-control-allow-origin"),
        Some("http://localhost:5173")
    );
    assert_eq!(res.header("referrer-policy"), Some("same-origin"));
}

#[tokio::test]
async fn the_openapi_document_is_served() {
    let res = get(app(no_db_state()), "/api/openapi.json").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("content-type"), Some("application/json"));
    let doc = res.json();
    assert_eq!(
        doc["paths"]["/api/health"]["get"]["operationId"],
        "platform_health"
    );
}

/// A built SPA in a temporary directory.
fn spa_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        r#"<!doctype html><script type="module" src="/assets/current.js"></script>"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/current.js"), "console.log(1)").unwrap();
    std::fs::write(dir.path().join("favicon.svg"), "<svg/>").unwrap();
    std::fs::create_dir_all(dir.path().join("nested")).unwrap();
    std::fs::write(
        dir.path().join("nested/index.html"),
        "<p>nested current.js</p>",
    )
    .unwrap();
    dir
}

fn spa_state(dir: &tempfile::TempDir) -> AppState {
    let static_dir = dir.path().to_str().unwrap();
    test_state_with(unreachable_db(), test_config(&[("STATIC_DIR", static_dir)]))
}

/// parity: test_spa_cache.py::test_app_html_is_not_cached_across_releases
#[tokio::test]
async fn the_spa_shell_is_never_cached() {
    let dir = spa_dir();
    for path in [
        "/",
        "/index.html",
        "/discover",
        "/book/ISBN9781408857892",
        "/nested",
        "/..%2f..%2fetc%2fpasswd",
    ] {
        let res = get(app(spa_state(&dir)), path).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
        assert!(res.text().contains("current.js"), "{path}");
        assert_eq!(res.header("cache-control"), Some("no-store"), "{path}");
        assert!(
            res.header("content-type").unwrap().starts_with("text/html"),
            "{path}"
        );
    }
    let res = get(app(spa_state(&dir)), "/nested/index.html").await;
    assert_eq!(res.text(), "<p>nested current.js</p>");
    assert_eq!(res.header("cache-control"), Some("no-store"));
}

#[tokio::test]
async fn spa_assets_are_immutable_and_misses_are_json() {
    let dir = spa_dir();
    let res = get(app(spa_state(&dir)), "/assets/current.js").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.text(), "console.log(1)");
    assert_eq!(
        res.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    get(app(spa_state(&dir)), "/assets/missing.js")
        .await
        .assert_detail(404, "Not found");

    let res = get(app(spa_state(&dir)), "/favicon.svg").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.text(), "<svg/>");
    assert_eq!(res.header("cache-control"), None);
}

#[tokio::test]
async fn the_spa_never_hides_the_api() {
    let dir = spa_dir();
    for path in ["/api/nope", "/api", "/api/"] {
        get(app(spa_state(&dir)), path)
            .await
            .assert_detail(404, "Not found");
    }
    // An unknown API path is a 404 for every method, never the 405 other paths get.
    let res = send(app(spa_state(&dir)), request(Method::POST, "/api/nope")).await;
    res.assert_detail(404, "Not found");
    // Only `/api` and `/api/…` are the API: `/apix` is an SPA route.
    let res = get(app(spa_state(&dir)), "/apix").await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.text().contains("current.js"));
    assert_eq!(res.header("cache-control"), Some("no-store"));
    get(app(spa_state(&dir)), "/media/abc/l")
        .await
        .assert_detail(404, "Not found");
    let res = send(app(spa_state(&dir)), request(Method::POST, "/discover")).await;
    res.assert_detail(405, "Method Not Allowed");
    let res = send(app(spa_state(&dir)), request(Method::HEAD, "/")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("cache-control"), Some("no-store"));
}

#[tokio::test]
async fn cross_site_cookie_requests_are_refused() {
    let evil = Request::post("/api/health")
        .header("cookie", "bookclub_session=abc")
        .header("origin", "https://evil.example")
        .body(Body::empty())
        .unwrap();
    send(app(no_db_state()), evil)
        .await
        .assert_detail(403, "Cross-site request refused");

    // The configured origin passes the check (and then meets the 405 of the route).
    let own = Request::post("/api/health")
        .header("cookie", "bookclub_session=abc")
        .header("origin", "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    send(app(no_db_state()), own)
        .await
        .assert_detail(405, "Method Not Allowed");

    let bearer = Request::post("/api/health")
        .header("cookie", "bookclub_session=abc")
        .header("authorization", "Bearer bc_x")
        .header("origin", "https://evil.example")
        .body(Body::empty())
        .unwrap();
    send(app(no_db_state()), bearer)
        .await
        .assert_detail(405, "Method Not Allowed");
}

fn preflight(origin: &str) -> Request<Body> {
    Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/auth/login")
        .header("origin", origin)
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .body(Body::empty())
        .unwrap()
}

/// parity: test_selfhost.py::test_debug_cors_keeps_vite
#[tokio::test]
async fn dev_mode_cors_allows_the_vite_origin() {
    let res = send(app(no_db_state()), preflight("http://localhost:5173")).await;
    assert_eq!(
        res.header("access-control-allow-origin"),
        Some("http://localhost:5173")
    );
    assert_eq!(res.header("access-control-allow-credentials"), Some("true"));
    let res = send(app(no_db_state()), preflight("https://evil.example")).await;
    assert_eq!(res.header("access-control-allow-origin"), None);
}

/// parity: test_selfhost.py::test_production_app_registers_and_hides_docs (the CORS part)
#[tokio::test]
async fn production_cors_only_allows_configured_origins() {
    let production = test_config(&[
        ("DEV_MODE", "false"),
        ("PUBLIC_URL", "https://books.example.com"),
        ("TRUSTED_PROXIES", "none"),
        ("CONTACT_EMAIL", "ops@example.com"),
        ("SMTP_URL", "smtps://user:pass@smtp.example.eu:465"),
        ("EMAIL_FROM", "Bookclub <noreply@example.com>"),
    ]);
    let state = test_state_with(unreachable_db(), production.clone());
    let res = send(app(state), preflight("http://localhost:5173")).await;
    assert_eq!(res.header("access-control-allow-origin"), None);

    let mut with_apps = production;
    with_apps.cors_origins = vec!["capacitor://localhost".into()];
    let state = test_state_with(unreachable_db(), with_apps);
    let res = send(app(state), preflight("capacitor://localhost")).await;
    assert_eq!(
        res.header("access-control-allow-origin"),
        Some("capacitor://localhost")
    );
}

#[sqlx::test]
async fn unique_violations_are_recognised(db: PgPool) {
    let insert = "INSERT INTO users (username, password_hash) VALUES ('ada', 'x')";
    sqlx::query(insert).execute(&db).await.unwrap();
    let error = sqlx::query(insert).execute(&db).await.unwrap_err();
    assert!(is_unique_violation(&error, "users_username_key"));
    assert!(!is_unique_violation(&error, "users_verified_email_key"));
}
