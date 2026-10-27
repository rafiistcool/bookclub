//! The physical shelf layout: saving, validation, pruning and who may see or change it
//! (endpoint-map §7, docs/phase1/spec/shelf.md §1.4, §1.5; #44).

mod common;

use common::*;
use serde_json::{Map, Value, json};
use sqlx::PgPool;

const PATH: &str = "/api/shelf/physical-layout";

fn default_layout() -> Value {
    json!({"rows": [[], [], []], "material": "oak", "spines": {}, "decorations": {}})
}

/// A layout as the API returns it: `lights` defaults to false and every spine has a `display`.
fn out(layout: Value) -> Value {
    let mut result = json!({"lights": false});
    let map = result.as_object_mut().unwrap();
    for (key, value) in layout.as_object().unwrap() {
        map.insert(key.clone(), value.clone());
    }
    let spines: Map<String, Value> = layout
        .get("spines")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|(key, spine)| {
            let mut full = json!({"display": "spine"});
            full.as_object_mut()
                .unwrap()
                .extend(spine.as_object().unwrap().clone());
            (key, full)
        })
        .collect();
    map.insert("spines".into(), Value::Object(spines));
    result
}

/// Shelves a new work `OL<n>W`; asserts 201.
async fn add(app: &TestApp, client: &TestClient, n: u32, extra: Value) -> Value {
    let work = match app
        .scalar_i64(&format!(
            "SELECT coalesce(max(id), 0) FROM works WHERE ol_work_id = 'OL{n}W'"
        ))
        .await
    {
        0 => {
            app.ol_work(&format!("OL{n}W"), &format!("Book {n}"), "")
                .await
                .id
        }
        id => id,
    };
    let mut body = json!({ "work_id": work });
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    let res = client.post("/api/shelf", body).await;
    res.assert_status(201);
    res.json()
}

async fn layout(client: &TestClient) -> Value {
    let res = client.get(PATH).await;
    res.assert_status(200);
    res.json()
}

async fn stored(app: &TestApp, user: &TestClient) -> Option<Value> {
    sqlx::query_scalar("SELECT layout FROM physical_shelf_layouts WHERE user_id = $1")
        .bind(user.id())
        .fetch_optional(&app.db)
        .await
        .unwrap()
}

