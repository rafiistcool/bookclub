//! Who sees whose shelf, favourites, physical layout, quotes and stats (#46, conventions §6.5):
//! the owner and their audience; strangers, former co-members and members of other clubs get the
//! same 404 as an unknown name (an empty list for `GET /api/quotes?username=`). `only_me`
//! ratings and quotes are left out for everyone but the owner, in the stats too. Only the owner
//! saves a layout: `PUT ?username=` naming anyone else is 404. Club stats are for members.

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

/// Keeps a quote; asserts 201.
async fn quote(client: &TestClient, work: &TestWork, body: &str, visibility: &str) -> Value {
    let res = client
        .post(
            "/api/quotes",
            json!({"work_id": work.id, "body": body, "visibility": visibility}),
        )
        .await;
    res.assert_status(201);
    res.json()
}

fn bodies(res: &TestResponse) -> Vec<String> {
    res.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["body"].as_str().unwrap().to_owned())
        .collect()
}

#[sqlx::test]
async fn quotes_are_visible_to_the_audience_without_only_me_ones(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.ol_work("OL1W", "Circe", "").await;
    let open = quote(&v.owner, &circe, "Open line", "audience").await;
    let private = quote(&v.owner, &circe, "Private line", "only_me").await;

    let own = v.owner.get("/api/quotes?username=owner").await;
    assert_eq!(bodies(&own), vec!["Private line", "Open line"]);
    assert_eq!(bodies(&v.owner.get("/api/quotes").await).len(), 2);

    for viewer in v.audience() {
        let res = viewer.get("/api/quotes?username=owner").await;
        res.assert_status(200);
        assert_eq!(bodies(&res), vec!["Open line"]);
        assert_eq!(res.json()["items"][0]["mine"], false);
        assert_eq!(res.json()["items"][0]["author"], "owner");
        // Without a filter too, and for the work.
        assert_eq!(bodies(&viewer.get("/api/quotes").await), vec!["Open line"]);
        let by_work = viewer.get(&format!("/api/quotes?work={}", circe.id)).await;
        assert_eq!(bodies(&by_work), vec!["Open line"]);
        // Someone else's quote: 403 when visible, 404 when not.
        viewer
            .patch(&format!("/api/quotes/{}", open["id"]), json!({"page": 1}))
            .await
            .assert_detail(403, "You can only edit your own quotes");
        viewer
            .delete(&format!("/api/quotes/{}", open["id"]))
            .await
            .assert_detail(403, "You can only delete your own quotes");
        viewer
            .patch(
                &format!("/api/quotes/{}", private["id"]),
                json!({"page": 1}),
            )
            .await
            .assert_detail(404, "No quote with that id");
        viewer
            .delete(&format!("/api/quotes/{}", private["id"]))
            .await
            .assert_detail(404, "No quote with that id");
    }
    for viewer in v.strangers() {
        // An unknown name and a stranger give the same empty list.
        let hidden = viewer.get("/api/quotes?username=owner").await;
        let unknown = viewer.get(&format!("/api/quotes?username={UNKNOWN}")).await;
        assert_eq!(hidden.json(), json!({"items": []}));
        assert_eq!(hidden.json(), unknown.json());
        assert_eq!(
            bodies(&viewer.get("/api/quotes").await),
            Vec::<String>::new()
        );
        for quote in [&open, &private] {
            viewer
                .patch(&format!("/api/quotes/{}", quote["id"]), json!({"page": 1}))
                .await
                .assert_detail(404, "No quote with that id");
            viewer
                .delete(&format!("/api/quotes/{}", quote["id"]))
                .await
                .assert_detail(404, "No quote with that id");
        }
    }
    // Nothing changed.
    assert_eq!(bodies(&v.owner.get("/api/quotes").await).len(), 2);
}

#[sqlx::test]
async fn personal_stats_leave_only_me_ratings_and_quotes_out_for_others(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let open = app.ol_work("OL1W", "Circe", "").await;
    let private = app.ol_work("OL2W", "Song", "").await;
    add(&v.owner, &open, json!({"status": "finished", "rating": 4})).await;
    add(
        &v.owner,
        &private,
        json!({"status": "finished", "rating": 5, "rating_visibility": "only_me"}),
    )
    .await;
    quote(&v.owner, &open, "Open line", "audience").await;
    quote(&v.owner, &private, "Private line", "only_me").await;

    let own = v.owner.get("/api/stats").await.json();
    assert_eq!(own["finished"], 2);
    assert_eq!(own["average_rating"], 4.5);
    assert_eq!(own["five_stars"], 1);
    assert_eq!(own["top_book"]["title"], "Song");
    assert_eq!(own["quotes"], 2);
    assert_eq!(v.owner.get("/api/stats?username=owner").await.json(), own);

    for viewer in v.audience() {
        let res = viewer.get("/api/stats?username=owner").await;
        res.assert_status(200);
        let seen = res.json();
        assert_eq!(seen["username"], "owner");
        // The book counts (shelf state); its rating does not.
        assert_eq!(seen["finished"], 2);
        assert_eq!(seen["average_rating"], 4.0);
        assert_eq!(seen["five_stars"], 0);
        assert_eq!(seen["top_book"]["title"], "Circe");
        assert_eq!(seen["quotes"], 1);
        // The viewer's own stats are their own.
        assert_eq!(viewer.get("/api/stats").await.json()["finished"], 0);
    }
    for viewer in v.strangers() {
        assert_hidden_like_unknown(
            viewer,
            |name| format!("/api/stats?username={name}"),
            "owner",
        )
        .await;
    }
    v.owner
        .get(&format!("/api/stats?username={UNKNOWN}"))
        .await
        .assert_detail(404, "No member with that name");
}

#[sqlx::test]
async fn club_stats_are_for_members_with_only_me_ratings_left_out(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let circe = app.ol_work("OL1W", "Circe", "").await;
    app.exec(&format!(
        "INSERT INTO club_picks (club_id, work_id, set_by) VALUES ({}, {}, {})",
        v.shared.id,
        circe.id,
        v.owner.id()
    ))
    .await;
    add(
        &v.owner,
        &circe,
        json!({"status": "finished", "rating": 5, "rating_visibility": "only_me"}),
    )
    .await;
    add(
        &v.co_member,
        &circe,
        json!({"status": "finished", "rating": 3}),
    )
    .await;
    quote(&v.owner, &circe, "Private line", "only_me").await;
    let path = format!("/api/clubs/{}/stats", v.shared.slug);

    let member = |stats: &Value, name: &str| -> Value {
        stats["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["username"] == name)
            .cloned()
            .unwrap()
    };
    let seen = v.co_member.get(&path).await.json();
    assert_eq!(seen["picks"][0]["ratings"], json!([3]));
    assert_eq!(seen["picks"][0]["finished"], 2);
    assert_eq!(seen["picks"][0]["average_rating"], 3.0);
    assert_eq!(seen["club_average_rating"], 3.0);
    let owner = member(&seen, "owner");
    assert_eq!(owner["finished"], 1);
    assert_eq!(owner["average_rating"], Value::Null);
    assert_eq!(owner["top_book"], Value::Null);
    assert_eq!(owner["quotes"], 0);
    // The former member is not a member any more.
    assert_eq!(seen["members"].as_array().unwrap().len(), 2);

    // The owner sees their own rating and quote, as everywhere.
    let own = v.owner.get(&path).await.json();
    assert_eq!(own["picks"][0]["ratings"], json!([5, 3]));
    assert_eq!(member(&own, "owner")["quotes"], 1);

    for viewer in v.strangers() {
        viewer.get(&path).await.assert_detail(404, "No such club");
    }
}
