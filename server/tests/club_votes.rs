//! The next-up vote: nominations, ballots, the deadline, applying the winner and the lazy
//! auto-close (#45, endpoint-map §9, docs/phase1/spec/club.md §1.17–§1.21, §2.8).

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

/// `ada` owns the club "Readers" (time zone UTC); Circe (OL1W), Achilles (OL2W) and The Wolf
/// Den (OL3W) are in the catalog.
struct Fixture {
    app: TestApp,
    ada: TestClient,
    club: TestClub,
    circe: TestWork,
    achilles: TestWork,
    wolf: TestWork,
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
        let wolf = app.ol_work("OL3W", "The Wolf Den", "Elodie Harper").await;
        Fixture {
            app,
            ada,
            club,
            circe,
            achilles,
            wolf,
        }
    }

    /// `/api/clubs/{slug}{rest}`.
    fn path(&self, rest: &str) -> String {
        format!("/api/clubs/{}{rest}", self.club.slug)
    }

    /// A member of the club with `role` (`"admin"` or `"member"`).
    async fn member(&self, name: &str, role: &str) -> TestClient {
        let user = self.app.user(name).await;
        self.app.join(&self.club, &user, role).await;
        user
    }

    /// `who` nominates `work`; 201, the vote.
    async fn nominate(&self, who: &TestClient, work: &TestWork) -> Value {
        let res = who
            .post(&self.path("/vote/nominations"), json!({"work_id": work.id}))
            .await;
        res.assert_status(201);
        res.json()
    }

    /// `who` casts for nomination `id`; 200, the vote.
    async fn cast(&self, who: &TestClient, id: &Value) -> Value {
        let res = who
            .post(&self.path("/vote/cast"), json!({"nomination_id": id}))
            .await;
        res.assert_status(200);
        res.json()
    }

    async fn vote(&self, who: &TestClient) -> Value {
        let res = who.get(&self.path("/vote")).await;
        res.assert_status(200);
        res.json()
    }

    async fn pick(&self, who: &TestClient) -> Value {
        who.get(&self.path("/pick")).await.json()["pick"].clone()
    }

    /// Moves the open vote's deadline a minute into the past.
    async fn expire_deadline(&self) {
        self.app
            .exec(&format!(
                "UPDATE next_up_votes SET closes_at = now() - interval '1 minute'
                 WHERE club_id = {} AND ended_at IS NULL",
                self.club.id
            ))
            .await;
    }

    /// The club's events, oldest first: kind, actor id, work id, pick id, payload.
    async fn events(&self) -> Vec<(String, Option<i64>, Option<i64>, Option<i64>, Value)> {
        sqlx::query_as(
            "SELECT kind, actor_id, work_id, pick_id, payload FROM events
             WHERE club_id = $1 AND kind <> 'member_joined' ORDER BY id",
        )
        .bind(self.club.id)
        .fetch_all(&self.app.db)
        .await
        .unwrap()
    }

    async fn count(&self, sql: &str) -> i64 {
        self.app
            .scalar_i64(&sql.replace("{club}", &self.club.id.to_string()))
            .await
    }
}

