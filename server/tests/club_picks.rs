//! The current pick, its history and the meeting download (#45, endpoint-map §9,
//! docs/phase1/spec/club.md §1.1–§1.4, §1.12, §2.3, §2.5, §2.12).

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// `ada` owns the club "Readers" (time zone UTC); Circe (OL1W) and Achilles (OL2W) are in the
/// catalog.
struct Fixture {
    app: TestApp,
    ada: TestClient,
    club: TestClub,
    circe: TestWork,
    achilles: TestWork,
}

impl Fixture {
    async fn new(db: PgPool) -> Self {
        let app = TestApp::new(db).await;
        let ada = app.user("ada").await;
        let club = app.club(&ada, "Readers").await;
        let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
        let achilles = app
            .ol_work("OL2W", "The Song of Achilles", "Madeline Miller")
            .await;
        Fixture {
            app,
            ada,
            club,
            circe,
            achilles,
        }
    }

    /// `/api/clubs/{slug}/pick{rest}`.
    fn pick(&self, rest: &str) -> String {
        format!("/api/clubs/{}/pick{rest}", self.club.slug)
    }

    /// A member of the club with `role` (`"admin"` or `"member"`).
    async fn member(&self, name: &str, role: &str) -> TestClient {
        let user = self.app.user(name).await;
        self.app.join(&self.club, &user, role).await;
        user
    }
}

/// Puts `work` on `who`'s shelf with `status` (SHELF's routes are another package's).
async fn shelve(app: &TestApp, who: &TestClient, work: &TestWork, status: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO shelf_entries (user_id, work_id, status) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(who.id())
    .bind(work.id)
    .bind(status)
    .fetch_one(&app.db)
    .await
    .unwrap()
}

/// The club's events, oldest first: kind, actor id, work id, pick id, payload.
async fn events(
    app: &TestApp,
    club: &TestClub,
) -> Vec<(String, Option<i64>, Option<i64>, Option<i64>, Value)> {
    sqlx::query_as(
        "SELECT kind, actor_id, work_id, pick_id, payload FROM events
         WHERE club_id = $1 AND kind <> 'member_joined' ORDER BY id",
    )
    .bind(club.id)
    .fetch_all(&app.db)
    .await
    .unwrap()
}

async fn current_picks(app: &TestApp, club: &TestClub) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM club_picks WHERE club_id = $1 AND ended_at IS NULL")
        .bind(club.id)
        .fetch_one(&app.db)
        .await
        .unwrap()
}

/// Waits until `n` sessions wait for a lock.
async fn wait_for_waiters(app: &TestApp, n: i64) {
    for _ in 0..500 {
        let waiting = app
            .scalar_i64(
                "SELECT count(*) FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock'",
            )
            .await;
        if waiting >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{n} requests never waited for the club lock");
}

/// parity: test_pick.py::test_pick_requires_auth
#[sqlx::test]
async fn pick_routes_require_a_session(db: PgPool) {
    let f = Fixture::new(db).await;
    let anon = f.app.anon();
    let body = json!({"work_id": f.circe.id});
    anon.get(&f.pick(""))
        .await
        .assert_detail(401, "Not signed in");
    anon.put(&f.pick(""), body.clone())
        .await
        .assert_detail(401, "Not signed in");
    anon.patch(&f.pick(""), json!({"note": "x"}))
        .await
        .assert_detail(401, "Not signed in");
    anon.delete(&f.pick(""))
        .await
        .assert_detail(401, "Not signed in");
    anon.get(&f.pick("/history"))
        .await
        .assert_detail(401, "Not signed in");
    anon.get(&f.pick("/meeting.ics"))
        .await
        .assert_detail(401, "Not signed in");
}

/// parity: test_pick.py::test_empty_pick_then_set_and_read
#[sqlx::test]
async fn an_empty_club_has_no_pick_until_an_admin_sets_one(db: PgPool) {
    let f = Fixture::new(db).await;
    let empty = f.ada.get(&f.pick("")).await;
    empty.assert_status(200);
    assert_eq!(empty.json(), json!({"pick": null, "timezone": "UTC"}));

    let created = f
        .ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "note": "August"}),
        )
        .await;
    created.assert_status(200);
    let body = created.json();
    assert_eq!(body["book"]["title"], "Circe");
    assert_eq!(body["book"]["id"], f.circe.id);
    assert_eq!(body["set_by"], "ada");
    assert_eq!(body["note"], "August");
    assert_eq!(body["ended_at"], Value::Null);
    assert_eq!(body["meeting_at"], Value::Null);
    assert_eq!(body["meeting_local"], Value::Null);
    assert_eq!(body["meeting_label"], Value::Null);
    assert_eq!(body["on_shelf"], Value::Null);
    assert_eq!(body["shelf_id"], Value::Null);
    assert_eq!(body["readers"], json!([]));
    assert_eq!(body["reading"], json!([]));
    assert_eq!(body["finished"], json!([]));
    let started = body["started_at"].as_str().unwrap();
    assert!(started.ends_with('Z'), "{started}");

    let current = f.ada.get(&f.pick("")).await.json();
    assert_eq!(current["pick"]["book"]["ol_work_key"], "/works/OL1W");
    assert_eq!(current["pick"]["set_by"], "ada");
    assert_eq!(current["pick"]["id"], body["id"]);
    assert_eq!(current["timezone"], "UTC");
}

