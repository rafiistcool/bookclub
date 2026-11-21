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
            "personal_cover": false,
            "edition": null,
            "rating": null,
            "rating_visibility": "audience",
            "take": "",
            "dnf_reason": "",
            "progress": null,
            "current_page": null
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
/// `progress` event per real change, none for rating edits (one right after finishing rewrites
/// the finish's payload instead); all personal (no club).
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

    // The removal keeps the ids of the finishes it sealed, for its Undo.
    let first_finish: i64 = sqlx::query_scalar(
        "SELECT min(id) FROM events WHERE actor_id = $1 AND kind = 'shelf_finished'",
    )
    .bind(ada.id())
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(
        events(&app, &ada).await,
        [
            ("shelf_added".to_owned(), json!({"status": "want_to_read"})),
            (
                "shelf_moved".to_owned(),
                json!({"status": "currently_reading"})
            ),
            ("progress".to_owned(), json!({"progress": 50})),
            // The rating edit right after finishing rewrote the payload; the removal sealed it
            // to the reader (security review 2026-10-28 #2).
            (
                "shelf_finished".to_owned(),
                json!({"rating": 5, "take": "Even better", "private": true})
            ),
            ("shelf_dnf".to_owned(), json!({})),
            ("shelf_finished".to_owned(), json!({})),
            (
                "shelf_removed".to_owned(),
                json!({"status": "finished", "sealed": [first_finish]})
            ),
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

/// The `shelf_finished` payloads the client recorded, oldest first.
async fn finish_payloads(app: &TestApp, user: &TestClient) -> Vec<Value> {
    events(app, user)
        .await
        .into_iter()
        .filter(|(kind, _)| kind == "shelf_finished")
        .map(|(_, payload)| payload)
        .collect()
}

/// The payloads of the `shelf_finished` items in the viewer's home feed, newest first.
async fn feed_finishes(viewer: &TestClient) -> Vec<Value> {
    let res = viewer.get("/api/activity").await;
    res.assert_status(200);
    res.json()["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "shelf_finished")
        .map(|item| item["payload"].clone())
        .collect()
}

/// Moves the client's newest `shelf_finished` event `minutes` into the past.
async fn age_last_finish(app: &TestApp, user: &TestClient, minutes: i32) {
    sqlx::query(
        "UPDATE events SET created_at = created_at - make_interval(mins => $2)
         WHERE id = (SELECT max(id) FROM events WHERE actor_id = $1 AND kind = 'shelf_finished')",
    )
    .bind(user.id())
    .bind(minutes)
    .execute(&app.db)
    .await
    .unwrap();
}

/// Stars and a take given right after finishing (the finished moment's own PATCH) rewrite the
/// finish's event, so the feeds show them; no second event is recorded, and a later edit of
/// the same finish keeps the payload in step while the window lasts.
#[sqlx::test]
async fn a_rating_right_after_finishing_reaches_the_finish_event(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let item = add(
        &ada,
        &circe(&app).await,
        json!({"status": "currently_reading"}),
    )
    .await;
    patch(
        &ada,
        &item["id"],
        json!({"status": "finished", "position": 0}),
    )
    .await;
    assert_eq!(feed_finishes(&grace).await, [json!({})]);

    patch(&ada, &item["id"], json!({"rating": 4, "take": "Witchy"})).await;
    assert_eq!(
        finish_payloads(&app, &ada).await,
        [json!({"rating": 4, "take": "Witchy"})]
    );
    assert_eq!(
        feed_finishes(&grace).await,
        [json!({"rating": 4, "take": "Witchy"})]
    );

    // Clearing the take (and a PATCH that changes nothing shown) keeps one event in step.
    patch(&ada, &item["id"], json!({"take": null})).await;
    patch(&ada, &item["id"], json!({"rating": 4, "progress": null})).await;
    assert_eq!(finish_payloads(&app, &ada).await, [json!({"rating": 4})]);
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM events WHERE actor_id = {}",
            ada.id()
        ))
        .await,
        2,
        "shelf_added and one shelf_finished"
    );
}

