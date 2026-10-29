//! The #46 matrix, checked for completeness (#49; docs/phase1/conventions.md §6.5): every kind of
//! personal data has a self, co-member, friend, stranger and unknown-name case, run end to end
//! against one world through the routes of every package that serves it.
//!
//! The kinds are enumerated, and nothing new can slip past them:
//! - [`check`] matches every [`Kind`], and each kind's function matches every [`Case`]: a new
//!   kind does not compile until it has its five cases.
//! - Every kind must be named by the discovery tables below, or it is never constructed and
//!   `dead_code` fails clippy. The matrix runs every kind those tables name.
//! - `every_personal_column_route_and_rating_field_is_in_the_matrix` reads the live schema and
//!   the OpenAPI document. Every foreign key to `users`, every operation with a `username`
//!   parameter and every schema with a rating or take field must be in a table below, either
//!   with the kinds that serve it or with the reason it is not personal data. So a new
//!   personal table, `?username=` route or rating field fails until the matrix covers it.
//! - `the_checklist_has_a_row_per_kind` compares the kinds with the rows of the checklist's
//!   "#46 visibility matrix, as built" table (docs/phase1/parity-checklist.md), by
//!   [`Kind::label`].
//!
//! The friend (FRIENDS, #57) is in no club: friends are audience for personal data, never for club
//! data. A stranger is each of `otherclub` (shares a club with the co-member but not with the
//! owner), `stranger` (in no club, a friend of the friend), `former` (left the owner's club),
//! `asked` and `requested` (a pending friend request, each direction), `exfriend` (a friendship
//! that was removed) and `blocked` (a friendship the owner's block ended); see
//! `tests/common/visibility.rs`.
//! A case is "not applicable" only for the unknown name, and only where the data has no name or id
//! a viewer could pass. The reason is printed with the matrix.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use common::routes::operations;
use common::visibility::{UNKNOWN, Viewers, assert_hidden_like_unknown};
use common::*;
use regex::Regex;
use serde_json::{Value, json};
use sqlx::PgPool;

/// A kind of personal data: shown to the owner's audience, or kept for the owner alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// Shelf entries with status, progress, `dnf_reason`, ratings and takes:
    /// `GET /api/shelf?username=`.
    Shelf,
    /// The three favourites: `favorites` of `GET /api/shelf?username=`, the owner's
    /// `/api/auth/me/favorites` and the book page's `favorite_position`.
    Favourites,
    /// The physical shelf: `GET` and `PUT /api/shelf/physical-layout?username=`.
    Layout,
    /// Quotes: `GET /api/quotes?username=` and `?work=`.
    Quotes,
    /// Personal stats: `GET /api/stats?username=`.
    Stats,
    /// Diary entries, replies and reactions, and `author_rating`: the book's thread, the diary
    /// feed, and the post routes.
    Diary,
    /// Personal events in the home feed and the club feeds, also with `?username=`.
    Activity,
    /// Readers, ratings, takes and the quote count of `GET /api/books/works/{work_id}`.
    BookPage,
    /// The readers of a club's current pick.
    PickReaders,
    /// Members' shelves inside a club: overlap, suggestions, the members' reading previews and
    /// the club stats.
    ClubShelves,
    /// The search history: its owner's alone.
    SearchHistory,
    /// Goodreads import jobs: their owner's alone.
    ImportJobs,
    /// Friends, requests and blocks (FRIENDS, #57): `GET /api/friends` and the `{username}`
    /// routes under `/api/friends`. Their owner's alone: a friend sees the owner in their own
    /// list, never the owner's other friends, requests or blocks.
    Friends,
}

impl Kind {
    /// Its row in the checklist's #46 table: the Kind column up to a parenthesis.
    fn label(self) -> &'static str {
        match self {
            Kind::Shelf => "Shelf",
            Kind::Favourites => "Favourites",
            Kind::Layout => "Physical layout",
            Kind::Quotes => "Quotes",
            Kind::Stats => "Stats",
            Kind::Diary => "Diary",
            Kind::Activity => "Activity",
            Kind::BookPage => "Book page",
            Kind::PickReaders => "Pick readers",
            Kind::ClubShelves => "Club shelves",
            Kind::SearchHistory => "Search history",
            Kind::ImportJobs => "Import jobs",
            Kind::Friends => "Friends",
        }
    }
}

/// The viewer cases of conventions §6.5, and the friend of Phase 3 (#57).
#[derive(Clone, Copy, Debug)]
enum Case {
    Myself,
    CoMember,
    Friend,
    Stranger,
    UnknownName,
}

const CASES: [Case; 5] = [
    Case::Myself,
    Case::CoMember,
    Case::Friend,
    Case::Stranger,
    Case::UnknownName,
];