/// The nomination of `title` in a vote.
fn nomination<'a>(vote: &'a Value, title: &str) -> &'a Value {
    vote["nominations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["book"]["title"] == title)
        .unwrap_or_else(|| panic!("{title} is not nominated: {vote}"))
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

/// parity: test_vote.py::test_vote_requires_auth
#[sqlx::test]
async fn vote_routes_require_a_session(db: PgPool) {
    let f = Fixture::new(db).await;
    let anon = f.app.anon();
    for res in [
        anon.get(&f.path("/vote")).await,
        anon.patch(&f.path("/vote"), json!({"closes_at": null}))
            .await,
        anon.post(&f.path("/vote/nominations"), json!({"work_id": f.circe.id}))
            .await,
        anon.post(&f.path("/vote/cast"), json!({"nomination_id": 1}))
            .await,
        anon.post(&f.path("/vote/apply"), json!({"nomination_id": 1}))
            .await,
        anon.get(&f.path("/vote/suggestions")).await,
    ] {
        res.assert_detail(401, "Not signed in");
    }
}

/// parity: test_vote.py::test_empty_vote_then_nominate_and_count
#[sqlx::test]
async fn an_empty_vote_opens_with_the_first_nomination(db: PgPool) {
    let f = Fixture::new(db).await;
    let empty = f.vote(&f.ada).await;
    assert_eq!(
        empty,
        json!({
            "vote_id": null, "nominations": [], "my_vote_id": null, "nomination_limit": 6,
            "can_nominate": true, "timezone": "UTC", "closes_at": null, "closes_local": null,
            "closes_label": null, "not_voted": [], "voted_count": 0, "member_count": 1,
            "leader_id": null
        })
    );

    let vote = f.nominate(&f.ada, &f.circe).await;
    assert!(vote["vote_id"].as_i64().is_some());
    assert_eq!(vote["can_nominate"], true);
    assert_eq!(vote["nominations"].as_array().unwrap().len(), 1);
    let row = &vote["nominations"][0];
    assert_eq!(row["book"]["title"], "Circe");
    assert_eq!(row["book"]["id"], f.circe.id);
    assert_eq!(row["nominated_by"], "ada");
    assert_eq!(row["votes"], 0);
    assert_eq!(row["voters"], json!([]));
    assert_eq!(row["mine"], false);
    assert_eq!(f.pick(&f.ada).await, Value::Null);
    assert_eq!(f.vote(&f.ada).await, vote);
}

/// parity: test_vote.py::test_members_can_vote_and_change_vote
#[sqlx::test]
async fn members_vote_and_change_their_vote(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "member").await;
    let circe = f.nominate(&f.ada, &f.circe).await["nominations"][0]["id"].clone();
    let achilles = f.nominate(&f.ada, &f.achilles).await;
    let achilles = nomination(&achilles, "The Song of Achilles")["id"].clone();

    let first = f.cast(&f.ada, &circe).await;
    assert_eq!(first["my_vote_id"], circe);
    let row = nomination(&first, "Circe");
    assert_eq!(
        (&row["votes"], &row["voters"], &row["mine"]),
        (&json!(1), &json!(["ada"]), &json!(true))
    );
    assert_eq!(nomination(&first, "The Song of Achilles")["votes"], 0);

    let second = f.cast(&grace, &circe).await;
    let row = nomination(&second, "Circe");
    assert_eq!(
        (&row["votes"], &row["voters"]),
        (&json!(2), &json!(["ada", "grace"]))
    );

    let switched = f.cast(&grace, &achilles).await;
    assert_eq!(nomination(&switched, "Circe")["votes"], 1);
    assert_eq!(nomination(&switched, "The Song of Achilles")["votes"], 1);
    assert_eq!(nomination(&switched, "The Song of Achilles")["mine"], true);
    assert_eq!(nomination(&switched, "Circe")["mine"], false);
    assert_eq!(switched["my_vote_id"], achilles);
    assert_eq!(f.pick(&f.ada).await, Value::Null);

    // A ballot is recorded once per member, however often it moves.
    let voted: Vec<_> = f
        .events()
        .await
        .into_iter()
        .filter(|e| e.0 == "voted")
        .map(|e| (e.1, e.2))
        .collect();
    assert_eq!(
        voted,
        [
            (Some(f.ada.id()), Some(f.circe.id)),
            (Some(grace.id()), Some(f.circe.id))
        ]
    );
}

/// parity: test_vote.py::test_cannot_nominate_current_pick_or_duplicate
#[sqlx::test]
async fn the_current_pick_and_duplicates_cannot_be_nominated(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .put(&f.path("/pick"), json!({"work_id": f.circe.id}))
        .await
        .assert_status(200);
    f.ada
        .post(&f.path("/vote/nominations"), json!({"work_id": f.circe.id}))
        .await
        .assert_detail(400, "That book is already the club pick");
    // The refused first nomination opened no vote.
    assert_eq!(f.count("SELECT count(*) FROM next_up_votes").await, 0);

    f.nominate(&f.ada, &f.achilles).await;
    f.ada
        .post(
            &f.path("/vote/nominations"),
            json!({"work_id": f.achilles.id}),
        )
        .await
        .assert_detail(400, "That book is already nominated");

    // A past pick may be nominated again.
    f.ada
        .put(&f.path("/pick"), json!({"work_id": f.wolf.id}))
        .await
        .assert_status(200);
    f.nominate(&f.ada, &f.circe).await;
}

/// parity: test_vote.py::test_nomination_limit
#[sqlx::test]
async fn a_vote_takes_six_nominations(db: PgPool) {
    let f = Fixture::new(db).await;
    for n in 1..=6 {
        let work = f.app.work(&format!("Book {n}")).await;
        let vote = f.nominate(&f.ada, &work).await;
        assert_eq!(vote["can_nominate"], n < 6, "{n}");
    }
    let seventh = f.app.work("Book 7").await;
    f.ada
        .post(&f.path("/vote/nominations"), json!({"work_id": seventh.id}))
        .await
        .assert_detail(400, "This vote already has enough books");
    assert_eq!(f.count("SELECT count(*) FROM next_up_nominations").await, 6);
}

