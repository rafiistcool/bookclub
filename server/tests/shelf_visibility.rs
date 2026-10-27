//! Who sees whose shelf, favourites and physical layout (#46, conventions §6.5): the owner and
//! their audience; strangers, former co-members and members of other clubs get the same 404 as
//! an unknown name. `only_me` ratings are redacted for everyone but the owner. Only the owner
//! saves a layout: `PUT ?username=` naming anyone else is 404.

mod common;

use common::visibility::{UNKNOWN, Viewers, assert_hidden_like_unknown};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn add(client: &TestClient, work: &TestWork, extra: Value) -> Value {
    let mut body = json!({ "work_id": work.id });
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    let res = client.post("/api/shelf", body).await;
    res.assert_status(201);
    res.json()
}

fn entry<'a>(shelf: &'a Value, id: &Value) -> &'a Value {
    shelf["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| &item["id"] == id)
        .unwrap()
}

#[sqlx::test]
async fn shelf_is_visible_to_the_audience_with_only_me_ratings_redacted(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let open = add(
        &v.owner,
        &app.ol_work("OL1W", "Circe", "").await,
        json!({"status": "finished", "rating": 5, "take": "Loved it"}),
    )
    .await;
    let private = add(
        &v.owner,
        &app.ol_work("OL2W", "Song", "").await,
        json!({"status": "finished", "rating": 2, "take": "Meh", "rating_visibility": "only_me"}),
    )
    .await;
    let dropped = add(
        &v.owner,
        &app.ol_work("OL3W", "Galatea", "").await,
        json!({"status": "did_not_finish", "dnf_reason": "Too grim", "rating_visibility": "only_me"}),
    )
    .await;
    let reading = add(
        &v.owner,
        &app.ol_work("OL4W", "Ariadne", "").await,
        json!({"status": "currently_reading", "progress": 40, "rating_visibility": "only_me"}),
    )
    .await;

    let own = v.owner.get("/api/shelf").await.json();
    assert_eq!(entry(&own, &private["id"])["rating"], 2);
    assert_eq!(entry(&own, &private["id"])["take"], "Meh");
    assert_eq!(entry(&own, &private["id"])["rating_visibility"], "only_me");
    assert_eq!(entry(&own, &open["id"])["rating_visibility"], "audience");

    for viewer in v.audience() {
        let res = viewer.get("/api/shelf?username=owner").await;
        res.assert_status(200);
        let shelf = res.json();
        assert_eq!(shelf["user"]["username"], "owner");
        assert_eq!(shelf["items"].as_array().unwrap().len(), 4);
        let open = entry(&shelf, &open["id"]);
        assert_eq!(
            (open["rating"].clone(), open["take"].clone()),
            (json!(5), json!("Loved it"))
        );
        let private = entry(&shelf, &private["id"]);
        assert_eq!(private["rating"], Value::Null);
        assert_eq!(private["take"], "");
        // dnf_reason and progress are shelf state, not part of the rating.
        assert_eq!(entry(&shelf, &dropped["id"])["dnf_reason"], "Too grim");
        assert_eq!(entry(&shelf, &reading["id"])["progress"], 40);
        for item in shelf["items"].as_array().unwrap() {
            assert_eq!(item["rating_visibility"], Value::Null);
        }
    }
    for viewer in v.strangers() {
        assert_hidden_like_unknown(
            viewer,
            |name| format!("/api/shelf?username={name}"),
            "owner",
        )
        .await;
    }
    for viewer in [&v.owner, &v.co_member, &v.stranger] {
        viewer
            .get(&format!("/api/shelf?username={UNKNOWN}"))
            .await
            .assert_detail(404, "No member with that name");
    }
    // The audience is not transitive: otherclub shares a club with comember only.
    v.other_club
        .get("/api/shelf?username=comember")
        .await
        .assert_status(200);
    v.co_member
        .get("/api/shelf?username=otherclub")
        .await
        .assert_status(200);
}

#[sqlx::test]
async fn favourites_are_shown_to_the_audience_only(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.ol_work("OL1W", "Circe", "").await;
    v.owner
        .put("/api/auth/me/favorites", json!({"work_ids": [circe.id]}))
        .await
        .assert_status(200);
    for viewer in v.audience() {
        let favorites = viewer.get("/api/shelf?username=owner").await.json()["favorites"].clone();
        assert_eq!(favorites[0]["book"]["title"], "Circe");
        assert_eq!(favorites[0]["position"], 1);
        // A viewer's own favourites list is their own.
        assert_eq!(
            viewer.get("/api/auth/me/favorites").await.json(),
            json!({"items": []})
        );
    }
    for viewer in v.strangers() {
        assert_hidden_like_unknown(
            viewer,
            |name| format!("/api/shelf?username={name}"),
            "owner",
        )
        .await;
    }
}

#[sqlx::test]
async fn layout_is_visible_to_the_audience_and_saved_by_its_owner_only(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let book = add(&v.owner, &app.ol_work("OL1W", "Circe", "").await, json!({})).await;
    let layout = json!({
        "rows": [[book["id"], "decor:plant"]],
        "material": "walnut",
        "lights": true,
        "spines": {book["id"].to_string(): {"color": "#123456", "width": 30, "display": "spine"}},
        "decorations": {"decor:plant": {"kind": "plant", "color": "#abcdef"}}
    });
    v.owner
        .put("/api/shelf/physical-layout?username=owner", layout.clone())
        .await
        .assert_status(200);

    for viewer in v.audience() {
        let res = viewer
            .get("/api/shelf/physical-layout?username=owner")
            .await;
        res.assert_status(200);
        assert_eq!(res.json(), layout);
    }
    for viewer in v.strangers() {
        assert_hidden_like_unknown(
            viewer,
            |name| format!("/api/shelf/physical-layout?username={name}"),
            "owner",
        )
        .await;
    }

    // Nobody else can save the owner's layout: the same 404 as an unknown name, for the
    // audience and for strangers, and the stored layout stays.
    let other = json!({"rows": [[]], "material": "black"});
    let unknown = v
        .co_member
        .put(
            &format!("/api/shelf/physical-layout?username={UNKNOWN}"),
            other.clone(),
        )
        .await;
    unknown.assert_detail(404, "No member with that name");
    for viewer in [&v.co_member, &v.other_club, &v.stranger, &v.former] {
        let res = viewer
            .put("/api/shelf/physical-layout?username=owner", other.clone())
            .await;
        res.assert_detail(404, "No member with that name");
        assert_eq!(res.json(), unknown.json());
    }
    assert_eq!(
        v.owner.get("/api/shelf/physical-layout").await.json(),
        layout
    );
    // The viewers' own layouts are untouched too.
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM physical_shelf_layouts")
            .await,
        1
    );
}