/// How the matrix covers one case of one kind.
#[derive(Debug)]
enum Covered {
    Checked,
    /// Only for [`Case::UnknownName`]: the data has no name or id to pass.
    NotApplicable(&'static str),
}

/// Where a schema column, route or field belongs.
enum Class {
    /// Personal data, served as these kinds.
    Kinds(&'static [Kind]),
    /// Not personal data, and why (with the tests that cover it).
    Not(&'static str),
}

use Class::{Kinds, Not};

/// Every foreign key to `users`, as `table.column`.
const USER_COLUMNS: &[(&str, Class)] = &[
    (
        "shelf_entries.user_id",
        Kinds(&[
            Kind::Shelf,
            Kind::Stats,
            Kind::Activity,
            Kind::BookPage,
            Kind::PickReaders,
            Kind::ClubShelves,
        ]),
    ),
    ("user_favorites.user_id", Kinds(&[Kind::Favourites])),
    ("physical_shelf_layouts.user_id", Kinds(&[Kind::Layout])),
    (
        "quotes.user_id",
        Kinds(&[
            Kind::Quotes,
            Kind::Stats,
            Kind::Activity,
            Kind::BookPage,
            Kind::ClubShelves,
        ]),
    ),
    // Diary entries (club_id NULL). Club posts are club data: posts_visibility.rs
    // club_posts_stay_inside_their_club.
    ("posts.author_id", Kinds(&[Kind::Diary])),
    ("post_reactions.user_id", Kinds(&[Kind::Diary])),
    // Personal events (club_id NULL); club events are club data (club_activity.rs).
    ("events.actor_id", Kinds(&[Kind::Activity])),
    ("search_history.user_id", Kinds(&[Kind::SearchHistory])),
    ("import_jobs.user_id", Kinds(&[Kind::ImportJobs])),
    (
        "sessions.user_id",
        Not("account: listed and revoked by their owner only (auth_sessions.rs)"),
    ),
    (
        "email_tokens.user_id",
        Not("account: one-time links, never served (auth_email.rs, auth_reset.rs)"),
    ),
    (
        "club_memberships.user_id",
        Not("club data: members of that club only (clubs_non_member.rs, clubs_members.rs)"),
    ),
    (
        "club_memberships.invited_by",
        Not("club data: members of that club only"),
    ),
    (
        "club_removals.user_id",
        Not("club data: never served, it blocks invites (clubs_members.rs)"),
    ),
    (
        "club_invites.created_by",
        Not(
            "invite data: `InviteOut.created_by`, for the club's admins only, or instance admins \
             for a sign-up invite (clubs_invites.rs)",
        ),
    ),
    (
        "club_invite_uses.user_id",
        Not(
            "invite data: who redeemed an invite, `InviteOut.used_by`, for the club's admins \
             only, or instance admins for a sign-up invite (clubs_invites.rs)",
        ),
    ),
    (
        "clubs.created_by",
        Not("club data: members of that club only"),
    ),
    (
        "club_picks.set_by",
        Not("club data: members of that club only (club_picks.rs)"),
    ),
    (
        "milestones.created_by",
        Not("club data: members of that club only (club_milestones.rs)"),
    ),
    (
        "next_up_nominations.nominated_by",
        Not("club data: members of that club only (club_votes.rs)"),
    ),
    (
        "next_up_ballots.user_id",
        Not("club data: members of that club only (club_votes.rs)"),
    ),
    (
        "works.created_by",
        Not("catalog provenance: never served (catalog_custom.rs)"),
    ),
    (
        "editions.created_by",
        Not("catalog provenance: never served (catalog_custom.rs)"),
    ),
    (
        "catalog_edit_requests.created_by",
        Not("catalog moderation: instance admins only (admin_guard.rs, catalog_requests.rs)"),
    ),
    (
        "catalog_edit_requests.decided_by",
        Not("catalog moderation: instance admins only"),
    ),
    (
        "media.uploaded_by",
        Not(
            "uploads: never served; an avatar is served by its unguessable public id (media_avatars.rs)",
        ),
    ),
    (
        "reports.created_by",
        Not("moderation: instance admins only (media_reports.rs)"),
    ),
    (
        "reports.resolved_by",
        Not("moderation: instance admins only"),
    ),
    (
        "reports.target_user_id",
        Not(
            "moderation: whose profile picture was reported, for instance admins only \
             (media_reports.rs); exported to that account without the reporter (auth_export.rs)",
        ),
    ),
    // Friendships widen every kind above through the policy functions (the Friend case); the
    // friend lists themselves are their owner's (`GET /api/friends`).
    ("friendships.requester_id", Kinds(&[Kind::Friends])),
    ("friendships.addressee_id", Kinds(&[Kind::Friends])),
    ("user_blocks.user_id", Kinds(&[Kind::Friends])),
    (
        "user_blocks.blocked_user_id",
        Not("never served: the blocked account is not told (friends_blocks.rs)"),
    ),
    (
        "club_invites.invitee_id",
        Not(
            "invite data: the invite's creator and the invitee (clubs_invites.rs, friends_invites.rs)",
        ),
    ),
];

/// Every operation with a `username` parameter, as `METHOD /path`.
const USERNAME_ROUTES: &[(&str, Class)] = &[
    ("GET /api/shelf", Kinds(&[Kind::Shelf, Kind::Favourites])),
    ("GET /api/shelf/physical-layout", Kinds(&[Kind::Layout])),
    ("PUT /api/shelf/physical-layout", Kinds(&[Kind::Layout])),
    ("GET /api/quotes", Kinds(&[Kind::Quotes])),
    ("GET /api/stats", Kinds(&[Kind::Stats])),
    ("GET /api/activity", Kinds(&[Kind::Activity])),
    ("GET /api/clubs/{slug}/activity", Kinds(&[Kind::Activity])),
    (
        "PATCH /api/clubs/{slug}/members/{username}",
        Not("club administration: admins of that club (clubs_roles.rs)"),
    ),
    (
        "DELETE /api/clubs/{slug}/members/{username}",
        Not("club administration: admins of that club (clubs_members.rs)"),
    ),
    (
        "POST /api/admin/users/{username}/suspend",
        Not("instance administration: instance admins only (admin_guard.rs, admin_users.rs)"),
    ),
    (
        "POST /api/admin/users/{username}/unsuspend",
        Not("instance administration: instance admins only (admin_guard.rs, admin_users.rs)"),
    ),
    (
        "POST /api/friends/requests/{username}/accept",
        Kinds(&[Kind::Friends]),
    ),
    (
        "DELETE /api/friends/requests/{username}",
        Kinds(&[Kind::Friends]),
    ),
    (
        "DELETE /api/friends/list/{username}",
        Kinds(&[Kind::Friends]),
    ),
    (
        "PUT /api/friends/blocks/{username}",
        Kinds(&[Kind::Friends]),
    ),
    (
        "DELETE /api/friends/blocks/{username}",
        Kinds(&[Kind::Friends]),
    ),
];

/// Every schema of the OpenAPI document with a rating or take field.
const RATING_SCHEMAS: &[(&str, Class)] = &[
    ("ShelfItemOut", Kinds(&[Kind::Shelf])),
    ("PersonalStatsOut", Kinds(&[Kind::Stats])),
    ("PostOut", Kinds(&[Kind::Diary])),
    ("BookDetailOut", Kinds(&[Kind::BookPage])),
    ("BookReader", Kinds(&[Kind::BookPage])),
    ("ClubPickReader", Kinds(&[Kind::PickReaders])),
    ("ClubStatsOut", Kinds(&[Kind::ClubShelves])),
    ("MemberYear", Kinds(&[Kind::ClubShelves])),
    ("PickStat", Kinds(&[Kind::ClubShelves])),
    ("ShelfAddIn", Not("a request body: the caller's own entry")),
    (
        "ShelfPatchIn",
        Not("a request body: the caller's own entry"),
    ),
];

/// A property that carries a member's rating or take.
fn is_rating_field(name: &str) -> bool {
    name == "rating"
        || name == "take"
        || name == "ratings"
        || (name.ends_with("_rating") && !name.starts_with("ol_"))
}

fn kinds_of(table: &[(&str, Class)]) -> BTreeSet<Kind> {
    table
        .iter()
        .filter_map(|(_, class)| match class {
            Kinds(kinds) => Some(kinds.iter().copied()),
            Not(_) => None,
        })
        .flatten()
        .collect()
}

/// The difference between what the system has and what a table lists, as messages.
fn compare(what: &str, actual: &BTreeSet<String>, table: &[(&str, Class)]) -> Vec<String> {
    let listed: BTreeSet<String> = table.iter().map(|(name, _)| name.to_string()).collect();
    let mut problems: Vec<String> = table
        .iter()
        .filter(|(_, class)| match class {
            Kinds(kinds) => kinds.is_empty(),
            Not(reason) => reason.is_empty(),
        })
        .map(|(name, _)| format!("{what} {name}: names no kind and gives no reason"))
        .collect();
    problems.extend(
        actual
            .difference(&listed)
            .map(|name| format!("{what} {name}: not in the #46 matrix (add it to its table)")),
    );
    problems.extend(
        listed
            .difference(actual)
            .map(|name| format!("{what} {name}: listed, but gone")),
    );
    problems
}

#[sqlx::test]
async fn every_personal_column_route_and_rating_field_is_in_the_matrix(db: PgPool) {
    let columns: BTreeSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT c.conrelid::regclass::text || '.' || a.attname::text
         FROM pg_constraint c
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
         WHERE c.contype = 'f' AND c.confrelid = 'users'::regclass
           AND cardinality(c.conkey) = 1",
    )
    .fetch_all(&db)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let mut problems = compare("column", &columns, USER_COLUMNS);

    let doc = serde_json::to_value(bookclub_server::openapi()).unwrap();
    let mut routes = BTreeSet::new();
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            let named = operation["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|param| param["name"] == "username");
            if named {
                routes.insert(format!("{} {path}", method.to_uppercase()));
            }
        }
    }
    problems.extend(compare("route", &routes, USERNAME_ROUTES));
    // Sanity: the walk saw the document.
    assert!(operations().len() > 100);

