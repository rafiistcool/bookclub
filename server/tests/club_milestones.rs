//! Milestones: the reading schedule of a pick (#45, endpoint-map §9,
//! docs/phase1/spec/club.md §1.13–§1.16, §2.7, §5.5).

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// `ada` owns "Readers" with Circe as the current pick; `grace` is a plain member.
struct Fixture {
    app: TestApp,
    ada: TestClient,
    grace: TestClient,
    club: TestClub,
    circe: TestWork,
    achilles: TestWork,
    pick_id: i64,
}

impl Fixture {
    async fn new(db: PgPool) -> Self {
        let app = TestApp::new(db).await;
        let ada = app.user("ada").await;
        let grace = app.user("grace").await;
        let club = app.club(&ada, "Readers").await;
        app.join(&club, &grace, "member").await;
        let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
        let achilles = app
            .ol_work("OL2W", "The Song of Achilles", "Madeline Miller")
            .await;
        let res = ada
            .put(
                &format!("/api/clubs/{}/pick", club.slug),
                json!({"work_id": circe.id}),
            )
            .await;
        res.assert_status(200);
        let pick_id = res.json()["id"].as_i64().unwrap();
        Fixture {
            app,
            ada,
            grace,
            club,
            circe,
            achilles,
            pick_id,
        }
    }

    /// `/api/clubs/{slug}/picks/{pick_id}/milestones`.
    fn url(&self) -> String {
        milestones_url(&self.club, self.pick_id)
    }

    async fn create(&self, body: Value) -> Value {
        let res = self.ada.post(&self.url(), body).await;
        res.assert_status(201);
        res.json()
    }
}

fn milestones_url(club: &TestClub, pick_id: i64) -> String {
    format!("/api/clubs/{}/picks/{pick_id}/milestones", club.slug)
}

/// A club post on the pick, inserted directly (the post routes are POSTS'); `deleted` makes it
/// a tombstone.
async fn club_post(f: &Fixture, milestone_id: Option<i64>, deleted: bool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO posts (work_id, author_id, club_id, pick_id, milestone_id, body, deleted_at)
         VALUES ($1, $2, $3, $4, $5, CASE WHEN $6 THEN '' ELSE 'Part one thoughts' END,
                 CASE WHEN $6 THEN now() END)
         RETURNING id",
    )
    .bind(f.circe.id)
    .bind(f.ada.id())
    .bind(f.club.id)
    .bind(f.pick_id)
    .bind(milestone_id)
    .bind(deleted)
    .fetch_one(&f.app.db)
    .await
    .unwrap()
}

fn titles(list: &Value) -> Vec<String> {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap().to_owned())
        .collect()
}