/// parity: test_vote.py::test_apply_winner_uses_club_pick_flow
///
/// The old pick's post is a row inserted with SQL (the thread routes are POSTS'); it stays on
/// the old pick.
#[sqlx::test]
async fn applying_the_winner_makes_it_the_pick(db: PgPool) {
    let f = Fixture::new(db).await;
    let old = f
        .ada
        .put(&f.path("/pick"), json!({"work_id": f.circe.id}))
        .await
        .json();
    let old_id = old["id"].as_i64().unwrap();
    f.app
        .exec(&format!(
            "INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
             VALUES ({}, {}, {}, {old_id}, 'Circe notes')",
            f.circe.id,
            f.ada.id(),
            f.club.id
        ))
        .await;

    let nom = f.nominate(&f.ada, &f.achilles).await["nominations"][0]["id"].clone();
    f.cast(&f.ada, &nom).await;
    assert_eq!(f.pick(&f.ada).await["book"]["title"], "Circe");

    let applied = f
        .ada
        .post(
            &f.path("/vote/apply"),
            json!({"nomination_id": nom, "meeting_at": "2026-09-12T19:00"}),
        )
        .await;
    applied.assert_status(200);
    let body = applied.json();
    let pick = &body["pick"];
    assert_ne!(pick["id"], old_id);
    assert_eq!(pick["book"]["title"], "The Song of Achilles");
    assert_eq!(pick["set_by"], "ada");
    assert_eq!(pick["meeting_local"], "2026-09-12T19:00");
    assert_eq!(pick["note"], "");
    assert_eq!(body["vote"]["vote_id"], Value::Null);
    assert_eq!(body["vote"]["nominations"], json!([]));

    assert_eq!(f.pick(&f.ada).await["id"], pick["id"]);
    let history = f.ada.get(&f.path("/pick/history")).await.json();
    assert_eq!(history["items"][0]["id"], old_id);
    assert_eq!(
        f.count(&format!(
            "SELECT count(*) FROM posts WHERE pick_id = {old_id} AND body = 'Circe notes'"
        ))
        .await,
        1
    );

    let fresh = f.vote(&f.ada).await;
    assert_eq!(fresh["vote_id"], Value::Null);
    let again = f.nominate(&f.ada, &f.wolf).await;
    assert_eq!(again["nominations"][0]["book"]["title"], "The Wolf Den");
    assert_ne!(again["vote_id"], Value::Null);

    // pick_set (by the admin who applied) before vote_closed, which has no `auto` key.
    let kinds: Vec<_> = f
        .events()
        .await
        .into_iter()
        .filter(|e| e.0 == "pick_set" || e.0 == "vote_closed")
        .collect();
    let new_pick = pick["id"].as_i64();
    assert_eq!(
        kinds[1..],
        [
            (
                "pick_set".to_owned(),
                Some(f.ada.id()),
                Some(f.achilles.id),
                new_pick,
                json!({"meeting_at": "2026-09-12T19:00:00Z"})
            ),
            (
                "vote_closed".to_owned(),
                Some(f.ada.id()),
                Some(f.achilles.id),
                new_pick,
                json!({"votes": 1})
            )
        ]
    );
    let winner = f
        .count("SELECT count(*) FROM next_up_votes WHERE winner_nomination_id IS NOT NULL")
        .await;
    assert_eq!(winner, 1);
}

/// parity: test_vote.py::test_apply_requires_an_open_nomination
#[sqlx::test]
async fn apply_needs_an_open_nomination(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .post(&f.path("/vote/apply"), json!({"nomination_id": 99}))
        .await
        .assert_detail(404, "No open next-up vote");
    let nom = f.nominate(&f.ada, &f.circe).await["nominations"][0]["id"].clone();
    f.ada
        .post(&f.path("/vote/apply"), json!({"nomination_id": 99}))
        .await
        .assert_detail(404, "No nomination with that id");
    f.ada
        .post(&f.path("/vote/apply"), json!({"nomination_id": nom}))
        .await
        .assert_status(200);
    f.ada
        .post(&f.path("/vote/apply"), json!({"nomination_id": nom}))
        .await
        .assert_detail(404, "No open next-up vote");
    // Zero votes are fine.
    let closed = f.events().await.into_iter().find(|e| e.0 == "vote_closed");
    assert_eq!(closed.unwrap().4, json!({"votes": 0}));
}