/// After the window a rating edit leaves the finish's event as it was written, and only the
/// newest finish of the work is ever rewritten.
#[sqlx::test]
async fn a_late_rating_leaves_the_finish_event_alone(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(
        &ada,
        &circe(&app).await,
        json!({"status": "finished", "rating": 3}),
    )
    .await;
    age_last_finish(&app, &ada, 31).await;
    patch(
        &ada,
        &item["id"],
        json!({"rating": 5, "take": "Grew on me"}),
    )
    .await;
    assert_eq!(finish_payloads(&app, &ada).await, [json!({"rating": 3})]);

    // A re-read: the new finish is rewritten, the old one keeps its payload.
    patch(&ada, &item["id"], json!({"status": "currently_reading"})).await;
    patch(&ada, &item["id"], json!({"status": "finished"})).await;
    patch(&ada, &item["id"], json!({"rating": 4})).await;
    assert_eq!(
        finish_payloads(&app, &ada).await,
        [json!({"rating": 3}), json!({"rating": 4})]
    );

    // Just inside the window still counts.
    age_last_finish(&app, &ada, 29).await;
    patch(&ada, &item["id"], json!({"rating": 2})).await;
    assert_eq!(
        finish_payloads(&app, &ada).await,
        [json!({"rating": 3}), json!({"rating": 2})]
    );
}

