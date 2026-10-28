//! Shelf entries: the list, add, change and remove, the two orders, the per-user lock and the
//! shelf events (endpoint-map §7, docs/phase1/spec/shelf.md §1.1, §1.2, §1.7, §1.8, §2; #44).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// Puts `work` on the client's shelf with the extra body fields; asserts 201.
async fn add(client: &TestClient, work: &TestWork, extra: Value) -> Value {
    let mut body = json!({ "work_id": work.id });
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    let res = client.post("/api/shelf", body).await;
    res.assert_status(201);
    res.json()
}

/// PATCHes an entry; asserts 200.
async fn patch(client: &TestClient, id: &Value, body: Value) -> Value {
    let res = client.patch(&format!("/api/shelf/{id}"), body).await;
    res.assert_status(200);
    res.json()
}

/// The shelf items the client sees (its own, or `username`'s).
async fn items(client: &TestClient, username: Option<&str>) -> Vec<Value> {
    let path = match username {
        Some(name) => format!("/api/shelf?username={name}"),
        None => "/api/shelf".to_owned(),
    };
    let res = client.get(&path).await;
    res.assert_status(200);
    res.json()["items"].as_array().unwrap().clone()
}

fn titles(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|item| item["book"]["title"].as_str().unwrap().to_owned())
        .collect()
}

/// ada and grace, who share a club, so each is in the other's audience.
async fn friends(app: &TestApp) -> (TestClient, TestClient) {
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    (ada, grace)
}

async fn circe(app: &TestApp) -> TestWork {
    app.ol_work("OL1W", "Circe", "Madeline Miller").await
}

/// The kind and payload of each event the user recorded, oldest first.
async fn events(app: &TestApp, user: &TestClient) -> Vec<(String, Value)> {
    sqlx::query_as::<_, (String, Value)>(
        "SELECT kind, payload FROM events WHERE actor_id = $1 ORDER BY id",
    )
    .bind(user.id())
    .fetch_all(&app.db)
    .await
    .unwrap()
}

/// parity: test_shelf.py::test_shelving_without_authors_does_not_clear_existing_authors
/// Reasoned change: a shelf add carries no book metadata, so it can never edit the catalog
/// row; the legacy keys are ignored.
#[sqlx::test]
async fn shelving_never_changes_the_catalog_row(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let circe = circe(&app).await;
    let first = add(&ada, &circe, json!({})).await;
    assert_eq!(first["book"]["authors"], "Madeline Miller");

    let second = add(
        &grace,
        &circe,
        json!({"ol_work_key": "/works/OL9W", "title": "Renamed", "authors": "", "cover_id": 5}),
    )
    .await;
    assert_eq!(second["book"]["title"], "Circe");
    assert_eq!(second["book"]["authors"], "Madeline Miller");
    let theirs = items(&grace, Some("ada")).await;
    assert_eq!(theirs[0]["book"]["authors"], "Madeline Miller");
    assert_eq!(theirs[0]["book"]["title"], "Circe");
}

/// parity: test_shelf.py::test_add_move_remove_and_unique
#[sqlx::test]
async fn add_move_remove_and_unique(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    let item = add(&ada, &circe, json!({"status": "want_to_read"})).await;
    assert_eq!(
        item,
        json!({
            "id": item["id"],
            "status": "want_to_read",
            "position": 0,
            "shelf_position": null,
            "updated_at": item["updated_at"],
            "started_at": null,
            "finished_at": null,
            "book": {
                "id": circe.id,
                "ol_work_key": "/works/OL1W",
                "title": "Circe",
                "authors": "Madeline Miller",
                "year": null,
                "cover_url": null,
                "cover_thumb_url": null,
                "cover_public_id": null,
                "pages": null,
                "spine_color": null,
                "source": "openlibrary",
                "custom": false
            },
            "edition": null,
            "rating": null,
            "rating_visibility": "audience",
            "take": "",
            "dnf_reason": "",
            "progress": null
        })
    );
    assert!(item["updated_at"].as_str().unwrap().ends_with('Z'));

    let duplicate = ada.post("/api/shelf", json!({"work_id": circe.id})).await;
    duplicate.assert_status(409);
    assert_eq!(
        duplicate.json(),
        json!({"detail": "Already on your shelf", "item": item})
    );

    let moved = patch(&ada, &item["id"], json!({"status": "currently_reading"})).await;
    assert_eq!(moved["status"], "currently_reading");

    let shelf = items(&ada, None).await;
    assert_eq!(shelf.len(), 1);
    assert_eq!(shelf[0]["status"], "currently_reading");

    ada.delete(&format!("/api/shelf/{}", item["id"]))
        .await
        .assert_status(204);
    assert!(items(&ada, None).await.is_empty());
}

