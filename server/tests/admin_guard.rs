//! Route-wide guard (docs/phase1/conventions.md §2, §17.7): a signed-in user who is not an
//! instance admin gets 404 `Not found` on every `/api/admin/*` operation, from the one
//! `InstanceAdmin` route layer. Generic: it walks the OpenAPI document, so every new admin route
//! is covered as soon as it is merged.
//!
//! The walk uses real ids and valid bodies: an open edit request, an open report, a live
//! picture, a work and an edition, an account to suspend and a suspended one, an active sign-up
//! invite. With dummy ids an
//! unguarded approve, resolve, takedown or suspension would answer 404 too (nothing to find), so
//! only real targets tell a guarded route from an unguarded one; the rows are compared before
//! and after the walk.

mod common;

use std::collections::HashMap;

use axum::http::Method;
use bookclub_server::catalog::shared::ingest::{CoverTarget, Source, ol_cover_media, set_cover};
use common::routes::{Operation, find, operations, outcome};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// Every admin operation. A new one is added here, with its seeded ids and a valid body below.
const ADMIN_OPERATIONS: [&str; 16] = [
    "DELETE /api/admin/invites/{invite_id}",
    "DELETE /api/admin/media/{public_id}",
    "GET /api/admin/edit-requests",
    "GET /api/admin/invites",
    "GET /api/admin/reports",
    "GET /api/admin/summary",
    "GET /api/admin/users/suspended",
    "PATCH /api/admin/editions/{edition_id}",
    "PATCH /api/admin/works/{work_id}",
    "POST /api/admin/edit-requests/{request_id}/approve",
    "POST /api/admin/edit-requests/{request_id}/reject",
    "POST /api/admin/invites",
    "POST /api/admin/reports/{report_id}/resolve",
    "POST /api/admin/users/{username}/suspend",
    "POST /api/admin/users/{username}/unsuspend",
    "POST /api/admin/works/{work_id}/merge",
];

/// The tables an admin operation writes; the walk must leave every row as it was.
const WATCHED_TABLES: [&str; 10] = [
    "works",
    "editions",
    "media",
    "media_removed_hashes",
    "catalog_edit_requests",
    "reports",
    "club_invites",
    "legacy_work_keys",
    "users",
    "sessions",
];

/// Every row of the watched tables, as JSON.
async fn snapshot(app: &TestApp) -> Vec<(&'static str, Value)> {
    let mut rows = Vec::new();
    for table in WATCHED_TABLES {
        let value: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), '[]'::jsonb)
             FROM {table} t"
        )))
        .fetch_one(&app.db)
        .await
        .unwrap();
        rows.push((table, value));
    }
    rows
}

/// The seeded targets: path values by parameter name (`op_ids` first, for one operation),
/// and a valid body per operation.
struct Seeded {
    ids: HashMap<&'static str, String>,
    op_ids: HashMap<(&'static str, &'static str), String>,
    bodies: HashMap<&'static str, Value>,
}

impl Seeded {
    fn path(&self, op: &Operation) -> String {
        let label = op.label();
        op.fill(|name, _| {
            self.op_ids
                .iter()
                .find(|((op_label, op_name), _)| *op_label == label && *op_name == name)
                .map(|(_, value)| value)
                .or_else(|| self.ids.get(name))
                .cloned()
                .unwrap_or_else(|| panic!("{label}: no seeded value for {{{name}}}"))
        })
    }

    fn body(&self, op: &Operation) -> Value {
        let label = op.label();
        match self.bodies.get(label.as_str()) {
            Some(body) => body.clone(),
            None if op.has_body => panic!("{label}: no valid body seeded"),
            None => json!({}),
        }
    }
}

/// An open edit request, an open report of a live picture, a work with an edition, a second
/// work to merge into, all made the way readers make them, plus a signed-in account to suspend
/// (`mallory`), a suspended one to unsuspend (`sleeper`) and an active sign-up invite.
async fn seed(app: &TestApp, reader: &TestClient, club: &TestClub) -> Seeded {
    let work = app.work("Circe").await;
    let edition = app.edition(&work, "9780316556347").await;
    let other = app.work("Ich bin Circe").await;
    let public_id: String = {
        let mut conn = app.db.acquire().await.unwrap();
        let media = ol_cover_media(&mut conn, 777).await.unwrap().unwrap();
        let target = CoverTarget::Work(work.id);
        let set = set_cover(&mut conn, target, media, Source::OpenLibrary, false).await;
        assert!(set.unwrap(), "the cover is set");
        sqlx::query_scalar("SELECT public_id FROM media WHERE id = $1")
            .bind(media)
            .fetch_one(&mut *conn)
            .await
            .unwrap()
    };
    let res = reader
        .post(
            &format!("/api/books/works/{}/edit-requests", work.id),
            json!({"fields": {"pages": 393}, "note": "From the back cover."}),
        )
        .await;
    res.assert_status(201);
    let request_id = res.json()["id"].as_i64().unwrap();
    let res = reader
        .post(
            &format!("/api/media/{public_id}/reports"),
            json!({"reason": "wrong_book"}),
        )
        .await;
    res.assert_status(201);
    let report_id = res.json()["id"].as_i64().unwrap();
    app.user("mallory").await;
    let sleeper = app.user("sleeper").await;
    app.exec(&format!(
        "UPDATE users SET suspended_at = now() - interval '1 day' WHERE id = {};
         DELETE FROM sessions WHERE user_id = {0}",
        sleeper.id()
    ))
    .await;

    let invite_id = app
        .scalar_i64(
            "INSERT INTO club_invites (club_id, code, max_uses, expires_at)
             VALUES (NULL, 'SEEDSIGNUP', 1, now() + interval '30 days')
             RETURNING id",
        )
        .await;

    let op_ids = HashMap::from([
        (
            ("POST /api/admin/users/{username}/suspend", "username"),
            "mallory".to_owned(),
        ),
        (
            ("POST /api/admin/users/{username}/unsuspend", "username"),
            "sleeper".to_owned(),
        ),
    ]);
    let ids = HashMap::from([
        ("slug", club.slug.clone()),
        ("request_id", request_id.to_string()),
        ("report_id", report_id.to_string()),
        ("public_id", public_id),
        ("work_id", work.id.to_string()),
        ("edition_id", edition.id.to_string()),
        ("invite_id", invite_id.to_string()),
    ]);
    let bodies = HashMap::from([
        // The takedown with the notice it answers, which also writes a report row.
        (
            "DELETE /api/admin/media/{public_id}",
            json!({
                "details": "A notice by email.",
                "received_at": "2026-01-05T09:30:00Z",
                "reason": "copyright"
            }),
        ),
        (
            "POST /api/admin/edit-requests/{request_id}/reject",
            json!({"note": "No."}),
        ),
        ("POST /api/admin/invites", json!({"max_uses": 5})),
        (
            "POST /api/admin/reports/{report_id}/resolve",
            json!({"action": "remove"}),
        ),
        (
            "POST /api/admin/works/{work_id}/merge",
            json!({"into_work_id": other.id}),
        ),
        (
            "PATCH /api/admin/works/{work_id}",
            json!({"fields": {"title": "Changed"}}),
        ),
        (
            "PATCH /api/admin/editions/{edition_id}",
            json!({"fields": {"pages": 999}}),
        ),
    ]);
    Seeded {
        ids,
        op_ids,
        bodies,
    }
}

