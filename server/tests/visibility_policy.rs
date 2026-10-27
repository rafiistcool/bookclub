//! The #46 matrix (docs/phase1/conventions.md §6.5): the SQL policy functions and
//! `crate::visibility` across viewer types (self, co-member, member of another club, stranger,
//! former co-member, unknown name), item visibility, a diary tombstone of a deleted account,
//! and NULL owners.

mod common;

use bookclub_server::clubs::extract::Role;
use bookclub_server::visibility::{self, Relation};
use common::visibility::{UNKNOWN, Viewers};
use common::*;
use sqlx::{AssertSqlSafe, PgPool};

/// A boolean the policy function returns; panics on NULL (scalar functions never return NULL).
async fn flag(app: &TestApp, sql: String) -> bool {
    sqlx::query_scalar::<_, Option<bool>>(AssertSqlSafe(sql.clone()))
        .fetch_one(&app.db)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .unwrap_or_else(|| panic!("{sql} returned NULL"))
}

async fn ids(app: &TestApp, sql: String) -> Vec<i64> {
    let mut ids: Vec<i64> = sqlx::query_scalar(AssertSqlSafe(sql))
        .fetch_all(&app.db)
        .await
        .unwrap();
    ids.sort_unstable();
    ids
}

async fn insert(app: &TestApp, sql: String) -> i64 {
    app.scalar_i64(&sql).await
}

async fn diary_root(app: &TestApp, work: i64, author: i64, visibility: &str) -> i64 {
    insert(
        app,
        format!(
            "INSERT INTO posts (work_id, author_id, visibility, body)
             VALUES ({work}, {author}, '{visibility}', 'Root') RETURNING id"
        ),
    )
    .await
}

async fn reply(app: &TestApp, work: i64, root: i64, author: i64) -> i64 {
    insert(
        app,
        format!(
            "INSERT INTO posts (work_id, author_id, parent_id, body)
             VALUES ({work}, {author}, {root}, 'Reply') RETURNING id"
        ),
    )
    .await
}

#[sqlx::test]
async fn relation_and_can_see_user_follow_shared_clubs(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let owner = v.owner.id();
    let mut conn = app.db.acquire().await.unwrap();
    let cases = [
        (&v.owner, Relation::Myself),
        (&v.co_member, Relation::Audience),
        (&v.other_club, Relation::Stranger),
        (&v.stranger, Relation::Stranger),
        (&v.former, Relation::Stranger),
    ];
    for (viewer, expected) in cases {
        let relation = visibility::relation(&mut conn, viewer.id(), owner)
            .await
            .unwrap();
        assert_eq!(relation, expected, "{}", viewer.username());
        let sees = flag(
            &app,
            format!("SELECT bc_can_see_user({}, {owner})", viewer.id()),
        )
        .await;
        assert_eq!(sees, expected.sees_profile(), "{}", viewer.username());
    }
    // Not transitive: otherclub shares a club with comember, who shares one with the owner.
    assert_eq!(
        visibility::relation(&mut conn, v.other_club.id(), v.co_member.id())
            .await
            .unwrap(),
        Relation::Audience
    );
    // An unknown owner is a stranger.
    assert_eq!(
        visibility::relation(&mut conn, v.co_member.id(), 999_999)
            .await
            .unwrap(),
        Relation::Stranger
    );
}

#[sqlx::test]
async fn visible_user_hides_strangers_like_unknown_names(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let mut conn = app.db.acquire().await.unwrap();
    for viewer in [&v.owner, &v.co_member] {
        let row = visibility::visible_user(&mut conn, viewer.id(), " Owner ")
            .await
            .unwrap();
        assert_eq!((row.id, row.username.as_str()), (v.owner.id(), "owner"));
        assert_eq!(row.avatar_public_id, None);
    }
    for viewer in v.strangers() {
        let error = visibility::visible_user(&mut conn, viewer.id(), "owner")
            .await
            .unwrap_err();
        assert_eq!(
            (error.status.as_u16(), error.detail.as_ref()),
            (404, "No member with that name")
        );
    }
    // A NUL character (which Postgres cannot compare) names nobody, also no stranger.
    for name in [UNKNOWN, "owner\u{0}", "\u{0}"] {
        let unknown = visibility::visible_user(&mut conn, v.co_member.id(), name)
            .await
            .unwrap_err();
        assert_eq!(
            (unknown.status.as_u16(), unknown.detail.as_ref()),
            (404, "No member with that name"),
            "{name:?}"
        );
    }
}

