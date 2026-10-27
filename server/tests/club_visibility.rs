//! Personal data CLUB serves, across the viewer types of `tests/common/visibility.rs`
//! (conventions §6.5): the pick's readers. Readers are the club's members only, read through
//! `bc_visible_shelf`, so an "only me" rating and take are hidden from everyone but the owner,
//! while `dnf_reason` and `progress` stay visible (work-plan appendix "club" 11).

mod common;

use common::visibility::Viewers;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn shelve(app: &TestApp, who: &TestClient, work: &TestWork, columns: &str, values: &str) {
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, {columns}) VALUES ({}, {}, {values})",
        who.id(),
        work.id
    ))
    .await;
}

fn readers(pick: &Value) -> Vec<(String, Value, Value, Value, Value)> {
    pick["readers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["username"].as_str().unwrap().to_owned(),
                r["status"].clone(),
                r["rating"].clone(),
                r["take"].clone(),
                json!([r["dnf_reason"], r["progress"]]),
            )
        })
        .collect()
}

fn name(n: &str) -> String {
    n.to_owned()
}

#[sqlx::test]
async fn pick_readers_are_club_members_with_only_me_ratings_redacted(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.work("Circe").await;
    let dnf = app.user("dnf").await;
    app.join(&v.shared, &dnf, "member").await;
    shelve(
        &app,
        &v.owner,
        &circe,
        "status, rating, rating_visibility, take",
        "'finished', 5, 'only_me', 'Loved it'",
    )
    .await;
    shelve(
        &app,
        &v.co_member,
        &circe,
        "status, progress",
        "'currently_reading', 40",
    )
    .await;
    shelve(
        &app,
        &dnf,
        &circe,
        "status, dnf_reason, rating_visibility",
        "'did_not_finish', 'Too slow', 'only_me'",
    )
    .await;
    // Not members of the club (any more): never readers there.
    shelve(&app, &v.other_club, &circe, "status", "'want_to_read'").await;
    shelve(&app, &v.former, &circe, "status", "'finished'").await;

    let shared_pick = format!("/api/clubs/{}/pick", v.shared.slug);
    v.owner
        .put(&shared_pick, json!({"work_id": circe.id}))
        .await
        .assert_status(200);

    let seen = v.co_member.get(&shared_pick).await.json()["pick"].clone();
    assert_eq!(
        readers(&seen),
        [
            (
                name("comember"),
                json!("currently_reading"),
                Value::Null,
                json!(""),
                json!(["", 40])
            ),
            (
                name("dnf"),
                json!("did_not_finish"),
                Value::Null,
                json!(""),
                json!(["Too slow", null])
            ),
            (
                name("owner"),
                json!("finished"),
                Value::Null,
                json!(""),
                json!(["", null])
            ),
        ]
    );
    assert_eq!(seen["reading"], json!(["comember"]));
    assert_eq!(seen["finished"], json!(["owner"]));

    let own = v.owner.get(&shared_pick).await.json()["pick"].clone();
    assert_eq!(
        readers(&own)[0],
        (
            name("owner"),
            json!("finished"),
            json!(5),
            json!("Loved it"),
            json!(["", null])
        )
    );
    assert_eq!(own["on_shelf"], "finished");

    // An audience rating is visible to the club.
    app.exec(&format!(
        "UPDATE shelf_entries SET rating_visibility = 'audience' WHERE user_id = {}",
        v.owner.id()
    ))
    .await;
    let seen = v.co_member.get(&shared_pick).await.json()["pick"].clone();
    let owner_row = readers(&seen).into_iter().find(|r| r.0 == "owner").unwrap();
    assert_eq!((owner_row.2, owner_row.3), (json!(5), json!("Loved it")));

    // Strangers, members of another club and former members do not see the club at all.
    for viewer in v.strangers() {
        viewer
            .get(&shared_pick)
            .await
            .assert_detail(404, "No such club");
    }

    // In the other club, the owner (whom the co-member can see through `shared`) is no reader:
    // readers are that club's members only.
    let other_pick = format!("/api/clubs/{}/pick", v.other.slug);
    v.other_club
        .put(&other_pick, json!({"work_id": circe.id}))
        .await
        .assert_status(200);
    let there = v.co_member.get(&other_pick).await.json()["pick"].clone();
    let names: Vec<String> = readers(&there).into_iter().map(|r| r.0).collect();
    assert_eq!(names, ["comember", "otherclub"]);
}

/// The readers are sorted with the viewer first, then by username.
#[sqlx::test]
async fn readers_put_the_viewer_first(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let club = app.club(&ada, "Readers").await;
    let circe = app.work("Circe").await;
    let mut members = Vec::new();
    for username in ["zed", "grace", "bob"] {
        let user = app.user(username).await;
        app.join(&club, &user, "member").await;
        members.push(user);
    }
    for user in members.iter().chain([&ada]) {
        shelve(&app, user, &circe, "status", "'want_to_read'").await;
    }
    let path = format!("/api/clubs/{}/pick", club.slug);
    ada.put(&path, json!({"work_id": circe.id}))
        .await
        .assert_status(200);
    let grace = &members[1];
    let pick = grace.get(&path).await.json()["pick"].clone();
    let names: Vec<String> = readers(&pick).into_iter().map(|r| r.0).collect();
    assert_eq!(names, ["grace", "ada", "bob", "zed"]);
    assert_eq!(pick["reading"], json!([]));
    assert_eq!(pick["finished"], json!([]));
    assert_eq!(pick["readers"][0]["avatar_url"], Value::Null);
}