/// Acceptance (#54): every admin operation, called on real rows with a valid body by a signed-in
/// user who is not an instance admin, answers 404 `Not found` and changes nothing.
#[sqlx::test]
async fn non_admins_get_404_on_every_admin_operation(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let seeded = seed(&app, &owner, &club).await;
    // A club owner is still no instance admin.
    let callers = [owner, app.user_unverified("grace").await];

    let admin_ops: Vec<_> = operations()
        .into_iter()
        .filter(|op| op.path.starts_with("/api/admin/"))
        .collect();
    let mut labels: Vec<String> = admin_ops.iter().map(Operation::label).collect();
    labels.sort();
    assert_eq!(labels, ADMIN_OPERATIONS, "the admin operations changed");
    let before = snapshot(&app).await;

    for op in &admin_ops {
        let path = seeded.path(op);
        let body = seeded.body(op);
        for caller in &callers {
            let res = op.send(caller, &path, &body).await;
            assert_eq!(
                outcome(&res),
                (404, json!({"detail": "Not found"})),
                "{} as {}",
                op.label(),
                caller.username()
            );
        }
    }

    let after = snapshot(&app).await;
    for ((table, rows), (_, now)) in before.iter().zip(&after) {
        assert_eq!(rows, now, "a refused admin request changed {table}");
    }
    let open = |table: &str, id: &str| {
        format!("SELECT count(*) FROM {table} WHERE id = {id} AND status = 'open'")
    };
    let request_id = &seeded.ids["request_id"];
    assert_eq!(
        app.scalar_i64(&open("catalog_edit_requests", request_id))
            .await,
        1
    );
    let report_id = &seeded.ids["report_id"];
    assert_eq!(app.scalar_i64(&open("reports", report_id)).await, 1);
    let suspended = "SELECT count(*) FROM users WHERE suspended_at IS NOT NULL";
    assert_eq!(app.scalar_i64(suspended).await, 1, "only sleeper");
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM media WHERE public_id = '{}' AND removed_at IS NULL",
            seeded.ids["public_id"]
        ))
        .await,
        1
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

/// The seeded requests are not vacuous: sent by an instance admin, the approval, the takedown,
/// the edition edit, the suspension and the invite's revocation change the rows the walk
/// watches.
#[sqlx::test]
async fn the_seeded_requests_act_for_an_admin(db: PgPool) {
    let app = TestApp::new(db).await;
    let owner = app.user("owner").await;
    let club = app.club(&owner, "Readers").await;
    let seeded = seed(&app, &owner, &club).await;
    let root = app.admin("root").await;
    let before = snapshot(&app).await;
    for (method, path) in [
        (Method::PATCH, "/api/admin/editions/{edition_id}"),
        (
            Method::POST,
            "/api/admin/edit-requests/{request_id}/approve",
        ),
        (Method::POST, "/api/admin/reports/{report_id}/resolve"),
        (Method::POST, "/api/admin/users/{username}/suspend"),
    ] {
        let op = find(&method, path).expect("documented");
        let res = op.send(&root, &seeded.path(&op), &seeded.body(&op)).await;
        res.assert_status(200);
    }
    let op = find(&Method::DELETE, "/api/admin/invites/{invite_id}").expect("documented");
    let res = op.send(&root, &seeded.path(&op), &seeded.body(&op)).await;
    res.assert_status(204);
    let after = snapshot(&app).await;
    let changed: Vec<&str> = before
        .iter()
        .zip(&after)
        .filter(|((_, rows), (_, now))| rows != now)
        .map(|((table, _), _)| *table)
        .collect();
    assert_eq!(
        changed,
        [
            "works",
            "editions",
            "media",
            "catalog_edit_requests",
            "reports",
            "club_invites",
            "users",
            "sessions"
        ]
    );
}