/// parity: test_pick.py::test_same_book_updates_meeting_without_history
#[sqlx::test]
async fn the_same_book_updates_the_meeting_without_history(db: PgPool) {
    let f = Fixture::new(db).await;
    let first = f
        .ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "meeting_at": "2026-08-22T19:00"}),
        )
        .await
        .json();
    let again = f
        .ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "meeting_at": "2026-08-29T18:30"}),
        )
        .await;
    again.assert_status(200);
    assert_eq!(again.json()["id"], first["id"]);
    assert_eq!(again.json()["meeting_local"], "2026-08-29T18:30");
    assert_eq!(again.json()["meeting_at"], "2026-08-29T18:30:00Z");
    assert_eq!(again.json()["meeting_label"], "Sat 29 Aug 2026, 18:30 UTC");
    let history = f.ada.get(&f.pick("/history")).await;
    history.assert_status(200);
    assert_eq!(history.json(), json!({"items": [], "timezone": "UTC"}));
}

/// The same work updates only the keys present: the note and the setter stay (the legacy app
/// wiped the note and took over `set_by`), `null` clears.
#[sqlx::test]
async fn the_same_work_changes_only_the_keys_present(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "admin").await;
    f.ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "note": "August", "meeting_at": "2026-08-22T19:00"}),
        )
        .await
        .assert_status(200);

    let kept = grace.put(&f.pick(""), json!({"work_id": f.circe.id})).await;
    kept.assert_status(200);
    assert_eq!(kept.json()["note"], "August");
    assert_eq!(kept.json()["meeting_at"], "2026-08-22T19:00:00Z");
    assert_eq!(kept.json()["set_by"], "ada");

    let cleared = grace
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "note": null, "meeting_at": "  "}),
        )
        .await;
    cleared.assert_status(200);
    assert_eq!(cleared.json()["note"], "");
    assert_eq!(cleared.json()["meeting_at"], Value::Null);
    assert_eq!(cleared.json()["set_by"], "ada");

    let trimmed = grace
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "note": "  Bring wine  ", "meeting_at": null}),
        )
        .await;
    assert_eq!(trimmed.json()["note"], "Bring wine");

    // One pick_set, for the first PUT only.
    let kinds: Vec<String> = events(&f.app, &f.club)
        .await
        .into_iter()
        .map(|e| e.0)
        .collect();
    assert_eq!(kinds, ["pick_set"]);
    assert_eq!(f.app.scalar_i64("SELECT count(*) FROM club_picks").await, 1);
}

