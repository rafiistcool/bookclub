//! Blocking (FRIENDS, #57; docs/phase3/work-plan.md §4.3, §6 answer 21): silent, it ends the
//! friendship and the pair's open friend invites, hides the blocked account's later requests,
//! and leaves the clubs both are in alone. Unblocking drops what was hidden.

mod common;

use bookclub_server::visibility::{Relation, relation};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn send(from: &TestClient, to: &str) -> TestResponse {
    from.post("/api/friends/requests", json!({ "username": to }))
        .await
}

async fn block(by: &TestClient, name: &str) -> TestResponse {
    by.put(&format!("/api/friends/blocks/{name}"), json!({}))
        .await
}

async fn unblock(by: &TestClient, name: &str) -> TestResponse {
    by.delete(&format!("/api/friends/blocks/{name}")).await
}

async fn friends_of(who: &TestClient) -> Value {
    let res = who.get("/api/friends").await;
    res.assert_status(200);
    res.json()
}

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

async fn pair_rows(app: &TestApp, a: &TestClient, b: &TestClient) -> i64 {
    app.scalar_i64(&format!(
        "SELECT count(*) FROM friendships
         WHERE least(requester_id, addressee_id) = least({a}, {b})
           AND greatest(requester_id, addressee_id) = greatest({a}, {b})",
        a = a.id(),
        b = b.id()
    ))
    .await
}

/// A block deletes the pair's row in every state: nothing, a request either way, a friendship.
#[sqlx::test]
async fn a_block_ends_whatever_the_pair_had(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for state in ["none", "asked", "was_asked", "friends"] {
        let other = app.user(&format!("bob_{state}")).await;
        let name = other.username().to_owned();
        if matches!(state, "asked" | "friends") {
            send(&ada, &name).await.assert_status(200);
        }
        if matches!(state, "was_asked" | "friends") {
            send(&other, "ada").await.assert_status(200);
        }
        let res = block(&ada, &name).await;
        res.assert_status(200);
        let out = res.json();
        assert_eq!(names(&out, "blocked")[0], name, "{state}");
        assert!(
            out["blocked"][0]["created_at"]
                .as_str()
                .unwrap()
                .ends_with('Z')
        );
        assert_eq!(out["blocked"][0].as_object().unwrap().len(), 2, "no avatar");
        for list in ["friends", "incoming", "outgoing"] {
            assert!(!names(&out, list).contains(&name), "{state}: {list}");
        }
        assert_eq!(pair_rows(&app, &ada, &other).await, 0, "{state}");
        // The blocked account sees the request or friendship gone, and nothing else.
        let theirs = friends_of(&other).await;
        for list in ["friends", "incoming", "outgoing", "blocked"] {
            assert_eq!(
                names(&theirs, list),
                Vec::<String>::new(),
                "{state}: {list}"
            );
        }
        // Blocking twice is harmless.
        block(&ada, &name).await.assert_status(200);
    }
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM user_blocks WHERE user_id = {}",
            ada.id()
        ))
        .await,
        4
    );
    // The caller's own name is 400; an unknown name (also one with NUL) 404.
    block(&ada, "ada")
        .await
        .assert_detail(400, "That is your own account");
    for name in ["nobody_here", "bob%00"] {
        block(&ada, name)
            .await
            .assert_detail(404, "No account with that name");
    }
}

/// The blocked account's request after the block is stored and looks exactly like a request
/// nobody answers (200, in its `outgoing`); the blocker never sees it and cannot act on it. The
/// blocker must unblock before asking themselves.
#[sqlx::test]
async fn a_request_after_the_block_is_hidden_from_the_blocker(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    app.user("dan").await;
    block(&ada, "bob").await.assert_status(200);

    // Bob's request to Ada, who blocked him, and Cy's to Dan, who simply never answers.
    let hidden = send(&bob, "ada").await;
    let ignored = send(&cy, "dan").await;
    hidden.assert_status(200);
    ignored.assert_status(200);
    let strip = |mut out: Value| {
        out["outgoing"][0]["created_at"] = Value::Null;
        out["outgoing"][0]["user"]["id"] = Value::Null;
        out["outgoing"][0]["user"]["username"] = Value::Null;
        out
    };
    assert_eq!(strip(hidden.json()), strip(ignored.json()));
    assert_eq!(names(&hidden.json(), "outgoing"), ["ada"]);
    assert_eq!(names(&friends_of(&bob).await, "outgoing"), ["ada"]);
    assert_eq!(pair_rows(&app, &ada, &bob).await, 1);

    let mine = friends_of(&ada).await;
    assert_eq!(names(&mine, "incoming"), Vec::<String>::new());
    assert_eq!(names(&mine, "blocked"), ["bob"]);
    ada.post("/api/friends/requests/bob/accept", json!({}))
        .await
        .assert_detail(404, "No such friend request");
    ada.delete("/api/friends/requests/bob")
        .await
        .assert_detail(404, "No such friend request");
    send(&ada, "bob")
        .await
        .assert_detail(409, "You blocked this account");
    assert_eq!(pair_rows(&app, &ada, &bob).await, 1);
    // Bob may cancel his request, as any sender may.
    let res = bob.delete("/api/friends/requests/ada").await;
    res.assert_status(200);
    assert_eq!(names(&res.json(), "outgoing"), Vec::<String>::new());
    assert_eq!(pair_rows(&app, &ada, &bob).await, 0);
}