/// parity: test_shelf.py::test_cannot_edit_someone_elses_row
#[sqlx::test]
async fn cannot_edit_someone_elses_row(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let item = add(&ada, &circe(&app).await, json!({})).await;
    let path = format!("/api/shelf/{}", item["id"]);
    grace
        .patch(&path, json!({"status": "finished"}))
        .await
        .assert_detail(404, "Book not on your shelf");
    grace
        .delete(&path)
        .await
        .assert_detail(404, "Book not on your shelf");
    assert_eq!(items(&grace, Some("ada")).await.len(), 1);
    assert_eq!(items(&ada, None).await[0]["status"], "want_to_read");
}

/// parity: test_shelf.py::test_reorder_within_and_across_columns
#[sqlx::test]
async fn reorder_within_and_across_columns(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(&ada, &circe(&app).await, json!({})).await;
    let song = app
        .ol_work("OL2W", "Song of Achilles", "Madeline Miller")
        .await;
    let second = add(&ada, &song, json!({})).await;
    // The newest add is at the top.
    let shelf = items(&ada, None).await;
    assert_eq!(titles(&shelf), ["Song of Achilles", "Circe"]);
    assert_eq!(shelf[0]["position"], 0);
    assert_eq!(shelf[1]["position"], 1);

    patch(
        &ada,
        &first["id"],
        json!({"position": 0, "status": "want_to_read"}),
    )
    .await;
    assert_eq!(
        titles(&items(&ada, None).await),
        ["Circe", "Song of Achilles"]
    );

    patch(
        &ada,
        &second["id"],
        json!({"status": "currently_reading", "position": 0}),
    )
    .await;
    let shelf = items(&ada, None).await;
    let want: Vec<Value> = shelf
        .iter()
        .filter(|row| row["status"] == "want_to_read")
        .cloned()
        .collect();
    let reading: Vec<Value> = shelf
        .iter()
        .filter(|row| row["status"] == "currently_reading")
        .cloned()
        .collect();
    assert_eq!(titles(&want), ["Circe"]);
    assert_eq!(want[0]["position"], 0);
    assert_eq!(titles(&reading), ["Song of Achilles"]);
}

/// parity: test_shelf.py::test_unknown_member_shelf_is_404
#[sqlx::test]
async fn unknown_member_shelf_is_404(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.get("/api/shelf?username=nobody")
        .await
        .assert_detail(404, "No member with that name");
    // An empty name is the caller's own shelf; the name is case-insensitive.
    ada.get("/api/shelf?username=").await.assert_status(200);
    let res = ada.get("/api/shelf?username=ADA").await;
    res.assert_status(200);
    assert_eq!(res.json()["user"]["username"], "ada");
}

/// parity: test_shelf.py::test_finish_rating_and_take_show_on_friend_shelf
#[sqlx::test]
async fn finish_rating_and_take_show_on_friend_shelf(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let item = add(&ada, &circe(&app).await, json!({})).await;
    let moved = patch(
        &ada,
        &item["id"],
        json!({"status": "finished", "rating": 4, "take": "  Witchy and sad  "}),
    )
    .await;
    assert_eq!(moved["status"], "finished");
    assert_eq!(moved["rating"], 4);
    assert_eq!(moved["take"], "Witchy and sad");
    assert_eq!(moved["dnf_reason"], "");

    let friend = &items(&grace, Some("ada")).await[0];
    assert_eq!(friend["rating"], 4);
    assert_eq!(friend["take"], "Witchy and sad");
    // The visibility setting itself is the owner's business.
    assert_eq!(friend["rating_visibility"], Value::Null);
}