/// parity: test_pick.py::test_replacing_pick_archives_previous_with_meeting
#[sqlx::test]
async fn replacing_the_pick_archives_the_previous_one_with_its_meeting(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "meeting_at": "2026-08-22T19:00", "note": "August"}),
        )
        .await
        .assert_status(200);
    let replaced = f
        .ada
        .put(&f.pick(""), json!({"work_id": f.achilles.id}))
        .await;
    replaced.assert_status(200);
    assert_eq!(replaced.json()["note"], "");

    let current = f.ada.get(&f.pick("")).await.json()["pick"].clone();
    assert_eq!(current["book"]["title"], "The Song of Achilles");
    assert_eq!(current["meeting_at"], Value::Null);

    let history = f.ada.get(&f.pick("/history")).await.json()["items"].clone();
    let history = history.as_array().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["book"]["title"], "Circe");
    assert_eq!(history[0]["note"], "August");
    assert!(history[0]["ended_at"].is_string());
    assert_eq!(history[0]["meeting_local"], "2026-08-22T19:00");
    assert_eq!(history[0]["readers"], json!([]));
    assert_eq!(history[0]["reading"], json!([]));
    assert_eq!(history[0]["finished"], json!([]));
    // The old pick ended no earlier than it started, and before the new one started.
    let ordered = f
        .app
        .scalar_i64(
            "SELECT count(*) FROM club_picks a JOIN club_picks b ON b.id > a.id
             WHERE a.ended_at >= a.started_at AND b.started_at >= a.ended_at",
        )
        .await;
    assert_eq!(ordered, 1);
}

/// parity: test_pick.py::test_reading_vs_finished_and_clear
#[sqlx::test]
async fn readers_split_into_reading_and_finished_and_an_admin_clears_the_pick(db: PgPool) {
    let f = Fixture::new(db).await;
    let ada_entry = shelve(&f.app, &f.ada, &f.circe, "currently_reading").await;
    f.ada
        .put(&f.pick(""), json!({"work_id": f.circe.id}))
        .await
        .assert_status(200);

    let pick = f.ada.get(&f.pick("")).await.json()["pick"].clone();
    assert_eq!(pick["on_shelf"], "currently_reading");
    assert_eq!(pick["shelf_id"], ada_entry);
    assert_eq!(pick["reading"], json!(["ada"]));
    assert_eq!(pick["finished"], json!([]));

    let grace = f.member("grace", "member").await;
    shelve(&f.app, &grace, &f.circe, "finished").await;
    let pick = grace.get(&f.pick("")).await.json()["pick"].clone();
    assert_eq!(pick["on_shelf"], "finished");
    assert_eq!(pick["reading"], json!(["ada"]));
    assert_eq!(pick["finished"], json!(["grace"]));
    // The viewer comes first among the readers.
    let names: Vec<&str> = pick["readers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["grace", "ada"]);

    // Clearing is an admin's job (any member could in the legacy app).
    grace
        .delete(&f.pick(""))
        .await
        .assert_detail(403, "Only club admins can do that");
    let cleared = f.ada.delete(&f.pick("")).await;
    cleared.assert_status(200);
    assert_eq!(cleared.json(), json!({"pick": null, "timezone": "UTC"}));
    assert_eq!(f.ada.get(&f.pick("")).await.json()["pick"], Value::Null);
    let history = f.ada.get(&f.pick("/history")).await.json()["items"].clone();
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["book"]["title"], "Circe");
    // The caller's own shelf status is still on an ended pick; the readers are not.
    assert_eq!(history[0]["on_shelf"], "currently_reading");
    assert_eq!(history[0]["shelf_id"], ada_entry);
    assert_eq!(history[0]["readers"], json!([]));
}

