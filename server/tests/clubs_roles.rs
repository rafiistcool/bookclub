//! The role table of #40 (endpoint-map §3): every club route with an allowed and a denied case.
//! Each row gets a fresh club, so a destructive allowed case cannot affect the next row, and the
//! denied case runs first and must change nothing.

mod common;

use axum::http::Method;
use common::routes::{operations, outcome};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    Anon,
    Unverified,
    /// A verified user in another club.
    Outsider,
    Member,
    Admin,
    Owner,
    InstanceAdmin,
}

/// One row: `METHOD path` with `{slug}`, `{target}` (a member), `{deputy}` (a second admin),
/// `{invite_id}` and `{code}` filled in; the denied caller with the expected answer; the allowed
/// caller with the expected status.
struct Row {
    route: &'static str,
    body: Option<Value>,
    denied: Option<(Who, u16, &'static str)>,
    allowed: (Who, u16),
}

fn rows() -> Vec<Row> {
    use Who::*;
    let row = |route, body, denied, allowed| Row {
        route,
        body,
        denied,
        allowed,
    };
    vec![
        row(
            "GET /api/clubs",
            None,
            Some((Anon, 401, "Not signed in")),
            (Member, 200),
        ),
        row(
            "POST /api/clubs",
            Some(json!({"name": "New club"})),
            Some((Unverified, 403, "Verify your email address first")),
            (Member, 201),
        ),
        row(
            "GET /api/clubs/{slug}",
            None,
            Some((Outsider, 404, "No such club")),
            (Member, 200),
        ),
        row(
            "PATCH /api/clubs/{slug}",
            Some(json!({"name": "Renamed"})),
            Some((Member, 403, "Only club admins can do that")),
            (Admin, 200),
        ),
        row(
            "DELETE /api/clubs/{slug}",
            None,
            Some((Admin, 403, "Only the club owner can do that")),
            (Owner, 204),
        ),
        row(
            "GET /api/clubs/{slug}/members",
            None,
            Some((Outsider, 404, "No such club")),
            (Member, 200),
        ),
        row(
            "PATCH /api/clubs/{slug}/members/{target}",
            Some(json!({"role": "admin"})),
            Some((Admin, 403, "Only the club owner can do that")),
            (Owner, 200),
        ),
        row(
            "DELETE /api/clubs/{slug}/members/{target}",
            None,
            Some((Member, 403, "Only club admins can do that")),
            (Admin, 204),
        ),
        row(
            "DELETE /api/clubs/{slug}/members/{deputy}",
            None,
            Some((Admin, 403, "Only the club owner can do that")),
            (Owner, 204),
        ),
        row(
            "POST /api/clubs/{slug}/leave",
            None,
            Some((Owner, 409, "Transfer ownership before leaving")),
            (Member, 204),
        ),
        row(
            "POST /api/clubs/{slug}/transfer",
            Some(json!({"username": "{target}"})),
            Some((Admin, 403, "Only the club owner can do that")),
            (Owner, 200),
        ),
        row(
            "GET /api/clubs/{slug}/invites",
            None,
            Some((Member, 403, "Only club admins can do that")),
            (Admin, 200),
        ),
        row(
            "POST /api/clubs/{slug}/invites",
            Some(json!({"kind": "link"})),
            Some((Member, 403, "Only club admins can do that")),
            (Admin, 201),
        ),
        row(
            "DELETE /api/clubs/{slug}/invites/{invite_id}",
            None,
            Some((Member, 403, "Only club admins can do that")),
            (Admin, 204),
        ),
        // Public: the join page shows the preview to anyone with the code.
        row("GET /api/invites/{code}", None, None, (Anon, 200)),
        row(
            "POST /api/invites/{code}/accept",
            None,
            Some((Anon, 401, "Not signed in")),
            (Outsider, 200),
        ),
        row(
            "GET /api/admin/invites",
            None,
            Some((Owner, 404, "Not found")),
            (InstanceAdmin, 200),
        ),
        row(
            "POST /api/admin/invites",
            Some(json!({})),
            Some((Admin, 404, "Not found")),
            (InstanceAdmin, 201),
        ),
    ]
}

/// One club with a caller of every kind, named after the row number.
struct Fixture {
    callers: Vec<(Who, TestClient)>,
    club: TestClub,
    target: String,
    deputy: String,
    invite_id: i64,
    code: String,
}

impl Fixture {
    async fn new(app: &TestApp, n: usize) -> Self {
        let owner = app.user(&format!("owner{n}")).await;
        let admin = app.user(&format!("admin{n}")).await;
        let deputy = app.user(&format!("deputy{n}")).await;
        let member = app.user(&format!("member{n}")).await;
        let target = app.user(&format!("target{n}")).await;
        let outsider = app.user(&format!("outsider{n}")).await;
        app.club(&outsider, "Elsewhere").await;
        let club = app.club(&owner, "Readers").await;
        app.join(&club, &admin, "admin").await;
        app.join(&club, &deputy, "admin").await;
        app.join(&club, &member, "member").await;
        app.join(&club, &target, "member").await;
        let invite = owner
            .post(
                &format!("/api/clubs/{}/invites", club.slug),
                json!({"kind": "link"}),
            )
            .await;
        invite.assert_status(201);
        let callers = vec![
            (Who::Anon, app.anon()),
            (
                Who::Unverified,
                app.user_unverified(&format!("unverified{n}")).await,
            ),
            (Who::Outsider, outsider),
            (Who::Member, member),
            (Who::Admin, admin),
            (Who::Owner, owner),
            (Who::InstanceAdmin, app.admin(&format!("root{n}")).await),
        ];
        Fixture {
            callers,
            club,
            target: target.username().to_owned(),
            deputy: deputy.username().to_owned(),
            invite_id: invite.json()["id"].as_i64().unwrap(),
            code: invite.json()["code"].as_str().unwrap().to_owned(),
        }
    }