#[sqlx::test]
async fn only_me_items_are_for_the_owner_alone(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let owner = v.owner.id();
    let expectations = [
        (&v.owner, true, true),
        (&v.co_member, true, false),
        (&v.other_club, false, false),
        (&v.stranger, false, false),
        (&v.former, false, false),
    ];
    for (viewer, audience, only_me) in expectations {
        let id = viewer.id();
        assert_eq!(
            flag(
                &app,
                format!("SELECT bc_can_see_item({id}, {owner}, 'audience')")
            )
            .await,
            audience,
            "{}",
            viewer.username()
        );
        assert_eq!(
            flag(
                &app,
                format!("SELECT bc_can_see_item({id}, {owner}, 'only_me')")
            )
            .await,
            only_me,
            "{}",
            viewer.username()
        );
    }
}

#[sqlx::test]
async fn visible_shelf_redacts_only_me_ratings_for_others(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let owner = v.owner.id();
    let (a, b, c, d) = (
        app.work("A").await,
        app.work("B").await,
        app.work("C").await,
        app.work("D").await,
    );
    app.exec(&format!(
        "INSERT INTO shelf_entries (user_id, work_id, status, rating, rating_visibility, take, dnf_reason, progress)
         VALUES ({owner}, {}, 'finished', 4, 'audience', 'Fine', '', NULL),
                ({owner}, {}, 'finished', 5, 'only_me', 'Secret', '', NULL),
                ({owner}, {}, 'did_not_finish', NULL, 'only_me', '', 'Boring', NULL),
                ({owner}, {}, 'currently_reading', NULL, 'only_me', '', '', 40)",
        a.id, b.id, c.id, d.id
    ))
    .await;

    type Row = (
        i64,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<bool>,
    );
    async fn rows(app: &TestApp, viewer: i64, owner: i64) -> Vec<Row> {
        sqlx::query_as(AssertSqlSafe(format!(
            "SELECT work_id, rating, rating_visibility, take, dnf_reason, progress, is_self
             FROM bc_visible_shelf({viewer}) WHERE user_id = {owner} ORDER BY work_id"
        )))
        .fetch_all(&app.db)
        .await
        .unwrap()
    }
    let some = |s: &str| Some(s.to_owned());

    let own = rows(&app, owner, owner).await;
    assert_eq!(
        own,
        vec![
            (
                a.id,
                Some(4),
                some("audience"),
                some("Fine"),
                some(""),
                None,
                Some(true)
            ),
            (
                b.id,
                Some(5),
                some("only_me"),
                some("Secret"),
                some(""),
                None,
                Some(true)
            ),
            (
                c.id,
                None,
                some("only_me"),
                some(""),
                some("Boring"),
                None,
                Some(true)
            ),
            (
                d.id,
                None,
                some("only_me"),
                some(""),
                some(""),
                Some(40),
                Some(true)
            ),
        ]
    );
    let audience = rows(&app, v.co_member.id(), owner).await;
    assert_eq!(
        audience,
        vec![
            (
                a.id,
                Some(4),
                None,
                some("Fine"),
                some(""),
                None,
                Some(false)
            ),
            (b.id, None, None, some(""), some(""), None, Some(false)),
            // dnf_reason and progress are shelf state: the audience sees them.
            (
                c.id,
                None,
                None,
                some(""),
                some("Boring"),
                None,
                Some(false)
            ),
            (d.id, None, None, some(""), some(""), Some(40), Some(false)),
        ]
    );
    for viewer in v.strangers() {
        assert!(
            rows(&app, viewer.id(), owner).await.is_empty(),
            "{}",
            viewer.username()
        );
    }
}