    let schemas: BTreeSet<String> = doc["components"]["schemas"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, schema)| {
            schema["properties"]
                .as_object()
                .is_some_and(|props| props.keys().any(|name| is_rating_field(name)))
        })
        .map(|(name, _)| name.clone())
        .collect();
    problems.extend(compare("schema", &schemas, RATING_SCHEMAS));

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every kind the tables above name: the kinds the matrix runs.
fn named_kinds() -> BTreeSet<Kind> {
    let mut kinds = kinds_of(USER_COLUMNS);
    kinds.extend(kinds_of(USERNAME_ROUTES));
    kinds.extend(kinds_of(RATING_SCHEMAS));
    kinds
}

#[test]
fn the_checklist_has_a_row_per_kind() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/phase1/parity-checklist.md");
    let text = fs::read_to_string(path).unwrap();
    let start = text
        .find("\n## #46 visibility matrix")
        .expect("the checklist has a #46 section");
    let section = &text[start + 1..];
    let section = &section[..section.find("\n## ").unwrap_or(section.len())];
    let row = Regex::new(r"(?m)^\| ([^|(]+?) (?:\([^|]*\) )?\|").unwrap();
    let rows: BTreeSet<&str> = row
        .captures_iter(section)
        .map(|found| found.get(1).unwrap().as_str())
        .filter(|label| *label != "Kind" && !label.starts_with("---"))
        .collect();
    let kinds: BTreeSet<&str> = named_kinds().into_iter().map(Kind::label).collect();
    let mut problems: Vec<String> = kinds
        .difference(&rows)
        .map(|label| format!("{label}: a kind of this test, but no row in the #46 table"))
        .collect();
    problems.extend(
        rows.difference(&kinds)
            .map(|label| format!("{label}: a row of the #46 table, but no kind of this test")),
    );
    assert!(
        problems.is_empty(),
        "docs/phase1/parity-checklist.md, #46 section:\n{}",
        problems.join("\n")
    );
}

/// The owner's data, written through the routes, and the viewers.
struct World {
    app: TestApp,
    v: Viewers,
    /// Finished, rated 5 with a take, `audience`; a favourite; an `audience` quote and diary
    /// entry; the pick of `other`; the co-member reads it too.
    circe: TestWork,
    /// Finished, rated 2 with a take, `only_me`; an `only_me` quote and diary entry, and an
    /// `audience` entry (whose `author_rating` hides the rating); the pick of `shared`.
    song: TestWork,
    /// Currently reading at 40 %.
    galatea: TestWork,
    /// On the want-to-read shelves of the owner, the co-member and `otherclub`.
    achilles: TestWork,
    open_post: i64,
    secret_post: i64,
    search_entry: i64,
    import_job: i64,
    layout: Value,
}

const OPEN_QUOTE: &str = "Open words";
const SECRET_QUOTE: &str = "Secret words";
const OPEN_DIARY: &str = "Open diary";
const SECRET_DIARY: &str = "Secret diary";
const SONG_DIARY: &str = "Song thoughts";
const CLUB_POST: &str = "For the club only";

async fn shelve(who: &TestClient, work: &TestWork, body: Value) -> Value {
    let mut full = json!({ "work_id": work.id });
    if let (Some(full), Value::Object(extra)) = (full.as_object_mut(), body) {
        full.extend(extra);
    }
    let res = who.post("/api/shelf", full).await;
    res.assert_status(201);
    res.json()
}

async fn post(who: &TestClient, work: &TestWork, body: &str, visibility: &str) -> i64 {
    let res = who
        .post(
            &format!("/api/books/works/{}/posts", work.id),
            json!({"body": body, "visibility": visibility}),
        )
        .await;
    res.assert_status(201);
    res.json()["id"].as_i64().unwrap()
}

async fn quote(who: &TestClient, work: &TestWork, body: &str, visibility: &str) {
    who.post(
        "/api/quotes",
        json!({"work_id": work.id, "body": body, "visibility": visibility}),
    )
    .await
    .assert_status(201);
}

impl World {
    async fn new(db: PgPool) -> World {
        let app = TestApp::new(db).await;
        let v = Viewers::new(&app).await;
        let circe = app.ol_work("OL1W", "Circe", "Madeline Miller").await;
        let song = app.ol_work("OL2W", "Song", "Madeline Miller").await;
        let galatea = app.ol_work("OL3W", "Galatea", "Madeline Miller").await;
        let achilles = app.ol_work("OL4W", "Achilles", "Madeline Miller").await;
        let owner = &v.owner;

        let circe_entry = shelve(
            owner,
            &circe,
            json!({"status": "finished", "rating": 5, "take": "Loved it"}),
        )
        .await;
        shelve(
            owner,
            &song,
            json!({"status": "finished", "rating": 2, "take": "Meh",
                   "rating_visibility": "only_me"}),
        )
        .await;
        shelve(
            owner,
            &galatea,
            json!({"status": "currently_reading", "progress": 40}),
        )
        .await;
        shelve(owner, &achilles, json!({"status": "want_to_read"})).await;
        shelve(
            &v.co_member,
            &circe,
            json!({"status": "currently_reading", "progress": 10}),
        )
        .await;
        shelve(&v.co_member, &achilles, json!({"status": "want_to_read"})).await;
        shelve(&v.other_club, &achilles, json!({"status": "want_to_read"})).await;

        owner
            .put("/api/auth/me/favorites", json!({"work_ids": [circe.id]}))
            .await
            .assert_status(200);
        let layout = json!({
            "rows": [[circe_entry["id"]]],
            "material": "walnut",
            "lights": true,
            "spines": {},
            "decorations": {}
        });
        let saved = owner
            .put("/api/shelf/physical-layout?username=owner", layout.clone())
            .await;
        saved.assert_status(200);
        let layout = saved.json();
        quote(owner, &circe, OPEN_QUOTE, "audience").await;
        quote(owner, &song, SECRET_QUOTE, "only_me").await;
        let open_post = post(owner, &circe, OPEN_DIARY, "audience").await;
        let secret_post = post(owner, &song, SECRET_DIARY, "only_me").await;
        post(owner, &song, SONG_DIARY, "audience").await;
        // The co-member reacts to the open entry: their name is in the reaction list.
        v.co_member
            .post(
                &format!("/api/posts/{open_post}/reactions"),
                json!({"emoji": "🔥"}),
            )
            .await
            .assert_status(200);

        owner
            .put(
                &format!("/api/clubs/{}/pick", v.shared.slug),
                json!({"work_id": song.id}),
            )
            .await
            .assert_status(200);
        v.other_club
            .put(
                &format!("/api/clubs/{}/pick", v.other.slug),
                json!({"work_id": circe.id}),
            )
            .await
            .assert_status(200);

        let history = owner
            .post("/api/search-history", json!({"query": "secret search"}))
            .await;
        history.assert_status(200);
        let search_entry = history.json()[0]["id"].as_i64().unwrap();
        let job = owner
            .post_multipart(
                "/api/shelf/import",
                vec![Part::file(
                    "goodreads.csv",
                    "text/csv",
                    b"Title,Author,ISBN13,My Rating,Date Read,Exclusive Shelf\n\
                      Shelved Elsewhere,Someone,,,,somewhere-else\n"
                        .to_vec(),
                )],
            )
            .await;
        job.assert_status(202);
        app.settle().await;
        let import_job = job.json()["id"].as_i64().unwrap();

        World {
            app,
            v,
            circe,
            song,
            galatea,
            achilles,
            open_post,
            secret_post,
            search_entry,
            import_job,
            layout,
        }
    }
}

fn items(body: &Value) -> Vec<Value> {
    body["items"].as_array().cloned().unwrap_or_default()
}

/// The item of `list` whose `key` is `value`.
fn find(list: &[Value], key: &str, value: impl Into<Value>) -> Option<Value> {
    let value = value.into();
    list.iter().find(|item| item[key] == value).cloned()
}