/// parity: test_physical_shelf.py::test_layout_requires_authentication
#[sqlx::test]
async fn layout_requires_authentication(db: PgPool) {
    let app = TestApp::new(db).await;
    app.anon()
        .get(PATH)
        .await
        .assert_detail(401, "Not signed in");
    app.anon()
        .put(PATH, default_layout())
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_physical_shelf.py::test_layout_persists_on_reload_and_restart_without_changing_reading_data
#[sqlx::test]
async fn layout_persists_on_reload_and_restart_without_changing_reading_data(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(
        &app,
        &ada,
        1,
        json!({"status": "currently_reading", "progress": 30}),
    )
    .await;
    let second = add(
        &app,
        &ada,
        2,
        json!({"status": "finished", "rating": 5, "take": "A favourite"}),
    )
    .await;
    let before = ada.get("/api/shelf").await.json()["items"].clone();
    assert_eq!(layout(&ada).await, out(default_layout()));
    assert_eq!(stored(&app, &ada).await, None);

    let (first_id, second_id) = (first["id"].clone(), second["id"].clone());
    let saved_layout = json!({
        "rows": [["decor:plant", second_id, "decor:vase"], [], [first_id, "decor:frame"], ["decor:candle"]],
        "material": "walnut",
        "spines": {first_id.to_string(): {"color": "#Ab3C45", "width": 37}},
        "decorations": {
            "decor:plant": {"kind": "plant", "color": "#AbCdEf"},
            "decor:vase": {"kind": "vase", "color": "#123456"},
            "decor:frame": {"kind": "frame", "color": "#654321"},
            "decor:candle": {"kind": "candle", "color": "#ffeedd"}
        }
    });
    let mut expected = saved_layout.clone();
    expected["spines"] = json!({first_id.to_string(): {"color": "#ab3c45", "width": 37}});
    expected["decorations"]["decor:plant"] = json!({"kind": "plant", "color": "#abcdef"});
    let saved = ada.put(PATH, saved_layout).await;
    saved.assert_status(200);
    assert_eq!(saved.json(), out(expected.clone()));
    assert_eq!(layout(&ada).await, out(expected.clone()));
    assert_eq!(ada.get("/api/shelf").await.json()["items"], before);

    // A new app instance on the same database: storage is durable.
    let restarted = TestApp::new(app.db.clone()).await;
    let again = restarted.anon();
    again
        .post(
            "/api/auth/login",
            json!({"username": "ada", "password": PASSWORD}),
        )
        .await
        .assert_status(204);
    assert_eq!(layout(&again).await, out(expected));
    let replacement = json!({"rows": [[], [first_id, second_id]], "material": "white", "spines": {}, "decorations": {}});
    let res = again.put(PATH, replacement.clone()).await;
    assert_eq!(res.json(), out(replacement.clone()));
    assert_eq!(layout(&ada).await, out(replacement.clone()));
    assert_eq!(stored(&app, &ada).await, Some(out(replacement)));
}

/// parity: test_physical_shelf.py::test_members_can_view_a_layout_but_only_its_owner_can_change_it
#[sqlx::test]
async fn members_can_view_a_layout_but_only_its_owner_can_change_it(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let club = app.club(&ada, "Readers").await;
    app.join(&club, &grace, "member").await;
    let ada_book = add(&app, &ada, 1, json!({})).await;
    let ada_layout = json!({
        "rows": [[ada_book["id"], "decor:plant"], []],
        "material": "walnut",
        "spines": {},
        "decorations": {"decor:plant": {"kind": "plant", "color": "#123456"}}
    });
    ada.put(PATH, ada_layout.clone()).await.assert_status(200);

    // The shared book has a different shelf entry id for each member.
    let grace_book = add(&app, &grace, 1, json!({})).await;
    assert_ne!(grace_book["id"], ada_book["id"]);
    assert_eq!(grace_book["book"]["id"], ada_book["book"]["id"]);
    // Grace sees Ada's shelf, and looking at it leaves her own untouched.
    for name in ["ada", "ADA"] {
        let res = grace.get(&format!("{PATH}?username={name}")).await;
        res.assert_status(200);
        assert_eq!(res.json(), out(ada_layout.clone()));
    }
    grace
        .get(&format!("{PATH}?username=nobody"))
        .await
        .assert_detail(404, "No member with that name");
    assert_eq!(layout(&grace).await, out(default_layout()));
    grace
        .put(PATH, ada_layout.clone())
        .await
        .assert_detail(400, "Physical shelf books must be on your shelf");
    grace
        .put(PATH, json!({"rows": [[ada_book["id"]]]}))
        .await
        .assert_detail(400, "Physical shelf books must be on your shelf");
    grace
        .put(
            PATH,
            json!({"spines": {ada_book["id"].to_string(): {"color": "#224466", "width": 30}}}),
        )
        .await
        .assert_detail(400, "Physical shelf books must be on your shelf");
    // Saving with Ada's name is refused like an unknown name.
    grace
        .put(&format!("{PATH}?username=ada"), json!({}))
        .await
        .assert_detail(404, "No member with that name");
    let grace_layout = json!({
        "rows": [[], ["decor:plant", grace_book["id"]]],
        "material": "white",
        "spines": {},
        "decorations": {"decor:plant": {"kind": "vase", "color": "#abcdef"}}
    });
    grace
        .put(&format!("{PATH}?username=Grace"), grace_layout.clone())
        .await
        .assert_status(200);
    assert_eq!(layout(&ada).await, out(ada_layout));
    assert_eq!(layout(&grace).await, out(grace_layout));
}

/// parity: test_physical_shelf.py::test_status_changes_do_not_move_books_and_deleted_entries_are_pruned
#[sqlx::test]
async fn status_changes_do_not_move_books_and_deleted_entries_are_pruned(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(&app, &ada, 1, json!({})).await;
    let second = add(&app, &ada, 2, json!({})).await;
    let (first_id, second_id) = (first["id"].clone(), second["id"].clone());
    let saved = json!({
        "rows": [["decor:plant"], [first_id, "decor:frame", second_id], []],
        "material": "oak",
        "spines": {
            first_id.to_string(): {"color": "#123456", "width": 26},
            second_id.to_string(): {"color": "#abcdef", "width": 64}
        },
        "decorations": {
            "decor:plant": {"kind": "plant", "color": "#123456"},
            "decor:frame": {"kind": "frame", "color": "#abcdef"}
        }
    });
    ada.put(PATH, saved.clone()).await.assert_status(200);
    ada.patch(
        &format!("/api/shelf/{first_id}"),
        json!({"status": "finished"}),
    )
    .await
    .assert_status(200);
    assert_eq!(layout(&ada).await, out(saved.clone()));
    let newest = add(&app, &ada, 3, json!({})).await;
    let placed: Vec<Value> = layout(&ada).await["rows"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row.as_array().unwrap().clone())
        .collect();
    assert!(!placed.contains(&newest["id"]));
    ada.delete(&format!("/api/shelf/{first_id}"))
        .await
        .assert_status(204);
    let mut expected = saved.clone();
    expected["rows"] = json!([["decor:plant"], ["decor:frame", second_id], []]);
    expected["spines"] = json!({second_id.to_string(): saved["spines"][second_id.to_string()]});
    assert_eq!(layout(&ada).await, out(expected));
}

/// parity: test_physical_shelf.py::test_deleted_spine_does_not_leak_to_a_book_with_a_reused_id
/// Postgres never reuses an id, so the stored row is checked directly, and a new entry is
/// inserted with the old id by hand to show nothing of the old one attaches to it.
#[sqlx::test]
async fn deleted_spine_does_not_leak_to_a_book_with_a_reused_id(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(&app, &ada, 1, json!({})).await;
    let id = first["id"].clone();
    ada.put(
        PATH,
        json!({"rows": [[], [id]], "spines": {id.to_string(): {"color": "#abcdef", "width": 44}}}),
    )
    .await
    .assert_status(200);
    ada.delete(&format!("/api/shelf/{id}"))
        .await
        .assert_status(204);
    // No GET in between: the delete itself cleaned the stored layout.
    let raw = stored(&app, &ada).await.unwrap();
    assert_eq!(raw["rows"], json!([[], []]));
    assert_eq!(raw["spines"], json!({}));

    app.exec(&format!(
        "INSERT INTO shelf_entries (id, user_id, work_id, status) VALUES ({id}, {}, {}, 'want_to_read')",
        ada.id(),
        first["book"]["id"]
    ))
    .await;
    assert_eq!(
        layout(&ada).await,
        out(json!({"rows": [[], []], "material": "oak", "spines": {}, "decorations": {}}))
    );
}

/// parity: test_physical_shelf.py::test_get_prunes_entries_removed_outside_the_shelf_endpoint
#[sqlx::test]
async fn get_prunes_entries_removed_outside_the_shelf_endpoint(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(&app, &ada, 1, json!({})).await;
    let id = first["id"].clone();
    ada.put(
        PATH,
        json!({
            "rows": [["decor:plant", id, "decor:vase"], [], []],
            "spines": {id.to_string(): {"color": "#abcdef", "width": 44}},
            "decorations": {
                "decor:plant": {"kind": "plant", "color": "#123456"},
                "decor:vase": {"kind": "vase", "color": "#abcdef"}
            }
        }),
    )
    .await
    .assert_status(200);
    app.exec(&format!("DELETE FROM shelf_entries WHERE id = {id}"))
        .await;
    let mut expected = default_layout();
    expected["rows"] = json!([["decor:plant", "decor:vase"], [], []]);
    expected["decorations"] = json!({
        "decor:plant": {"kind": "plant", "color": "#123456"},
        "decor:vase": {"kind": "vase", "color": "#abcdef"}
    });
    assert_eq!(layout(&ada).await, out(expected));
    // The read does not write the pruned layout back.
    assert_eq!(stored(&app, &ada).await.unwrap()["rows"][0][1], id);
}

/// The 35 invalid layouts of the legacy parametrised test. `{rows: [[9999]]}`, the spine
/// `9999` and the 5000-digit spine stand for entries that are not the caller's.
fn invalid_layouts() -> Vec<Value> {
    let decoration = json!({"kind": "plant", "color": "#123456"});
    let long_id = format!("decor:{}", "a".repeat(65));
    let mut many = Map::new();
    for i in 0..101 {
        many.insert(format!("decor:{i}"), decoration.clone());
    }
    let many_rows: Vec<String> = (0..101).map(|i| format!("decor:{i}")).collect();
    vec![
        json!({"rows": []}),
        json!({"rows": vec![json!([]); 13]}),
        json!({"rows": [[1], [1]]}),
        json!({"rows": [(1..=2001).collect::<Vec<i64>>()]}),
        json!({"rows": [(1..=1001).collect::<Vec<i64>>(), (1002..=2001).collect::<Vec<i64>>()]}),
        json!({"rows": [[0]]}),
        json!({"rows": [[true]]}),
        json!({"rows": [["1"]]}),
        json!({"rows": [[1.5]]}),
        json!({"rows": [[9999]]}),
        json!({"material": "pine"}),
        json!({"spines": {"1": {"color": "red", "width": 30}}}),
        json!({"spines": {"1": {"color": "#abcd", "width": 30}}}),
        json!({"spines": {"1": {"color": "#123456", "width": 25}}}),
        json!({"spines": {"1": {"color": "#123456", "width": 65}}}),
        json!({"spines": {"1": {"color": "#123456", "width": 30.5}}}),
        json!({"spines": {"01": {"color": "#123456", "width": 30}}}),
        json!({"spines": {"foo": {"color": "#123456", "width": 30}}}),
        json!({"spines": {"9999": {"color": "#123456", "width": 30}}}),
        json!({"spines": {"9".repeat(5000): {"color": "#123456", "width": 30}}}),
        json!({"rows": [["decor:missing"]]}),
        json!({"decorations": {"decor:orphan": decoration}}),
        json!({"rows": [["decor:plant"], ["decor:plant"]], "decorations": {"decor:plant": decoration}}),
        json!({"rows": [["decor:plant"]], "decorations": {"decor:other": decoration}}),
        json!({"rows": [["decor:"]], "decorations": {"decor:": decoration}}),
        json!({"rows": [["plant"]], "decorations": {"plant": decoration}}),
        json!({"rows": [["decor:space name"]], "decorations": {"decor:space name": decoration}}),
        json!({"rows": [[long_id]], "decorations": {long_id: decoration}}),
        json!({"rows": [["decor:plant"]], "decorations": {"decor:plant": {"kind": "unknown", "color": "#123456"}}}),
        json!({"rows": [["decor:plant"]], "decorations": {"decor:plant": {"kind": "plant", "color": "red"}}}),
        json!({"rows": [["decor:plant"]], "decorations": {"decor:plant": {"kind": "plant", "color": "#abcd"}}}),
        json!({"rows": [["decor:plant"]], "decorations": {"decor:plant": {"kind": "plant"}}}),
        json!({"rows": [["decor:plant"]], "decorations": {"decor:plant": {"color": "#123456"}}}),
        json!({"decorations": []}),
        json!({"rows": [many_rows], "decorations": many}),
    ]
}

/// parity: test_physical_shelf.py::test_invalid_layout_does_not_replace_previous_layout
/// All 35 parametrised cases.
#[sqlx::test]
async fn invalid_layout_does_not_replace_previous_layout(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(&app, &ada, 1, json!({})).await;
    let saved =
        json!({"rows": [[first["id"]]], "material": "walnut", "spines": {}, "decorations": {}});
    ada.put(PATH, saved.clone()).await.assert_status(200);
    let cases = invalid_layouts();
    assert_eq!(cases.len(), 35);
    for (n, invalid) in cases.into_iter().enumerate() {
        let rejected = ada.put(PATH, invalid).await;
        assert_eq!(
            rejected.status.as_u16(),
            400,
            "case {}: {}",
            n + 1,
            rejected.text()
        );
        assert_eq!(layout(&ada).await, out(saved.clone()), "case {}", n + 1);
    }
}

/// The verbatim messages of the custom validators and the ownership check.
#[sqlx::test]
async fn invalid_layouts_name_the_rule_they_break(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(&app, &ada, 1, json!({})).await;
    let decoration = json!({"kind": "plant", "color": "#123456"});
    for (body, detail) in [
        (
            json!({"rows": [[first["id"]], [first["id"]]]}),
            "Each item can only appear once on the physical shelf",
        ),
        (
            json!({"rows": [(1..=2001).collect::<Vec<i64>>()]}),
            "At most 2000 books fit on the physical shelf",
        ),
        (
            json!({"spines": {"01": {}}}),
            "Spine appearances must refer to shelf entry IDs",
        ),
        (
            json!({"decorations": {"decor:orphan": decoration}}),
            "Each decoration must appear exactly once on the physical shelf",
        ),
        (
            json!({"rows": [[9999]]}),
            "Physical shelf books must be on your shelf",
        ),
        (
            json!({"spines": {"9".repeat(19): {}}}),
            "Physical shelf books must be on your shelf",
        ),
        (
            json!({"rows": []}),
            "List should have at least 1 item after validation, not 0",
        ),
        (
            json!({"rows": vec![json!([]); 13]}),
            "List should have at most 12 items after validation, not 13",
        ),
        (json!({"rows": null}), "Input should be a valid list"),
    ] {
        ada.put(PATH, body).await.assert_detail(400, detail);
    }
}

/// parity: test_physical_shelf.py::test_legacy_stored_layout_still_loads_and_prunes_missing_books
#[sqlx::test]
async fn legacy_stored_layout_still_loads_and_prunes_missing_books(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(&app, &ada, 1, json!({})).await;
    let id = first["id"].clone();
    let legacy = json!({
        "rows": [[], [id, 9999], []],
        "material": "walnut",
        "spines": {
            id.to_string(): {"color": "#abcdef", "width": 44},
            "9999": {"color": "#123456", "width": 26}
        }
    });
    sqlx::query("INSERT INTO physical_shelf_layouts (user_id, layout) VALUES ($1, $2)")
        .bind(ada.id())
        .bind(&legacy)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        layout(&ada).await,
        out(json!({
            "rows": [[], [id], []],
            "material": "walnut",
            "spines": {id.to_string(): legacy["spines"][id.to_string()]},
            "decorations": {}
        }))
    );
    ada.delete(&format!("/api/shelf/{id}"))
        .await
        .assert_status(204);
    let mut expected = default_layout();
    expected["material"] = json!("walnut");
    assert_eq!(layout(&ada).await, out(expected));
}

