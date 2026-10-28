//! Favourites: up to three works per user, in order (endpoint-map §2,
//! docs/phase1/spec/shelf.md §1.9–§1.12; #44).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const PATH: &str = "/api/auth/me/favorites";

/// Four works: Circe, Song of Achilles, Galatea, Circe leftover.
async fn works(app: &TestApp) -> [i64; 4] {
    let mut ids = [0; 4];
    for (n, title) in ["Circe", "Song of Achilles", "Galatea", "Circe leftover"]
        .into_iter()
        .enumerate()
    {
        ids[n] = app
            .ol_work(&format!("OL{}W", n + 1), title, "An Author")
            .await
            .id;
    }
    ids
}

fn titles(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|row| row["book"]["title"].as_str().unwrap().to_owned())
        .collect()
}

fn positions(list: &Value) -> Vec<i64> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|row| row["position"].as_i64().unwrap())
        .collect()
}

async fn favorites(client: &TestClient) -> Value {
    let res = client.get(PATH).await;
    res.assert_status(200);
    res.json()["items"].clone()
}

async fn shelf_favorites(client: &TestClient, username: Option<&str>) -> Value {
    let path = match username {
        Some(name) => format!("/api/shelf?username={name}"),
        None => "/api/shelf".to_owned(),
    };
    client.get(&path).await.json()["favorites"].clone()
}

/// parity: test_favorites.py::test_replace_order_clear_and_eligibility
/// The book-detail `favorite_position` assertions belong to CAT's `BookDetailOut`.
#[sqlx::test]
async fn replace_order_clear_and_eligibility(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let [circe, song, galatea, _] = works(&app).await;

    let empty = ada.get(PATH).await;
    empty.assert_status(200);
    assert_eq!(empty.json(), json!({"items": []}));
    assert_eq!(shelf_favorites(&ada, None).await, json!([]));

    let replaced = ada.put(PATH, json!({"work_ids": [song, circe]})).await;
    replaced.assert_status(200);
    assert_eq!(
        titles(&replaced.json()["items"]),
        ["Song of Achilles", "Circe"]
    );
    assert_eq!(positions(&replaced.json()["items"]), [1, 2]);

    let reordered = ada
        .put(PATH, json!({"work_ids": [circe, galatea, song]}))
        .await;
    reordered.assert_status(200);
    assert_eq!(
        titles(&reordered.json()["items"]),
        ["Circe", "Galatea", "Song of Achilles"]
    );
    assert_eq!(positions(&reordered.json()["items"]), [1, 2, 3]);

    let shelf = shelf_favorites(&ada, None).await;
    assert_eq!(titles(&shelf), ["Circe", "Galatea", "Song of Achilles"]);
    assert!(
        shelf
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["book"].as_object().unwrap().contains_key("cover_url"))
    );
    assert_eq!(shelf[0]["book"]["id"], circe);

    let cleared = ada.put(PATH, json!({"work_ids": []})).await;
    cleared.assert_status(200);
    assert_eq!(cleared.json(), json!({"items": []}));
    assert_eq!(shelf_favorites(&ada, None).await, json!([]));
}

/// parity: test_favorites.py::test_unknown_and_duplicate_book_ids_are_rejected
/// The "not in the club" string is now `That book is not in the catalog` (endpoint-map §2).
#[sqlx::test]
async fn unknown_and_duplicate_work_ids_are_rejected(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let ids = works(&app).await;

    ada.put(PATH, json!({"work_ids": [ids[0], 999999]}))
        .await
        .assert_detail(400, "That book is not in the catalog");
    assert_eq!(favorites(&ada).await, json!([]));
    ada.put(PATH, json!({"work_ids": [ids[0], ids[0]]}))
        .await
        .assert_detail(400, "Each book can only be a favourite once");
    ada.put(PATH, json!({"work_ids": ids}))
        .await
        .assert_detail(400, "At most 3 favourites");
    assert_eq!(favorites(&ada).await, json!([]));
    ada.put(PATH, json!({"work_ids": [ids[0], -1]}))
        .await
        .assert_detail(400, "That book is not in the catalog");
    ada.put(PATH, json!({"work_ids": null}))
        .await
        .assert_detail(400, "Input should be a valid list");
}

/// parity: test_favorites.py::test_favorites_in_length_check_is_the_translatable_validator
/// The legacy test inspected the pydantic error; here the answer itself: the length check comes
/// first, alone, with the app's own (translatable) message and no generic list-length error.
#[sqlx::test]
async fn the_length_check_is_the_translatable_message(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    ada.put(PATH, json!({"work_ids": [1, 2, 3, 4]}))
        .await
        .assert_detail(400, "At most 3 favourites");
    ada.put(PATH, json!({"work_ids": [0, 0, 0, 0, 0]}))
        .await
        .assert_detail(400, "At most 3 favourites");
}