/// The shelf entry of `work` in a shelf list.
fn entry_of(list: &[Value], work: &TestWork) -> Value {
    list.iter()
        .find(|item| item["book"]["id"] == work.id)
        .cloned()
        .unwrap_or_else(|| panic!("{} is not on the shelf", work.title))
}

fn names(list: &Value, key: &str) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .map(|item| item[key].as_str().unwrap_or_default().to_owned())
        .collect()
}

async fn ok(viewer: &TestClient, path: &str) -> Value {
    let res = viewer.get(path).await;
    assert_eq!(
        res.status.as_u16(),
        200,
        "{} GET {path}: {}",
        viewer.username(),
        res.text()
    );
    res.json()
}

/// Asserts that every viewer passing an unknown name to `path_for` gets 404 `No member with
/// that name`.
async fn unknown_is_no_member(w: &World, path_for: impl Fn(&str) -> String) {
    for viewer in [&w.v.owner, &w.v.co_member, &w.v.stranger] {
        viewer
            .get(&path_for(UNKNOWN))
            .await
            .assert_detail(404, "No member with that name");
    }
}

/// The viewer of an audience case: the co-member or the friend.
fn audience_viewer(w: &World, case: Case) -> &TestClient {
    match case {
        Case::Friend => &w.v.friend,
        _ => &w.v.co_member,
    }
}

async fn check(w: &World, kind: Kind, case: Case) -> Covered {
    match kind {
        Kind::Shelf => shelf(w, case).await,
        Kind::Favourites => favourites(w, case).await,
        Kind::Layout => layout(w, case).await,
        Kind::Quotes => quotes(w, case).await,
        Kind::Stats => stats(w, case).await,
        Kind::Diary => diary(w, case).await,
        Kind::Activity => activity(w, case).await,
        Kind::BookPage => book_page(w, case).await,
        Kind::PickReaders => pick_readers(w, case).await,
        Kind::ClubShelves => club_shelves(w, case).await,
        Kind::SearchHistory => search_history(w, case).await,
        Kind::ImportJobs => import_jobs(w, case).await,
        Kind::Friends => friends(w, case).await,
    }
}

async fn shelf(w: &World, case: Case) -> Covered {
    let shelf_of = |name: &str| format!("/api/shelf?username={name}");
    match case {
        Case::Myself => {
            let own = items(&ok(&w.v.owner, "/api/shelf").await);
            let song = entry_of(&own, &w.song);
            assert_eq!(
                (&song["rating"], &song["take"], &song["rating_visibility"]),
                (&json!(2), &json!("Meh"), &json!("only_me"))
            );
            assert_eq!(items(&ok(&w.v.owner, &shelf_of("owner")).await), own);
        }
        Case::CoMember | Case::Friend => {
            let seen = ok(audience_viewer(w, case), &shelf_of("owner")).await;
            assert_eq!(seen["user"]["username"], "owner");
            let list = items(&seen);
            assert_eq!(list.len(), 4);
            let circe = entry_of(&list, &w.circe);
            assert_eq!(
                (&circe["rating"], &circe["take"]),
                (&json!(5), &json!("Loved it"))
            );
            let song = entry_of(&list, &w.song);
            assert_eq!(
                (&song["status"], &song["rating"], &song["take"]),
                (&json!("finished"), &Value::Null, &json!(""))
            );
            let galatea = entry_of(&list, &w.galatea);
            assert_eq!(galatea["progress"], 40);
            assert!(list.iter().all(|item| item["rating_visibility"].is_null()));
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                assert_hidden_like_unknown(viewer, shelf_of, "owner").await;
            }
        }
        Case::UnknownName => unknown_is_no_member(w, shelf_of).await,
    }
    Covered::Checked
}

async fn favourites(w: &World, case: Case) -> Covered {
    let shelf_of = |name: &str| format!("/api/shelf?username={name}");
    let circe_page = format!("/api/books/works/{}", w.circe.id);
    match case {
        Case::Myself => {
            let own = ok(&w.v.owner, "/api/auth/me/favorites").await;
            assert_eq!(own["items"][0]["position"], 1);
            assert_eq!(own["items"][0]["book"]["id"], w.circe.id);
            let shelf = ok(&w.v.owner, "/api/shelf").await;
            assert_eq!(shelf["favorites"][0]["book"]["id"], w.circe.id);
            assert_eq!(ok(&w.v.owner, &circe_page).await["favorite_position"], 1);
        }
        Case::CoMember | Case::Friend => {
            let viewer = audience_viewer(w, case);
            let seen = ok(viewer, &shelf_of("owner")).await;
            assert_eq!(seen["favorites"][0]["book"]["id"], w.circe.id);
            assert_eq!(seen["favorites"][0]["position"], 1);
            // Their own favourites and the book page's position stay theirs.
            assert_eq!(
                ok(viewer, "/api/auth/me/favorites").await,
                json!({"items": []})
            );
            assert_eq!(
                ok(viewer, &circe_page).await["favorite_position"],
                Value::Null
            );
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                assert_hidden_like_unknown(viewer, shelf_of, "owner").await;
                assert_eq!(
                    ok(viewer, &circe_page).await["favorite_position"],
                    Value::Null
                );
            }
        }
        Case::UnknownName => unknown_is_no_member(w, shelf_of).await,
    }
    Covered::Checked
}

async fn layout(w: &World, case: Case) -> Covered {
    let layout_of = |name: &str| format!("/api/shelf/physical-layout?username={name}");
    let other = json!({"rows": [[]], "material": "black"});
    match case {
        Case::Myself => {
            assert_eq!(ok(&w.v.owner, "/api/shelf/physical-layout").await, w.layout);
            assert_eq!(ok(&w.v.owner, &layout_of("owner")).await, w.layout);
        }
        Case::CoMember | Case::Friend => {
            let viewer = audience_viewer(w, case);
            assert_eq!(ok(viewer, &layout_of("owner")).await, w.layout);
            viewer
                .put(&layout_of("owner"), other.clone())
                .await
                .assert_detail(404, "No member with that name");
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                assert_hidden_like_unknown(viewer, layout_of, "owner").await;
                viewer
                    .put(&layout_of("owner"), other.clone())
                    .await
                    .assert_detail(404, "No member with that name");
            }
            assert_eq!(ok(&w.v.owner, "/api/shelf/physical-layout").await, w.layout);
        }
        Case::UnknownName => {
            unknown_is_no_member(w, layout_of).await;
            w.v.co_member
                .put(&layout_of(UNKNOWN), other)
                .await
                .assert_detail(404, "No member with that name");
        }
    }
    Covered::Checked
}

fn bodies(list: &Value) -> Vec<String> {
    names(&list["items"], "body")
}

async fn quotes(w: &World, case: Case) -> Covered {
    let quotes_of = |name: &str| format!("/api/quotes?username={name}");
    let on = |work: &TestWork| format!("/api/quotes?work={}", work.id);
    match case {
        Case::Myself => {
            let own = ok(&w.v.owner, "/api/quotes").await;
            let mut seen = bodies(&own);
            seen.sort();
            assert_eq!(seen, [OPEN_QUOTE, SECRET_QUOTE]);
            assert_eq!(ok(&w.v.owner, &quotes_of("owner")).await, own);
            assert_eq!(bodies(&ok(&w.v.owner, &on(&w.song)).await), [SECRET_QUOTE]);
        }
        Case::CoMember | Case::Friend => {
            let viewer = audience_viewer(w, case);
            let seen = ok(viewer, &quotes_of("owner")).await;
            assert_eq!(bodies(&seen), [OPEN_QUOTE]);
            assert_eq!(seen["items"][0]["author"], "owner");
            assert_eq!(bodies(&ok(viewer, &on(&w.circe)).await), [OPEN_QUOTE]);
            assert!(bodies(&ok(viewer, &on(&w.song)).await).is_empty());
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                // The list answers 200 and empty, exactly like an unknown name.
                let hidden = ok(viewer, &quotes_of("owner")).await;
                assert_eq!(hidden, ok(viewer, &quotes_of(UNKNOWN)).await);
                assert_eq!(hidden, json!({"items": []}));
                assert!(bodies(&ok(viewer, &on(&w.circe)).await).is_empty());
            }
        }
        Case::UnknownName => {
            for viewer in [&w.v.owner, &w.v.co_member, &w.v.stranger] {
                assert_eq!(ok(viewer, &quotes_of(UNKNOWN)).await, json!({"items": []}));
            }
        }
    }
    Covered::Checked
}

