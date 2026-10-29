//! Friend requests and friendships (FRIENDS, #57; docs/phase3/work-plan.md §4.2, §4.5, §4.9):
//! every transition of the request table, the mutual send and its race, the lookups of a name
//! (also of an account deleted meanwhile), the daily limits, and the work an uncounted route
//! does for an unknown name.

mod common;

use std::net::IpAddr;
use std::time::Duration;

use bookclub_server::friends::{NO_ACCOUNT, PAIR_LOCK_SPACE, pair_key};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn send(from: &TestClient, to: &str) -> TestResponse {
    from.post("/api/friends/requests", json!({ "username": to }))
        .await
}

async fn accept(by: &TestClient, from: &str) -> TestResponse {
    by.post(&format!("/api/friends/requests/{from}/accept"), json!({}))
        .await
}

async fn drop_request(by: &TestClient, other: &str) -> TestResponse {
    by.delete(&format!("/api/friends/requests/{other}")).await
}

async fn remove(by: &TestClient, friend: &str) -> TestResponse {
    by.delete(&format!("/api/friends/list/{friend}")).await
}

async fn friends_of(who: &TestClient) -> Value {
    let res = who.get("/api/friends").await;
    res.assert_status(200);
    res.json()
}

/// The usernames of one list of a `FriendsOut`.
fn names(out: &Value, list: &str) -> Vec<String> {
    out[list]
        .as_array()
        .unwrap_or_else(|| panic!("no list {list} in {out}"))
        .iter()
        .map(|item| {
            item["user"]["username"]
                .as_str()
                .or(item["username"].as_str())
                .unwrap()
                .to_owned()
        })
        .collect()
}

/// The pair's row as (requester, addressee, accepted), or `None`.
async fn row(app: &TestApp, a: &TestClient, b: &TestClient) -> Option<(i64, i64, bool)> {
    sqlx::query_as::<_, (i64, i64, bool)>(
        "SELECT requester_id, addressee_id, accepted_at IS NOT NULL FROM friendships
         WHERE least(requester_id, addressee_id) = least($1::int8, $2::int8)
           AND greatest(requester_id, addressee_id) = greatest($1::int8, $2::int8)",
    )
    .bind(a.id())
    .bind(b.id())
    .fetch_optional(&app.db)
    .await
    .unwrap()
}