#[sqlx::test]
async fn visible_quotes_hide_only_me_quotes_from_others(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let owner = v.owner.id();
    let work = app.work("Circe").await;
    let public = insert(
        &app,
        format!(
            "INSERT INTO quotes (user_id, work_id, body, visibility)
             VALUES ({owner}, {}, 'Public', 'audience') RETURNING id",
            work.id
        ),
    )
    .await;
    let private = insert(
        &app,
        format!(
            "INSERT INTO quotes (user_id, work_id, body, visibility)
             VALUES ({owner}, {}, 'Private', 'only_me') RETURNING id",
            work.id
        ),
    )
    .await;
    let seen = |viewer: i64| {
        ids(
            &app,
            format!("SELECT id FROM bc_visible_quotes({viewer}) WHERE user_id = {owner}"),
        )
    };
    assert_eq!(seen(owner).await, vec![public, private]);
    assert_eq!(seen(v.co_member.id()).await, vec![public]);
    for viewer in v.strangers() {
        assert!(seen(viewer.id()).await.is_empty(), "{}", viewer.username());
    }
    // The owner's quote count includes the private quote; nobody else's does.
    for (viewer, expected) in [(owner, 2), (v.co_member.id(), 1)] {
        let count = app
            .scalar_i64(&format!(
                "SELECT count(*) FROM bc_visible_quotes({viewer}) WHERE user_id = {owner}"
            ))
            .await;
        assert_eq!(count, expected);
    }
}

#[sqlx::test]
async fn visible_owners_and_member_clubs(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let rows: Vec<(i64, bool)> = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT user_id, is_self FROM bc_visible_owners({}) ORDER BY user_id",
        v.co_member.id()
    )))
    .fetch_all(&app.db)
    .await
    .unwrap();
    let mut expected = vec![
        (v.owner.id(), false),
        (v.co_member.id(), true),
        (v.other_club.id(), false),
    ];
    expected.sort_unstable();
    assert_eq!(rows, expected);
    let alone: Vec<(i64, bool)> = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT user_id, is_self FROM bc_visible_owners({}) ORDER BY user_id",
        v.stranger.id()
    )))
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(alone, vec![(v.stranger.id(), true)]);

    let clubs = ids(
        &app,
        format!("SELECT club_id FROM bc_member_clubs({})", v.co_member.id()),
    )
    .await;
    let mut expected = vec![v.shared.id, v.other.id];
    expected.sort_unstable();
    assert_eq!(clubs, expected);
    assert!(
        flag(
            &app,
            format!("SELECT bc_is_member({}, {})", v.co_member.id(), v.shared.id)
        )
        .await
    );
    for viewer in [&v.stranger, &v.former, &v.other_club] {
        assert!(
            !flag(
                &app,
                format!("SELECT bc_is_member({}, {})", viewer.id(), v.shared.id)
            )
            .await
        );
    }
}