async fn stats(w: &World, case: Case) -> Covered {
    let stats_of = |name: &str| format!("/api/stats?username={name}");
    match case {
        Case::Myself => {
            let own = ok(&w.v.owner, "/api/stats").await;
            assert_eq!(own["finished"], 2);
            assert_eq!(own["average_rating"], 3.5);
            assert_eq!(own["quotes"], 2);
            assert_eq!(ok(&w.v.owner, &stats_of("owner")).await, own);
        }
        Case::CoMember | Case::Friend => {
            let seen = ok(audience_viewer(w, case), &stats_of("owner")).await;
            assert_eq!(seen["username"], "owner");
            assert_eq!(seen["finished"], 2);
            assert_eq!(seen["average_rating"], 5.0);
            assert_eq!(seen["quotes"], 1);
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                assert_hidden_like_unknown(viewer, stats_of, "owner").await;
            }
        }
        Case::UnknownName => unknown_is_no_member(w, stats_of).await,
    }
    Covered::Checked
}

/// The bodies of a work's thread as `viewer` sees it.
async fn thread(viewer: &TestClient, work: &TestWork) -> Vec<String> {
    bodies(&ok(viewer, &format!("/api/books/works/{}/posts", work.id)).await)
}

/// The bodies of the diary feed as `viewer` sees it.
async fn diary_feed(viewer: &TestClient) -> Vec<String> {
    let feed = ok(viewer, "/api/diary").await;
    items(&feed)
        .iter()
        .map(|item| item["entry"]["body"].as_str().unwrap().to_owned())
        .collect()
}

async fn diary(w: &World, case: Case) -> Covered {
    let react = |id: i64| format!("/api/posts/{id}/reactions");
    match case {
        Case::Myself => {
            assert_eq!(thread(&w.v.owner, &w.circe).await, [OPEN_DIARY]);
            let song = ok(&w.v.owner, &format!("/api/books/works/{}/posts", w.song.id)).await;
            assert_eq!(bodies(&song), [SECRET_DIARY, SONG_DIARY]);
            // The author sees their own rating, only_me or not.
            assert_eq!(song["items"][1]["author_rating"], 2);
            let feed = diary_feed(&w.v.owner).await;
            for body in [OPEN_DIARY, SECRET_DIARY, SONG_DIARY] {
                assert!(feed.contains(&body.to_owned()), "{body}: {feed:?}");
            }
            let circe = ok(
                &w.v.owner,
                &format!("/api/books/works/{}/posts", w.circe.id),
            )
            .await;
            assert_eq!(
                circe["items"][0]["reactions"][0]["users"],
                json!(["comember"])
            );
        }
        Case::CoMember => {
            assert_eq!(thread(&w.v.co_member, &w.circe).await, [OPEN_DIARY]);
            let song = ok(
                &w.v.co_member,
                &format!("/api/books/works/{}/posts", w.song.id),
            )
            .await;
            assert_eq!(bodies(&song), [SONG_DIARY]);
            assert_eq!(song["items"][0]["author_rating"], Value::Null);
            let circe = ok(
                &w.v.co_member,
                &format!("/api/books/works/{}/posts", w.circe.id),
            )
            .await;
            assert_eq!(circe["items"][0]["author_rating"], 5);
            let feed = diary_feed(&w.v.co_member).await;
            assert!(feed.contains(&OPEN_DIARY.to_owned()), "{feed:?}");
            assert!(!feed.contains(&SECRET_DIARY.to_owned()), "{feed:?}");
            w.v.co_member
                .post(&react(w.secret_post), json!({"emoji": "🔥"}))
                .await
                .assert_detail(404, "No entry with that id");
        }
        Case::Friend => {
            let friend = &w.v.friend;
            assert_eq!(thread(friend, &w.circe).await, [OPEN_DIARY]);
            let song = ok(friend, &format!("/api/books/works/{}/posts", w.song.id)).await;
            assert_eq!(bodies(&song), [SONG_DIARY]);
            assert_eq!(song["items"][0]["author_rating"], Value::Null);
            let circe = ok(friend, &format!("/api/books/works/{}/posts", w.circe.id)).await;
            assert_eq!(circe["items"][0]["author_rating"], 5);
            // The co-member reacted, but the friend cannot see them: counted, not named.
            let reaction = &circe["items"][0]["reactions"][0];
            assert_eq!(
                (&reaction["count"], &reaction["users"]),
                (&json!(1), &json!([]))
            );
            let feed = diary_feed(friend).await;
            assert!(feed.contains(&OPEN_DIARY.to_owned()), "{feed:?}");
            assert!(!feed.contains(&SECRET_DIARY.to_owned()), "{feed:?}");
            friend
                .post(&react(w.secret_post), json!({"emoji": "🔥"}))
                .await
                .assert_detail(404, "No entry with that id");
            // The owner's club posts stay in the club: the friend is in none of the owner's.
            let res =
                w.v.owner
                    .post(
                        &format!("/api/clubs/{}/pick/posts", w.v.shared.slug),
                        json!({"body": CLUB_POST}),
                    )
                    .await;
            res.assert_status(201);
            let club_post = res.json()["id"].as_i64().unwrap();
            assert!(
                !thread(friend, &w.song)
                    .await
                    .contains(&CLUB_POST.to_owned())
            );
            assert!(!diary_feed(friend).await.contains(&CLUB_POST.to_owned()));
            friend
                .post(&react(club_post), json!({"emoji": "🔥"}))
                .await
                .assert_detail(404, "No entry with that id");
            w.v.owner
                .delete(&format!("/api/posts/{club_post}"))
                .await
                .assert_status(204);
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                assert!(thread(viewer, &w.circe).await.is_empty());
                assert!(thread(viewer, &w.song).await.is_empty());
                let feed = diary_feed(viewer).await;
                assert!(!feed.contains(&OPEN_DIARY.to_owned()), "{feed:?}");
                viewer
                    .post(&react(w.open_post), json!({"emoji": "🔥"}))
                    .await
                    .assert_detail(404, "No entry with that id");
                viewer
                    .patch(&format!("/api/posts/{}", w.open_post), json!({"body": "x"}))
                    .await
                    .assert_detail(404, "No entry with that id");
            }
        }
        Case::UnknownName => {
            // A post id that names nothing answers like a post the viewer may not see.
            let hidden =
                w.v.stranger
                    .post(&react(w.open_post), json!({"emoji": "🔥"}))
                    .await;
            let unknown =
                w.v.stranger
                    .post(&react(999_999_999), json!({"emoji": "🔥"}))
                    .await;
            assert_eq!(
                (hidden.status, hidden.json()),
                (unknown.status, unknown.json())
            );
            unknown.assert_detail(404, "No entry with that id");
        }
    }
    Covered::Checked
}

