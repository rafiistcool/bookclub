//! The Wave 1 exit criterion (docs/phase1/work-plan.md, Wave 1): a signed-in user creates a
//! club and invites a second user, who joins; both then see each other's (empty) shelf through
//! the visibility policy, and a third user in no club sees neither. The shelf endpoint comes in
//! Wave 2, so the test calls the policy directly.

mod common;

use bookclub_server::visibility::{self, Relation};
use common::*;
use serde_json::json;
use sqlx::PgPool;

/// The rows of `owner`'s shelf `viewer` may read (`bc_visible_shelf`), and whether the owner is
/// among the owners `viewer` may see at all (`bc_visible_owners`).
async fn shelf_view(app: &TestApp, viewer: &TestClient, owner: &TestClient) -> (i64, bool) {
    let rows = app
        .scalar_i64(&format!(
            "SELECT count(*) FROM bc_visible_shelf({}) WHERE user_id = {}",
            viewer.id(),
            owner.id()
        ))
        .await;
    let visible = app
        .scalar_i64(&format!(
            "SELECT count(*) FROM bc_visible_owners({}) WHERE user_id = {}",
            viewer.id(),
            owner.id()
        ))
        .await;
    (rows, visible == 1)
}

#[sqlx::test]
async fn a_club_and_an_invite_make_two_users_each_others_audience(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let stranger = app.user("stranger").await;

    // Before: nobody sees anybody.
    let mut conn = app.db.acquire().await.unwrap();
    assert_eq!(
        visibility::relation(&mut conn, ada.id(), grace.id())
            .await
            .unwrap(),
        Relation::Stranger
    );

    let res = ada
        .post("/api/clubs", json!({"name": "Thursday Readers"}))
        .await;
    res.assert_status(201);
    let slug = res.json()["slug"].as_str().unwrap().to_owned();
    let res = ada
        .post(
            &format!("/api/clubs/{slug}/invites"),
            json!({"kind": "single_use"}),
        )
        .await;
    res.assert_status(201);
    let code = res.json()["code"].as_str().unwrap().to_owned();
    let preview = grace.get(&format!("/api/invites/{code}")).await;
    assert_eq!(
        preview.json(),
        json!({"club": {"slug": slug, "name": "Thursday Readers"}, "kind": "club", "personal": false})
    );
    let res = grace
        .post(&format!("/api/invites/{code}/accept"), json!({}))
        .await;
    res.assert_status(200);
    assert_eq!(res.json()["member_count"], 2);
    let members = ada.get(&format!("/api/clubs/{slug}/members")).await.json();
    assert_eq!(members["items"][1]["username"], "grace");

    // The two are each other's audience: the empty shelf is visible (no rows, but the owner is).
    for (viewer, owner) in [(&ada, &grace), (&grace, &ada)] {
        assert_eq!(
            visibility::relation(&mut conn, viewer.id(), owner.id())
                .await
                .unwrap(),
            Relation::Audience
        );
        let seen = visibility::visible_user(&mut conn, viewer.id(), owner.username())
            .await
            .unwrap();
        assert_eq!(seen.id, owner.id());
        assert_eq!(shelf_view(&app, viewer, owner).await, (0, true));
    }

    // The stranger sees neither, exactly like an unknown name, and neither sees the stranger.
    for owner in [&ada, &grace] {
        assert_eq!(
            visibility::relation(&mut conn, stranger.id(), owner.id())
                .await
                .unwrap(),
            Relation::Stranger
        );
        for name in [owner.username(), "nobody_here"] {
            let refused = visibility::visible_user(&mut conn, stranger.id(), name)
                .await
                .unwrap_err();
            assert_eq!(
                (refused.status.as_u16(), refused.detail.as_ref()),
                (404, "No member with that name")
            );
        }
        assert_eq!(shelf_view(&app, &stranger, owner).await, (0, false));
        assert_eq!(shelf_view(&app, owner, &stranger).await, (0, false));
    }

    // Once shelves have something on them, the audience reads the rows and the stranger not.
    let circe = app.work("Circe").await;
    for owner in [&ada, &grace] {
        app.exec(&format!(
            "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ({}, {}, 'want_to_read')",
            owner.id(),
            circe.id
        ))
        .await;
    }
    assert_eq!(shelf_view(&app, &ada, &grace).await, (1, true));
    assert_eq!(shelf_view(&app, &grace, &ada).await, (1, true));
    assert_eq!(shelf_view(&app, &stranger, &ada).await, (0, false));
    assert_eq!(shelf_view(&app, &stranger, &grace).await, (0, false));
}
