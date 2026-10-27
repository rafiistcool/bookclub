//! The role table of #45 (endpoint-map §9, work-plan appendix "club" 1): every CLUB route with
//! an allowed and a denied case. Members read everything, nominate and cast; admins set and
//! clear the pick and the meeting, manage milestones, set the vote's deadline and apply the
//! winner. The home feed needs only a session. Each row gets a fresh club, the denied case runs
//! first and must change nothing, and the allowed caller is the least privileged one that may.

mod common;

use axum::http::Method;
use common::routes::{operations, outcome};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// The module tags whose every operation must have a row.
const TAGS: [&str; 4] = ["picks", "votes", "overlap", "activity"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    /// No session.
    Anonymous,
    /// A verified user in another club.
    Outsider,
    /// A member whose address is not verified.
    UnverifiedMember,
    Member,
    Admin,
}

/// One row: `METHOD path` with `{slug}`, `{pick_id}`, `{milestone_id}`, `{work_id}` and
/// `{nomination_id}` filled in; the denied caller with the expected answer; the allowed caller
/// with the expected status.
struct Row {
    route: &'static str,
    body: Option<Value>,
    denied: (Who, u16, &'static str),
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
    let not_a_member = (Outsider, 404, "No such club");
    let not_an_admin = (Member, 403, "Only club admins can do that");
    vec![
        row(
            "GET /api/clubs/{slug}/pick",
            None,
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "PUT /api/clubs/{slug}/pick",
            Some(json!({"work_id": "{work_id}"})),
            not_an_admin,
            (Admin, 200),
        ),
        row(
            "PATCH /api/clubs/{slug}/pick",
            Some(json!({"note": "Bring wine", "meeting_at": "2026-09-12T19:00"})),
            not_an_admin,
            (Admin, 200),
        ),
        row(
            "DELETE /api/clubs/{slug}/pick",
            None,
            not_an_admin,
            (Admin, 200),
        ),
        row(
            "GET /api/clubs/{slug}/pick/history",
            None,
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "GET /api/clubs/{slug}/pick/meeting.ics",
            None,
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "GET /api/clubs/{slug}/picks/{pick_id}/milestones",
            None,
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "POST /api/clubs/{slug}/picks/{pick_id}/milestones",
            Some(json!({"title": "Part two", "unit": "percent", "range_to": 50})),
            not_an_admin,
            (Admin, 201),
        ),
        row(
            "PATCH /api/clubs/{slug}/picks/{pick_id}/milestones/{milestone_id}",
            Some(json!({"title": "Part 1"})),
            not_an_admin,
            (Admin, 200),
        ),
        row(
            "DELETE /api/clubs/{slug}/picks/{pick_id}/milestones/{milestone_id}",
            None,
            not_an_admin,
            (Admin, 204),
        ),
        row(
            "GET /api/clubs/{slug}/vote",
            None,
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "PATCH /api/clubs/{slug}/vote",
            Some(json!({"closes_at": "2030-01-01T18:00"})),
            not_an_admin,
            (Admin, 200),
        ),
        row(
            "POST /api/clubs/{slug}/vote/nominations",
            Some(json!({"work_id": "{work_id}"})),
            not_a_member,
            (UnverifiedMember, 201),
        ),
        row(
            "POST /api/clubs/{slug}/vote/cast",
            Some(json!({"nomination_id": "{nomination_id}"})),
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "POST /api/clubs/{slug}/vote/apply",
            Some(json!({"nomination_id": "{nomination_id}"})),
            not_an_admin,
            (Admin, 200),
        ),
        row(
            "GET /api/clubs/{slug}/vote/suggestions",
            None,
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "GET /api/clubs/{slug}/overlap",
            None,
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "GET /api/clubs/{slug}/activity",
            None,
            not_a_member,
            (UnverifiedMember, 200),
        ),
        row(
            "GET /api/activity",
            None,
            (Anonymous, 401, "Not signed in"),
            (UnverifiedMember, 200),
        ),
    ]
}

/// One club with a caller of every kind, a current pick with a meeting and a milestone, an open
/// vote with one nomination, and a second work to pick or nominate.
struct Fixture {
    callers: Vec<(Who, TestClient)>,
    club: TestClub,
    pick_id: i64,
    milestone_id: i64,
    work_id: i64,
    nomination_id: i64,
}

impl Fixture {
    async fn new(app: &TestApp, n: usize) -> Self {
        let owner = app.user(&format!("owner{n}")).await;
        let admin = app.user(&format!("admin{n}")).await;
        let member = app.user(&format!("member{n}")).await;
        let outsider = app.user(&format!("outsider{n}")).await;
        let unverified_member = app.user_unverified(&format!("unverified{n}")).await;
        app.club(&outsider, "Elsewhere").await;
        let club = app.club(&owner, "Readers").await;
        app.join(&club, &admin, "admin").await;
        app.join(&club, &member, "member").await;
        app.join(&club, &unverified_member, "member").await;
        let circe = app.work(&format!("Circe {n}")).await;
        let next = app.work(&format!("Achilles {n}")).await;
        let pick = owner
            .put(
                &format!("/api/clubs/{}/pick", club.slug),
                json!({"work_id": circe.id, "meeting_at": "2026-09-10T19:30"}),
            )
            .await;
        pick.assert_status(200);
        let pick_id = pick.json()["id"].as_i64().unwrap();
        let milestone = owner
            .post(
                &format!("/api/clubs/{}/picks/{pick_id}/milestones", club.slug),
                json!({"title": "Part one", "unit": "chapter", "range_from": 1, "range_to": 8}),
            )
            .await;
        milestone.assert_status(201);
        let wolf = app.work(&format!("Wolf {n}")).await;
        let vote = owner
            .post(
                &format!("/api/clubs/{}/vote/nominations", club.slug),
                json!({"work_id": wolf.id}),
            )
            .await;
        vote.assert_status(201);
        Fixture {
            callers: vec![
                (Who::Anonymous, app.anon()),
                (Who::Outsider, outsider),
                (Who::UnverifiedMember, unverified_member),
                (Who::Member, member),
                (Who::Admin, admin),
            ],
            club,
            pick_id,
            milestone_id: milestone.json()["id"].as_i64().unwrap(),
            work_id: next.id,
            nomination_id: vote.json()["nominations"][0]["id"].as_i64().unwrap(),
        }
    }