/// parity: test_vote_deadline.py::test_deadline_and_turnout_reporting
#[sqlx::test]
async fn the_deadline_and_turnout_are_reported(db: PgPool) {
    let f = Fixture::new(db).await;
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_detail(404, "Nominate a book first");

    f.nominate(&f.ada, &f.circe).await;
    let vote = f.vote(&f.ada).await;
    assert_eq!(vote["closes_at"], Value::Null);
    assert_eq!(vote["member_count"], 1);
    assert_eq!(vote["not_voted"], json!(["ada"]));
    assert_eq!(vote["voted_count"], 0);
    assert_eq!(vote["leader_id"], Value::Null);

    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2000-01-01T18:00"}))
        .await
        .assert_detail(400, "The deadline has to be in the future");
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "not a date"}))
        .await
        .assert_detail(400, "meeting_at must be a date and time");

    let set = f
        .ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await;
    set.assert_status(200);
    let set = set.json();
    assert_eq!(set["closes_at"], "2030-01-01T18:00:00Z");
    assert_eq!(set["closes_local"], "2030-01-01T18:00");
    assert_eq!(set["closes_label"], "Tue 01 Jan 2030, 18:00 UTC");

    let grace = f.member("grace", "member").await;
    let nomination_id = f.vote(&grace).await["nominations"][0]["id"].clone();
    f.cast(&grace, &nomination_id).await;
    let vote = f.vote(&f.ada).await;
    assert_eq!(vote["member_count"], 2);
    assert_eq!(vote["voted_count"], 1);
    assert_eq!(vote["not_voted"], json!(["ada"]));
    assert_eq!(vote["leader_id"], nomination_id);

    let cleared = f
        .ada
        .patch(&f.path("/vote"), json!({"closes_at": null}))
        .await
        .json();
    assert_eq!(cleared["closes_at"], Value::Null);
    assert_eq!(cleared["closes_label"], Value::Null);
    // `{}` clears it too (parity), as does whitespace.
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    let cleared = f.ada.patch(&f.path("/vote"), json!({})).await.json();
    assert_eq!(cleared["closes_at"], Value::Null);
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "  "}))
        .await
        .assert_status(200);
    // No event for deadline changes.
    assert!(f.events().await.iter().all(|e| e.0 != "vote_closed"));
}

/// The deadline is entered and labelled in the club's zone.
#[sqlx::test]
async fn the_deadline_uses_the_club_time_zone(db: PgPool) {
    let f = Fixture::new(db).await;
    let res = f
        .ada
        .post(
            "/api/clubs",
            json!({"name": "Berlin", "timezone": "Europe/Berlin"}),
        )
        .await;
    res.assert_status(201);
    let slug = res.json()["slug"].as_str().unwrap().to_owned();
    let path = |rest: &str| format!("/api/clubs/{slug}{rest}");
    f.ada
        .post(&path("/vote/nominations"), json!({"work_id": f.circe.id}))
        .await
        .assert_status(201);
    let set = f
        .ada
        .patch(&path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .json();
    assert_eq!(set["timezone"], "Europe/Berlin");
    assert_eq!(set["closes_at"], "2030-01-01T17:00:00Z");
    assert_eq!(set["closes_local"], "2030-01-01T18:00");
    assert_eq!(set["closes_label"], "Tue 01 Jan 2030, 18:00 CET");
    let explicit = f
        .ada
        .patch(&path("/vote"), json!({"closes_at": "2030-07-01T18:00Z"}))
        .await
        .json();
    assert_eq!(explicit["closes_local"], "2030-07-01T20:00");
}

/// parity: test_vote_deadline.py::test_deadline_auto_applies_the_leader
#[sqlx::test]
async fn the_deadline_applies_the_leader(db: PgPool) {
    let f = Fixture::new(db).await;
    f.nominate(&f.ada, &f.circe).await;
    let grace = f.member("grace", "member").await;
    let vote = f.nominate(&grace, &f.achilles).await;
    let achilles = nomination(&vote, "The Song of Achilles")["id"].clone();
    f.cast(&grace, &achilles).await;
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    f.expire_deadline().await;

    let after = f.vote(&f.ada).await;
    assert_eq!(after["vote_id"], Value::Null);
    assert_eq!(after["nominations"], json!([]));
    let pick = f.pick(&f.ada).await;
    assert_eq!(pick["book"]["title"], "The Song of Achilles");
    assert_eq!(pick["set_by"], "grace");
    assert_eq!(pick["meeting_at"], Value::Null);

    let feed = f.ada.get(&f.path("/activity")).await.json();
    let items = feed["items"].as_array().unwrap();
    let kinds: Vec<&str> = items[..2]
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["vote_closed", "pick_set"]);
    assert_eq!(items[0]["payload"], json!({"votes": 1, "auto": true}));
    assert_eq!(items[0]["actor"], "grace");
    assert_eq!(items[1]["actor"], "grace");
    assert_eq!(items[0]["pick_id"], pick["id"]);
}

/// parity: test_vote_deadline.py::test_deadline_with_no_ballots_reopens_without_deadline
#[sqlx::test]
async fn a_deadline_without_ballots_is_cleared(db: PgPool) {
    let f = Fixture::new(db).await;
    f.nominate(&f.ada, &f.circe).await;
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    f.expire_deadline().await;
    let after = f.vote(&f.ada).await;
    assert_ne!(after["vote_id"], Value::Null);
    assert_eq!(after["closes_at"], Value::Null);
    assert_eq!(after["nominations"].as_array().unwrap().len(), 1);
    assert_eq!(f.pick(&f.ada).await, Value::Null);
    assert_eq!(
        f.count("SELECT count(*) FROM next_up_votes WHERE closes_at IS NULL")
            .await,
        1
    );
}