/// parity: test_stats_milestones_quotes.py::test_milestones_crud_and_threads
///
/// Milestone parts only (the stats are SHELF's, the post routes POSTS'): posts tied to a
/// milestone are inserted directly. PATCH is partial now, so clearing `due_at` takes an
/// explicit `null`; the chapter fields are `unit: "chapter"` with `range_from`/`range_to`.
#[sqlx::test]
async fn milestones_crud_and_threads(db: PgPool) {
    let f = Fixture::new(db).await;
    let url = f.url();
    assert_eq!(
        f.ada.get(&url).await.json(),
        json!({"pick_id": f.pick_id, "items": [], "timezone": "UTC"})
    );

    let first = f
        .create(json!({
            "title": "Part one", "unit": "chapter", "range_from": 1, "range_to": 8,
            "due_at": "2030-01-05T20:00"
        }))
        .await;
    assert_eq!(first["position"], 0);
    assert_eq!(first["pick_id"], f.pick_id);
    assert_eq!(first["title"], "Part one");
    assert_eq!(first["unit"], "chapter");
    assert_eq!(first["range_from"], 1);
    assert_eq!(first["range_to"], 8);
    assert_eq!(first["due_at"], "2030-01-05T20:00:00Z");
    assert_eq!(first["due_local"], "2030-01-05T20:00");
    assert_eq!(first["due_label"], "Sat 05 Jan 2030, 20:00 UTC");
    assert_eq!(first["passed"], false);
    assert_eq!(first["note_count"], 0);

    let second = f
        .create(json!({"title": "Part two", "unit": "chapter", "range_from": 9, "range_to": 16}))
        .await;
    assert_eq!(second["position"], 1);
    assert_eq!(second["due_at"], Value::Null);
    assert_eq!(second["due_local"], Value::Null);
    assert_eq!(second["due_label"], Value::Null);

    f.ada
        .post(
            &url,
            json!({"title": "", "unit": "chapter", "range_from": 1}),
        )
        .await
        .assert_detail(400, "Give the milestone a title");
    f.ada
        .post(
            &url,
            json!({"title": "Backwards", "unit": "chapter", "range_from": 9, "range_to": 2}),
        )
        .await
        .assert_detail(400, "Chapter range is backwards");

    // Posts tied to a milestone are counted.
    let first_id = first["id"].as_i64().unwrap();
    let post = club_post(&f, Some(first_id), false).await;
    let items = f.ada.get(&url).await.json();
    assert_eq!(items["items"][0]["note_count"], 1);
    assert_eq!(items["items"][1]["note_count"], 0);

    let edited = f
        .ada
        .patch(
            &format!("{url}/{first_id}"),
            json!({"title": "Part 1", "range_from": 1, "range_to": 8, "due_at": null}),
        )
        .await;
    edited.assert_status(200);
    assert_eq!(edited.json()["title"], "Part 1");
    assert_eq!(edited.json()["due_at"], Value::Null);
    assert_eq!(edited.json()["position"], 0);

    f.ada
        .delete(&format!("{url}/{first_id}"))
        .await
        .assert_status(204);
    let items = f.ada.get(&url).await.json();
    assert_eq!(titles(&items), ["Part two"]);
    assert_eq!(items["items"][0]["position"], 0);
    // The post survives, detached from the deleted milestone.
    let detached: Option<i64> = sqlx::query_scalar("SELECT milestone_id FROM posts WHERE id = $1")
        .bind(post)
        .fetch_one(&f.app.db)
        .await
        .unwrap();
    assert_eq!(detached, None);

    f.ada
        .get(&milestones_url(&f.club, 987_654))
        .await
        .assert_detail(404, "No club pick with that id");
    f.ada
        .delete(&format!("{url}/987654"))
        .await
        .assert_detail(404, "No milestone with that id");

    f.ada
        .put(
            &format!("/api/clubs/{}/pick", f.club.slug),
            json!({"work_id": f.achilles.id}),
        )
        .await
        .assert_status(200);
    f.ada
        .post(&url, json!({"title": "Too late"}))
        .await
        .assert_detail(400, "That pick is closed");
    let added: Vec<(Value,)> = sqlx::query_as(
        "SELECT payload FROM events WHERE kind = 'milestone_added' AND club_id = $1 ORDER BY id",
    )
    .bind(f.club.id)
    .fetch_all(&f.app.db)
    .await
    .unwrap();
    assert_eq!(
        added,
        [
            (json!({"title": "Part one", "due_at": "2030-01-05T20:00:00Z"}),),
            (json!({"title": "Part two"}),)
        ]
    );
}

/// Percent and chapter ranges, title-only checkpoints, and the range rules on create.
#[sqlx::test]
async fn units_and_ranges_are_checked_on_create(db: PgPool) {
    let f = Fixture::new(db).await;
    let url = f.url();
    let checkpoint = f.create(json!({"title": "  Halfway party  "})).await;
    assert_eq!(checkpoint["title"], "Halfway party");
    assert_eq!(checkpoint["unit"], Value::Null);
    assert_eq!(checkpoint["range_from"], Value::Null);
    assert_eq!(checkpoint["range_to"], Value::Null);

    let percent = f
        .create(json!({"title": "First half", "unit": "percent", "range_to": 50}))
        .await;
    assert_eq!(percent["unit"], "percent");
    assert_eq!(percent["range_from"], Value::Null);
    assert_eq!(percent["range_to"], 50);
    let whole = f
        .create(json!({"title": "All of it", "unit": "percent", "range_from": 0, "range_to": 100}))
        .await;
    assert_eq!(whole["range_to"], 100);
    let chapters = f
        .create(json!({"title": "Long one", "unit": "chapter", "range_from": 101, "range_to": 101}))
        .await;
    assert_eq!(chapters["range_from"], 101);

    for (body, detail) in [
        (
            json!({"title": "Over", "unit": "percent", "range_to": 101}),
            "Percent milestones go up to 100",
        ),
        (
            json!({"title": "Over", "unit": "percent", "range_from": 150}),
            "Percent milestones go up to 100",
        ),
        (
            json!({"title": "Unitless", "range_to": 5}),
            "A range needs a unit",
        ),
        (
            json!({"title": "Unitless", "unit": null, "range_from": 5}),
            "A range needs a unit",
        ),
        (
            json!({"title": "Backwards", "unit": "percent", "range_from": 60, "range_to": 40}),
            "Chapter range is backwards",
        ),
        (
            json!({"title": "Negative", "unit": "chapter", "range_from": -1}),
            "Input should be greater than or equal to 0",
        ),
        (json!({"title": "Pages", "unit": "pages"}), "Invalid value"),
        (json!({"unit": "chapter"}), "Field required"),
        (
            json!({"title": "x".repeat(121)}),
            "Keep the title to 120 characters",
        ),
        (
            json!({"title": "  ", "range_to": 5, "unit": null}),
            "Give the milestone a title; A range needs a unit",
        ),
        (
            json!({"title": "Due", "due_at": "next Tuesday"}),
            "meeting_at must be a date and time",
        ),
    ] {
        f.ada
            .post(&url, body.clone())
            .await
            .assert_detail(400, detail);
    }
    // 120 characters after trimming are fine.
    f.create(json!({"title": format!(" {} ", "ü".repeat(120))}))
        .await;
    assert_eq!(f.app.scalar_i64("SELECT count(*) FROM milestones").await, 5);
}