/// A stored layout the current schema refuses reads as the default instead of failing.
#[sqlx::test]
async fn a_broken_stored_layout_reads_as_the_default(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let first = add(&app, &ada, 1, json!({})).await;
    for broken in [
        json!({"rows": "nonsense"}),
        json!({"rows": [["decor:orphan"]]}),
        json!({"rows": [[1.5]], "material": "pine"}),
    ] {
        app.exec(&format!(
            "DELETE FROM physical_shelf_layouts WHERE user_id = {}",
            ada.id()
        ))
        .await;
        sqlx::query("INSERT INTO physical_shelf_layouts (user_id, layout) VALUES ($1, $2)")
            .bind(ada.id())
            .bind(&broken)
            .execute(&app.db)
            .await
            .unwrap();
        assert_eq!(layout(&ada).await, out(default_layout()), "{broken}");
    }
    // Deleting an entry still cleans such a layout.
    ada.delete(&format!("/api/shelf/{}", first["id"]))
        .await
        .assert_status(204);
}

/// parity: test_physical_shelf.py::test_decorations_can_be_saved_without_books_and_removed
#[sqlx::test]
async fn decorations_can_be_saved_without_books_and_removed(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let decoration_id = format!("decor:Aa_0-{}", "b".repeat(59));
    let mut saved = default_layout();
    saved["rows"] = json!([[decoration_id]]);
    saved["decorations"] = json!({decoration_id.clone(): {"kind": "plant", "color": "#123456"}});
    assert_eq!(
        ada.put(PATH, saved.clone()).await.json(),
        out(saved.clone())
    );
    assert_eq!(layout(&ada).await, out(saved));
    assert_eq!(
        ada.put(PATH, default_layout()).await.json(),
        out(default_layout())
    );
    assert_eq!(layout(&ada).await, out(default_layout()));
}