/// None → pending → friends, each step answered with the caller's whole `FriendsOut`; a repeat
/// changes nothing.
#[sqlx::test]
async fn a_request_becomes_a_friendship_when_it_is_accepted(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let empty = json!({"friends": [], "incoming": [], "outgoing": [], "blocked": [],
                       "club_invites": []});
    assert_eq!(friends_of(&ada).await, empty);

    let res = send(&ada, "bob").await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(names(&out, "outgoing"), ["bob"]);
    assert_eq!(out["outgoing"][0]["user"]["id"], bob.id());
    assert!(
        out["outgoing"][0]["created_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    assert_eq!(names(&out, "friends"), Vec::<String>::new());
    assert_eq!(
        row(&app, &ada, &bob).await,
        Some((ada.id(), bob.id(), false))
    );
    assert_eq!(names(&friends_of(&bob).await, "incoming"), ["ada"]);

    // Sending again changes nothing.
    let again = send(&ada, "bob").await;
    again.assert_status(200);
    assert_eq!(again.json(), out);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM friendships").await, 1);

    let res = accept(&bob, "ada").await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(names(&out, "friends"), ["ada"]);
    assert_eq!(names(&out, "incoming"), Vec::<String>::new());
    let friend = &out["friends"][0];
    assert!(friend["since"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        (
            &friend["currently_reading_count"],
            &friend["currently_reading"]
        ),
        (&json!(0), &json!([]))
    );
    assert_eq!(
        row(&app, &ada, &bob).await,
        Some((ada.id(), bob.id(), true))
    );
    assert_eq!(names(&friends_of(&ada).await, "friends"), ["bob"]);
    assert_eq!(
        names(&friends_of(&ada).await, "outgoing"),
        Vec::<String>::new()
    );

    // Sending to a friend changes nothing either, from both sides.
    for (from, to) in [(&ada, "bob"), (&bob, "ada")] {
        let res = send(from, to).await;
        res.assert_status(200);
        assert_eq!(names(&res.json(), "friends"), [to]);
    }
    assert_eq!(
        row(&app, &ada, &bob).await,
        Some((ada.id(), bob.id(), true))
    );
}

/// §6 answer 20: a request to someone who already asked the caller accepts theirs.
#[sqlx::test]
async fn sending_back_accepts_the_request(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    send(&ada, "bob").await.assert_status(200);
    let res = send(&bob, "ada").await;
    res.assert_status(200);
    assert_eq!(names(&res.json(), "friends"), ["ada"]);
    assert_eq!(names(&res.json(), "incoming"), Vec::<String>::new());
    assert_eq!(
        row(&app, &ada, &bob).await,
        Some((ada.id(), bob.id(), true))
    );
    assert_eq!(app.scalar_i64("SELECT count(*) FROM friendships").await, 1);
}

/// Two people asking each other at the same moment end as friends, in one row.
#[sqlx::test]
async fn concurrent_requests_to_each_other_end_as_one_friendship(db: PgPool) {
    let app = TestApp::new(db).await;
    for round in 0..5 {
        let ada = app.user(&format!("ada{round}")).await;
        let bob = app.user(&format!("bob{round}")).await;
        let tasks =
            [(ada.clone(), bob.username()), (bob.clone(), ada.username())].map(|(from, to)| {
                let to = to.to_owned();
                tokio::spawn(async move { send(&from, &to).await })
            });
        for task in tasks {
            task.await.unwrap().assert_status(200);
        }
        let (_, _, accepted) = row(&app, &ada, &bob).await.expect("a row");
        assert!(accepted, "round {round}: the pair is not friends");
        assert_eq!(names(&friends_of(&ada).await, "friends"), [bob.username()]);
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM friendships").await, 5);
}

/// Decline (the addressee) and cancel (the requester) are one route; the request goes without a
/// trace and either may ask again.
#[sqlx::test]
async fn decline_and_cancel_delete_the_request(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;

    send(&ada, "bob").await.assert_status(200);
    let res = drop_request(&bob, "ada").await;
    res.assert_status(200);
    assert_eq!(names(&res.json(), "incoming"), Vec::<String>::new());
    assert_eq!(row(&app, &ada, &bob).await, None);
    // The sender sees the request gone, as after a cancel, and may ask again.
    assert_eq!(
        names(&friends_of(&ada).await, "outgoing"),
        Vec::<String>::new()
    );
    send(&ada, "bob").await.assert_status(200);

    let res = drop_request(&ada, "bob").await;
    res.assert_status(200);
    assert_eq!(names(&res.json(), "outgoing"), Vec::<String>::new());
    assert_eq!(row(&app, &ada, &bob).await, None);
    assert_eq!(
        names(&friends_of(&bob).await, "incoming"),
        Vec::<String>::new()
    );
    send(&bob, "ada").await.assert_status(200);
    assert_eq!(
        row(&app, &ada, &bob).await,
        Some((bob.id(), ada.id(), false))
    );
}

/// Accept, decline and cancel act only on a pending request they apply to: anything else is 404
/// `No such friend request`, and nothing changes.
#[sqlx::test]
async fn accept_decline_and_cancel_need_their_request(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    let no_request = |res: TestResponse| {
        res.assert_detail(404, "No such friend request");
    };

    // No request at all, an unknown name, a NUL character, the caller.
    for name in ["bob", "nobody_here", "bob%00", "ada"] {
        no_request(accept(&ada, name).await);
        no_request(drop_request(&ada, name).await);
    }
    // The caller's own request cannot be accepted by the caller.
    send(&ada, "bob").await.assert_status(200);
    no_request(accept(&ada, "bob").await);
    // A third account has no request with either.
    no_request(accept(&cy, "ada").await);
    no_request(drop_request(&cy, "ada").await);
    assert_eq!(
        row(&app, &ada, &bob).await,
        Some((ada.id(), bob.id(), false))
    );
    // A friendship is no request.
    accept(&bob, "ada").await.assert_status(200);
    no_request(accept(&bob, "ada").await);
    no_request(drop_request(&bob, "ada").await);
    no_request(drop_request(&ada, "bob").await);
    assert_eq!(
        row(&app, &ada, &bob).await,
        Some((ada.id(), bob.id(), true))
    );
}

/// Removing a friend ends the friendship for both; it needs a friendship.
#[sqlx::test]
async fn removing_a_friend_ends_the_friendship(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    let no_friend = |res: TestResponse| {
        res.assert_detail(404, "No friend with that name");
    };
    for name in ["bob", "nobody_here", "bob%00", "ada"] {
        no_friend(remove(&ada, name).await);
    }
    // A pending request, in either direction, is no friendship.
    send(&ada, "bob").await.assert_status(200);
    no_friend(remove(&ada, "bob").await);
    no_friend(remove(&bob, "ada").await);
    accept(&bob, "ada").await.assert_status(200);
    no_friend(remove(&cy, "ada").await);

    let res = remove(&bob, "ada").await;
    res.assert_status(200);
    assert_eq!(names(&res.json(), "friends"), Vec::<String>::new());
    assert_eq!(row(&app, &ada, &bob).await, None);
    assert_eq!(
        names(&friends_of(&ada).await, "friends"),
        Vec::<String>::new()
    );
    no_friend(remove(&ada, "bob").await);
    // Either may ask again.
    send(&ada, "bob").await.assert_status(200);
}

/// A request names another existing account by its exact username (case and surrounding spaces
/// ignored); the caller's own name is 400, an unknown or deleted one 404.
#[sqlx::test]
async fn a_request_names_another_existing_account(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let gone = app.user("gone").await;
    app.exec(&format!("DELETE FROM users WHERE id = {}", gone.id()))
        .await;

    send(&bob, " Ada ").await.assert_status(200);
    assert_eq!(names(&friends_of(&ada).await, "incoming"), ["bob"]);
    accept(&ada, "%20BOB%20").await.assert_status(200);
    assert_eq!(names(&friends_of(&ada).await, "friends"), ["bob"]);

    for name in ["ada", " ADA"] {
        send(&ada, name)
            .await
            .assert_detail(400, "That is your own account");
    }
    for name in ["nobody_here", "gone", ""] {
        send(&ada, name)
            .await
            .assert_detail(404, "No account with that name");
    }
    ada.post("/api/friends/requests", json!({}))
        .await
        .assert_detail(400, "Field required");
    // The JSON body's rule for U+0000 (conventions §4.3), before any lookup.
    send(&ada, "bob\u{0}")
        .await
        .assert_detail(400, "Invalid value");
    assert_eq!(app.scalar_i64("SELECT count(*) FROM friendships").await, 1);
}

/// 20 requests and blocks per account a day (§4.9), unknown names and repeats included; the 21st
/// is 429 with `Retry-After`. Other accounts keep their own budget.
#[sqlx::test]
async fn twenty_requests_a_day(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    for i in 0..10 {
        send(&ada, &format!("nobody{i}"))
            .await
            .assert_detail(404, "No account with that name");
    }
    for _ in 0..9 {
        send(&ada, "bob").await.assert_status(200);
    }
    // The block of someone without a request or friendship counts too.
    ada.put("/api/friends/blocks/cy", json!({}))
        .await
        .assert_status(200);
    let res = send(&ada, "bob").await;
    res.assert_detail(429, "Slow down a little. Try again later.");
    let wait: u64 = res.header("retry-after").unwrap().parse().unwrap();
    assert!((86_000..=86_400).contains(&wait), "{wait}");
    send(&ada, "nobody_here")
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
    // Accepting, declining, removing and unblocking are not counted.
    send(&bob, "ada").await.assert_status(200);
    remove(&ada, "bob").await.assert_status(200);
    ada.delete("/api/friends/blocks/cy")
        .await
        .assert_status(200);
    send(&cy, "ada").await.assert_status(200);
    drop_request(&ada, "cy").await.assert_status(200);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM friendships").await, 0);
}

/// Every account seen from one address shares 60 counted requests and blocks a day (FRIENDS
/// security review): accounts made for the purpose, each with its own 20, cannot multiply the
/// probing for names. A block of someone who already asked stays free; another address has its
/// own budget.
#[sqlx::test]
async fn sixty_requests_a_day_from_one_address(db: PgPool) {
    let app = TestApp::new(db).await;
    let home: IpAddr = "203.0.113.7".parse().unwrap();
    let mut accounts = Vec::new();
    for name in ["ada", "bob", "cy", "dee"] {
        accounts.push(app.user(name).await.with_ip(home));
    }
    let eve = app
        .user("eve")
        .await
        .with_ip("198.51.100.9".parse().unwrap());
    send(&eve, "dee").await.assert_status(200);
    for (i, who) in accounts[..3].iter().enumerate() {
        for j in 0..20 {
            send(who, &format!("nobody_{i}_{j}"))
                .await
                .assert_detail(404, "No account with that name");
        }
    }
    let dee = &accounts[3];
    send(dee, "nobody_here")
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
    dee.put("/api/friends/blocks/nobody_here", json!({}))
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
    dee.put("/api/friends/blocks/eve", json!({}))
        .await
        .assert_status(200);
    let away = dee.clone().with_ip("198.51.100.10".parse().unwrap());
    send(&away, "nobody_here")
        .await
        .assert_detail(404, "No account with that name");
}

/// Accept, decline or cancel, remove and unblock are not counted (§4.9), so an unknown name, or
/// the caller's own, must not answer sooner than an existing one (FRIENDS security review): it
/// takes the turn of the caller and `friends::NO_ACCOUNT` and runs the same statements with it.
/// Holding that turn by hand holds the answer, as holding a pair's turn holds an existing name's.
#[sqlx::test]
async fn an_unknown_name_answers_after_the_same_work(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    for (name, other) in [
        ("nobody_here", NO_ACCOUNT),
        ("ada", NO_ACCOUNT),
        ("bob", bob.id()),
    ] {
        for route in ["accept", "decline", "remove", "unblock"] {
            let mut turn = app.db.begin().await.unwrap();
            sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2))")
                .bind(PAIR_LOCK_SPACE)
                .bind(pair_key(ada.id(), other))
                .execute(&mut *turn)
                .await
                .unwrap();
            let task = tokio::spawn({
                let ada = ada.clone();
                async move {
                    match route {
                        "accept" => accept(&ada, name).await,
                        "decline" => drop_request(&ada, name).await,
                        "remove" => remove(&ada, name).await,
                        _ => ada.delete(&format!("/api/friends/blocks/{name}")).await,
                    }
                }
            });
            wait_for_a_lock(&app).await;
            assert!(
                !task.is_finished(),
                "{route} of {name} answered before its turn"
            );
            turn.rollback().await.unwrap();
            let res = task.await.unwrap();
            match route {
                "accept" | "decline" => res.assert_detail(404, "No such friend request"),
                "remove" => res.assert_detail(404, "No friend with that name"),
                _ => res.assert_status(200),
            };
        }
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM friendships").await, 0);
}

/// §6 answer 27: a suspension changes nothing about friends; the friend stays, without a marker,
/// and a request to the suspended account stays.
#[sqlx::test]
async fn a_suspended_account_stays_a_friend(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    send(&ada, "bob").await.assert_status(200);
    accept(&bob, "ada").await.assert_status(200);
    send(&cy, "bob").await.assert_status(200);
    app.exec(&format!(
        "UPDATE users SET suspended_at = now() WHERE id = {}",
        bob.id()
    ))
    .await;
    let out = friends_of(&ada).await;
    assert_eq!(names(&out, "friends"), ["bob"]);
    assert_eq!(
        out["friends"][0]["user"],
        json!({"id": bob.id(), "username": "bob", "avatar_url": null})
    );
    assert_eq!(names(&friends_of(&cy).await, "outgoing"), ["bob"]);
}

/// An account deleted while a request or a block names it: the write waits for the deletion's
/// lock on the account (`friends::lock_pair` takes both accounts' rows `FOR KEY SHARE` before
/// it writes) and then answers as for an unknown name. Without that lock the write would insert
/// its row, wait in the foreign-key check instead, and fail with a 500.
#[sqlx::test]
async fn a_request_or_a_block_waits_for_a_deletion_in_flight(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for route in ["request", "block"] {
        let gone = app.user(&format!("gone_{route}")).await;
        // What `auth::deletion::delete_user` holds before its cascade.
        let mut deletion = app.db.begin().await.unwrap();
        sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
            .bind(gone.id())
            .execute(&mut *deletion)
            .await
            .unwrap();
        let task = tokio::spawn({
            let ada = ada.clone();
            let name = gone.username().to_owned();
            async move {
                match route {
                    "request" => send(&ada, &name).await,
                    _ => {
                        ada.put(&format!("/api/friends/blocks/{name}"), json!({}))
                            .await
                    }
                }
            }
        });
        wait_for_a_lock(&app).await;
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(gone.id())
            .execute(&mut *deletion)
            .await
            .unwrap();
        deletion.commit().await.unwrap();
        task.await
            .unwrap()
            .assert_detail(404, "No account with that name");
    }
    assert_eq!(app.scalar_i64("SELECT count(*) FROM friendships").await, 0);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM user_blocks").await, 0);
}

/// Waits until a session of the test database waits for a lock.
async fn wait_for_a_lock(app: &TestApp) {
    for _ in 0..500 {
        let waiting = app
            .scalar_i64(
                "SELECT count(*) FROM pg_stat_activity
                 WHERE datname = current_database() AND wait_event_type = 'Lock'",
            )
            .await;
        if waiting >= 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the write never waited for a lock");
}
