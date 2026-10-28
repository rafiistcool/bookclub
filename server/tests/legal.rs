//! The legal texts (#53): `GET /api/legal/{doc}`, the `/api/config` fields `legal`,
//! `terms_version` and `contact_email`, and the DEV_MODE placeholders.

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const DOCS: [&str; 3] = ["imprint", "privacy", "terms"];

/// `PRODUCTION_ENV` plus `extra`.
fn production<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut env: Vec<(&str, &str)> = PRODUCTION_ENV.to_vec();
    env.extend_from_slice(extra);
    env
}

#[sqlx::test]
async fn every_document_is_served_anonymously_in_both_languages(db: PgPool) {
    let dir = legal_dir("2026-11-01");
    let env = production(&[("LEGAL_DIR", dir.path().to_str().unwrap())]);
    let app = TestApp::with_env(db, &env).await;
    let anon = app.anon();
    for doc in DOCS {
        for locale in ["de", "en"] {
            let res = anon.get(&format!("/api/legal/{doc}?locale={locale}")).await;
            res.assert_status(200);
            assert_eq!(res.header("cache-control"), Some("no-store"));
            assert_eq!(
                res.json(),
                json!({
                    "doc": doc,
                    "locale": locale,
                    "fallback": false,
                    "updated": "2026-11-01",
                    "html": format!("<p>{doc} {locale}</p>"),
                }),
                "{doc} {locale}"
            );
        }
        // Without a locale: the app's default, English. Case and spaces do not matter.
        let res = anon.get(&format!("/api/legal/{doc}")).await;
        assert_eq!(res.json()["locale"], "en");
        let res = anon.get(&format!("/api/legal/{doc}?locale=%20DE")).await;
        assert_eq!(res.json()["locale"], "de");
    }
    // Signed in, the same.
    let ada = app.user("ada").await;
    ada.get("/api/legal/terms?locale=de")
        .await
        .assert_status(200);
}

#[sqlx::test]
async fn a_missing_language_falls_back_and_says_so(db: PgPool) {
    let dir = legal_dir("1");
    std::fs::remove_file(dir.path().join("imprint.en.html")).unwrap();
    std::fs::remove_file(dir.path().join("privacy.de.html")).unwrap();
    let app = TestApp::with_env(db, &[("LEGAL_DIR", dir.path().to_str().unwrap())]).await;
    let anon = app.anon();

    let imprint = anon.get("/api/legal/imprint?locale=en").await.json();
    assert_eq!(imprint["locale"], "de");
    assert_eq!(imprint["fallback"], true);
    assert_eq!(imprint["html"], "<p>imprint de</p>");
    let privacy = anon.get("/api/legal/privacy?locale=de").await.json();
    assert_eq!(privacy["locale"], "en");
    assert_eq!(privacy["fallback"], true);
    let terms = anon.get("/api/legal/terms?locale=de").await.json();
    assert_eq!(terms["locale"], "de");
    assert_eq!(terms["fallback"], false);

    let config = anon.get("/api/config").await.json();
    assert_eq!(config["legal"], json!(["imprint", "privacy", "terms"]));
}

#[sqlx::test]
async fn unknown_documents_get_404_and_unknown_locales_400(db: PgPool) {
    let dir = legal_dir("1");
    let app = TestApp::with_env(db, &[("LEGAL_DIR", dir.path().to_str().unwrap())]).await;
    let anon = app.anon();
    for path in [
        "/api/legal/impressum",
        "/api/legal/Terms",
        "/api/legal/terms.de.html",
        "/api/legal/manifest.json",
        "/api/legal/..%2Fmanifest.json",
        "/api/legal/%2E%2E",
    ] {
        anon.get(path).await.assert_detail(404, "Not found");
    }
    anon.get("/api/legal/terms?locale=fr")
        .await
        .assert_detail(400, "locale must be one of: en, de");
    anon.post("/api/legal/terms", json!({}))
        .await
        .assert_detail(405, "Method Not Allowed");
}

/// The terms version and the documents come from `LEGAL_DIR`, the contact address from
/// `CONTACT_EMAIL`.
#[sqlx::test]
async fn the_manifest_version_and_the_contact_are_in_the_config(db: PgPool) {
    let dir = legal_dir("2026-11-01");
    let env = production(&[
        ("SIGNUP_MODE", "open"),
        ("LEGAL_DIR", dir.path().to_str().unwrap()),
    ]);
    let app = TestApp::with_env(db, &env).await;
    let config = app.anon().get("/api/config").await.json();
    assert_eq!(config["signup"], "open");
    assert_eq!(config["legal"], json!(["imprint", "privacy", "terms"]));
    assert_eq!(config["terms_version"], "2026-11-01");
    assert_eq!(config["contact_email"], "ops@example.com");
}

/// Production without `LEGAL_DIR` (invite mode) serves no legal page and asks nobody to accept
/// anything.
#[sqlx::test]
async fn production_without_texts_serves_none(db: PgPool) {
    let app = TestApp::with_env(db, &PRODUCTION_ENV).await;
    let anon = app.anon();
    let config = anon.get("/api/config").await.json();
    assert_eq!(config["legal"], json!([]));
    assert_eq!(config["terms_version"], Value::Null);
    for doc in DOCS {
        anon.get(&format!("/api/legal/{doc}"))
            .await
            .assert_detail(404, "Not found");
    }
}

/// DEV_MODE without `LEGAL_DIR` serves the marked placeholders, without a terms version, so a
/// development sign-up needs no acceptance.
#[sqlx::test]
async fn dev_mode_serves_the_placeholders(db: PgPool) {
    let app = TestApp::new(db).await;
    let anon = app.anon();
    for doc in DOCS {
        for locale in ["de", "en"] {
            let res = anon.get(&format!("/api/legal/{doc}?locale={locale}")).await;
            res.assert_status(200);
            let body = res.json();
            assert_eq!(body["locale"], locale);
            assert_eq!(body["updated"], Value::Null);
            let html = body["html"].as_str().unwrap();
            assert!(html.contains("data-legal-placeholder"), "{doc} {locale}");
        }
    }
    let config = anon.get("/api/config").await.json();
    assert_eq!(config["legal"], json!(["imprint", "privacy", "terms"]));
    assert_eq!(config["terms_version"], Value::Null);
    assert_eq!(config["contact_email"], Value::Null);
}