/// parity: test_physical_shelf.py::test_mixed_layout_limits_books_and_decorations_separately
/// Through the API: the schema half is the unit test in `shelf::layout`.
#[sqlx::test]
async fn mixed_layout_limits_books_and_decorations_separately(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let decorations: Map<String, Value> = (0..100)
        .map(|i| {
            (
                format!("decor:{i}"),
                json!({"kind": "plant", "color": "#123456"}),
            )
        })
        .collect();
    let decoration_row: Vec<String> = decorations.keys().cloned().collect();
    ada.put(
        PATH,
        json!({"rows": [(1..=2001).collect::<Vec<i64>>(), decoration_row], "decorations": decorations}),
    )
    .await
    .assert_detail(400, "At most 2100 items fit on the physical shelf");
    // 2000 books and 100 decorations pass the schema; the books are not Ada's.
    ada.put(
        PATH,
        json!({"rows": [(1..=2000).collect::<Vec<i64>>(), decoration_row], "decorations": decorations}),
    )
    .await
    .assert_detail(400, "Physical shelf books must be on your shelf");
}

/// parity: test_physical_shelf.py::test_shelf_book_exposes_known_page_count
#[sqlx::test]
async fn shelf_book_exposes_known_page_count(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let entry = add(&app, &ada, 1, json!({})).await;
    assert_eq!(entry["book"]["pages"], Value::Null);
    app.exec(&format!(
        "UPDATE works SET pages = 480 WHERE id = {}",
        entry["book"]["id"]
    ))
    .await;
    assert_eq!(
        ada.get("/api/shelf").await.json()["items"][0]["book"]["pages"],
        480
    );
}