    fn fill(&self, text: &str) -> String {
        text.replace("{slug}", &self.club.slug)
            .replace("{target}", &self.target)
            .replace("{deputy}", &self.deputy)
            .replace("{invite_id}", &self.invite_id.to_string())
            .replace("{code}", &self.code)
    }

    fn caller(&self, who: Who) -> &TestClient {
        &self.callers.iter().find(|(w, _)| *w == who).unwrap().1
    }

    async fn send(&self, row: &Row, who: Who) -> TestResponse {
        let (method, path) = row.route.split_once(' ').unwrap();
        let method: Method = method.parse().unwrap();
        let path = self.fill(path);
        let client = self.caller(who);
        match &row.body {
            Some(body) => {
                let body = self.fill(&body.to_string());
                client
                    .request(method, &path, Some("application/json"), body.into_bytes())
                    .await
            }
            None => client.request(method, &path, None, Vec::new()).await,
        }
    }
}

/// The state a denied request must leave alone.
async fn state_of(app: &TestApp) -> Vec<i64> {
    let mut out = Vec::new();
    for sql in [
        "SELECT count(*) FROM clubs",
        "SELECT count(*) FROM clubs WHERE name = 'Readers'",
        "SELECT count(*) FROM club_memberships",
        "SELECT count(*) FROM club_memberships WHERE role = 'owner'",
        "SELECT count(*) FROM club_memberships WHERE role = 'admin'",
        "SELECT count(*) FROM club_invites",
        "SELECT count(*) FROM club_invites WHERE revoked_at IS NOT NULL",
        "SELECT coalesce(sum(use_count), 0)::bigint FROM club_invites",
        "SELECT count(*) FROM club_removals",
    ] {
        out.push(app.scalar_i64(sql).await);
    }
    out
}

#[sqlx::test]
async fn every_role_row_has_an_allowed_and_a_denied_case(db: PgPool) {
    let app = TestApp::new(db).await;
    let rows = rows();
    for (n, row) in rows.iter().enumerate() {
        let fixture = Fixture::new(&app, n).await;
        if let Some((who, status, detail)) = row.denied {
            let before = state_of(&app).await;
            let res = fixture.send(row, who).await;
            assert_eq!(
                outcome(&res),
                (status, json!({ "detail": detail })),
                "{} denied to {who:?}",
                row.route
            );
            assert_eq!(
                state_of(&app).await,
                before,
                "{} denied to {who:?} changed something",
                row.route
            );
        }
        let (who, status) = row.allowed;
        let res = fixture.send(row, who).await;
        assert_eq!(
            res.status.as_u16(),
            status,
            "{} allowed to {who:?}: {}",
            row.route,
            res.text()
        );
    }
    // Every operation of the clubs module (tag `clubs`) and the sign-up invites have a row.
    for op in operations() {
        if op.tag != "clubs" && op.path != "/api/admin/invites" {
            continue;
        }
        let covered = rows.iter().any(|row| {
            let (method, path) = row.route.split_once(' ').unwrap();
            let path = path
                .replace("{target}", "{username}")
                .replace("{deputy}", "{username}");
            method == op.method.as_str() && path == op.path
        });
        assert!(covered, "{} has no row", op.label());
    }
}