    fn fill(&self, text: &str) -> String {
        text.replace("{slug}", &self.club.slug)
            .replace("{pick_id}", &self.pick_id.to_string())
            .replace("{milestone_id}", &self.milestone_id.to_string())
            .replace("\"{work_id}\"", &self.work_id.to_string())
            .replace("\"{nomination_id}\"", &self.nomination_id.to_string())
    }

    async fn send(&self, row: &Row, who: Who) -> TestResponse {
        let (method, path) = row.route.split_once(' ').unwrap();
        let method: Method = method.parse().unwrap();
        let path = self.fill(path);
        let client = &self.callers.iter().find(|(w, _)| *w == who).unwrap().1;
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
        "SELECT count(*) FROM club_picks",
        "SELECT count(*) FROM club_picks WHERE ended_at IS NULL",
        "SELECT count(meeting_at) FROM club_picks",
        "SELECT coalesce(sum(length(note)), 0)::bigint FROM club_picks",
        "SELECT count(*) FROM milestones",
        "SELECT coalesce(sum(length(title) + coalesce(range_to, 0)), 0)::bigint FROM milestones",
        "SELECT count(*) FROM events",
        "SELECT count(*) FROM next_up_votes",
        "SELECT count(*) FROM next_up_votes WHERE ended_at IS NULL",
        "SELECT count(closes_at) FROM next_up_votes",
        "SELECT count(*) FROM next_up_nominations",
        "SELECT count(*) FROM next_up_ballots",
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
        let (who, status, detail) = row.denied;
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
        let (who, status) = row.allowed;
        let res = fixture.send(row, who).await;
        assert_eq!(
            res.status.as_u16(),
            status,
            "{} allowed to {who:?}: {}",
            row.route,
            res.text()
        );
        if row.route.starts_with("GET") {
            assert_eq!(
                state_of(&app).await,
                before,
                "{} wrote something",
                row.route
            );
        } else {
            assert_ne!(
                state_of(&app).await,
                before,
                "{} changed nothing",
                row.route
            );
        }
    }
    // Every operation of the CLUB modules has a row.
    let mut covered_ops = 0;
    for op in operations() {
        if !TAGS.contains(&op.tag.as_str()) {
            continue;
        }
        covered_ops += 1;
        let covered = rows.iter().any(|row| {
            let (method, path) = row.route.split_once(' ').unwrap();
            method == op.method.as_str() && path == op.path
        });
        assert!(covered, "{} has no row", op.label());
    }
    assert_eq!(covered_ops, rows.len());
}