/// The legacy order of checks on create: validation, the pick (404), closed, backwards, then
/// the due time.
#[sqlx::test]
async fn create_checks_run_in_the_legacy_order(db: PgPool) {
    let f = Fixture::new(db).await;
    let missing = milestones_url(&f.club, 987_654);
    f.ada
        .post(&missing, json!({"title": ""}))
        .await
        .assert_detail(400, "Give the milestone a title");
    f.ada
        .post(&missing, json!({"title": "x", "due_at": "soon"}))
        .await
        .assert_detail(404, "No club pick with that id");
    let backwards_and_bad_due =
        json!({"title": "x", "unit": "chapter", "range_from": 9, "range_to": 1, "due_at": "soon"});
    f.ada
        .post(&f.url(), backwards_and_bad_due.clone())
        .await
        .assert_detail(400, "Chapter range is backwards");
    f.ada
        .put(
            &format!("/api/clubs/{}/pick", f.club.slug),
            json!({"work_id": f.achilles.id}),
        )
        .await
        .assert_status(200);
    f.ada
        .post(&f.url(), backwards_and_bad_due)
        .await
        .assert_detail(400, "That pick is closed");
}

/// PATCH changes only the keys present; the range rules (including "backwards", which the
/// legacy PATCH skipped) apply to the result; `null` clears.
#[sqlx::test]
async fn patch_is_partial_and_checks_the_result(db: PgPool) {
    let f = Fixture::new(db).await;
    let milestone = f
        .create(json!({
            "title": "Part one", "unit": "chapter", "range_from": 1, "range_to": 8,
            "due_at": "2030-01-05T20:00"
        }))
        .await;
    let url = format!("{}/{}", f.url(), milestone["id"]);

    let renamed = f.ada.patch(&url, json!({"title": "  Part 1 "})).await;
    renamed.assert_status(200);
    assert_eq!(renamed.json()["title"], "Part 1");
    assert_eq!(renamed.json()["unit"], "chapter");
    assert_eq!(renamed.json()["range_from"], 1);
    assert_eq!(renamed.json()["range_to"], 8);
    assert_eq!(renamed.json()["due_at"], "2030-01-05T20:00:00Z");

    let same = f.ada.patch(&url, json!({})).await;
    same.assert_status(200);
    assert_eq!(same.json(), renamed.json());

    f.ada
        .patch(&url, json!({"range_from": 9}))
        .await
        .assert_detail(400, "Chapter range is backwards");
    f.ada
        .patch(&url, json!({"unit": "percent", "range_to": 120}))
        .await
        .assert_detail(400, "Percent milestones go up to 100");
    f.ada
        .patch(&url, json!({"unit": null}))
        .await
        .assert_detail(400, "A range needs a unit");
    f.ada
        .patch(&url, json!({"title": null}))
        .await
        .assert_detail(400, "Give the milestone a title");
    f.ada
        .patch(&url, json!({"title": ""}))
        .await
        .assert_detail(400, "Give the milestone a title");
    f.ada
        .patch(&url, json!({"range_to": -3}))
        .await
        .assert_detail(400, "Input should be greater than or equal to 0");
    f.ada
        .patch(&url, json!({"due_at": "soon"}))
        .await
        .assert_detail(400, "meeting_at must be a date and time");

    let percent = f
        .ada
        .patch(
            &url,
            json!({"unit": "percent", "range_from": 50, "range_to": 100}),
        )
        .await;
    percent.assert_status(200);
    assert_eq!(percent.json()["unit"], "percent");
    assert_eq!(percent.json()["range_from"], 50);

    let open_start = f.ada.patch(&url, json!({"range_from": null})).await;
    assert_eq!(open_start.json()["range_from"], Value::Null);
    assert_eq!(open_start.json()["range_to"], 100);

    let checkpoint = f
        .ada
        .patch(&url, json!({"unit": null, "range_to": null, "due_at": ""}))
        .await;
    checkpoint.assert_status(200);
    assert_eq!(checkpoint.json()["unit"], Value::Null);
    assert_eq!(checkpoint.json()["range_to"], Value::Null);
    assert_eq!(checkpoint.json()["due_at"], Value::Null);
    assert_eq!(checkpoint.json()["title"], "Part 1");

    let due = f
        .ada
        .patch(&url, json!({"due_at": "2020-01-01T10:00"}))
        .await;
    assert_eq!(due.json()["due_local"], "2020-01-01T10:00");
    assert_eq!(due.json()["passed"], true);

    // Edits record no event.
    assert_eq!(
        f.app
            .scalar_i64("SELECT count(*) FROM events WHERE kind = 'milestone_added'")
            .await,
        1
    );
}