/// §6.3: a diary reply is visible when its root is visible and the viewer is the reply's
/// author, the root's author, or in the reply author's audience. The replies by `former` and
/// `otherclub` (whom the owner does not see) decide the root-author and audience clauses.
#[sqlx::test]
async fn diary_roots_and_replies(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let work = app.work("Circe").await.id;
    let open = diary_root(&app, work, v.owner.id(), "audience").await;
    let private = diary_root(&app, work, v.owner.id(), "only_me").await;
    let answer = reply(&app, work, open, v.co_member.id()).await;
    // Stands for a reply written before former left `shared`: the policy reads only today's
    // memberships.
    let by_former = reply(&app, work, open, v.former.id()).await;
    let by_other_club = reply(&app, work, open, v.other_club.id()).await;
    let all = [open, private, answer, by_former, by_other_club];

    let expectations: [(&TestClient, &[i64]); 5] = [
        // The root's author sees every reply, also from people outside their audience.
        (&v.owner, &all),
        // comember sees otherclub (club `other`), but former is in none of their clubs.
        (&v.co_member, &[open, answer, by_other_club]),
        // otherclub can see comember, but not the owner's root, so no reply either, not even
        // their own.
        (&v.other_club, &[]),
        (&v.stranger, &[]),
        (&v.former, &[]),
    ];
    let list = all.map(|id| id.to_string()).join(", ");
    for (viewer, expected) in expectations {
        let seen = ids(
            &app,
            format!(
                "SELECT id FROM bc_visible_posts({}) WHERE id IN ({list})",
                viewer.id()
            ),
        )
        .await;
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(seen, expected, "{}", viewer.username());
        for post in all {
            assert_eq!(
                flag(
                    &app,
                    format!("SELECT bc_can_see_post({}, {post})", viewer.id())
                )
                .await,
                expected.contains(&post),
                "{} post {post}",
                viewer.username()
            );
        }
    }
}

/// A diary root whose author deleted the account stays visible to whoever can see the author of
/// one of its replies; each reply keeps its own rule.
#[sqlx::test]
async fn tombstone_of_a_deleted_account(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let gone = app.user("gone").await;
    // gone shares one club with comember and another with stranger.
    let with_co_member = app.club(&v.co_member, "Gone A").await;
    app.join(&with_co_member, &gone, "member").await;
    let with_stranger = app.club(&v.stranger, "Gone B").await;
    app.join(&with_stranger, &gone, "member").await;

    let work = app.work("Circe").await.id;
    let root = diary_root(&app, work, gone.id(), "audience").await;
    let by_co_member = reply(&app, work, root, v.co_member.id()).await;
    let by_stranger = reply(&app, work, root, v.stranger.id()).await;
    // Account deletion (conventions §5.6): the root has replies, so it becomes a tombstone,
    // then the author's id becomes NULL through the foreign key.
    app.exec(&format!(
        "UPDATE posts SET body = '', spoiler_upto = NULL, deleted_at = now() WHERE id = {root};
         DELETE FROM users WHERE id = {};",
        gone.id()
    ))
    .await;
    assert_eq!(
        app.scalar_i64(&format!(
            "SELECT count(*) FROM posts WHERE id = {root} AND author_id IS NULL"
        ))
        .await,
        1
    );

    let expectations: [(&TestClient, &[i64]); 5] = [
        // The owner sees comember (shared club), so the root and comember's reply.
        (&v.owner, &[root, by_co_member]),
        (&v.co_member, &[root, by_co_member]),
        // otherclub shares a club with comember.
        (&v.other_club, &[root, by_co_member]),
        (&v.stranger, &[root, by_stranger]),
        (&v.former, &[]),
    ];
    for (viewer, expected) in expectations {
        let seen = ids(
            &app,
            format!(
                "SELECT id FROM bc_visible_posts({}) WHERE id IN ({root}, {by_co_member}, {by_stranger})",
                viewer.id()
            ),
        )
        .await;
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(seen, expected, "{}", viewer.username());
    }

    let mut conn = app.db.acquire().await.unwrap();
    let error = visibility::post_access(&mut conn, v.owner.id(), root, false)
        .await
        .unwrap_err();
    assert_eq!(
        (error.status.as_u16(), error.detail.as_ref()),
        (404, "No entry with that id")
    );
    let access = visibility::post_access(&mut conn, v.owner.id(), root, true)
        .await
        .unwrap();
    assert!(access.deleted);
    assert_eq!(access.author_id, None);
    assert_eq!(access.club_id, None);
    assert_eq!(access.club_role, None);
}