/// Unblocking deletes the block and the requests the blocked account sent meanwhile: it never
/// surfaces an old request. It always answers 200 with the caller's friends, so an unknown name
/// and an existing name the caller never blocked look the same.
#[sqlx::test]
async fn unblocking_drops_the_hidden_requests(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    send(&cy, "ada").await.assert_status(200);
    block(&ada, "bob").await.assert_status(200);
    send(&bob, "ada").await.assert_status(200);

    let res = unblock(&ada, "bob").await;
    res.assert_status(200);
    let out = res.json();
    assert_eq!(names(&out, "blocked"), Vec::<String>::new());
    assert_eq!(
        names(&out, "incoming"),
        ["cy"],
        "only the visible request stays"
    );
    assert_eq!(pair_rows(&app, &ada, &bob).await, 0);
    assert_eq!(
        names(&friends_of(&bob).await, "outgoing"),
        Vec::<String>::new()
    );
    // Bob may ask again, and now Ada sees it.
    send(&bob, "ada").await.assert_status(200);
    assert_eq!(names(&friends_of(&ada).await, "incoming"), ["bob", "cy"]);

    // Unblocking someone never blocked (whose visible request stays), an unknown name, a name
    // with NUL and the caller: the same 200.
    let expected = friends_of(&ada).await;
    for name in ["bob", "dan", "nobody_here", "bob%00", "ada"] {
        let res = unblock(&ada, name).await;
        res.assert_status(200);
        assert_eq!(res.json(), expected, "{name}");
    }
    app.user("dan").await;
    assert_eq!(unblock(&ada, "dan").await.json(), expected);
}

/// Blocking whoever already asked costs nothing against the daily limit (§4.9): 25 requesters
/// in one day can all be blocked, and the budget stays untouched.
#[sqlx::test]
async fn blocking_twenty_five_requesters_in_a_day_works(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    for i in 0..25 {
        let fan = app.user(&format!("fan{i}")).await;
        send(&fan, "ada").await.assert_status(200);
    }
    for i in 0..25 {
        block(&ada, &format!("fan{i}")).await.assert_status(200);
    }
    let out = friends_of(&ada).await;
    assert_eq!(names(&out, "blocked").len(), 25);
    assert_eq!(names(&out, "incoming"), Vec::<String>::new());
    // A block of someone without a request counts; an unknown name too.
    let key = format!("u:{}", ada.id());
    assert!(!app.state.limits.friend_request_user.is_limited(&key));
    for i in 0..19 {
        app.user(&format!("other{i}")).await;
        block(&ada, &format!("other{i}")).await.assert_status(200);
    }
    block(&ada, "nobody_here")
        .await
        .assert_detail(404, "No account with that name");
    app.user("one_more").await;
    block(&ada, "one_more")
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
    block(&ada, "nobody_else")
        .await
        .assert_detail(429, "Slow down a little. Try again later.");
}