/// The (kind, work id, payload) of the owner's events in a feed.
fn owner_events(feed: &Value) -> Vec<(String, i64, Value)> {
    items(feed)
        .iter()
        .filter(|item| item["actor"] == "owner")
        .map(|item| {
            (
                item["kind"].as_str().unwrap().to_owned(),
                item["book"]["id"].as_i64().unwrap_or_default(),
                item["payload"].clone(),
            )
        })
        .collect()
}

async fn activity(w: &World, case: Case) -> Covered {
    let home_of = |name: &str| format!("/api/activity?username={name}");
    let shared = format!("/api/clubs/{}/activity", w.v.shared.slug);
    let other = format!("/api/clubs/{}/activity", w.v.other.slug);
    let finished = |events: &[(String, i64, Value)], work: &TestWork| -> Value {
        events
            .iter()
            .find(|(kind, id, _)| kind == "shelf_finished" && *id == work.id)
            .map(|(_, _, payload)| payload.clone())
            .unwrap_or_else(|| panic!("no shelf_finished for {}: {events:?}", work.title))
    };
    let quote_events = |events: &[(String, i64, Value)]| -> usize {
        events
            .iter()
            .filter(|(kind, _, _)| kind == "quote_added")
            .count()
    };
    match case {
        Case::Myself => {
            let own = owner_events(&ok(&w.v.owner, "/api/activity").await);
            assert_eq!(finished(&own, &w.song), json!({"rating": 2, "take": "Meh"}));
            assert_eq!(quote_events(&own), 2);
            assert_eq!(owner_events(&ok(&w.v.owner, &home_of("owner")).await), own);
        }
        Case::CoMember => {
            for path in [
                home_of("owner"),
                shared.clone(),
                format!("{shared}?username=owner"),
            ] {
                let seen = ok(&w.v.co_member, &path).await;
                let events = owner_events(&seen);
                assert_eq!(
                    finished(&events, &w.circe),
                    json!({"rating": 5, "take": "Loved it"}),
                    "{path}"
                );
                assert_eq!(finished(&events, &w.song), json!({}), "{path}");
                assert_eq!(quote_events(&events), 1, "{path}");
            }
            // The owner is not in `other`: their personal events stay out of its feed, though
            // the co-member can see them.
            assert!(owner_events(&ok(&w.v.co_member, &other).await).is_empty());
            assert!(
                items(&ok(&w.v.co_member, &format!("{other}?username=owner")).await).is_empty()
            );
        }
        Case::Friend => {
            // The home feed only, from the friendship on, redacted as for a co-member. The
            // owner's club events stay in the owner's clubs.
            for path in ["/api/activity".to_owned(), home_of("owner")] {
                let seen = ok(&w.v.friend, &path).await;
                let events = owner_events(&seen);
                assert_eq!(
                    finished(&events, &w.circe),
                    json!({"rating": 5, "take": "Loved it"}),
                    "{path}"
                );
                assert_eq!(finished(&events, &w.song), json!({}), "{path}");
                assert_eq!(quote_events(&events), 1, "{path}");
                assert!(
                    items(&seen)
                        .iter()
                        .filter(|item| item["actor"] == "owner")
                        .all(|item| item["club"].is_null()),
                    "{path}: a club event of the owner"
                );
            }
            w.v.friend
                .get(&shared)
                .await
                .assert_detail(404, "No such club");
            // An event from before the friendship began stays hidden: every event of the owner
            // is, once the friendship is a day younger than it. The shift is undone after.
            let pair = format!(
                "WHERE requester_id = {} AND addressee_id = {}",
                w.v.owner.id(),
                w.v.friend.id()
            );
            w.app
                .exec(&format!(
                    "UPDATE friendships SET accepted_at = accepted_at + interval '1 day' {pair}"
                ))
                .await;
            for path in ["/api/activity".to_owned(), home_of("owner")] {
                let seen = ok(&w.v.friend, &path).await;
                assert!(owner_events(&seen).is_empty(), "{path}: {seen}");
            }
            w.app
                .exec(&format!(
                    "UPDATE friendships SET accepted_at = accepted_at - interval '1 day' {pair}"
                ))
                .await;
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                let hidden = ok(viewer, &home_of("owner")).await;
                assert_eq!(hidden, ok(viewer, &home_of(UNKNOWN)).await);
                assert!(items(&hidden).is_empty());
                assert!(owner_events(&ok(viewer, "/api/activity").await).is_empty());
                viewer.get(&shared).await.assert_detail(404, "No such club");
            }
            assert!(owner_events(&ok(&w.v.other_club, &other).await).is_empty());
        }
        Case::UnknownName => {
            for viewer in [&w.v.owner, &w.v.co_member, &w.v.stranger] {
                let res = ok(viewer, &home_of(UNKNOWN)).await;
                assert!(items(&res).is_empty());
                assert_eq!(res["has_more"], false);
            }
            assert!(
                items(&ok(&w.v.co_member, &format!("{shared}?username={UNKNOWN}")).await)
                    .is_empty()
            );
        }
    }
    Covered::Checked
}

fn readers(page: &Value) -> Vec<(String, Value)> {
    page["readers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["username"].as_str().unwrap().to_owned(),
                r["rating"].clone(),
            )
        })
        .collect()
}

async fn book_page(w: &World, case: Case) -> Covered {
    let page = |work: &TestWork| format!("/api/books/works/{}", work.id);
    match case {
        Case::Myself => {
            let song = ok(&w.v.owner, &page(&w.song)).await;
            assert_eq!(
                (&song["rating"], &song["take"], &song["rating_visibility"]),
                (&json!(2), &json!("Meh"), &json!("only_me"))
            );
            assert_eq!(song["quote_count"], 1);
            let circe = ok(&w.v.owner, &page(&w.circe)).await;
            assert_eq!(readers(&circe), [("comember".to_owned(), Value::Null)]);
            assert_eq!(circe["quote_count"], 1);
        }
        Case::CoMember => {
            let song = ok(&w.v.co_member, &page(&w.song)).await;
            assert_eq!(readers(&song), [("owner".to_owned(), Value::Null)]);
            assert_eq!(song["readers"][0]["status"], "finished");
            assert_eq!(
                (&song["rating_count"], &song["club_rating"]),
                (&json!(0), &Value::Null)
            );
            assert_eq!(song["quote_count"], 0);
            assert_eq!((&song["rating"], &song["take"]), (&Value::Null, &json!("")));
            let circe = ok(&w.v.co_member, &page(&w.circe)).await;
            assert_eq!(readers(&circe), [("owner".to_owned(), json!(5))]);
            assert_eq!(circe["rating_count"], 1);
            assert_eq!(circe["quote_count"], 1);
            let galatea = ok(&w.v.co_member, &page(&w.galatea)).await;
            assert_eq!(galatea["readers"][0]["progress"], 40);
        }
        Case::Friend => {
            let song = ok(&w.v.friend, &page(&w.song)).await;
            assert_eq!(readers(&song), [("owner".to_owned(), Value::Null)]);
            assert_eq!(
                (&song["rating_count"], &song["club_rating"]),
                (&json!(0), &Value::Null)
            );
            assert_eq!(song["quote_count"], 0);
            // The co-member reads it too, but the friend cannot see them: the owner alone, once.
            let circe = ok(&w.v.friend, &page(&w.circe)).await;
            assert_eq!(readers(&circe), [("owner".to_owned(), json!(5))]);
            assert_eq!(circe["rating_count"], 1);
            assert_eq!(circe["quote_count"], 1);
            let galatea = ok(&w.v.friend, &page(&w.galatea)).await;
            assert_eq!(galatea["readers"][0]["progress"], 40);
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                for work in [&w.circe, &w.song, &w.galatea] {
                    let seen = ok(viewer, &page(work)).await;
                    assert!(
                        readers(&seen).iter().all(|(name, _)| name != "owner"),
                        "{} sees the owner on {}",
                        viewer.username(),
                        work.title
                    );
                    assert_eq!(seen["quote_count"], 0);
                }
                let circe = ok(viewer, &page(&w.circe)).await;
                assert_eq!(circe["rating_count"], 0);
            }
        }
        Case::UnknownName => {
            return Covered::NotApplicable(
                "the book page names no user: a stranger's view of the owner's books, checked \
                 above, is that of books an unknown user shelved",
            );
        }
    }
    Covered::Checked
}