/// parity: test_vote_deadline.py::test_tie_goes_to_earliest_nomination
#[sqlx::test]
async fn a_tie_goes_to_the_earliest_nomination(db: PgPool) {
    let f = Fixture::new(db).await;
    f.nominate(&f.ada, &f.circe).await;
    let grace = f.member("grace", "member").await;
    let vote = f.nominate(&grace, &f.achilles).await;
    let circe = nomination(&vote, "Circe")["id"].clone();
    let achilles = nomination(&vote, "The Song of Achilles")["id"].clone();
    f.cast(&grace, &achilles).await;
    f.cast(&f.ada, &circe).await;
    let vote = f.vote(&f.ada).await;
    assert_eq!(vote["leader_id"], circe);
    // Equal votes: nominations by id.
    assert_eq!(vote["nominations"][0]["id"], circe);
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    f.expire_deadline().await;
    f.vote(&f.ada).await;
    let pick = f.pick(&f.ada).await;
    assert_eq!(pick["book"]["title"], "Circe");
    assert_eq!(pick["set_by"], "ada");
}

/// Nominations are listed by votes, then id; the leader is decided by creation order.
#[sqlx::test]
async fn nominations_are_ordered_by_votes(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "member").await;
    f.nominate(&f.ada, &f.circe).await;
    f.nominate(&f.ada, &f.achilles).await;
    let vote = f.nominate(&f.ada, &f.wolf).await;
    let wolf = nomination(&vote, "The Wolf Den")["id"].clone();
    f.cast(&grace, &wolf).await;
    let vote = f.cast(&f.ada, &wolf).await;
    let titles: Vec<&str> = vote["nominations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["book"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["The Wolf Den", "Circe", "The Song of Achilles"]);
    assert_eq!(vote["leader_id"], wolf);
    assert_eq!(vote["voted_count"], 2);
    assert_eq!(vote["not_voted"], json!([]));
}

/// Body and lookup errors, in their documented order.
#[sqlx::test]
async fn vote_requests_are_checked_in_order(db: PgPool) {
    let f = Fixture::new(db).await;
    let nominate = f.path("/vote/nominations");
    f.ada
        .post(&nominate, json!({}))
        .await
        .assert_detail(400, "Field required");
    f.ada
        .post(&nominate, json!({"work_id": "OL1W"}))
        .await
        .assert_detail(400, "Input should be a valid integer");
    f.ada
        .post(&nominate, json!({"work_id": 999_999}))
        .await
        .assert_detail(404, "No such book.");
    assert_eq!(f.count("SELECT count(*) FROM next_up_votes").await, 0);
    f.ada
        .post(&f.path("/vote/cast"), json!({}))
        .await
        .assert_detail(400, "Field required");
    f.ada
        .post(&f.path("/vote/cast"), json!({"nomination_id": 1}))
        .await
        .assert_detail(404, "No open next-up vote");
    // The deadline: no vote is 404 before the date is parsed.
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "garbage"}))
        .await
        .assert_detail(404, "Nominate a book first");
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": 5}))
        .await
        .assert_detail(400, "Input should be a valid string");

    let nom = f.nominate(&f.ada, &f.circe).await["nominations"][0]["id"].clone();
    f.ada
        .post(&f.path("/vote/cast"), json!({"nomination_id": 999_999}))
        .await
        .assert_detail(404, "No nomination with that id");
    let apply = f.path("/vote/apply");
    f.ada
        .post(&apply, json!({"nomination_id": nom, "meeting_at": 5}))
        .await
        .assert_detail(400, "Input should be a valid string");
    // The nomination is checked before the meeting.
    f.ada
        .post(
            &apply,
            json!({"nomination_id": 999_999, "meeting_at": "next Tuesday"}),
        )
        .await
        .assert_detail(404, "No nomination with that id");
    f.ada
        .post(
            &apply,
            json!({"nomination_id": nom, "meeting_at": "next Tuesday"}),
        )
        .await
        .assert_detail(400, "meeting_at must be a date and time");
    // Nothing was applied.
    assert_eq!(f.pick(&f.ada).await, Value::Null);
    assert_ne!(f.vote(&f.ada).await["vote_id"], Value::Null);
}

/// A merged-away work id resolves to its survivor.
#[sqlx::test]
async fn a_merged_work_is_nominated_as_its_survivor(db: PgPool) {
    let f = Fixture::new(db).await;
    let dup = f.app.work("Circe (dup)").await;
    f.app
        .exec(&format!(
            "UPDATE works SET merged_into_id = {} WHERE id = {}",
            f.circe.id, dup.id
        ))
        .await;
    let vote = f.nominate(&f.ada, &dup).await;
    assert_eq!(vote["nominations"][0]["book"]["id"], f.circe.id);
    f.ada
        .post(&f.path("/vote/nominations"), json!({"work_id": f.circe.id}))
        .await
        .assert_detail(400, "That book is already nominated");
}