/// parity: test_pick.py::test_meeting_uses_club_timezone
#[sqlx::test]
async fn the_meeting_uses_the_club_time_zone(db: PgPool) {
    let f = Fixture::new(db).await;
    let res = f
        .ada
        .post(
            "/api/clubs",
            json!({"name": "Night Owls", "timezone": "America/New_York"}),
        )
        .await;
    res.assert_status(201);
    let slug = res.json()["slug"].as_str().unwrap().to_owned();
    let path = format!("/api/clubs/{slug}/pick");

    let created = f
        .ada
        .put(
            &path,
            json!({"work_id": f.circe.id, "meeting_at": "2026-08-22T19:00"}),
        )
        .await;
    created.assert_status(200);
    assert_eq!(created.json()["meeting_local"], "2026-08-22T19:00");
    assert!(
        created.json()["meeting_at"]
            .as_str()
            .unwrap()
            .starts_with("2026-08-22T23:00:00")
    );
    assert_eq!(
        created.json()["meeting_label"],
        "Sat 22 Aug 2026, 19:00 EDT"
    );
    assert_eq!(
        f.ada.get(&path).await.json()["timezone"],
        "America/New_York"
    );
    assert_eq!(
        f.ada.get(&format!("{path}/history")).await.json()["timezone"],
        "America/New_York"
    );
    // An explicit offset wins over the club's zone.
    let offset = f
        .ada
        .patch(&path, json!({"meeting_at": "2026-08-22T19:00+02:00"}))
        .await;
    assert_eq!(offset.json()["meeting_at"], "2026-08-22T17:00:00Z");
    assert_eq!(offset.json()["meeting_local"], "2026-08-22T13:00");
}

/// parity: test_pick.py::test_invalid_timezone_falls_back_to_utc
///
/// The zone is a club setting now, so an invalid name is refused when it is set (400 on club
/// update) instead of falling back silently. A corrupt stored value still reads as UTC.
#[sqlx::test]
async fn an_invalid_zone_is_refused_and_a_corrupt_one_reads_as_utc(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .patch(
            &format!("/api/clubs/{}", f.club.slug),
            json!({"timezone": "Not/AZone"}),
        )
        .await
        .assert_detail(400, "That is not a valid time zone");
    let created = f
        .ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "meeting_at": "2026-08-22T19:00"}),
        )
        .await;
    assert_eq!(created.json()["meeting_local"], "2026-08-22T19:00");
    assert!(
        created.json()["meeting_at"]
            .as_str()
            .unwrap()
            .starts_with("2026-08-22T19:00:00")
    );

    f.app
        .exec(&format!(
            "UPDATE clubs SET timezone = 'Not/AZone' WHERE id = {}",
            f.club.id
        ))
        .await;
    let current = f.ada.get(&f.pick("")).await;
    current.assert_status(200);
    assert_eq!(current.json()["timezone"], "UTC");
    assert_eq!(current.json()["pick"]["meeting_local"], "2026-08-22T19:00");
    let moved = f
        .ada
        .patch(&f.pick(""), json!({"meeting_at": "2026-08-23T20:00"}))
        .await;
    assert_eq!(moved.json()["meeting_at"], "2026-08-23T20:00:00Z");
}

/// parity: test_pick.py::test_invalid_pick_payload
///
/// The body names a catalog work (`work_id`), so the legacy `ol_work_key` and `title` checks are
/// gone: an unknown work is 404 `No such book.`. Validation still runs before that 404.
#[sqlx::test]
async fn invalid_pick_payloads_are_refused(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .put(&f.pick(""), json!({}))
        .await
        .assert_detail(400, "Field required");
    f.ada
        .put(&f.pick(""), json!({"work_id": "OL1W"}))
        .await
        .assert_status(400);
    f.ada
        .put(&f.pick(""), json!({"work_id": 987_654_321}))
        .await
        .assert_detail(404, "No such book.");
    let long = "x".repeat(281);
    f.ada
        .put(&f.pick(""), json!({"work_id": f.circe.id, "note": long}))
        .await
        .assert_detail(400, "note must be 280 characters or fewer");
    f.ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "meeting_at": "next Tuesday"}),
        )
        .await
        .assert_detail(400, "meeting_at must be a date and time");
    // Validation and the meeting come before the unknown work.
    f.ada
        .put(
            &f.pick(""),
            json!({"work_id": 987_654_321, "note": "x".repeat(281)}),
        )
        .await
        .assert_detail(400, "note must be 280 characters or fewer");
    f.ada
        .put(
            &f.pick(""),
            json!({"work_id": 987_654_321, "meeting_at": "next Tuesday"}),
        )
        .await
        .assert_detail(400, "meeting_at must be a date and time");
    // 280 characters after trimming are fine.
    let fits = format!("  {}  ", "é".repeat(280));
    let ok = f
        .ada
        .put(&f.pick(""), json!({"work_id": f.circe.id, "note": fits}))
        .await;
    ok.assert_status(200);
    assert_eq!(ok.json()["note"], "é".repeat(280));
    assert_eq!(f.app.scalar_i64("SELECT count(*) FROM club_picks").await, 1);
}

