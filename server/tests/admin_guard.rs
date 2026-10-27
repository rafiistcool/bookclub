//! Route-wide guard (docs/phase1/conventions.md §2, §17.7): a signed-in user who is not an
//! instance admin gets 404 `Not found` on every `/api/admin/*` operation, from the one
//! `InstanceAdmin` route layer. Generic: it walks the OpenAPI document, so CAT's and MED's admin
//! routes are covered as soon as they merge.

mod common;

use axum::http::Method;
use common::routes::{find, operations, outcome};
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn non_admins_get_404_on_every_admin_operation(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    // A club owner is still no instance admin.
    let callers = [owner, app.user_unverified("grace").await];

    let admin_ops: Vec<_> = operations()
        .into_iter()
        .filter(|op| op.path.starts_with("/api/admin/"))
        .collect();
    for label in ["GET /api/admin/invites", "POST /api/admin/invites"] {
        assert!(
            admin_ops.iter().any(|op| op.label() == label),
            "{label} is not in the document"
        );
    }
    for op in &admin_ops {
        for caller in &callers {
            let res = op
                .send(caller, &op.fill_dummy(&club.slug), &json!({}))
                .await;
            assert_eq!(
                outcome(&res),
                (404, json!({"detail": "Not found"})),
                "{} as {}",
                op.label(),
                caller.username()
            );
        }
    }
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM club_invites").await,
        0,
        "a refused admin request wrote something"
    );
}

/// The guard lets instance admins through (a positive control for the walk above), and the
/// same request without a session is 401, not 404.
#[sqlx::test]
async fn instance_admins_pass_the_guard(db: PgPool) {
    let app = TestApp::new(db).await;
    let admin = app.admin("root").await;
    let list = find(&Method::GET, "/api/admin/invites").expect("documented");
    let res = list.send(&admin, &list.path, &json!({})).await;
    res.assert_status(200);
    assert_eq!(res.json(), json!({"items": []}));
    let res = list.send(&app.anon(), &list.path, &json!({})).await;
    res.assert_detail(401, "Not signed in");
}