/// A block revokes the friend invites the pair made for each other, in both directions.
#[sqlx::test]
async fn a_block_revokes_the_pairs_friend_invites(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    send(&ada, "bob").await.assert_status(200);
    send(&bob, "ada").await.assert_status(200);
    send(&ada, "cy").await.assert_status(200);
    send(&cy, "ada").await.assert_status(200);
    let ada_club = app.club(&ada, "Ada's").await;
    let bob_club = app.club(&bob, "Bob's").await;
    let invite = |admin: &TestClient, club: &TestClub, friend: &str| {
        let (admin, path) = (admin.clone(), format!("/api/clubs/{}/invites", club.slug));
        let body = json!({"kind": "friend", "username": friend});
        async move {
            let res = admin.post(&path, body).await;
            res.assert_status(201);
            res.json()["code"].as_str().unwrap().to_owned()
        }
    };
    let for_bob = invite(&ada, &ada_club, "bob").await;
    let for_ada = invite(&bob, &bob_club, "ada").await;
    let for_cy = invite(&ada, &ada_club, "cy").await;

    let res = block(&bob, "ada").await;
    res.assert_status(200);
    assert_eq!(res.json()["club_invites"], json!([]));
    assert_eq!(friends_of(&ada).await["club_invites"], json!([]));
    for (who, code) in [(&bob, &for_bob), (&ada, &for_ada)] {
        who.post(&format!("/api/invites/{code}/accept"), json!({}))
            .await
            .assert_detail(400, "That invite code is not valid");
    }
    let listed = ada
        .get(&format!("/api/clubs/{}/invites", ada_club.slug))
        .await
        .json();
    let active: Vec<(String, bool)> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["code"].as_str().unwrap().to_owned(), i["active"] == true))
        .collect();
    assert_eq!(active, [(for_cy, true), (for_bob, false)]);
}

/// §6 answer 21: a block does not hide anything in a club both are in, now or later. Joining
/// one of the blocker's clubs later, through a third admin's friend invite, makes the blocked
/// account audience again.
#[sqlx::test]
async fn a_block_leaves_the_clubs_alone(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    let cy = app.user("cy").await;
    let shared = app.club(&ada, "Shared").await;
    app.join(&shared, &bob, "member").await;
    block(&ada, "bob").await.assert_status(200);
    let mut conn = app.db.acquire().await.unwrap();
    assert_eq!(
        relation(&mut conn, bob.id(), ada.id()).await.unwrap(),
        Relation::Audience
    );
    bob.get("/api/shelf?username=ada").await.assert_status(200);

    // Ada's second club, which Bob joins later through Cy, its admin and Bob's friend.
    let later = app.club(&ada, "Later").await;
    app.join(&later, &cy, "admin").await;
    app.exec(&format!(
        "DELETE FROM club_memberships WHERE club_id = {} AND user_id = {}",
        shared.id,
        bob.id()
    ))
    .await;
    assert_eq!(
        relation(&mut conn, bob.id(), ada.id()).await.unwrap(),
        Relation::Stranger
    );
    send(&cy, "bob").await.assert_status(200);
    send(&bob, "cy").await.assert_status(200);
    let res = cy
        .post(
            &format!("/api/clubs/{}/invites", later.slug),
            json!({"kind": "friend", "username": "bob"}),
        )
        .await;
    res.assert_status(201);
    let code = res.json()["code"].as_str().unwrap().to_owned();
    bob.post(&format!("/api/invites/{code}/accept"), json!({}))
        .await
        .assert_status(200);
    assert_eq!(
        relation(&mut conn, bob.id(), ada.id()).await.unwrap(),
        Relation::Audience
    );
    assert_eq!(
        relation(&mut conn, ada.id(), bob.id()).await.unwrap(),
        Relation::Audience
    );
    bob.get("/api/shelf?username=ada").await.assert_status(200);
    // The block itself stands.
    assert_eq!(names(&friends_of(&ada).await, "blocked"), ["bob"]);
}

/// A blocked account can block back (its own row); either must unblock before asking the other.
#[sqlx::test]
async fn a_blocked_account_can_block_back(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let bob = app.user("bob").await;
    block(&ada, "bob").await.assert_status(200);
    let res = block(&bob, "ada").await;
    res.assert_status(200);
    assert_eq!(names(&res.json(), "blocked"), ["ada"]);
    assert_eq!(app.scalar_i64("SELECT count(*) FROM user_blocks").await, 2);
    send(&bob, "ada")
        .await
        .assert_detail(409, "You blocked this account");
    // Ada unblocks; Bob's block stands, so Ada's request is hidden from Bob.
    unblock(&ada, "bob").await.assert_status(200);
    send(&ada, "bob").await.assert_status(200);
    assert_eq!(
        names(&friends_of(&bob).await, "incoming"),
        Vec::<String>::new()
    );
    assert_eq!(names(&friends_of(&ada).await, "outgoing"), ["bob"]);
}