/// A merged-away work id resolves to the work it was merged into.
#[sqlx::test]
async fn a_merged_work_resolves_to_its_survivor(db: PgPool) {
    let f = Fixture::new(db).await;
    let duplicate = f.app.work("Circe (duplicate)").await;
    f.app
        .exec(&format!(
            "UPDATE works SET merged_into_id = {} WHERE id = {}",
            f.circe.id, duplicate.id
        ))
        .await;
    let set = f
        .ada
        .put(&f.pick(""), json!({"work_id": duplicate.id}))
        .await;
    set.assert_status(200);
    assert_eq!(set.json()["book"]["id"], f.circe.id);
    let again = f
        .ada
        .put(&f.pick(""), json!({"work_id": f.circe.id, "note": "same"}))
        .await;
    assert_eq!(again.json()["id"], set.json()["id"]);
}

/// `PATCH …/pick` edits the current pick's note and meeting without touching the setter.
#[sqlx::test]
async fn patch_edits_the_meeting_and_note_of_the_current_pick(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "admin").await;
    grace
        .patch(&f.pick(""), json!({"note": "x"}))
        .await
        .assert_detail(404, "No club pick yet");
    f.ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "note": "August", "meeting_at": "2026-08-22T19:00"}),
        )
        .await
        .assert_status(200);

    let moved = grace
        .patch(&f.pick(""), json!({"meeting_at": "2026-08-29T18:30"}))
        .await;
    moved.assert_status(200);
    assert_eq!(moved.json()["meeting_local"], "2026-08-29T18:30");
    assert_eq!(moved.json()["note"], "August");
    assert_eq!(moved.json()["set_by"], "ada");

    let same = grace.patch(&f.pick(""), json!({})).await;
    same.assert_status(200);
    assert_eq!(same.json(), moved.json());

    let noted = grace
        .patch(&f.pick(""), json!({"note": "  Bring wine  "}))
        .await;
    assert_eq!(noted.json()["note"], "Bring wine");
    assert_eq!(noted.json()["meeting_local"], "2026-08-29T18:30");

    let cleared = grace
        .patch(&f.pick(""), json!({"note": null, "meeting_at": null}))
        .await;
    assert_eq!(cleared.json()["note"], "");
    assert_eq!(cleared.json()["meeting_at"], Value::Null);
    assert_eq!(cleared.json()["meeting_label"], Value::Null);

    grace
        .patch(&f.pick(""), json!({"note": "x".repeat(281)}))
        .await
        .assert_detail(400, "note must be 280 characters or fewer");
    grace
        .patch(&f.pick(""), json!({"meeting_at": "soon"}))
        .await
        .assert_detail(400, "meeting_at must be a date and time");
    let kinds: Vec<String> = events(&f.app, &f.club)
        .await
        .into_iter()
        .map(|e| e.0)
        .collect();
    assert_eq!(kinds, ["pick_set"]);

    // An ended pick is not the current one.
    f.ada.delete(&f.pick("")).await.assert_status(200);
    grace
        .patch(&f.pick(""), json!({"note": "x"}))
        .await
        .assert_detail(404, "No club pick yet");
}