/// Applying a nomination whose book became the current pick meanwhile keeps the pick: only the
/// meeting changes (cleared when none is given); `set_by`, the note and the history stay, and no
/// `pick_set` is recorded. The vote still closes.
#[sqlx::test]
async fn applying_the_current_pick_only_changes_its_meeting(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "admin").await;
    let nom = f.nominate(&f.ada, &f.circe).await["nominations"][0]["id"].clone();
    let pick = f
        .ada
        .put(
            &f.path("/pick"),
            json!({"work_id": f.circe.id, "note": "Bring snacks", "meeting_at": "2026-09-01T19:00"}),
        )
        .await
        .json();
    let applied = grace
        .post(&f.path("/vote/apply"), json!({"nomination_id": nom}))
        .await;
    applied.assert_status(200);
    let applied = applied.json();
    assert_eq!(applied["pick"]["id"], pick["id"]);
    assert_eq!(applied["pick"]["set_by"], "ada");
    assert_eq!(applied["pick"]["note"], "Bring snacks");
    assert_eq!(applied["pick"]["meeting_at"], Value::Null);
    assert_eq!(applied["vote"]["vote_id"], Value::Null);
    let kinds: Vec<String> = f.events().await.into_iter().map(|e| e.0).collect();
    assert_eq!(kinds, ["nominated", "pick_set", "vote_closed"]);
    assert_eq!(
        f.count("SELECT count(*) FROM club_picks").await,
        1,
        "no history entry"
    );
}

/// The auto-close credits the winning nominator after they left the club; for a deleted
/// account, the pick's `set_by` and the events' actor are NULL (work-plan appendix "club" 9).
#[sqlx::test]
async fn the_auto_close_credits_a_nominator_who_left_or_is_gone(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "member").await;
    let zed = f.member("zed", "member").await;
    let vote = f.nominate(&grace, &f.achilles).await;
    let achilles = nomination(&vote, "The Song of Achilles")["id"].clone();
    f.cast(&f.ada, &achilles).await;
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    grace
        .post(&f.path("/leave"), json!({}))
        .await
        .assert_status(204);
    f.expire_deadline().await;
    f.vote(&zed).await;
    let pick = f.pick(&f.ada).await;
    assert_eq!(pick["book"]["title"], "The Song of Achilles");
    assert_eq!(pick["set_by"], "grace");

    // A second round, won by an account that is deleted before the deadline.
    let vote = f.nominate(&zed, &f.wolf).await;
    let wolf = nomination(&vote, "The Wolf Den")["id"].clone();
    f.cast(&f.ada, &wolf).await;
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    f.app
        .exec(&format!("DELETE FROM users WHERE id = {}", zed.id()))
        .await;
    f.expire_deadline().await;
    let vote = f.vote(&f.ada).await;
    assert_eq!(vote["vote_id"], Value::Null);
    let pick = f.pick(&f.ada).await;
    assert_eq!(pick["book"]["title"], "The Wolf Den");
    assert_eq!(pick["set_by"], Value::Null);
    let feed = f.ada.get(&f.path("/activity")).await.json();
    assert_eq!(feed["items"][0]["kind"], "vote_closed");
    assert_eq!(feed["items"][0]["actor"], Value::Null);
    assert_eq!(feed["items"][0]["mine"], false);
    assert_eq!(feed["items"][1]["kind"], "pick_set");
    assert_eq!(feed["items"][1]["actor"], Value::Null);
}

/// The electorate is the club's current members: a former member's ballot does not count, and
/// counts again when they rejoin; a deleted account's nomination shows no nominator.
#[sqlx::test]
async fn only_current_members_ballots_count(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "member").await;
    let zed = f.member("zed", "member").await;
    let vote = f.nominate(&zed, &f.circe).await;
    let circe = vote["nominations"][0]["id"].clone();
    f.cast(&grace, &circe).await;
    grace
        .post(&f.path("/leave"), json!({}))
        .await
        .assert_status(204);
    let vote = f.vote(&f.ada).await;
    assert_eq!(vote["nominations"][0]["votes"], 0);
    assert_eq!(vote["nominations"][0]["voters"], json!([]));
    assert_eq!(vote["nominations"][0]["nominated_by"], "zed");
    assert_eq!(vote["leader_id"], Value::Null);
    assert_eq!(vote["voted_count"], 0);
    assert_eq!(vote["member_count"], 2);
    assert_eq!(vote["not_voted"], json!(["ada", "zed"]));
    f.app.join(&f.club, &grace, "member").await;
    let vote = f.vote(&f.ada).await;
    assert_eq!(vote["nominations"][0]["voters"], json!(["grace"]));
    assert_eq!(vote["leader_id"], circe);

    f.app
        .exec(&format!("DELETE FROM users WHERE id = {}", zed.id()))
        .await;
    let vote = f.vote(&f.ada).await;
    assert_eq!(vote["nominations"][0]["nominated_by"], Value::Null);
    assert_eq!(vote["member_count"], 2);
}