/// parity: test_favorites.py::test_favourite_does_not_require_own_shelf
#[sqlx::test]
async fn favourite_does_not_require_own_shelf(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let [circe, ..] = works(&app).await;
    ada.post("/api/shelf", json!({"work_id": circe}))
        .await
        .assert_status(201);

    let added = grace.post(&format!("{PATH}/{circe}"), json!({})).await;
    added.assert_status(200);
    assert_eq!(titles(&added.json()["items"]), ["Circe"]);
    let mine = grace.get("/api/shelf").await.json();
    assert_eq!(mine["items"], json!([]));
    assert_eq!(mine["favorites"][0]["book"]["title"], "Circe");
    assert_eq!(shelf_favorites(&grace, Some("ada")).await, json!([]));
}

/// parity: test_favorites.py::test_add_helper_appends_and_rejects_a_fourth
#[sqlx::test]
async fn add_appends_and_rejects_a_fourth(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let ids = works(&app).await;

    ada.post(&format!("{PATH}/{}", ids[0]), json!({}))
        .await
        .assert_status(200);
    let again = ada.post(&format!("{PATH}/{}", ids[0]), json!({})).await;
    again.assert_status(200);
    assert_eq!(again.json()["items"].as_array().unwrap().len(), 1);

    ada.post(&format!("{PATH}/{}", ids[1]), json!({}))
        .await
        .assert_status(200);
    ada.post(&format!("{PATH}/{}", ids[2]), json!({}))
        .await
        .assert_status(200);
    ada.post(&format!("{PATH}/{}", ids[3]), json!({}))
        .await
        .assert_detail(409, "You already have 3 favourites");
    // The fourth is refused even when it is already listed elsewhere; a listed one is still 200.
    ada.post(&format!("{PATH}/{}", ids[1]), json!({}))
        .await
        .assert_status(200);
    assert_eq!(
        titles(&favorites(&ada).await),
        ["Circe", "Song of Achilles", "Galatea"]
    );
    for unknown in ["999999", "0", "-1"] {
        ada.post(&format!("{PATH}/{unknown}"), json!({}))
            .await
            .assert_detail(400, "That book is not in the catalog");
    }
    ada.post(&format!("{PATH}/abc"), json!({}))
        .await
        .assert_detail(
            400,
            "Input should be a valid integer, unable to parse string as an integer",
        );
}

/// parity: test_favorites.py::test_remove_helper_compacts_positions
#[sqlx::test]
async fn remove_compacts_positions(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let [circe, song, galatea, _] = works(&app).await;
    ada.put(PATH, json!({"work_ids": [circe, song, galatea]}))
        .await
        .assert_status(200);

    let removed = ada.delete(&format!("{PATH}/{song}")).await;
    removed.assert_status(200);
    assert_eq!(titles(&removed.json()["items"]), ["Circe", "Galatea"]);
    assert_eq!(positions(&removed.json()["items"]), [1, 2]);

    let missing = ada.delete(&format!("{PATH}/{song}")).await;
    missing.assert_status(200);
    assert_eq!(titles(&missing.json()["items"]), ["Circe", "Galatea"]);
    // An id that names no work at all is no favourite either.
    ada.delete(&format!("{PATH}/999999"))
        .await
        .assert_status(200);
}

/// parity: test_favorites.py::test_member_shelf_shows_their_favourites_not_yours
#[sqlx::test]
async fn member_shelf_shows_their_favourites_not_yours(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let [circe, song, ..] = works(&app).await;
    ada.put(PATH, json!({"work_ids": [circe]}))
        .await
        .assert_status(200);
    grace
        .put(PATH, json!({"work_ids": [song]}))
        .await
        .assert_status(200);
    assert_eq!(
        titles(&shelf_favorites(&grace, None).await),
        ["Song of Achilles"]
    );
    assert_eq!(
        titles(&shelf_favorites(&grace, Some("ada")).await),
        ["Circe"]
    );
}

/// parity: test_favorites.py::test_omitted_book_ids_clears_favourites
#[sqlx::test]
async fn omitted_work_ids_clears_favourites(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let [circe, ..] = works(&app).await;
    ada.put(PATH, json!({"work_ids": [circe]}))
        .await
        .assert_status(200);
    let cleared = ada.put(PATH, json!({})).await;
    cleared.assert_status(200);
    assert_eq!(cleared.json(), json!({"items": []}));
}