/// `pick_set` carries the meeting (RFC 3339, `Z`) when there is one; `pick_cleared` has an
/// empty payload; clearing without a pick writes nothing.
#[sqlx::test]
async fn pick_events_are_recorded(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada.delete(&f.pick("")).await.assert_status(200);
    assert!(events(&f.app, &f.club).await.is_empty());

    let first = f
        .ada
        .put(
            &f.pick(""),
            json!({"work_id": f.circe.id, "meeting_at": "2026-08-22T19:00"}),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let second = f
        .ada
        .put(&f.pick(""), json!({"work_id": f.achilles.id}))
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    f.ada.delete(&f.pick("")).await.assert_status(200);
    f.ada.delete(&f.pick("")).await.assert_status(200);

    let ada = Some(f.ada.id());
    assert_eq!(
        events(&f.app, &f.club).await,
        [
            (
                "pick_set".to_owned(),
                ada,
                Some(f.circe.id),
                Some(first),
                json!({"meeting_at": "2026-08-22T19:00:00Z"})
            ),
            (
                "pick_set".to_owned(),
                ada,
                Some(f.achilles.id),
                Some(second),
                json!({})
            ),
            (
                "pick_cleared".to_owned(),
                ada,
                Some(f.achilles.id),
                Some(second),
                json!({})
            ),
        ]
    );
}

/// History is every ended pick, the most recently ended first.
#[sqlx::test]
async fn history_lists_ended_picks_newest_first(db: PgPool) {
    let f = Fixture::new(db).await;
    let wolf = f.app.work("Wolf Den").await;
    for work in [&f.circe, &f.achilles, &wolf] {
        f.ada
            .put(&f.pick(""), json!({"work_id": work.id}))
            .await
            .assert_status(200);
    }
    let history = f.ada.get(&f.pick("/history")).await.json();
    let titles: Vec<&str> = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["book"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["The Song of Achilles", "Circe"]);
    // Equal end times fall back to the id, newest first.
    f.app
        .exec("UPDATE club_picks SET ended_at = '2026-01-01T00:00:00Z' WHERE ended_at IS NOT NULL")
        .await;
    let history = f.ada.get(&f.pick("/history")).await.json();
    let ids: Vec<i64> = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(ids, sorted);
}

/// A user in two clubs sees two separate picks and histories (#45 acceptance).
#[sqlx::test]
async fn a_user_in_two_clubs_sees_separate_picks(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.app.user("grace").await;
    let other = f.app.club(&grace, "Other").await;
    f.app.join(&other, &f.ada, "member").await;
    let other_pick = format!("/api/clubs/{}/pick", other.slug);

    f.ada
        .put(&f.pick(""), json!({"work_id": f.circe.id}))
        .await
        .assert_status(200);
    assert_eq!(f.ada.get(&other_pick).await.json()["pick"], Value::Null);
    grace
        .put(&other_pick, json!({"work_id": f.achilles.id}))
        .await
        .assert_status(200);
    grace
        .put(&other_pick, json!({"work_id": f.circe.id}))
        .await
        .assert_status(200);

    assert_eq!(
        f.ada.get(&f.pick("")).await.json()["pick"]["book"]["title"],
        "Circe"
    );
    assert_eq!(
        f.ada.get(&f.pick("/history")).await.json()["items"],
        json!([])
    );
    let history = f.ada.get(&format!("{other_pick}/history")).await.json();
    assert_eq!(history["items"][0]["book"]["title"], "The Song of Achilles");
    // A plain member of the other club cannot set its pick; a non-member cannot see this one.
    f.ada
        .put(&other_pick, json!({"work_id": f.achilles.id}))
        .await
        .assert_detail(403, "Only club admins can do that");
    grace
        .get(&f.pick(""))
        .await
        .assert_detail(404, "No such club");
    assert_eq!(current_picks(&f.app, &f.club).await, 1);
    assert_eq!(current_picks(&f.app, &other).await, 1);
}

/// Two `PUT pick` calls that meet at the club lock run one after the other: the second ends
/// the first one's pick, so one current pick is left (the legacy app could leave two).
#[sqlx::test]
async fn two_concurrent_puts_leave_one_current_pick(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "admin").await;
    let mut hold = f.app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(f.club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let first = {
        let (client, path, work) = (f.ada.clone(), f.pick(""), f.circe.id);
        tokio::spawn(async move { client.put(&path, json!({"work_id": work})).await })
    };
    wait_for_waiters(&f.app, 1).await;
    let second = {
        let (client, path, work) = (grace.clone(), f.pick(""), f.achilles.id);
        tokio::spawn(async move { client.put(&path, json!({"work_id": work})).await })
    };
    wait_for_waiters(&f.app, 2).await;
    hold.commit().await.unwrap();
    let (first, second) = (first.await.unwrap(), second.await.unwrap());
    first.assert_status(200);
    second.assert_status(200);
    assert_eq!(current_picks(&f.app, &f.club).await, 1);
    let current = f.ada.get(&f.pick("")).await.json()["pick"].clone();
    assert_eq!(current["book"]["title"], "The Song of Achilles");
    assert_eq!(current["set_by"], "grace");
    let history = f.ada.get(&f.pick("/history")).await.json()["items"].clone();
    assert_eq!(history[0]["id"], first.json()["id"]);
    assert_eq!(history[0]["set_by"], "ada");
}

/// A burst of concurrent `PUT pick` calls, alternating works, never fails and leaves exactly
/// one current pick and one `pick_set` per pick.
#[sqlx::test]
async fn a_burst_of_puts_leaves_one_current_pick(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "admin").await;
    let mut tasks = Vec::new();
    for n in 0..8 {
        let client = if n % 2 == 0 {
            f.ada.clone()
        } else {
            grace.clone()
        };
        let work = if n % 3 == 0 {
            f.circe.id
        } else {
            f.achilles.id
        };
        let path = f.pick("");
        tasks.push(tokio::spawn(async move {
            client.put(&path, json!({"work_id": work})).await
        }));
    }
    for task in tasks {
        task.await.unwrap().assert_status(200);
    }
    assert_eq!(current_picks(&f.app, &f.club).await, 1);
    let picks = f.app.scalar_i64("SELECT count(*) FROM club_picks").await;
    let sets = f
        .app
        .scalar_i64("SELECT count(*) FROM events WHERE kind = 'pick_set'")
        .await;
    assert_eq!(picks, sets);
    let disordered = f
        .app
        .scalar_i64("SELECT count(*) FROM club_picks WHERE ended_at < started_at")
        .await;
    assert_eq!(disordered, 0);
}