/// An only-me rating given right after finishing is sealed in the rewritten payload: the
/// others never see it, the reader does, and the seal stays when the reader shares the rating
/// again inside the window (activity::seal_finished_ratings, conventions.md §6.4).
#[sqlx::test]
async fn an_only_me_rating_after_finishing_stays_sealed(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let item = add(&ada, &circe(&app).await, json!({})).await;
    patch(
        &ada,
        &item["id"],
        json!({"status": "finished", "position": 0}),
    )
    .await;
    patch(
        &ada,
        &item["id"],
        json!({"rating": 5, "take": "Mine alone", "rating_visibility": "only_me"}),
    )
    .await;
    assert_eq!(
        finish_payloads(&app, &ada).await,
        [json!({"rating": 5, "take": "Mine alone", "private": true})]
    );
    assert_eq!(feed_finishes(&grace).await, [json!({})]);
    assert_eq!(
        feed_finishes(&ada).await,
        [json!({"rating": 5, "take": "Mine alone"})]
    );

    patch(
        &ada,
        &item["id"],
        json!({"rating_visibility": "audience", "rating": 4}),
    )
    .await;
    assert_eq!(
        finish_payloads(&app, &ada).await,
        [json!({"rating": 4, "take": "Mine alone", "private": true})]
    );
    assert_eq!(feed_finishes(&grace).await, [json!({})]);

    // Making a shared rating only-me inside the window seals the rewritten payload as well.
    let song = add(&ada, &app.ol_work("OL2W", "Song", "").await, json!({})).await;
    patch(
        &ada,
        &song["id"],
        json!({"status": "finished", "rating": 3}),
    )
    .await;
    patch(&ada, &song["id"], json!({"rating_visibility": "only_me"})).await;
    assert_eq!(
        finish_payloads(&app, &ada).await[1],
        json!({"rating": 3, "private": true})
    );
    assert_eq!(feed_finishes(&grace).await, [json!({}), json!({})]);
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

/// Sets the page count of a work (`edition` None) or of an edition.
async fn set_pages(app: &TestApp, table: &str, id: i64, pages: Option<i32>) {
    let pages = pages.map_or("NULL".to_owned(), |n| n.to_string());
    app.exec(&format!(
        "UPDATE {table} SET pages = {pages} WHERE id = {id}"
    ))
    .await;
}

/// Progress in pages (docs/phase3/notes/reading-trust.md): with a page count the server keeps
/// the page and derives the percent, so one page more is a change the reader sees; past the
/// last page is the last page; a percent alone replaces the page.
#[sqlx::test]
async fn a_page_is_kept_and_sets_the_percent(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let circe = circe(&app).await;
    set_pages(&app, "works", circe.id, Some(334)).await;
    let item = add(&ada, &circe, json!({"status": "currently_reading"})).await;

    let at = patch(&ada, &item["id"], json!({"current_page": 207})).await;
    assert_eq!(
        (&at["current_page"], &at["progress"]),
        (&json!(207), &json!(62))
    );
    // The critique's case: 208 is still 62 %, but the page is the reader's, not a rounding.
    let next = patch(&ada, &item["id"], json!({"current_page": 208})).await;
    assert_eq!(
        (&next["current_page"], &next["progress"]),
        (&json!(208), &json!(62))
    );
    let typed = patch(&ada, &item["id"], json!({"current_page": 218})).await;
    assert_eq!(
        typed["current_page"], 218,
        "a typed page comes back as typed"
    );
    // The others see the page too: it is shelf state, like the percent.
    let seen = items(&grace, Some("ada")).await;
    assert_eq!(
        (&seen[0]["current_page"], &seen[0]["progress"]),
        (&json!(218), &json!(65))
    );

    let past = patch(&ada, &item["id"], json!({"current_page": 999})).await;
    assert_eq!(
        (&past["current_page"], &past["progress"]),
        (&json!(334), &json!(100))
    );
    let short = patch(&ada, &item["id"], json!({"current_page": 333})).await;
    assert_eq!(
        short["progress"], 99,
        "a page short of the end is not finished"
    );

    // A page wins over a percent sent with it; a percent alone replaces the page.
    let both = patch(
        &ada,
        &item["id"],
        json!({"current_page": 100, "progress": 10}),
    )
    .await;
    assert_eq!(
        (&both["current_page"], &both["progress"]),
        (&json!(100), &json!(30))
    );
    let percent = patch(&ada, &item["id"], json!({"progress": 40})).await;
    assert_eq!(
        (&percent["current_page"], &percent["progress"]),
        (&Value::Null, &json!(40))
    );
    let again = patch(&ada, &item["id"], json!({"current_page": 150})).await;
    let cleared = patch(&ada, &again["id"], json!({"current_page": null})).await;
    assert_eq!(
        (&cleared["current_page"], &cleared["progress"]),
        (&Value::Null, &json!(45))
    );

    // Leaving the book clears the page with the percent (the CHECK holds it to reading).
    patch(&ada, &item["id"], json!({"current_page": 150})).await;
    let finished = patch(&ada, &item["id"], json!({"status": "finished"})).await;
    assert_eq!(
        (&finished["current_page"], &finished["progress"]),
        (&Value::Null, &Value::Null)
    );
    // A page sent with another status is dropped, like progress.
    let wants = patch(
        &ada,
        &item["id"],
        json!({"status": "want_to_read", "current_page": 9}),
    )
    .await;
    assert_eq!(wants["current_page"], Value::Null);
    // Back to reading with a page in one PATCH.
    let back = patch(
        &ada,
        &item["id"],
        json!({"status": "currently_reading", "current_page": 167}),
    )
    .await;
    assert_eq!(
        (&back["current_page"], &back["progress"]),
        (&json!(167), &json!(50))
    );
}

/// Without a page count the percent holds the position: a page is dropped and the percent
/// stays. The edition's page count wins over the work's; another edition drops the page.
#[sqlx::test]
async fn the_page_counts_against_the_edition_else_the_work(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    let edition = app.edition(&circe, "9780316556347").await;
    set_pages(&app, "works", circe.id, None).await;
    set_pages(&app, "editions", edition.id, None).await;
    let item = add(
        &ada,
        &circe,
        json!({"status": "currently_reading", "progress": 30}),
    )
    .await;

    let unknown = patch(&ada, &item["id"], json!({"current_page": 50})).await;
    assert_eq!(
        (&unknown["current_page"], &unknown["progress"]),
        (&Value::Null, &json!(30))
    );
    let sent = patch(
        &ada,
        &item["id"],
        json!({"current_page": 50, "progress": 35}),
    )
    .await;
    assert_eq!(
        (&sent["current_page"], &sent["progress"]),
        (&Value::Null, &json!(35))
    );

    set_pages(&app, "works", circe.id, Some(400)).await;
    set_pages(&app, "editions", edition.id, Some(200)).await;
    let work = patch(&ada, &item["id"], json!({"current_page": 100})).await;
    assert_eq!(
        work["progress"], 25,
        "the work's 400 pages without an edition"
    );
    let both = patch(
        &ada,
        &item["id"],
        json!({"edition_id": edition.id, "current_page": 100}),
    )
    .await;
    assert_eq!(both["book"]["pages"], 200);
    assert_eq!(
        (&both["current_page"], &both["progress"]),
        (&json!(100), &json!(50))
    );
    // Another edition: a page of one is no page of the other, the percent stays.
    let other = patch(&ada, &item["id"], json!({"edition_id": null})).await;
    assert_eq!(
        (&other["current_page"], &other["progress"]),
        (&Value::Null, &json!(50))
    );

    // The book page and the club's readers carry the page and the count it is in.
    patch(
        &ada,
        &item["id"],
        json!({"edition_id": edition.id, "current_page": 40}),
    )
    .await;
    let detail = ada.get(&format!("/api/books/works/{}", circe.id)).await;
    detail.assert_status(200);
    let detail = detail.json();
    assert_eq!(
        (
            &detail["pages"],
            &detail["progress_pages"],
            &detail["current_page"],
            &detail["progress"]
        ),
        (&json!(400), &json!(200), &json!(40), &json!(20))
    );
    let club = app.club(&ada, "Readers").await;
    ada.put(
        &format!("/api/clubs/{}/pick", club.slug),
        json!({"work_id": circe.id}),
    )
    .await
    .assert_status(200);
    let pick = ada.get(&format!("/api/clubs/{}/pick", club.slug)).await;
    pick.assert_status(200);
    let me = &pick.json()["pick"]["readers"][0];
    assert_eq!(
        (&me["current_page"], &me["progress_pages"], &me["progress"]),
        (&json!(40), &json!(200), &json!(20))
    );
}

/// An Undo of a removal adds the entry again with the page it had: the page comes back as it
/// was, not as a page rounded from the percent; without a page count it is dropped.
#[sqlx::test]
async fn an_add_keeps_the_page_it_brings(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    set_pages(&app, "works", circe.id, Some(334)).await;
    let back = add(
        &ada,
        &circe,
        json!({"status": "currently_reading", "progress": 62, "current_page": 208}),
    )
    .await;
    assert_eq!(
        (&back["current_page"], &back["progress"]),
        (&json!(208), &json!(62))
    );
    ada.delete(&format!("/api/shelf/{}", back["id"]))
        .await
        .assert_status(204);
    // The page wins over a percent sent with it, and is clamped to the page count.
    let past = add(
        &ada,
        &circe,
        json!({"status": "currently_reading", "progress": 10, "current_page": 999}),
    )
    .await;
    assert_eq!(
        (&past["current_page"], &past["progress"]),
        (&json!(334), &json!(100))
    );
    ada.delete(&format!("/api/shelf/{}", past["id"]))
        .await
        .assert_status(204);
    // Kept only while reading, like progress.
    let wants = add(
        &ada,
        &circe,
        json!({"status": "want_to_read", "current_page": 9}),
    )
    .await;
    assert_eq!(
        (&wants["current_page"], &wants["progress"]),
        (&Value::Null, &Value::Null)
    );
    ada.delete(&format!("/api/shelf/{}", wants["id"]))
        .await
        .assert_status(204);
    ada.post(
        "/api/shelf",
        json!({"work_id": circe.id, "current_page": -1}),
    )
    .await
    .assert_status(400);
    set_pages(&app, "works", circe.id, None).await;
    let unknown = add(
        &ada,
        &circe,
        json!({"status": "currently_reading", "progress": 30, "current_page": 50}),
    )
    .await;
    assert_eq!(
        (&unknown["current_page"], &unknown["progress"]),
        (&Value::Null, &json!(30))
    );
}

/// A page is validated like progress, and alone it is something to update.
#[sqlx::test]
async fn invalid_page(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let item = add(
        &ada,
        &circe(&app).await,
        json!({"status": "currently_reading"}),
    )
    .await;
    let path = format!("/api/shelf/{}", item["id"]);
    ada.patch(&path, json!({"current_page": -1}))
        .await
        .assert_status(400);
    ada.patch(&path, json!({"current_page": 100_001}))
        .await
        .assert_status(400);
    ada.patch(&path, json!({"current_page": "12"}))
        .await
        .assert_status(400);
    patch(&ada, &item["id"], json!({"current_page": 0})).await;
}

/// A page records a `progress` event only when it moves the percent: one page more that stays
/// at 62 % tells the feeds nothing new.
#[sqlx::test]
async fn a_page_records_progress_only_when_the_percent_moves(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    set_pages(&app, "works", circe.id, Some(334)).await;
    let item = add(&ada, &circe, json!({"status": "currently_reading"})).await;
    patch(&ada, &item["id"], json!({"current_page": 207})).await;
    patch(&ada, &item["id"], json!({"current_page": 208})).await;
    patch(&ada, &item["id"], json!({"current_page": 218})).await;
    let progress: Vec<Value> = events(&app, &ada)
        .await
        .into_iter()
        .filter(|(kind, _)| kind == "progress")
        .map(|(_, payload)| payload)
        .collect();
    assert_eq!(progress, [json!({"progress": 62}), json!({"progress": 65})]);
}

/// The `restore` part of an Undo: the removed entry's dates and places from its `ShelfItemOut`.
fn restore_of(item: &Value) -> Value {
    json!({
        "started_at": item["started_at"],
        "finished_at": item["finished_at"],
        "updated_at": item["updated_at"],
        "position": item["position"],
        "shelf_position": item["shelf_position"],
    })
}

/// The add an Undo sends for a removed item.
fn undo_body(item: &Value) -> Value {
    json!({
        "work_id": item["book"]["id"],
        "edition_id": item["edition"]["id"],
        "status": item["status"],
        "rating": item["rating"],
        "rating_visibility": item["rating_visibility"],
        "take": item["take"],
        "dnf_reason": item["dnf_reason"],
        "progress": item["progress"],
        "current_page": item["current_page"],
        "restore": restore_of(item),
    })
}

/// Removes the entry; asserts 204.
async fn remove(client: &TestClient, item: &Value) {
    client
        .delete(&format!("/api/shelf/{}", item["id"]))
        .await
        .assert_status(204);
}

/// Moves the client's newest `shelf_removed` event `minutes` into the past.
async fn age_last_removal(app: &TestApp, user: &TestClient, minutes: i32) {
    sqlx::query(
        "UPDATE events SET created_at = created_at - make_interval(mins => $2)
         WHERE id = (SELECT max(id) FROM events WHERE actor_id = $1 AND kind = 'shelf_removed')",
    )
    .bind(user.id())
    .bind(minutes)
    .execute(&app.db)
    .await
    .unwrap();
}

fn instant(value: &Value) -> time::OffsetDateTime {
    time::OffsetDateTime::parse(
        value.as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap()
}

/// The Undo of a removal brings the entry back as it was: its dates, its last change, its place
/// in its column and in the free order; the removal's event is gone and no new one is
/// recorded, so the feeds read as if the book had never left. Only the id is new.
#[sqlx::test]
async fn an_undo_brings_the_entry_back_as_it_was(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let mut ids = Vec::new();
    for n in 1..=4 {
        let work = app
            .ol_work(&format!("OL{n}W"), &format!("Book {n}"), "")
            .await;
        let item = add(&ada, &work, json!({"status": "finished", "rating": n})).await;
        ids.push(item["id"].clone());
    }
    // Column (finished): [4, 3, 2, 1]; the free order is arranged by hand: [1, 3, 4, 2].
    patch(&ada, &ids[2], json!({"finished_on": "2025-04-12"})).await;
    patch(&ada, &ids[0], json!({"shelf_position": 0})).await;
    patch(&ada, &ids[2], json!({"shelf_position": 1})).await;
    // Added after the arranging: unarranged, so it sorts first in the free order.
    let want = add(&ada, &app.ol_work("OL9W", "Circe", "").await, json!({})).await;
    // Took a while: the last change is older than the removal.
    sqlx::query(
        "UPDATE shelf_entries SET updated_at = '2025-05-01T08:00:00Z',
                                  started_at = '2025-03-01T08:00:00Z'
         WHERE id = $1",
    )
    .bind(ids[2].as_i64().unwrap())
    .execute(&app.db)
    .await
    .unwrap();
    let before = items(&ada, None).await;
    let events_before = events(&app, &ada).await;
    let removed = before
        .iter()
        .find(|row| row["id"] == ids[2])
        .unwrap()
        .clone();
    assert_eq!(removed["position"], 1);
    assert_eq!(removed["shelf_position"], 1);

    remove(&ada, &removed).await;
    let res = ada.post("/api/shelf", undo_body(&removed)).await;
    res.assert_status(201);
    let back = res.json();
    let mut expected = removed.clone();
    expected["id"] = back["id"].clone();
    // The free order is renumbered densely; its order is what counts (below).
    expected["shelf_position"] = back["shelf_position"].clone();
    assert_eq!(back, expected);

    let after = items(&ada, None).await;
    let same_ids = |rows: &[Value]| -> Vec<Value> {
        rows.iter()
            .map(|row| {
                if row["id"] == back["id"] {
                    removed["id"].clone()
                } else {
                    row["id"].clone()
                }
            })
            .collect()
    };
    assert_eq!(same_ids(&after), same_ids(&before), "the column order");
    let free = |rows: &[Value]| -> Vec<Value> {
        let ordered: Vec<Value> = by_shelf_position(rows).into_iter().cloned().collect();
        same_ids(
            &ordered
                .iter()
                .map(|id| json!({"id": id}))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(free(&after), free(&before), "the free order");
    assert_eq!(
        free(&before)[0],
        want["id"],
        "an unarranged entry sorts first"
    );
    // The same events: the removal's seal on the finish is lifted with it, since the entry is
    // audience again (`withdraw_removal`).
    assert_eq!(events(&app, &ada).await, events_before);
}

/// The Undo of a removal gives the friends and club mates back the stars and take of an
/// audience finish, which the removal had sealed. It lifts only the seals of that removal: one
/// from an only-me time stays, and an Undo that comes back only-me keeps them all. A plain add
/// again, or a restore after the window, unseals nothing.
#[sqlx::test]
async fn an_undo_unseals_only_what_its_removal_sealed(db: PgPool) {
    let app = TestApp::new(db).await;
    let (ada, grace) = friends(&app).await;
    let circe = circe(&app).await;
    let item = add(
        &ada,
        &circe,
        json!({"status": "finished", "rating": 5, "take": "Loved it"}),
    )
    .await;
    let shared = json!({"rating": 5, "take": "Loved it"});
    assert_eq!(feed_finishes(&grace).await, std::slice::from_ref(&shared));

    remove(&ada, &item).await;
    assert_eq!(feed_finishes(&grace).await, [json!({})]);
    let removal = ada.get("/api/activity").await.json()["items"][0].clone();
    assert_eq!(removal["kind"], "shelf_removed");
    assert_eq!(removal["payload"], json!({"status": "finished"}), "no ids");
    let item = add(&ada, &circe, undo_body(&item)).await;
    assert_eq!(
        finish_payloads(&app, &ada).await,
        std::slice::from_ref(&shared)
    );
    assert_eq!(feed_finishes(&grace).await, std::slice::from_ref(&shared));

    // Back only-me: the seal stays, and so it does once the entry is shared again.
    remove(&ada, &item).await;
    let mut body = undo_body(&item);
    body["rating_visibility"] = json!("only_me");
    let item = add(&ada, &circe, body).await;
    let sealed = json!({"rating": 5, "take": "Loved it", "private": true});
    assert_eq!(
        finish_payloads(&app, &ada).await,
        std::slice::from_ref(&sealed)
    );
    patch(&ada, &item["id"], json!({"rating_visibility": "audience"})).await;
    assert_eq!(feed_finishes(&grace).await, [json!({})]);

    // That only-me seal is older than the next removal, so its Undo leaves it.
    let item = items(&ada, None).await[0].clone();
    remove(&ada, &item).await;
    add(&ada, &circe, undo_body(&item)).await;
    assert_eq!(
        finish_payloads(&app, &ada).await,
        std::slice::from_ref(&sealed)
    );
    assert_eq!(feed_finishes(&grace).await, [json!({})]);

    // A second, audience finish: a removal past the window, or a plain add, keeps its seal.
    let song = app.ol_work("OL2W", "Song", "").await;
    let item = add(&ada, &song, json!({"status": "finished", "rating": 4})).await;
    remove(&ada, &item).await;
    age_last_removal(&app, &ada, 11).await;
    let item = add(&ada, &song, undo_body(&item)).await;
    let song_sealed = json!({"rating": 4, "private": true});
    assert_eq!(finish_payloads(&app, &ada).await[1], song_sealed);
    remove(&ada, &item).await;
    add(&ada, &song, json!({"status": "finished", "rating": 4})).await;
    assert_eq!(finish_payloads(&app, &ada).await[1], song_sealed);
}

/// A restore keeps only the dates its status keeps, none later than now and no start after the
/// finish; the last change is not later than now either, and the column place is clamped.
#[sqlx::test]
async fn a_restore_keeps_the_dates_its_status_keeps_and_none_ahead(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let past = "2025-03-01T08:00:00Z";
    let earlier = "2025-01-01T08:00:00Z";
    let future = "2999-01-01T00:00:00Z";
    let both = json!({"started_at": past, "finished_at": past, "updated_at": earlier});
    let mut works = Vec::new();
    for n in 1..=6 {
        works.push(
            app.ol_work(&format!("OL{n}W"), &format!("Book {n}"), "")
                .await,
        );
    }

    let wanted = add(&ada, &works[0], json!({"restore": both})).await;
    assert_eq!(
        (&wanted["started_at"], &wanted["finished_at"]),
        (&json!(null), &json!(null))
    );
    assert_eq!(wanted["updated_at"], earlier);
    for (work, status) in [
        (&works[1], "currently_reading"),
        (&works[2], "did_not_finish"),
    ] {
        let item = add(&ada, work, json!({"status": status, "restore": both})).await;
        assert_eq!(
            (&item["started_at"], &item["finished_at"]),
            (&json!(past), &json!(null))
        );
    }
    let finished = add(
        &ada,
        &works[3],
        json!({"status": "finished", "restore": both}),
    )
    .await;
    assert_eq!(
        (&finished["started_at"], &finished["finished_at"]),
        (&json!(past), &json!(past))
    );

    let start = time::OffsetDateTime::now_utc();
    let ahead = add(
        &ada,
        &works[4],
        json!({
            "status": "finished",
            "restore": {"started_at": future, "finished_at": future, "updated_at": future,
                        "position": 99},
        }),
    )
    .await;
    let end = time::OffsetDateTime::now_utc();
    for field in ["started_at", "finished_at", "updated_at"] {
        let at = instant(&ahead[field]);
        assert!(
            start - time::Duration::seconds(1) <= at && at <= end,
            "{field}: {at}"
        );
    }
    // Two finished entries: 99 is the bottom of the column.
    assert_eq!(ahead["position"], 1);

    let crossed = add(
        &ada,
        &works[5],
        json!({"status": "finished", "restore": {"started_at": past, "finished_at": earlier}}),
    )
    .await;
    assert_eq!(
        (&crossed["started_at"], &crossed["finished_at"]),
        (&json!(earlier), &json!(earlier))
    );
    assert!(
        instant(&crossed["updated_at"]) >= start,
        "no updated_at: now"
    );
    assert_eq!(crossed["position"], 0, "no position: the top");
    assert_eq!(crossed["shelf_position"], Value::Null);
}

/// A restore with a negative place or an unstorable date is a 400, as in the PATCH; nothing is
/// added.
#[sqlx::test]
async fn an_invalid_restore_is_rejected(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    for (restore, detail) in [
        (
            json!({"position": -1}),
            "Input should be greater than or equal to 0",
        ),
        (
            json!({"shelf_position": -1}),
            "Input should be greater than or equal to 0",
        ),
        (
            json!({"finished_at": "0000-01-01T00:00:00+01:00"}),
            "Invalid value",
        ),
    ] {
        ada.post(
            "/api/shelf",
            json!({"work_id": circe.id, "status": "finished", "restore": restore}),
        )
        .await
        .assert_detail(400, detail);
    }
    assert!(items(&ada, None).await.is_empty());
}

/// The removal's event goes only when the restore undoes it: the newest shelf event of the work,
/// a removal of the same status, younger than 10 minutes. Otherwise the add records its event
/// as any add does, and an add without `restore` never touches the removal.
#[sqlx::test]
async fn a_restore_records_events_unless_it_undoes_a_recent_removal(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let circe = circe(&app).await;
    let removed = |status: &str| ("shelf_removed".to_owned(), json!({"status": status}));
    let added = ("shelf_added".to_owned(), json!({"status": "want_to_read"}));

    // Restored without a removal before it: a new entry's event.
    let item = add(&ada, &circe, json!({"restore": {"position": 0}})).await;
    assert_eq!(events(&app, &ada).await, std::slice::from_ref(&added));

    // A removal 11 minutes ago stays, and the restore records its event.
    remove(&ada, &item).await;
    age_last_removal(&app, &ada, 11).await;
    let item = add(&ada, &circe, undo_body(&item)).await;
    assert_eq!(
        events(&app, &ada).await,
        [added.clone(), removed("want_to_read"), added.clone()]
    );

    // A plain add after a removal leaves the removal and records its event.
    remove(&ada, &item).await;
    let item = add(&ada, &circe, json!({})).await;
    assert_eq!(events(&app, &ada).await.len(), 5);

    // A restore in another status than the removed one is a change, not an undo.
    remove(&ada, &item).await;
    let mut body = undo_body(&item);
    body["status"] = json!("finished");
    let item = add(&ada, &circe, body).await;
    let recorded = events(&app, &ada).await;
    assert_eq!(recorded.len(), 7);
    assert_eq!(recorded[5], removed("want_to_read"));
    assert_eq!(recorded[6], ("shelf_finished".to_owned(), json!({})));

    // A recent removal of the same status is undone: gone, and nothing new.
    remove(&ada, &item).await;
    add(&ada, &circe, undo_body(&item)).await;
    assert_eq!(events(&app, &ada).await, recorded);
}