async fn pick_readers(w: &World, case: Case) -> Covered {
    let shared = format!("/api/clubs/{}/pick", w.v.shared.slug);
    let other = format!("/api/clubs/{}/pick", w.v.other.slug);
    let pick_readers = |pick: &Value| -> Vec<(String, Value, Value)> {
        pick["readers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["username"].as_str().unwrap().to_owned(),
                    r["rating"].clone(),
                    r["take"].clone(),
                )
            })
            .collect()
    };
    match case {
        Case::Myself => {
            let pick = ok(&w.v.owner, &shared).await["pick"].clone();
            assert_eq!(
                pick_readers(&pick),
                [("owner".to_owned(), json!(2), json!("Meh"))]
            );
        }
        Case::CoMember => {
            let pick = ok(&w.v.co_member, &shared).await["pick"].clone();
            assert_eq!(
                pick_readers(&pick),
                [("owner".to_owned(), Value::Null, json!(""))]
            );
            // In `other`, whose pick the owner has finished, only that club's members read.
            let pick = ok(&w.v.co_member, &other).await["pick"].clone();
            assert_eq!(pick["book"]["id"], w.circe.id);
            assert_eq!(names(&pick["readers"], "username"), ["comember"]);
        }
        Case::Friend => {
            // Club data: the friend is in no club of the owner's.
            for path in [&shared, &other] {
                w.v.friend
                    .get(path)
                    .await
                    .assert_detail(404, "No such club");
            }
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                viewer.get(&shared).await.assert_detail(404, "No such club");
            }
            let pick = ok(&w.v.other_club, &other).await["pick"].clone();
            assert_eq!(names(&pick["readers"], "username"), ["comember"]);
        }
        Case::UnknownName => {
            return Covered::NotApplicable(
                "the pick names no user: its readers are the club's members, checked above",
            );
        }
    }
    Covered::Checked
}

/// The usernames per book title of a club's overlap.
async fn overlap(viewer: &TestClient, club: &TestClub) -> Vec<(String, Vec<String>)> {
    let res = ok(viewer, &format!("/api/clubs/{}/overlap", club.slug)).await;
    items(&res)
        .iter()
        .map(|item| {
            let mut members = names(&item["members"], "username");
            members.sort();
            (item["book"]["title"].as_str().unwrap().to_owned(), members)
        })
        .collect()
}

/// The reasons of every suggestion of a club's next vote.
async fn suggestion_reasons(viewer: &TestClient, club: &TestClub) -> Vec<String> {
    let res = ok(
        viewer,
        &format!("/api/clubs/{}/vote/suggestions", club.slug),
    )
    .await;
    items(&res)
        .iter()
        .flat_map(|item| item["reasons"].as_array().cloned().unwrap_or_default())
        .map(|reason| reason.as_str().unwrap().to_owned())
        .collect()
}

async fn club_shelves(w: &World, case: Case) -> Covered {
    let (shared, other) = (&w.v.shared, &w.v.other);
    let both = vec![(
        w.achilles.title.clone(),
        vec!["comember".to_owned(), "owner".to_owned()],
    )];
    let member = |stats: &Value, name: &str| -> Value {
        find(stats["members"].as_array().unwrap(), "username", name).unwrap_or(Value::Null)
    };
    match case {
        Case::Myself => {
            assert_eq!(overlap(&w.v.owner, shared).await, both);
            assert_eq!(
                suggestion_reasons(&w.v.owner, shared).await,
                ["On 2 shelves: comember, owner"]
            );
            let stats = ok(&w.v.owner, &format!("/api/clubs/{}/stats", shared.slug)).await;
            assert_eq!(member(&stats, "owner")["average_rating"], 3.5);
            assert_eq!(member(&stats, "owner")["quotes"], 2);
        }
        Case::CoMember => {
            assert_eq!(overlap(&w.v.co_member, shared).await, both);
            assert_eq!(
                suggestion_reasons(&w.v.co_member, shared).await,
                ["On 2 shelves: comember, owner"]
            );
            let members = ok(
                &w.v.co_member,
                &format!("/api/clubs/{}/members", shared.slug),
            )
            .await;
            let owner = find(&items(&members), "username", "owner").expect("owner");
            assert_eq!(owner["currently_reading_count"], 1);
            assert_eq!(owner["currently_reading_preview"][0]["id"], w.galatea.id);
            let stats = ok(&w.v.co_member, &format!("/api/clubs/{}/stats", shared.slug)).await;
            assert_eq!(member(&stats, "owner")["average_rating"], 5.0);
            assert_eq!(member(&stats, "owner")["quotes"], 1);
            // In `other` the owner is no member: the co-member's view of its shelves leaves
            // the owner out, though the co-member can see the owner's shelf.
            let mine = vec![(
                w.achilles.title.clone(),
                vec!["comember".to_owned(), "otherclub".to_owned()],
            )];
            assert_eq!(overlap(&w.v.co_member, other).await, mine);
            assert_eq!(
                suggestion_reasons(&w.v.co_member, other).await,
                ["On 2 shelves: comember, otherclub"]
            );
            let stats = ok(&w.v.co_member, &format!("/api/clubs/{}/stats", other.slug)).await;
            assert_eq!(member(&stats, "owner"), Value::Null);
        }
        Case::Friend => {
            for path in ["overlap", "vote/suggestions", "members", "stats"] {
                w.v.friend
                    .get(&format!("/api/clubs/{}/{path}", shared.slug))
                    .await
                    .assert_detail(404, "No such club");
            }
        }
        Case::Stranger => {
            for viewer in w.v.strangers() {
                for path in ["overlap", "vote/suggestions", "members", "stats"] {
                    viewer
                        .get(&format!("/api/clubs/{}/{path}", shared.slug))
                        .await
                        .assert_detail(404, "No such club");
                }
            }
            let mine = vec![(
                w.achilles.title.clone(),
                vec!["comember".to_owned(), "otherclub".to_owned()],
            )];
            assert_eq!(overlap(&w.v.other_club, other).await, mine);
            let members = ok(
                &w.v.other_club,
                &format!("/api/clubs/{}/members", other.slug),
            )
            .await;
            assert!(find(&items(&members), "username", "owner").is_none());
        }
        Case::UnknownName => {
            return Covered::NotApplicable(
                "club views name no user: they show the club's members, checked above",
            );
        }
    }
    Covered::Checked
}