/// The role is read again under the club lock: an admin demoted while their `PUT` waited for
/// the lock gets 403 and changes nothing.
#[sqlx::test]
async fn an_admin_demoted_while_waiting_sets_nothing(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "admin").await;
    let mut hold = f.app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(f.club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let put = {
        let (client, path, work) = (grace.clone(), f.pick(""), f.circe.id);
        tokio::spawn(async move { client.put(&path, json!({"work_id": work})).await })
    };
    wait_for_waiters(&f.app, 1).await;
    sqlx::query("UPDATE club_memberships SET role = 'member' WHERE club_id = $1 AND user_id = $2")
        .bind(f.club.id)
        .bind(grace.id())
        .execute(&mut *hold)
        .await
        .unwrap();
    hold.commit().await.unwrap();
    put.await
        .unwrap()
        .assert_detail(403, "Only club admins can do that");
    assert_eq!(f.app.scalar_i64("SELECT count(*) FROM club_picks").await, 0);
    assert!(events(&f.app, &f.club).await.is_empty());
}

/// parity: test_notes_and_meeting.py::test_meeting_ics_download
///
/// The calendar entry names the club (`PRODID`, `SUMMARY`) and links the book's page and the
/// club's page instead of openlibrary.org (endpoint-map §9).
#[sqlx::test]
async fn the_meeting_downloads_as_an_ics_file(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .get(&f.pick("/meeting.ics"))
        .await
        .assert_detail(404, "No meeting scheduled");
    f.ada
        .put(&f.pick(""), json!({"work_id": f.circe.id}))
        .await
        .assert_status(200);
    f.ada
        .get(&f.pick("/meeting.ics"))
        .await
        .assert_detail(404, "No meeting scheduled");

    let pick_id = f
        .ada
        .put(
            &f.pick(""),
            json!({
                "work_id": f.circe.id,
                "meeting_at": "2026-09-10T19:30",
                "note": "Bring snacks; wine, too"
            }),
        )
        .await
        .json()["id"]
        .as_i64()
        .unwrap();
    let grace = f.member("grace", "member").await;
    let res = grace.get(&f.pick("/meeting.ics")).await;
    res.assert_status(200);
    assert_eq!(
        res.header("content-type"),
        Some("text/calendar; charset=utf-8")
    );
    assert_eq!(
        res.header("content-disposition").map(str::to_owned),
        Some(format!("attachment; filename=\"bookclub-{pick_id}.ics\""))
    );
    let body = res.text();
    assert!(body.starts_with("BEGIN:VCALENDAR\r\n"));
    assert!(body.ends_with("END:VCALENDAR\r\n"));
    for line in body.split("\r\n") {
        assert!(line.len() <= 75, "{line:?} is {} bytes", line.len());
    }
    let unfolded = body.replace("\r\n ", "");
    let public = &f.app.config().public_url;
    for expected in [
        "BEGIN:VEVENT".to_owned(),
        "PRODID:-//Readers//Bookclub//EN".to_owned(),
        format!("UID:pick-{pick_id}@bookclub"),
        "DTSTART:20260910T193000Z".to_owned(),
        "DTEND:20260910T210000Z".to_owned(),
        "SUMMARY:Readers: Circe".to_owned(),
        "DESCRIPTION:Circe — Madeline Miller\\nBring snacks\x5c; wine\x5c, too\\n".to_owned(),
        format!(
            "{public}/book/{}\\n{public}/c/{}\r\n",
            f.circe.id, f.club.slug
        ),
        format!("\r\nURL:{public}/c/{}\r\n", f.club.slug),
    ] {
        assert!(unfolded.contains(&expected), "{expected:?} in {unfolded}");
    }
    assert!(!unfolded.contains("openlibrary.org"));
    let stamp = unfolded
        .lines()
        .find_map(|line| line.strip_prefix("DTSTAMP:"))
        .unwrap();
    assert_eq!(stamp.len(), 16, "{stamp}");
    assert!(stamp.ends_with('Z'));

    // Only the current pick has a meeting to download.
    f.ada.delete(&f.pick("")).await.assert_status(200);
    grace
        .get(&f.pick("/meeting.ics"))
        .await
        .assert_detail(404, "No meeting scheduled");
}

/// The meeting is stored in UTC and rendered in UTC in the calendar file, whatever the club's
/// zone.
#[sqlx::test]
async fn the_ics_times_are_utc(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .patch(
            &format!("/api/clubs/{}", f.club.slug),
            json!({"timezone": "Europe/Berlin"}),
        )
        .await
        .assert_status(200);
    f.ada
        .put(
            &f.pick(""),
            json!({"work_id": f.achilles.id, "meeting_at": "2026-07-01T20:00"}),
        )
        .await
        .assert_status(200);
    let body = f.ada.get(&f.pick("/meeting.ics")).await.text();
    let unfolded = body.replace("\r\n ", "");
    assert!(unfolded.contains("DTSTART:20260701T180000Z"));
    assert!(unfolded.contains("DTEND:20260701T193000Z"));
    assert!(unfolded.contains("SUMMARY:Readers: The Song of Achilles"));
    let label = f.ada.get(&f.pick("")).await.json()["pick"]["meeting_label"].clone();
    assert_eq!(label, "Wed 01 Jul 2026, 20:00 CEST");
}