/// The auto-close commits before the request's own work: a nomination refused afterwards does
/// not undo it (parity). `PATCH vote`, apply, suggestions and the pick do not run it.
#[sqlx::test]
async fn the_auto_close_runs_only_on_read_nominate_and_cast(db: PgPool) {
    let f = Fixture::new(db).await;
    let vote = f.nominate(&f.ada, &f.circe).await;
    let circe = vote["nominations"][0]["id"].clone();
    f.cast(&f.ada, &circe).await;
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    f.expire_deadline().await;
    assert_eq!(f.pick(&f.ada).await, Value::Null);
    f.ada
        .get(&f.path("/vote/suggestions"))
        .await
        .assert_status(200);
    assert_eq!(f.pick(&f.ada).await, Value::Null);
    // The nomination of Circe runs the close first, then is refused: Circe is the pick now.
    f.ada
        .post(&f.path("/vote/nominations"), json!({"work_id": f.circe.id}))
        .await
        .assert_detail(400, "That book is already the club pick");
    assert_eq!(f.pick(&f.ada).await["book"]["title"], "Circe");
    assert_eq!(f.count("SELECT count(*) FROM next_up_votes").await, 1);
    assert_eq!(
        f.count("SELECT count(*) FROM next_up_votes WHERE ended_at IS NULL")
            .await,
        0
    );
}

/// A user in two clubs sees two separate votes, and a nomination of one club is unknown under
/// the other's path (#45 acceptance).
#[sqlx::test]
async fn a_user_in_two_clubs_sees_separate_votes(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.app.user("grace").await;
    let other = f.app.club(&grace, "Other").await;
    f.app.join(&other, &f.ada, "admin").await;
    let other_path = |rest: &str| format!("/api/clubs/{}{rest}", other.slug);

    let mine = f.nominate(&f.ada, &f.circe).await;
    let circe = mine["nominations"][0]["id"].clone();
    let theirs = grace
        .post(
            &other_path("/vote/nominations"),
            json!({"work_id": f.achilles.id}),
        )
        .await;
    theirs.assert_status(201);
    let theirs = theirs.json();
    assert_ne!(theirs["vote_id"], mine["vote_id"]);
    assert_eq!(theirs["nominations"].as_array().unwrap().len(), 1);
    let achilles = theirs["nominations"][0]["id"].clone();

    let seen = f.ada.get(&other_path("/vote")).await.json();
    assert_eq!(
        seen["nominations"][0]["book"]["title"],
        "The Song of Achilles"
    );
    assert_eq!(seen["member_count"], 2);
    f.ada
        .post(&other_path("/vote/cast"), json!({"nomination_id": circe}))
        .await
        .assert_detail(404, "No nomination with that id");
    f.ada
        .post(&other_path("/vote/apply"), json!({"nomination_id": circe}))
        .await
        .assert_detail(404, "No nomination with that id");
    f.ada
        .post(&f.path("/vote/cast"), json!({"nomination_id": achilles}))
        .await
        .assert_detail(404, "No nomination with that id");
    grace
        .get(&f.path("/vote"))
        .await
        .assert_detail(404, "No such club");
    assert_eq!(f.count("SELECT count(*) FROM next_up_ballots").await, 0);
    assert_eq!(f.pick(&f.ada).await, Value::Null);
}

/// Two first nominations that meet at the club lock open one vote between them (the legacy app
/// could open two).
#[sqlx::test]
async fn two_first_nominations_open_one_vote(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "member").await;
    let mut hold = f.app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(f.club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let first = {
        let (client, path, work) = (f.ada.clone(), f.path("/vote/nominations"), f.circe.id);
        tokio::spawn(async move { client.post(&path, json!({"work_id": work})).await })
    };
    wait_for_waiters(&f.app, 1).await;
    let second = {
        let (client, path, work) = (grace.clone(), f.path("/vote/nominations"), f.achilles.id);
        tokio::spawn(async move { client.post(&path, json!({"work_id": work})).await })
    };
    wait_for_waiters(&f.app, 2).await;
    hold.commit().await.unwrap();
    let (first, second) = (first.await.unwrap(), second.await.unwrap());
    first.assert_status(201);
    second.assert_status(201);
    assert_eq!(f.count("SELECT count(*) FROM next_up_votes").await, 1);
    assert_eq!(second.json()["nominations"].as_array().unwrap().len(), 2);
}