/// parity: test_physical_shelf.py::test_spines_can_follow_the_book_and_choose_how_it_stands
#[sqlx::test]
async fn spines_can_follow_the_book_and_choose_how_it_stands(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let ids: Vec<Value> = [
        add(&app, &ada, 1, json!({})).await,
        add(&app, &ada, 2, json!({})).await,
        add(&app, &ada, 3, json!({})).await,
    ]
    .iter()
    .map(|entry| entry["id"].clone())
    .collect();
    let saved = json!({
        "rows": [[ids[0], ids[1], ids[2]]],
        "material": "black",
        "lights": true,
        "spines": {
            // Only the placement is chosen; colour and width keep following the book.
            ids[0].to_string(): {"display": "flat"},
            ids[1].to_string(): {"display": "cover", "color": "#ABCDEF"},
            ids[2].to_string(): {"width": 40}
        },
        "decorations": {}
    });
    let mut expected = saved.clone();
    expected["spines"] = json!({
        ids[0].to_string(): {"display": "flat", "color": null, "width": null},
        ids[1].to_string(): {"display": "cover", "color": "#abcdef", "width": null},
        ids[2].to_string(): {"display": "spine", "color": null, "width": 40}
    });
    assert_eq!(ada.put(PATH, saved).await.json(), expected);
    // Lights and placements survive the pruning pass on read.
    assert_eq!(layout(&ada).await, expected);
}

