//! Database helpers outside the request path: `admin::make_admin` (the CLI) and the frozen
//! `dto` loaders (docs/phase1/conventions.md §20).

use bookclub_server::{admin, dto};
use sqlx::PgPool;

async fn insert_user(db: &PgPool, username: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO users (username, password_hash) VALUES ($1, 'x') RETURNING id")
        .bind(username)
        .fetch_one(db)
        .await
        .unwrap()
}

/// Gives the user an avatar row with this public id, taken down or not.
async fn set_avatar(db: &PgPool, user_id: i64, public_id: &str, removed: bool) {
    sqlx::query(
        "WITH avatar AS (
             INSERT INTO media (public_id, kind, source, uploaded_by, removed_at)
             VALUES ($2, 'avatar', 'upload', $1, CASE WHEN $3 THEN now() END)
             RETURNING id
         )
         UPDATE users SET avatar_media_id = (SELECT id FROM avatar) WHERE id = $1",
    )
    .bind(user_id)
    .bind(public_id)
    .bind(removed)
    .execute(db)
    .await
    .unwrap();
}

#[sqlx::test]
async fn make_admin_promotes_exactly_the_named_user(db: PgPool) {
    insert_user(&db, "ada").await;
    insert_user(&db, "bob").await;
    let mut conn = db.acquire().await.unwrap();

    assert!(admin::make_admin(&mut conn, "  Ada ").await.unwrap());
    assert!(!admin::make_admin(&mut conn, "nobody").await.unwrap());

    let admins: Vec<String> = sqlx::query_scalar("SELECT username FROM users WHERE is_admin")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(admins, ["ada"]);
}

#[sqlx::test]
async fn user_summaries_leave_out_removed_avatars(db: PgPool) {
    let ada = insert_user(&db, "ada").await;
    let bob = insert_user(&db, "bob").await;
    let cy = insert_user(&db, "cy").await;
    set_avatar(&db, ada, "aaaaaaaaaaaaaaaaaaaaaaaaaa", false).await;
    set_avatar(&db, bob, "bbbbbbbbbbbbbbbbbbbbbbbbbb", true).await;
    let mut conn = db.acquire().await.unwrap();

    let missing = cy + 1000;
    let summaries = dto::user_summaries(&mut conn, &[ada, bob, cy, missing])
        .await
        .unwrap();

    assert_eq!(summaries.len(), 3);
    assert!(!summaries.contains_key(&missing));
    assert_eq!(summaries[&ada].username, "ada");
    assert_eq!(
        summaries[&ada].avatar_url.as_deref(),
        Some("/media/aaaaaaaaaaaaaaaaaaaaaaaaaa/a")
    );
    assert_eq!(summaries[&bob].username, "bob");
    assert_eq!(summaries[&bob].avatar_url, None);
    assert_eq!(summaries[&cy].avatar_url, None);
}
