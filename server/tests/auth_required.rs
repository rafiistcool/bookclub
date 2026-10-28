//! Route-wide guard (docs/phase1/conventions.md §17.7): an anonymous caller gets 401 `Not
//! signed in` on every documented operation except the public allow-list. Generic: it walks the
//! OpenAPI document, so Wave 2 routes are covered as soon as they merge.

mod common;

use common::routes::{operations, outcome};
use common::*;
use serde_json::json;
use sqlx::PgPool;

/// The operations an anonymous caller may use (§17.7). Entries a later package adds are
/// skipped until their route exists.
const PUBLIC: [&str; 14] = [
    "GET /api/health",
    "GET /api/config",
    "GET /api/openapi.json",
    "POST /api/auth/register",
    "POST /api/auth/login",
    "POST /api/auth/logout",
    "POST /api/auth/tokens",
    "POST /api/auth/verify-email",
    "POST /api/auth/verify-email/preview",
    "POST /api/auth/password-reset",
    "POST /api/auth/password-reset/confirm",
    "GET /api/invites/{code}",
    "GET /media/{public_id}/{variant}",
    "GET /api/legal/{doc}",
];

/// Operations of Wave 1 that must be in the walk, so it can never pass vacuously.
const WAVE1_PROTECTED: [&str; 6] = [
    "GET /api/auth/me",
    "GET /api/clubs",
    "PATCH /api/clubs/{slug}/members/{username}",
    "POST /api/invites/{code}/accept",
    "GET /api/admin/invites",
    "GET /api/auth/me/export",
];

#[sqlx::test]
async fn anonymous_callers_get_401_outside_the_public_allow_list(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let anon = app.anon();

    let mut protected = Vec::new();
    for op in operations() {
        let label = op.label();
        let res = op.send(&anon, &op.fill_dummy(&club.slug), &json!({})).await;
        if PUBLIC.contains(&label.as_str()) {
            assert_ne!(
                res.status.as_u16(),
                401,
                "{label} is public but answers 401: {}",
                res.text()
            );
        } else {
            assert_eq!(
                outcome(&res),
                (401, json!({"detail": "Not signed in"})),
                "{label}"
            );
            protected.push(label);
        }
    }
    for label in WAVE1_PROTECTED {
        assert!(
            protected.iter().any(|walked| walked == label),
            "{label} was not walked"
        );
    }
}

/// A dead session cookie is no way around the guard either: the answer is the same 401.
#[sqlx::test]
async fn a_dead_session_cookie_gets_401_too(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let stale = app.anon();
    for op in operations() {
        let label = op.label();
        if PUBLIC.contains(&label.as_str()) {
            continue;
        }
        stale.set_cookie(Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
        let res = op
            .send(&stale, &op.fill_dummy(&club.slug), &json!({}))
            .await;
        res.assert_detail(401, "Not signed in");
    }
}