#[sqlx::test]
async fn club_posts_are_for_members_of_that_club(db: PgPool) {
    let app = TestApp::new(db).await;
    let v = Viewers::new(&app).await;
    let work = app.work("Circe").await.id;
    let pick = insert(
        &app,
        format!(
            "INSERT INTO club_picks (club_id, work_id, set_by) VALUES ({}, {work}, {}) RETURNING id",
            v.shared.id,
            v.owner.id()
        ),
    )
    .await;
    let post = insert(
        &app,
        format!(
            "INSERT INTO posts (work_id, author_id, club_id, pick_id, body)
             VALUES ({work}, {}, {}, {pick}, 'Chapter one') RETURNING id",
            v.co_member.id(),
            v.shared.id
        ),
    )
    .await;
    let mut conn = app.db.acquire().await.unwrap();
    for (viewer, role) in [(&v.owner, Role::Owner), (&v.co_member, Role::Member)] {
        let access = visibility::post_access(&mut conn, viewer.id(), post, false)
            .await
            .unwrap();
        assert_eq!(access.post_id, post);
        assert_eq!(access.work_id, work);
        assert_eq!(access.author_id, Some(v.co_member.id()));
        assert_eq!(access.parent_id, None);
        assert_eq!(access.club_id, Some(v.shared.id));
        assert_eq!(access.pick_id, Some(pick));
        assert_eq!(access.club_role, Some(role));
        assert!(!access.deleted);
    }
    for viewer in v.strangers() {
        let error = visibility::post_access(&mut conn, viewer.id(), post, true)
            .await
            .unwrap_err();
        assert_eq!(
            (error.status.as_u16(), error.detail.as_ref()),
            (404, "No entry with that id"),
            "{}",
            viewer.username()
        );
    }
    let missing = visibility::post_access(&mut conn, v.owner.id(), 999_999, true)
        .await
        .unwrap_err();
    assert_eq!(missing.status.as_u16(), 404);
}

/// The scalar functions never return NULL, also for a NULL owner (a deleted account).
#[sqlx::test]
async fn scalar_functions_return_false_for_null_owners(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await.id();
    for sql in [
        format!("SELECT bc_can_see_user({ada}, NULL)"),
        format!("SELECT bc_can_see_item({ada}, NULL, 'audience')"),
        format!("SELECT bc_can_see_item({ada}, NULL, 'only_me')"),
        format!("SELECT bc_can_see_post({ada}, NULL)"),
        format!("SELECT bc_is_member({ada}, NULL)"),
        "SELECT bc_can_see_user(NULL, NULL)".to_owned(),
        "SELECT bc_can_see_item(NULL, NULL, 'audience')".to_owned(),
    ] {
        assert!(!flag(&app, sql.clone()).await, "{sql}");
    }
    // The viewer always sees their own items.
    assert!(
        flag(
            &app,
            format!("SELECT bc_can_see_item({ada}, {ada}, 'only_me')")
        )
        .await
    );
}

/// Wave 1 exit criterion (work-plan "Wave 1"), at the policy level: two members of one club see
/// each other's (empty) shelves; a non-member sees neither.
#[sqlx::test]
async fn club_members_see_each_others_shelves(db: PgPool) {
    let app = TestApp::new(db).await;
    let ada = app.user("ada").await;
    let grace = app.user("grace").await;
    let mallory = app.user("mallory").await;
    let club = app.club(&ada, "Thursday Readers").await;
    app.join(&club, &grace, "member").await;
    for (viewer, owner) in [(&ada, &grace), (&grace, &ada)] {
        assert!(
            flag(
                &app,
                format!("SELECT bc_can_see_user({}, {})", viewer.id(), owner.id())
            )
            .await
        );
        assert_eq!(
            app.scalar_i64(&format!(
                "SELECT count(*) FROM bc_visible_shelf({}) WHERE user_id = {}",
                viewer.id(),
                owner.id()
            ))
            .await,
            0
        );
    }
    for owner in [&ada, &grace] {
        assert!(
            !flag(
                &app,
                format!("SELECT bc_can_see_user({}, {})", mallory.id(), owner.id())
            )
            .await
        );
    }
}