/// parity: test_shelf.py::test_dnf_reason_shows_on_friend_shelf
#[sqlx::test]
async fn dnf_reason_shows_on_friend_shelf(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let item = add(&ada, &circe(&app).await, json!({})).await;
    let moved = patch(
        &ada,
        &item["id"],
        json!({"status": "did_not_finish", "dnf_reason": "  Too grim  "}),
    )
    .await;
    assert_eq!(moved["status"], "did_not_finish");
    assert_eq!(moved["dnf_reason"], "Too grim");
    assert_eq!(moved["rating"], Value::Null);
    assert_eq!(moved["take"], "");

    let friend = &items(&grace, Some("ada")).await[0];
    assert_eq!(friend["dnf_reason"], "Too grim");
    assert_eq!(friend["rating"], Value::Null);
}

/// parity: test_shelf.py::test_dnf_reason_is_optional
#[sqlx::test]
async fn dnf_reason_is_optional(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, json!({})).await;
    let moved = patch(&ada, &item["id"], json!({"status": "did_not_finish"})).await;
    assert_eq!(moved["dnf_reason"], "");
}

/// parity: test_shelf.py::test_moving_off_finished_clears_notes
#[sqlx::test]
async fn moving_off_finished_clears_notes(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(
        &ada,
        &circe(&app).await,
        json!({"status": "finished", "rating": 5, "take": "Loved it"}),
    )
    .await;
    assert_eq!(
        (item["rating"].clone(), item["take"].clone()),
        (json!(5), json!("Loved it"))
    );
    let moved = patch(&ada, &item["id"], json!({"status": "currently_reading"})).await;
    assert_eq!(moved["rating"], Value::Null);
    assert_eq!(moved["take"], "");
    assert_eq!(moved["dnf_reason"], "");
}