/// The closed-pick check is create-only (parity): an ended pick's milestones can still be
/// listed, edited and deleted.
#[sqlx::test]
async fn an_ended_picks_milestones_can_still_be_edited_and_deleted(db: PgPool) {
    let f = Fixture::new(db).await;
    let a = f.create(json!({"title": "A"})).await;
    let b = f.create(json!({"title": "B"})).await;
    let c = f.create(json!({"title": "C"})).await;
    f.ada
        .delete(&format!("/api/clubs/{}/pick", f.club.slug))
        .await
        .assert_status(200);
    let url = f.url();
    let edited = f
        .ada
        .patch(&format!("{url}/{}", b["id"]), json!({"title": "B2"}))
        .await;
    edited.assert_status(200);
    f.ada
        .delete(&format!("{url}/{}", a["id"]))
        .await
        .assert_status(204);
    let list = f.grace.get(&url).await.json();
    assert_eq!(titles(&list), ["B2", "C"]);
    let positions: Vec<i64> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["position"].as_i64().unwrap())
        .collect();
    assert_eq!(positions, [0, 1]);
    assert_eq!(list["items"][1]["id"], c["id"]);
}

/// Deleting renumbers the schedule densely in `(position, id)` order and detaches posts.
#[sqlx::test]
async fn delete_renumbers_densely(db: PgPool) {
    let f = Fixture::new(db).await;
    let mut ids = Vec::new();
    for title in ["A", "B", "C", "D"] {
        ids.push(
            f.create(json!({"title": title})).await["id"]
                .as_i64()
                .unwrap(),
        );
    }
    // Two rows share a position (as concurrent legacy inserts could): the id breaks the tie.
    f.app
        .exec(&format!(
            "UPDATE milestones SET position = 1 WHERE id = {}",
            ids[3]
        ))
        .await;
    let url = f.url();
    let tied = f.ada.get(&url).await.json();
    assert_eq!(titles(&tied), ["A", "B", "D", "C"]);
    f.ada
        .delete(&format!("{url}/{}", ids[0]))
        .await
        .assert_status(204);
    let list = f.ada.get(&url).await.json();
    assert_eq!(titles(&list), ["B", "D", "C"]);
    let positions: Vec<i64> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["position"].as_i64().unwrap())
        .collect();
    assert_eq!(positions, [0, 1, 2]);
    // A new milestone goes to the end.
    assert_eq!(f.create(json!({"title": "E"})).await["position"], 3);
}

/// `note_count` counts the live club posts tied to the milestone; a tombstone does not count.
#[sqlx::test]
async fn note_count_counts_live_posts(db: PgPool) {
    let f = Fixture::new(db).await;
    let milestone = f.create(json!({"title": "Part one"})).await;
    let id = milestone["id"].as_i64().unwrap();
    club_post(&f, Some(id), false).await;
    club_post(&f, Some(id), false).await;
    club_post(&f, Some(id), true).await;
    club_post(&f, None, false).await;
    let list = f.grace.get(&f.url()).await.json();
    assert_eq!(list["items"][0]["note_count"], 2);
}