/// parity: test_favorites.py::test_stale_last_slot_insert_maps_integrity_error_to_409
/// The legacy race (a stale list read) cannot happen under the user lock; ported as the same
/// setup, a third favourite written behind the API's back, and the same answer. A gap left in
/// the positions is closed before the next add (the latent legacy bug).
#[sqlx::test]
async fn a_full_list_written_elsewhere_still_refuses_a_fourth(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let [circe, song, galatea, leftover] = works(&app).await;
    ada.put(PATH, json!({"work_ids": [circe, song]}))
        .await
        .assert_status(200);
    app.exec(&format!(
        "INSERT INTO user_favorites (user_id, work_id, position) VALUES ({}, {galatea}, 3)",
        ada.id()
    ))
    .await;
    ada.post(&format!("{PATH}/{leftover}"), json!({}))
        .await
        .assert_detail(409, "You already have 3 favourites");
    assert_eq!(
        titles(&favorites(&ada).await),
        ["Circe", "Song of Achilles", "Galatea"]
    );

    // A gap (positions 1 and 3) no longer blocks the third slot.
    app.exec(&format!(
        "DELETE FROM user_favorites WHERE user_id = {} AND work_id = {song}",
        ada.id()
    ))
    .await;
    let added = ada.post(&format!("{PATH}/{leftover}"), json!({})).await;
    added.assert_status(200);
    assert_eq!(
        titles(&added.json()["items"]),
        ["Circe", "Galatea", "Circe leftover"]
    );
    assert_eq!(positions(&added.json()["items"]), [1, 2, 3]);
}

/// parity: test_favorites.py::test_favorite_relationships_use_delete_cascade
/// Reasoned change: favourites go with their user (ON DELETE CASCADE), while works are never
/// deleted (merges re-point favourites), so the work reference is RESTRICT.
#[sqlx::test]
async fn favourites_go_with_their_user(db: PgPool) {
    let app = TestApp::new(db).await;
    let rules: Vec<(String, String)> = sqlx::query_as(
        "SELECT a.attname::text, c.confdeltype::text
         FROM pg_constraint c
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
         WHERE c.conrelid = 'user_favorites'::regclass AND c.contype = 'f'
         ORDER BY 1",
    )
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        rules,
        [
            ("user_id".to_owned(), "c".to_owned()),
            ("work_id".to_owned(), "r".to_owned())
        ]
    );
    let ada = app.user("ada").await;
    let [circe, ..] = works(&app).await;
    ada.put(PATH, json!({"work_ids": [circe]}))
        .await
        .assert_status(200);
    app.exec(&format!("DELETE FROM users WHERE id = {}", ada.id()))
        .await;
    assert_eq!(
        app.scalar_i64("SELECT count(*) FROM user_favorites").await,
        0
    );
}

/// parity: test_favorites.py::test_concurrent_add_for_last_slot_is_409_not_500
#[sqlx::test]
async fn concurrent_add_for_last_slot_is_409_not_500(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let [circe, song, galatea, leftover] = works(&app).await;
    ada.put(PATH, json!({"work_ids": [circe, song]}))
        .await
        .assert_status(200);
    let (first, second) = (format!("{PATH}/{galatea}"), format!("{PATH}/{leftover}"));
    let (a, b) = tokio::join!(ada.post(&first, json!({})), ada.post(&second, json!({})));
    let mut statuses = [a.status.as_u16(), b.status.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    assert_eq!(favorites(&ada).await.as_array().unwrap().len(), 3);
}

/// parity: test_favorites.py::test_favorites_require_a_session
#[sqlx::test]
async fn favorites_require_a_session(db: PgPool) {
    let app = TestApp::new(db).await;
    let anon = app.anon();
    anon.get(PATH).await.assert_detail(401, "Not signed in");
    anon.put(PATH, json!({"work_ids": []}))
        .await
        .assert_detail(401, "Not signed in");
    anon.post(&format!("{PATH}/1"), json!({}))
        .await
        .assert_detail(401, "Not signed in");
    anon.delete(&format!("{PATH}/1"))
        .await
        .assert_detail(401, "Not signed in");
}

/// Favourites point at current works: a merged-away id resolves to its target, and two ids
/// that are one work now count as the same favourite.
#[sqlx::test]
async fn merged_works_resolve_to_their_target(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let [circe, song, ..] = works(&app).await;
    app.exec(&format!(
        "UPDATE works SET merged_into_id = {song} WHERE id = {circe}"
    ))
    .await;
    let added = ada.post(&format!("{PATH}/{circe}"), json!({})).await;
    added.assert_status(200);
    assert_eq!(added.json()["items"][0]["book"]["id"], song);
    ada.put(PATH, json!({"work_ids": [circe, song]}))
        .await
        .assert_detail(400, "Each book can only be a favourite once");
    let removed = ada.delete(&format!("{PATH}/{circe}")).await;
    assert_eq!(removed.json(), json!({"items": []}));
}