/// What club readers, rating means and stats read about one owner's entry: the row of
/// `bc_visible_shelf(viewer)` (rating, take, progress).
async fn seen_by(
    app: &TestApp,
    viewer: &TestClient,
    entry: &Value,
) -> (Option<i32>, String, Option<i32>) {
    sqlx::query_as("SELECT rating, take, progress FROM bc_visible_shelf($1) WHERE id = $2")
        .bind(viewer.id())
        .bind(entry["id"].as_i64().unwrap())
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// parity: test_shelf.py::test_finish_notes_appear_on_club_pick
/// The pick's reader list is CLUB's (tests/club_*.rs); it reads the shelf through
/// `bc_visible_shelf`, which this test checks for a co-member.
#[sqlx::test]
async fn finish_notes_reach_the_club_readers(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let item = add(&ada, &circe(&app).await, json!({})).await;
    patch(
        &ada,
        &item["id"],
        json!({"status": "finished", "rating": 5, "take": "What a book"}),
    )
    .await;
    assert_eq!(
        seen_by(&app, &grace, &item).await,
        (Some(5), "What a book".to_owned(), None)
    );
}

/// parity: test_shelf.py::test_reading_progress_shows_on_friend_shelf_and_pick
/// As above, the pick half is checked on what the pick reads (`bc_visible_shelf`).
#[sqlx::test]
async fn reading_progress_shows_on_friend_shelf_and_to_club_readers(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let item = add(
        &ada,
        &circe(&app).await,
        json!({"status": "currently_reading"}),
    )
    .await;
    let updated = patch(&ada, &item["id"], json!({"progress": 35})).await;
    assert_eq!(updated["status"], "currently_reading");
    assert_eq!(updated["progress"], 35);
    assert_eq!(items(&grace, Some("ada")).await[0]["progress"], 35);
    assert_eq!(
        seen_by(&app, &grace, &item).await,
        (None, String::new(), Some(35))
    );
}

/// parity: test_shelf.py::test_progress_is_optional_and_clears_when_leaving_reading
#[sqlx::test]
async fn progress_is_optional_and_clears_when_leaving_reading(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let created = add(
        &ada,
        &circe(&app).await,
        json!({"status": "currently_reading"}),
    )
    .await;
    assert_eq!(created["progress"], Value::Null);
    assert_eq!(
        patch(&ada, &created["id"], json!({"progress": 80})).await["progress"],
        80
    );
    let moved = patch(&ada, &created["id"], json!({"status": "finished"})).await;
    assert_eq!(moved["progress"], Value::Null);
}

/// parity: test_shelf.py::test_invalid_progress
#[sqlx::test]
async fn invalid_progress(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(
        &ada,
        &circe(&app).await,
        json!({"status": "currently_reading"}),
    )
    .await;
    let path = format!("/api/shelf/{}", item["id"]);
    ada.patch(&path, json!({"progress": -1}))
        .await
        .assert_detail(400, "Input should be greater than or equal to 0");
    ada.patch(&path, json!({"progress": 101}))
        .await
        .assert_detail(400, "Input should be less than or equal to 100");
    ada.post("/api/shelf", json!({"work_id": 1, "progress": 101}))
        .await
        .assert_status(400);
}

/// parity: test_shelf.py::test_invalid_finish_fields
#[sqlx::test]
async fn invalid_finish_fields(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(&ada, &circe(&app).await, json!({})).await;
    let path = format!("/api/shelf/{}", item["id"]);
    ada.patch(&path, json!({"status": "finished", "rating": 0}))
        .await
        .assert_detail(400, "Input should be greater than or equal to 1");
    ada.patch(&path, json!({"status": "finished", "rating": 6}))
        .await
        .assert_detail(400, "Input should be less than or equal to 5");
    ada.patch(
        &path,
        json!({"status": "finished", "take": "x".repeat(141)}),
    )
    .await
    .assert_detail(400, "Keep the take to 140 characters");
    ada.patch(
        &path,
        json!({"status": "did_not_finish", "dnf_reason": "x".repeat(201)}),
    )
    .await
    .assert_detail(400, "Keep the reason to 200 characters");
    // Limits count characters after trimming, not bytes.
    let take = format!("  {}  ", "ä".repeat(140));
    let finished = patch(
        &ada,
        &item["id"],
        json!({"status": "finished", "take": take}),
    )
    .await;
    assert_eq!(finished["take"], "ä".repeat(140));
    // Every message is reported, in field order.
    ada.post(
        "/api/shelf",
        json!({"work_id": 1, "rating": 9, "take": "x".repeat(141), "dnf_reason": "y".repeat(201)}),
    )
    .await
    .assert_detail(
        400,
        "Input should be less than or equal to 5; Keep the take to 140 characters; Keep the reason to 200 characters",
    );
    assert_eq!(items(&ada, None).await.len(), 1);
}

/// parity: test_shelf.py::test_free_shelf_order_persists_without_changing_reading_metadata
#[sqlx::test]
async fn free_shelf_order_persists_without_changing_reading_metadata(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let want = add(&ada, &circe(&app).await, json!({})).await;
    let reading = add(
        &ada,
        &app.ol_work("OL2W", "Song", "").await,
        json!({"status": "currently_reading", "progress": 35}),
    )
    .await;
    let finished = add(
        &ada,
        &app.ol_work("OL3W", "Galatea", "").await,
        json!({"status": "finished", "rating": 5, "take": "Loved it"}),
    )
    .await;
    patch(&ada, &finished["id"], json!({"finished_on": "2025-04-12"})).await;
    let before = items(&ada, None).await;

    patch(&ada, &finished["id"], json!({"shelf_position": 0})).await;
    let rows = items(&ada, None).await;
    assert_eq!(
        by_shelf_position(&rows),
        [&finished["id"], &want["id"], &reading["id"]]
    );
    for row in &rows {
        let old = before.iter().find(|old| old["id"] == row["id"]).unwrap();
        for field in [
            "status",
            "position",
            "rating",
            "take",
            "progress",
            "started_at",
            "finished_at",
            "updated_at",
        ] {
            assert_eq!(row[field], old[field], "{field} of {}", row["id"]);
        }
    }

    // Reordering at the far end clamps safely and produces contiguous ranks.
    patch(&ada, &finished["id"], json!({"shelf_position": 99})).await;
    let rows = items(&ada, None).await;
    assert_eq!(
        by_shelf_position(&rows),
        [&want["id"], &reading["id"], &finished["id"]]
    );
    let mut ranks: Vec<i64> = rows
        .iter()
        .map(|row| row["shelf_position"].as_i64().unwrap())
        .collect();
    ranks.sort();
    assert_eq!(ranks, [0, 1, 2]);
    ada.patch(
        &format!("/api/shelf/{}", finished["id"]),
        json!({"shelf_position": -1}),
    )
    .await
    .assert_detail(400, "Input should be greater than or equal to 0");
}

/// The entry ids ordered by `shelf_position`.
fn by_shelf_position(rows: &[Value]) -> Vec<&Value> {
    let mut sorted: Vec<&Value> = rows.iter().collect();
    sorted.sort_by_key(|row| row["shelf_position"].as_i64());
    sorted.into_iter().map(|row| &row["id"]).collect()
}

/// parity: test_shelf.py::test_free_order_is_private_and_survives_status_changes_and_new_books
#[sqlx::test]
async fn free_order_is_private_and_survives_status_changes_and_new_books(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let first = add(&ada, &circe(&app).await, json!({})).await;
    let second = add(
        &ada,
        &app.ol_work("OL2W", "Song", "").await,
        json!({"status": "finished"}),
    )
    .await;
    patch(&ada, &second["id"], json!({"shelf_position": 0})).await;
    let changed = patch(&ada, &first["id"], json!({"status": "currently_reading"})).await;
    assert_eq!(changed["shelf_position"], 1);
    let newest = add(&ada, &app.ol_work("OL3W", "Galatea", "").await, json!({})).await;
    assert_eq!(newest["shelf_position"], Value::Null);
    patch(&ada, &newest["id"], json!({"shelf_position": 2})).await;
    let before = items(&ada, None).await;
    assert_eq!(
        by_shelf_position(&before),
        [&second["id"], &first["id"], &newest["id"]]
    );

    grace
        .patch(
            &format!("/api/shelf/{}", first["id"]),
            json!({"shelf_position": 0}),
        )
        .await
        .assert_detail(404, "Book not on your shelf");
    // Others see the same rows, without the owner's rating setting.
    let mut seen = before.clone();
    for item in &mut seen {
        item["rating_visibility"] = Value::Null;
    }
    assert_eq!(items(&grace, Some("ada")).await, seen);
}

/// `Nothing to update` is answered before the entry is looked up; null counts as absent for
/// status, position, shelf_position and rating_visibility, while any present tri-state field
/// is something to update.
#[sqlx::test]
async fn nothing_to_update_comes_before_the_404(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let item = add(&ada, &circe(&app).await, json!({})).await;
    let theirs = format!("/api/shelf/{}", item["id"]);
    for body in [
        json!({}),
        json!({"status": null, "position": null, "shelf_position": null, "rating_visibility": null}),
        json!({"unknown": 1}),
    ] {
        grace
            .patch(&theirs, body.clone())
            .await
            .assert_detail(400, "Nothing to update");
        grace
            .patch("/api/shelf/999999", body)
            .await
            .assert_detail(400, "Nothing to update");
    }
    for body in [
        json!({"rating": null}),
        json!({"take": null}),
        json!({"dnf_reason": null}),
        json!({"progress": null}),
        json!({"finished_on": null}),
        json!({"edition_id": null}),
        json!({"rating_visibility": "only_me"}),
    ] {
        grace
            .patch(&theirs, body)
            .await
            .assert_detail(404, "Book not on your shelf");
    }
    ada.patch("/api/shelf/abc", json!({"status": "finished"}))
        .await
        .assert_detail(
            400,
            "Input should be a valid integer, unable to parse string as an integer",
        );
}

/// The work must exist (a merged one resolves to its target); an edition must belong to it.
#[sqlx::test]
async fn add_checks_the_work_and_its_edition(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    let edition = app.edition(&circe, "9780316556347").await;
    let other = app.ol_work("OL2W", "Song", "").await;
    let other_edition = app.edition(&other, "9781408816035").await;

    ada.post("/api/shelf", json!({"work_id": 999999}))
        .await
        .assert_detail(404, "No such book.");
    ada.post(
        "/api/shelf",
        json!({"work_id": circe.id, "edition_id": other_edition.id}),
    )
    .await
    .assert_detail(400, "That edition belongs to another book");
    ada.post("/api/shelf", json!({"edition_id": edition.id}))
        .await
        .assert_detail(400, "Field required");
    ada.post(
        "/api/shelf",
        json!({"work_id": circe.id, "status": "reading"}),
    )
    .await
    .assert_status(400);

    app.exec(&format!(
        "UPDATE editions SET title = 'Kirke', pages = 400 WHERE id = {}",
        edition.id
    ))
    .await;
    let item = add(&ada, &circe, json!({"edition_id": edition.id})).await;
    assert_eq!(item["edition"]["id"], edition.id);
    assert_eq!(item["edition"]["isbn13"], "9780316556347");
    assert_eq!(item["book"]["id"], circe.id);
    assert_eq!(item["book"]["title"], "Kirke");
    assert_eq!(item["book"]["pages"], 400);

    // A merged-away work resolves to the work it was merged into.
    app.exec(&format!(
        "UPDATE works SET merged_into_id = {} WHERE id = {}",
        other.id, circe.id
    ))
    .await;
    let merged = ada.post("/api/shelf", json!({"work_id": circe.id})).await;
    merged.assert_status(201);
    assert_eq!(merged.json()["book"]["id"], other.id);
}

/// PATCH sets, keeps and clears the edition; an edition of another work is refused unchanged.
#[sqlx::test]
async fn patch_changes_the_edition_and_the_rating_visibility(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    let edition = app.edition(&circe, "9780316556347").await;
    let other = app
        .edition(&app.ol_work("OL2W", "Song", "").await, "9781408816035")
        .await;
    let item = add(&ada, &circe, json!({"status": "finished", "rating": 4})).await;
    let path = format!("/api/shelf/{}", item["id"]);

    let set = patch(&ada, &item["id"], json!({"edition_id": edition.id})).await;
    assert_eq!(set["edition"]["id"], edition.id);
    let kept = patch(&ada, &item["id"], json!({"rating": 5})).await;
    assert_eq!(kept["edition"]["id"], edition.id);
    assert_eq!(kept["rating"], 5);
    ada.patch(&path, json!({"edition_id": other.id}))
        .await
        .assert_detail(400, "That edition belongs to another book");
    let cleared = patch(&ada, &item["id"], json!({"edition_id": null})).await;
    assert_eq!(cleared["edition"], Value::Null);

    let hidden = patch(&ada, &item["id"], json!({"rating_visibility": "only_me"})).await;
    assert_eq!(hidden["rating_visibility"], "only_me");
    assert_eq!(hidden["rating"], 5);
    // A status change keeps the setting.
    let moved = patch(&ada, &item["id"], json!({"status": "currently_reading"})).await;
    assert_eq!(moved["rating_visibility"], "only_me");
    ada.patch(&path, json!({"rating_visibility": "friends"}))
        .await
        .assert_status(400);
}

/// The client's abandoned books in `year` of the stats.
async fn dnf_in(client: &TestClient, year: i32) -> i64 {
    let res = client.get(&format!("/api/stats?year={year}")).await;
    res.assert_status(200);
    res.json()["dnf"].as_i64().unwrap()
}

/// Hiding or showing a rating is no change to the entry: a PATCH with only `rating_visibility`
/// leaves `updated_at` and the column alone, so an abandoned book stays in the year of its
/// last change in the stats. Any other field sent with it still moves `updated_at`.
#[sqlx::test]
async fn a_rating_visibility_change_alone_is_no_change(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let dropped = add(
        &ada,
        &circe(&app).await,
        json!({"status": "did_not_finish", "rating": 2, "dnf_reason": "Too slow"}),
    )
    .await;
    add(
        &ada,
        &app.ol_work("OL2W", "Song", "").await,
        json!({"status": "did_not_finish"}),
    )
    .await;
    // Last changed in 2025; it sits below the newer book in its column.
    sqlx::query("UPDATE shelf_entries SET updated_at = '2025-03-01T12:00:00Z' WHERE id = $1")
        .bind(dropped["id"].as_i64().unwrap())
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(dnf_in(&ada, 2025).await, 1);

    for visibility in ["only_me", "audience"] {
        let item = patch(
            &ada,
            &dropped["id"],
            json!({"rating_visibility": visibility}),
        )
        .await;
        assert_eq!(item["rating_visibility"], visibility);
        assert_eq!(item["updated_at"], "2025-03-01T12:00:00Z");
        assert_eq!(item["position"], 1);
        assert_eq!(dnf_in(&ada, 2025).await, 1);
    }

    let edited = patch(
        &ada,
        &dropped["id"],
        json!({"rating_visibility": "only_me", "dnf_reason": "Too slow, really"}),
    )
    .await;
    assert_eq!(edited["rating_visibility"], "only_me");
    assert_ne!(edited["updated_at"], "2025-03-01T12:00:00Z");
    assert_eq!(dnf_in(&ada, 2025).await, 0);
}

/// The list is ordered by the status text (alphabetically), then position and id.
#[sqlx::test]
async fn the_list_is_ordered_by_status_text(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for (n, status) in [
        "want_to_read",
        "finished",
        "did_not_finish",
        "currently_reading",
    ]
    .into_iter()
    .enumerate()
    {
        let work = app.ol_work(&format!("OL{}W", n + 1), status, "").await;
        add(&ada, &work, json!({"status": status})).await;
    }
    let shelf = items(&ada, None).await;
    let statuses: Vec<&str> = shelf
        .iter()
        .map(|row| row["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        [
            "currently_reading",
            "did_not_finish",
            "finished",
            "want_to_read"
        ]
    );
    let res = ada.get("/api/shelf").await.json();
    assert_eq!(res["timezone"], "UTC");
    assert_eq!(
        res["user"],
        json!({"id": ada.id(), "username": "ada", "avatar_url": null})
    );
    assert_eq!(res["favorites"], json!([]));
}

/// DELETE renumbers the column it left and does not compact the free order.
#[sqlx::test]
async fn delete_renumbers_the_column(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut ids = Vec::new();
    for n in 1..=3 {
        let work = app
            .ol_work(&format!("OL{n}W"), &format!("Book {n}"), "")
            .await;
        ids.push(add(&ada, &work, json!({})).await["id"].clone());
    }
    patch(&ada, &ids[0], json!({"shelf_position": 2})).await;
    // Column: [3, 2, 1]; free order: [3, 2, 1] with ranks 0, 1, 2.
    ada.delete(&format!("/api/shelf/{}", ids[1]))
        .await
        .assert_status(204);
    let shelf = items(&ada, None).await;
    let summary: Vec<(Value, Value, Value)> = shelf
        .iter()
        .map(|row| {
            (
                row["id"].clone(),
                row["position"].clone(),
                row["shelf_position"].clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (ids[2].clone(), json!(0), json!(0)),
            (ids[0].clone(), json!(1), json!(2))
        ]
    );
    ada.delete(&format!("/api/shelf/{}", ids[1]))
        .await
        .assert_detail(404, "Book not on your shelf");
}

/// Two adds of one work at once: one 201, one 409 with the entry, never a 500.
#[sqlx::test]
async fn concurrent_adds_of_one_work_give_one_conflict(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    let body = json!({"work_id": circe.id});
    let (a, b) = tokio::join!(
        ada.post("/api/shelf", body.clone()),
        ada.post("/api/shelf", body)
    );
    let mut statuses = [a.status.as_u16(), b.status.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [201, 409]);
    let (created, conflict) = if a.status == 201 { (a, b) } else { (b, a) };
    assert_eq!(conflict.json()["item"]["id"], created.json()["id"]);
    assert_eq!(items(&ada, None).await.len(), 1);
}

/// Concurrent reorders from several devices serialise on the user lock: the column stays
/// numbered 0..n without duplicates.
#[sqlx::test]
async fn concurrent_reorders_keep_the_column_dense(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut ids = Vec::new();
    for n in 1..=8 {
        let work = app
            .ol_work(&format!("OL{n}W"), &format!("Book {n}"), "")
            .await;
        ids.push(add(&ada, &work, json!({})).await["id"].clone());
    }
    let paths: Vec<String> = ids.iter().map(|id| format!("/api/shelf/{id}")).collect();
    let moves = paths
        .iter()
        .enumerate()
        .map(|(n, path)| ada.patch(path, json!({"position": n % 3, "shelf_position": 0})));
    for res in futures::future::join_all(moves).await {
        res.assert_status(200);
    }
    let shelf = items(&ada, None).await;
    let mut positions: Vec<i64> = shelf
        .iter()
        .map(|row| row["position"].as_i64().unwrap())
        .collect();
    positions.sort();
    assert_eq!(positions, (0..8).collect::<Vec<_>>());
    let mut ranks: Vec<i64> = shelf
        .iter()
        .map(|row| row["shelf_position"].as_i64().unwrap())
        .collect();
    ranks.sort();
    assert_eq!(ranks, (0..8).collect::<Vec<_>>());
}

/// The shelf events (docs/phase1/spec/shelf.md §2.6): kinds, payloads without nulls, one
/// `progress` event per real change, none for rating edits; all personal (no club).
#[sqlx::test]
async fn shelf_writes_record_their_events(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    let item = add(&ada, &circe, json!({})).await;
    patch(&ada, &item["id"], json!({"status": "currently_reading"})).await;
    patch(&ada, &item["id"], json!({"progress": 50})).await;
    patch(&ada, &item["id"], json!({"progress": 50})).await;
    patch(&ada, &item["id"], json!({"progress": null})).await;
    patch(
        &ada,
        &item["id"],
        json!({"status": "finished", "rating": 4, "take": "Great"}),
    )
    .await;
    patch(
        &ada,
        &item["id"],
        json!({"rating": 5, "take": "Even better"}),
    )
    .await;
    patch(&ada, &item["id"], json!({"status": "did_not_finish"})).await;
    patch(&ada, &item["id"], json!({"status": "finished"})).await;
    ada.delete(&format!("/api/shelf/{}", item["id"]))
        .await
        .assert_status(204);
    let song = app.ol_work("OL2W", "Song", "").await;
    add(
        &ada,
        &song,
        json!({"status": "did_not_finish", "dnf_reason": "Too grim"}),
    )
    .await;
    add(
        &ada,
        &app.ol_work("OL3W", "Galatea", "").await,
        json!({"status": "finished", "rating": 3}),
    )
    .await;

    assert_eq!(
        events(&app, &ada).await,
        [
            ("shelf_added".to_owned(), json!({"status": "want_to_read"})),
            (
                "shelf_moved".to_owned(),
                json!({"status": "currently_reading"})
            ),
            ("progress".to_owned(), json!({"progress": 50})),
            // The removal sealed them to the reader (security review 2026-10-28 #2).
            (
                "shelf_finished".to_owned(),
                json!({"rating": 4, "take": "Great", "private": true})
            ),
            ("shelf_dnf".to_owned(), json!({})),
            ("shelf_finished".to_owned(), json!({})),
            ("shelf_removed".to_owned(), json!({"status": "finished"})),
            ("shelf_dnf".to_owned(), json!({"reason": "Too grim"})),
            ("shelf_finished".to_owned(), json!({"rating": 3})),
        ]
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM events WHERE actor_id = {} AND (club_id IS NOT NULL OR work_id IS NULL)",
            ada.id()
        ))
        .await,
        0
    );
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM events WHERE actor_id = {} AND work_id = {}",
            ada.id(),
            circe.id
        ))
        .await,
        7
    );
}

/// Every shelf route needs a session.
#[sqlx::test]
async fn the_shelf_needs_a_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let anon = app.anon();
    anon.get("/api/shelf")
        .await
        .assert_detail(401, "Not signed in");
    anon.post("/api/shelf", json!({"work_id": 1}))
        .await
        .assert_detail(401, "Not signed in");
    anon.patch("/api/shelf/1", json!({"status": "finished"}))
        .await
        .assert_detail(401, "Not signed in");
    anon.delete("/api/shelf/1")
        .await
        .assert_detail(401, "Not signed in");
}