/// Times are given in the club's zone, and a due time in the past counts as passed.
#[sqlx::test]
async fn due_times_use_the_club_zone(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .patch(
            &format!("/api/clubs/{}", f.club.slug),
            json!({"timezone": "America/Sao_Paulo"}),
        )
        .await
        .assert_status(200);
    let due = f
        .create(json!({"title": "Part one", "due_at": "2026-08-22T19:00"}))
        .await;
    assert_eq!(due["due_at"], "2026-08-22T22:00:00Z");
    assert_eq!(due["due_local"], "2026-08-22T19:00");
    assert_eq!(due["due_label"], "Sat 22 Aug 2026, 19:00 -03");
    let past = f
        .create(json!({"title": "Prologue", "due_at": "2001-01-01"}))
        .await;
    assert_eq!(past["passed"], true);
    assert_eq!(
        f.ada.get(&f.url()).await.json()["timezone"],
        "America/Sao_Paulo"
    );
}

/// Ids from another club, or a milestone of another pick, are 404 like missing ones; a user in
/// two clubs sees two separate schedules.
#[sqlx::test]
async fn ids_from_another_club_or_pick_are_not_found(db: PgPool) {
    let f = Fixture::new(db).await;
    let mine = f.create(json!({"title": "Mine"})).await;
    let other = f.app.club(&f.grace, "Other").await;
    f.app.join(&other, &f.ada, "member").await;
    let other_pick = f
        .grace
        .put(
            &format!("/api/clubs/{}/pick", other.slug),
            json!({"work_id": f.circe.id}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let theirs = f
        .grace
        .post(
            &milestones_url(&other, other_pick),
            json!({"title": "Theirs"}),
        )
        .await
        .json();

    // The other club's pick under this club's path.
    let foreign_pick = milestones_url(&f.club, other_pick);
    f.ada
        .get(&foreign_pick)
        .await
        .assert_detail(404, "No club pick with that id");
    f.ada
        .post(&foreign_pick, json!({"title": "Sneaky"}))
        .await
        .assert_detail(404, "No club pick with that id");
    f.ada
        .patch(
            &format!("{foreign_pick}/{}", theirs["id"]),
            json!({"title": "x"}),
        )
        .await
        .assert_detail(404, "No club pick with that id");
    f.ada
        .delete(&format!("{foreign_pick}/{}", theirs["id"]))
        .await
        .assert_detail(404, "No club pick with that id");
    // The other pick's milestone under this pick.
    f.ada
        .patch(
            &format!("{}/{}", f.url(), theirs["id"]),
            json!({"title": "x"}),
        )
        .await
        .assert_detail(404, "No milestone with that id");
    f.ada
        .delete(&format!("{}/{}", f.url(), theirs["id"]))
        .await
        .assert_detail(404, "No milestone with that id");

    assert_eq!(titles(&f.ada.get(&f.url()).await.json()), ["Mine"]);
    assert_eq!(
        titles(&f.ada.get(&milestones_url(&other, other_pick)).await.json()),
        ["Theirs"]
    );
    assert_eq!(mine["position"], 0);
    assert_eq!(theirs["position"], 0);
    // Nothing of the other club changed.
    let titles_now: Vec<String> = sqlx::query_scalar("SELECT title FROM milestones ORDER BY id")
        .fetch_all(&f.app.db)
        .await
        .unwrap();
    assert_eq!(titles_now, ["Mine", "Theirs"]);
}

/// Milestone writes are for admins; members read the schedule. A non-numeric id is a 400.
#[sqlx::test]
async fn members_read_and_admins_write(db: PgPool) {
    let f = Fixture::new(db).await;
    let created = f.create(json!({"title": "Part one"})).await;
    let url = f.url();
    let one = format!("{url}/{}", created["id"]);
    f.grace.get(&url).await.assert_status(200);
    f.grace
        .post(&url, json!({"title": "x"}))
        .await
        .assert_detail(403, "Only club admins can do that");
    f.grace
        .patch(&one, json!({"title": "x"}))
        .await
        .assert_detail(403, "Only club admins can do that");
    f.grace
        .delete(&one)
        .await
        .assert_detail(403, "Only club admins can do that");
    f.ada
        .get(&format!("/api/clubs/{}/picks/abc/milestones", f.club.slug))
        .await
        .assert_status(400);
    assert_eq!(f.app.scalar_i64("SELECT count(*) FROM milestones").await, 1);
}