/// parity: test_physical_shelf.py::test_new_layout_options_are_validated
/// All 3 parametrised cases (with the caller's real entry id on the row).
#[sqlx::test]
async fn new_layout_options_are_validated(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let id = add(&app, &ada, 1, json!({})).await["id"].clone();
    ada.put(PATH, json!({"rows": [[id]]}))
        .await
        .assert_status(200);
    for invalid in [
        json!({"spines": {id.to_string(): {"display": "upside-down"}}}),
        json!({"lights": "sometimes"}),
        json!({"material": "pine"}),
    ] {
        let mut body = invalid.clone();
        body["rows"] = json!([[id]]);
        ada.put(PATH, body).await.assert_status(400);
    }
    // Lax booleans for the lights, as in the legacy model.
    let lit = ada
        .put(PATH, json!({"rows": [[id]], "lights": "yes"}))
        .await;
    assert_eq!(lit.json()["lights"], true);
}

/// parity: test_physical_shelf.py::test_new_decorations_are_accepted
#[sqlx::test]
async fn new_decorations_are_accepted(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let saved = json!({
        "rows": [["decor:bookend", "decor:globe"]],
        "material": "oak",
        "spines": {},
        "decorations": {
            "decor:bookend": {"kind": "bookend", "color": "#363638"},
            "decor:globe": {"kind": "globe", "color": "#33485f"}
        }
    });
    assert_eq!(ada.put(PATH, saved.clone()).await.json(), out(saved));
}