async fn search_history(w: &World, case: Case) -> Covered {
    let forget = |id: i64| format!("/api/search-history/{id}");
    match case {
        Case::Myself => {
            let own = ok(&w.v.owner, "/api/search-history").await;
            assert_eq!(names(&own, "query"), ["secret search"]);
        }
        Case::CoMember | Case::Friend | Case::Stranger => {
            let viewers = match case {
                Case::CoMember => vec![&w.v.co_member],
                Case::Friend => vec![&w.v.friend],
                _ => w.v.strangers(),
            };
            for viewer in viewers {
                assert_eq!(ok(viewer, "/api/search-history").await, json!([]));
                viewer
                    .delete(&forget(w.search_entry))
                    .await
                    .assert_detail(404, "Search history entry not found");
            }
            assert_eq!(
                names(&ok(&w.v.owner, "/api/search-history").await, "query"),
                ["secret search"]
            );
        }
        Case::UnknownName => {
            let hidden = w.v.co_member.delete(&forget(w.search_entry)).await;
            let unknown = w.v.co_member.delete(&forget(999_999_999)).await;
            assert_eq!(
                (hidden.status, hidden.json()),
                (unknown.status, unknown.json())
            );
        }
    }
    Covered::Checked
}

async fn import_jobs(w: &World, case: Case) -> Covered {
    let job = |id: i64| format!("/api/shelf/import/{id}");
    match case {
        Case::Myself => {
            let own = ok(&w.v.owner, &job(w.import_job)).await;
            assert_eq!(own["status"], "done");
        }
        Case::CoMember | Case::Friend | Case::Stranger => {
            let viewers = match case {
                Case::CoMember => vec![&w.v.co_member],
                Case::Friend => vec![&w.v.friend],
                _ => w.v.strangers(),
            };
            for viewer in viewers {
                viewer
                    .get(&job(w.import_job))
                    .await
                    .assert_detail(404, "No such import");
            }
        }
        Case::UnknownName => {
            let hidden = w.v.co_member.get(&job(w.import_job)).await;
            let unknown = w.v.co_member.get(&job(999_999_999)).await;
            assert_eq!(
                (hidden.status, hidden.json()),
                (unknown.status, unknown.json())
            );
        }
    }
    Covered::Checked
}

/// Every username of a list of `GET /api/friends`, as (list, username).
fn friend_lists(out: &Value) -> BTreeSet<(String, String)> {
    ["friends", "incoming", "outgoing", "blocked"]
        .into_iter()
        .flat_map(|list| {
            out[list]
                .as_array()
                .unwrap_or_else(|| panic!("no list {list} in {out}"))
                .iter()
                .map(move |item| {
                    let name = item["user"]["username"]
                        .as_str()
                        .or(item["username"].as_str());
                    (list.to_owned(), name.unwrap().to_owned())
                })
        })
        .collect()
}

fn lists(entries: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    entries
        .iter()
        .map(|(list, name)| (list.to_string(), name.to_string()))
        .collect()
}

/// The `{username}` routes that act on the caller's own rows (all but the block, which tells an
/// unknown name apart, §4.9), as (method, path) for a name.
fn own_row_routes(name: &str) -> [(&'static str, String); 4] {
    [
        ("POST", format!("/api/friends/requests/{name}/accept")),
        ("DELETE", format!("/api/friends/requests/{name}")),
        ("DELETE", format!("/api/friends/list/{name}")),
        ("DELETE", format!("/api/friends/blocks/{name}")),
    ]
}

async fn call(viewer: &TestClient, method: &str, path: &str) -> TestResponse {
    match method {
        "POST" => viewer.post(path, json!({})).await,
        _ => viewer.delete(path).await,
    }
}

async fn friends(w: &World, case: Case) -> Covered {
    match case {
        Case::Myself => {
            let mine = ok(&w.v.owner, "/api/friends").await;
            assert_eq!(
                friend_lists(&mine),
                lists(&[
                    ("friends", "friend"),
                    ("incoming", "asked"),
                    ("outgoing", "requested"),
                    ("blocked", "blocked"),
                ])
            );
        }
        Case::CoMember | Case::Friend | Case::Stranger => {
            // Each viewer's list holds only their own rows: the friend sees the owner as their
            // friend, the two sides of a pending request see their request; nobody sees the
            // owner's other friends, requests or blocks, and the blocked account is not told.
            let expected: Vec<(&TestClient, BTreeSet<(String, String)>)> = match case {
                Case::CoMember => vec![(&w.v.co_member, lists(&[]))],
                Case::Friend => vec![(
                    &w.v.friend,
                    lists(&[("friends", "owner"), ("friends", "stranger")]),
                )],
                _ => vec![
                    (&w.v.other_club, lists(&[])),
                    (&w.v.stranger, lists(&[("friends", "friend")])),
                    (&w.v.former, lists(&[])),
                    (&w.v.asked, lists(&[("outgoing", "owner")])),
                    (&w.v.requested, lists(&[("incoming", "owner")])),
                    (&w.v.exfriend, lists(&[])),
                    (&w.v.blocked, lists(&[])),
                ],
            };
            for (viewer, rows) in expected {
                let theirs = ok(viewer, "/api/friends").await;
                assert_eq!(friend_lists(&theirs), rows, "{}", viewer.username());
                // The routes about the owner act on the viewer's own rows only. A viewer without
                // a row with the owner (the blocked account too) has nothing to accept, decline,
                // cancel or remove, and unblocking is a no-op. (The others would change the
                // world: the friend could unfriend, the requested account accept.)
                if rows.iter().any(|(_, name)| name == "owner") {
                    continue;
                }
                for (method, path) in own_row_routes("owner") {
                    let res = call(viewer, method, &path).await;
                    let status = if path.contains("/blocks/") { 200 } else { 404 };
                    assert_eq!(
                        res.status.as_u16(),
                        status,
                        "{} {method} {path}",
                        viewer.username()
                    );
                }
            }
            assert_eq!(
                friend_lists(&ok(&w.v.owner, "/api/friends").await).len(),
                4,
                "no viewer changed the owner's lists"
            );
        }
        Case::UnknownName => {
            // An unknown name answers like a name the caller has nothing with; only the block
            // tells them apart (404 `No account with that name`, counted against the limit).
            for viewer in [&w.v.co_member, &w.v.stranger] {
                for ((method, owner_path), (_, unknown_path)) in own_row_routes("owner")
                    .into_iter()
                    .zip(own_row_routes(UNKNOWN))
                {
                    let hidden = call(viewer, method, &owner_path).await;
                    let unknown = call(viewer, method, &unknown_path).await;
                    assert_eq!(
                        (hidden.status, hidden.json()),
                        (unknown.status, unknown.json()),
                        "{} {method} {owner_path}",
                        viewer.username()
                    );
                }
                viewer
                    .put(&format!("/api/friends/blocks/{UNKNOWN}"), json!({}))
                    .await
                    .assert_detail(404, "No account with that name");
            }
        }
    }
    Covered::Checked
}

/// Every kind the discovery tables name, across every case (five since FRIENDS added the friend;
/// the name stays, because the checklist names this test), in one world.
#[sqlx::test]
async fn every_personal_data_kind_has_its_four_viewer_cases(db: PgPool) {
    let w = World::new(db).await;
    let kinds = named_kinds();
    let mut matrix = Vec::new();
    for kind in &kinds {
        for case in CASES {
            let covered = check(&w, *kind, case).await;
            if let Covered::NotApplicable(reason) = &covered {
                assert!(
                    matches!(case, Case::UnknownName),
                    "{kind:?}: only the unknown-name case may be not applicable ({reason})"
                );
            }
            matrix.push(format!("{kind:?} / {case:?}: {covered:?}"));
        }
    }
    println!("{}", matrix.join("\n"));
    assert_eq!(matrix.len(), kinds.len() * CASES.len());
    // No case changed the owner's data.
    assert_eq!(items(&ok(&w.v.owner, "/api/shelf").await).len(), 4);
    assert_eq!(
        w.app
            .scalar_i64("SELECT count(*) FROM search_history")
            .await,
        1
    );
}