/// A burst of concurrent nominations of different books never fails with a 500 and never
/// exceeds the limit: one vote, six nominations, the rest refused.
#[sqlx::test]
async fn a_burst_of_nominations_respects_the_limit(db: PgPool) {
    let f = Fixture::new(db).await;
    let mut tasks = Vec::new();
    for n in 0..9 {
        let work = f.app.work(&format!("Book {n}")).await;
        let (client, path) = (f.ada.clone(), f.path("/vote/nominations"));
        tasks.push(tokio::spawn(async move {
            client.post(&path, json!({"work_id": work.id})).await
        }));
    }
    let mut created = 0;
    for task in tasks {
        let res = task.await.unwrap();
        if res.status.as_u16() == 201 {
            created += 1;
        } else {
            res.assert_detail(400, "This vote already has enough books");
        }
    }
    assert_eq!(created, 6);
    assert_eq!(f.count("SELECT count(*) FROM next_up_votes").await, 1);
    assert_eq!(f.count("SELECT count(*) FROM next_up_nominations").await, 6);
    assert_eq!(
        f.count("SELECT count(*) FROM events WHERE kind = 'nominated'")
            .await,
        6
    );
}

/// Two requests that find the deadline passed at the same time close the vote once: one pick,
/// one `pick_set`, one `vote_closed`.
#[sqlx::test]
async fn a_concurrent_auto_close_applies_once(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "member").await;
    let vote = f.nominate(&grace, &f.achilles).await;
    let achilles = vote["nominations"][0]["id"].clone();
    f.cast(&grace, &achilles).await;
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    f.expire_deadline().await;

    let mut hold = f.app.db.begin().await.unwrap();
    sqlx::query("SELECT id FROM clubs WHERE id = $1 FOR UPDATE")
        .bind(f.club.id)
        .execute(&mut *hold)
        .await
        .unwrap();
    let first = {
        let (client, path) = (f.ada.clone(), f.path("/vote"));
        tokio::spawn(async move { client.get(&path).await })
    };
    wait_for_waiters(&f.app, 1).await;
    let second = {
        let (client, path) = (grace.clone(), f.path("/vote"));
        tokio::spawn(async move { client.get(&path).await })
    };
    wait_for_waiters(&f.app, 2).await;
    hold.commit().await.unwrap();
    for res in [first.await.unwrap(), second.await.unwrap()] {
        res.assert_status(200);
        assert_eq!(res.json()["vote_id"], Value::Null);
    }
    let kinds: Vec<String> = f.events().await.into_iter().map(|e| e.0).collect();
    assert_eq!(kinds, ["nominated", "voted", "pick_set", "vote_closed"]);
    assert_eq!(f.count("SELECT count(*) FROM club_picks").await, 1);
    assert_eq!(f.pick(&f.ada).await["set_by"], "grace");
}

/// A burst of reads, ballots and nominations after the deadline: the vote closes once and the
/// requests that come after it see no open vote (cast) or open a new one (nominate).
#[sqlx::test]
async fn a_burst_after_the_deadline_closes_the_vote_once(db: PgPool) {
    let f = Fixture::new(db).await;
    let grace = f.member("grace", "member").await;
    let vote = f.nominate(&grace, &f.achilles).await;
    let achilles = vote["nominations"][0]["id"].clone();
    f.cast(&grace, &achilles).await;
    f.ada
        .patch(&f.path("/vote"), json!({"closes_at": "2030-01-01T18:00"}))
        .await
        .assert_status(200);
    f.expire_deadline().await;
    let mut tasks = Vec::new();
    for n in 0..8 {
        let client = if n % 2 == 0 {
            f.ada.clone()
        } else {
            grace.clone()
        };
        let path = f.path("");
        let (achilles, wolf) = (achilles.clone(), f.wolf.id);
        tasks.push(tokio::spawn(async move {
            match n % 3 {
                0 => client.get(&format!("{path}/vote")).await,
                1 => {
                    client
                        .post(
                            &format!("{path}/vote/cast"),
                            json!({"nomination_id": achilles}),
                        )
                        .await
                }
                _ => {
                    client
                        .post(
                            &format!("{path}/vote/nominations"),
                            json!({"work_id": wolf}),
                        )
                        .await
                }
            }
        }));
    }
    for task in tasks {
        let res = task.await.unwrap();
        assert!(
            matches!(res.status.as_u16(), 200 | 201 | 400 | 404),
            "{} {}",
            res.status,
            res.text()
        );
    }
    assert_eq!(
        f.count("SELECT count(*) FROM events WHERE kind = 'vote_closed'")
            .await,
        1
    );
    assert_eq!(
        f.count("SELECT count(*) FROM events WHERE kind = 'pick_set'")
            .await,
        1
    );
    assert_eq!(
        f.pick(&f.ada).await["book"]["title"],
        "The Song of Achilles"
    );
    assert!(
        f.count("SELECT count(*) FROM next_up_votes WHERE ended_at IS NULL")
            .await
            <= 1
    );
}
