//! Reports on what readers write (#55, Q11 option C): posts (club posts, replies and diary
//! entries, one kind), quotes, and the one-line take on a shelf entry. They land in the one admin
//! queue beside the pictures and books of Q11 option B (`media::reports`), which decides them
//! through [`decide`]. `docs/phase3/notes/texts.md` records the contract.
//!
//! **Filing** (`POST /api/posts/{post_id}/reports`, `POST /api/quotes/{quote_id}/reports`,
//! `POST /api/books/works/{work_id}/takes/{username}/reports`): only a text the reporter can see
//! through the visibility policy (`bc_can_see_post`; `bc_can_see_item` for a quote and for a
//! take with its entry's `rating_visibility`), else 404 like an unknown id. A tombstone, an empty
//! take and a text already gone are not reportable either (404); one an admin removed on a report
//! is answered with a report closed on arrival (`removed`), like a picture taken down already.
//! One's own text is 400 [`OWN_TEXT`], and `wrong_book` 400 [`TEXT_WRONG_BOOK`]. The rest is the
//! picture reports' rule: idempotent per reader and text (a repeat answers with the reader's
//! report), the shared limits (`report_request_user` for every request, first; `report_user` for
//! every new row), and the reader's turn on `REPORT_LOCK_SPACE` with the keys `<user>:post:<id>`,
//! `<user>:quote:<id>` and `<user>:take:<entry id>`.
//!
//! **The report** keeps the text as it was (`reports.text_snapshot`, at most 1000 characters, the
//! longest text kind), its author (`target_user_id`) and its book (`work_id`). The snapshot is for
//! the instance admin only: the reporter's export leaves it out, and the author's account
//! deletion clears it.
//!
//! **"Remove"** withdraws the text as it is now (an edit since the report included): a post
//! goes the way its author's delete takes it (`posts::edit::withdraw`: a tombstone when it has
//! replies, else deleted), a quote is deleted (its `quote_added` events lose their stored
//! excerpt), a take is cleared to `''` with the rating kept (its `shelf_finished` events lose the
//! reported and the current take). "Dismiss" changes nothing. Either way every open report of
//! the same text is resolved with it, before the text goes (a deleted post's or quote's reports
//! lose their `post_id` or `quote_id`). A text its author deleted since (or emptied, for a take)
//! is **gone**: "remove" is 409 [`TEXT_GONE`], "dismiss" closes the report.
//!
//! **Lock order** of a report and a decision: the reader's turn (reports only), the author's
//! `users` row `FOR KEY SHARE` (`media::reports::lock_owner`), the text's row (`FOR SHARE` to
//! report, `FOR UPDATE` to decide), then the `reports` rows (and, for a removal, the author's
//! events). Account deletion and work merges lock the user row before the author's posts,
//! quotes, entries and events, and a shelf write locks it `FOR UPDATE` before the entry, so the
//! user row comes first here too; a report's `target_user_id` would take it through its foreign
//! key anyway.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use sqlx::{Connection, PgConnection};

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::auth::rules::normalize_username;
use crate::catalog::detail::require_local_work;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath};
use crate::media::reports::{REPORT_LOCK_SPACE, ReportCreated, ReportIn, created, lock_owner};
use crate::shelf::quotes::NO_QUOTE;
use crate::text::truncate_chars;

/// 400 for a report of one's own post, quote or take.
pub const OWN_TEXT: &str = "You cannot report your own text";
/// 400 for a text reported as showing the wrong book (a reason for covers and books only).
pub const TEXT_WRONG_BOOK: &str = "A text cannot be reported as the wrong book";
/// 409 for "remove" on a report whose text its author deleted since (admin only).
pub const TEXT_GONE: &str = "The reported text is no longer online";
/// How much of a removed text its author's statement of reasons quotes.
const EXCERPT_CHARS: usize = 200;

/// A reported text, by its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTarget {
    /// A post: a club post, a reply or a diary entry.
    Post(i64),
    Quote(i64),
    /// The take on this shelf entry.
    Take(i64),
}

impl TextTarget {
    /// The target of a report row, if it is a text report with its row still named.
    pub fn of(
        kind: &str,
        post_id: Option<i64>,
        quote_id: Option<i64>,
        shelf_entry_id: Option<i64>,
    ) -> Option<Self> {
        match kind {
            "post" => post_id.map(TextTarget::Post),
            "quote" => quote_id.map(TextTarget::Quote),
            "take" => shelf_entry_id.map(TextTarget::Take),
            _ => None,
        }
    }

    /// `reports.target_kind`.
    pub fn kind(self) -> &'static str {
        match self {
            TextTarget::Post(_) => "post",
            TextTarget::Quote(_) => "quote",
            TextTarget::Take(_) => "take",
        }
    }

    fn id(self) -> i64 {
        match self {
            TextTarget::Post(id) | TextTarget::Quote(id) | TextTarget::Take(id) => id,
        }
    }

    /// `reports.post_id`, `reports.quote_id` and `reports.shelf_entry_id`.
    fn columns(self) -> (Option<i64>, Option<i64>, Option<i64>) {
        match self {
            TextTarget::Post(id) => (Some(id), None, None),
            TextTarget::Quote(id) => (None, Some(id), None),
            TextTarget::Take(id) => (None, None, Some(id)),
        }
    }

    /// The advisory key of one reader's reports of this text (`REPORT_LOCK_SPACE`).
    fn lock_key(self, user_id: i64) -> String {
        format!("{user_id}:{}:{}", self.kind(), self.id())
    }

    /// The 404 of a text the reader cannot report: the kind's "no such id".
    fn not_found(self) -> AppError {
        AppError::not_found(match self {
            TextTarget::Post(_) => msg::NO_ENTRY,
            TextTarget::Quote(_) => NO_QUOTE,
            TextTarget::Take(_) => msg::NOT_FOUND,
        })
    }
}

/// Reports a post: a club post, a reply or a diary entry the reader can see, written by someone
/// else (`copyright`, `offensive` or `other`). 20 new reports per day and 30 requests per
/// minute, shared with every other report. Idempotent per reader and post.
#[utoipa::path(
    post,
    path = "/api/posts/{post_id}/reports",
    operation_id = "posts_report",
    tag = "posts",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("post_id" = i64, Path, description = "The post")),
    request_body = ReportIn,
    responses(
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`), or an admin already removed the post (`removed`, closed on arrival); a repeat names the same report"),
        (status = 400, body = ErrorBody, description = "`Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`, `A text cannot be reported as the wrong book`, `You cannot report your own text`, a non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No entry with that id` (also a tombstone or a post the caller cannot see)"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (20 new reports per day; 30 requests per minute, repeats included)")
    )
)]
pub async fn report_post(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(post_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ReportIn>,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    file(&state, user.id, TextTarget::Post(post_id), body).await
}

/// Reports a quote the reader can see, saved by someone else. Same limits and queue as every
/// report.
#[utoipa::path(
    post,
    path = "/api/quotes/{quote_id}/reports",
    operation_id = "shelf_report_quote",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("quote_id" = i64, Path, description = "The quote")),
    request_body = ReportIn,
    responses(
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`); a repeat names the same report"),
        (status = 400, body = ErrorBody, description = "`Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`, `A text cannot be reported as the wrong book`, `You cannot report your own text`, a non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No quote with that id` (also a quote the caller cannot see)"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (20 new reports per day; 30 requests per minute, repeats included)")
    )
)]
pub async fn report_quote(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(quote_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ReportIn>,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    file(&state, user.id, TextTarget::Quote(quote_id), body).await
}

/// Reports the take another reader wrote on a book (`ShelfItemOut.take`, a club pick reader's
/// `take`, a `shelf_finished` event's `take`), addressed by the book and the reader's name: the
/// take must show to the caller (the reader is in their audience and keeps rating and take
/// `audience`), else 404 `Not found`. Same limits and queue as every report.
#[utoipa::path(
    post,
    path = "/api/books/works/{work_id}/takes/{username}/reports",
    operation_id = "shelf_report_take",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("work_id" = String, Path, description = "A work id, `OL…W`, or a legacy key; a merged work follows the merge"),
        ("username" = String, Path, description = "Whose take")
    ),
    request_body = ReportIn,
    responses(
        (status = 201, body = ReportCreated, description = "The report is in the admin queue (`open`), or an admin already removed the take (`removed`, closed on arrival); a repeat names the same report"),
        (status = 400, body = ErrorBody, description = "`Input should be 'copyright', 'wrong_book', 'offensive' or 'other'`, `A text cannot be reported as the wrong book`, `You cannot report your own text`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`; `Not found`: no such reader, no take, or a take the caller cannot see"),
        (status = 429, body = ErrorBody, description = "`Slow down a little. Try again later.` (20 new reports per day; 30 requests per minute, repeats included)")
    )
)]
pub async fn report_take(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath((raw, username)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<ReportIn>,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    preflight(&state, user.id, &body)?;
    let mut conn = state.db.acquire().await?;
    let work_id = require_local_work(&mut conn, &raw).await?;
    let entry = sqlx::query_scalar!(
        "SELECT s.id FROM shelf_entries s JOIN users u ON u.id = s.user_id
         WHERE u.username = $1 AND s.work_id = $2",
        normalize_username(&username),
        work_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(msg::NOT_FOUND))?;
    drop(conn);
    filed(&state, user.id, TextTarget::Take(entry), body).await
}

/// Every report request, a repeat or a refusal too, counts first (30 a minute, shared with
/// picture and book reports); then a text reported as the wrong book is refused.
fn preflight(state: &AppState, user_id: i64, body: &ReportIn) -> ApiResult<()> {
    state
        .limits
        .report_request_user
        .check_and_hit(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    if body.reason == "wrong_book" {
        return Err(AppError::bad_request(TEXT_WRONG_BOOK));
    }
    Ok(())
}

/// [`preflight`], then [`filed`].
async fn file(
    state: &AppState,
    user_id: i64,
    target: TextTarget,
    body: ReportIn,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    preflight(state, user_id, &body)?;
    filed(state, user_id, target, body).await
}

/// A text as the reporter may see it.
struct Seen {
    author: Option<i64>,
    work_id: i64,
    /// The text; `None` for a tombstone or an empty take.
    text: Option<String>,
}

/// The text `target` as `viewer` may see it, `None` when it is missing or hidden from them. With
/// `lock`, the row is read `FOR SHARE`: a decision under way finishes first and this report
/// sees it, and a later one waits and resolves the report.
async fn seen(
    conn: &mut PgConnection,
    viewer: i64,
    target: TextTarget,
    lock: bool,
) -> sqlx::Result<Option<Seen>> {
    let row = match (target, lock) {
        (TextTarget::Post(id), false) => sqlx::query!(
            r#"SELECT p.author_id, p.work_id,
                      CASE WHEN p.deleted_at IS NULL THEN p.body END AS text
               FROM posts p WHERE p.id = $1 AND bc_can_see_post($2, p.id)"#,
            id,
            viewer
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| (row.author_id, row.work_id, row.text)),
        (TextTarget::Post(id), true) => sqlx::query!(
            r#"SELECT p.author_id, p.work_id,
                      CASE WHEN p.deleted_at IS NULL THEN p.body END AS text
               FROM posts p WHERE p.id = $1 AND bc_can_see_post($2, p.id)
               FOR SHARE OF p"#,
            id,
            viewer
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| (row.author_id, row.work_id, row.text)),
        (TextTarget::Quote(id), false) => sqlx::query!(
            r#"SELECT q.user_id, q.work_id, q.body FROM quotes q
               WHERE q.id = $1 AND bc_can_see_item($2, q.user_id, q.visibility)"#,
            id,
            viewer
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| (Some(row.user_id), row.work_id, Some(row.body))),
        (TextTarget::Quote(id), true) => sqlx::query!(
            r#"SELECT q.user_id, q.work_id, q.body FROM quotes q
               WHERE q.id = $1 AND bc_can_see_item($2, q.user_id, q.visibility)
               FOR SHARE OF q"#,
            id,
            viewer
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| (Some(row.user_id), row.work_id, Some(row.body))),
        (TextTarget::Take(id), false) => sqlx::query!(
            r#"SELECT s.user_id, s.work_id, nullif(s.take, '') AS take FROM shelf_entries s
               WHERE s.id = $1 AND bc_can_see_item($2, s.user_id, s.rating_visibility)"#,
            id,
            viewer
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| (Some(row.user_id), row.work_id, row.take)),
        (TextTarget::Take(id), true) => sqlx::query!(
            r#"SELECT s.user_id, s.work_id, nullif(s.take, '') AS take FROM shelf_entries s
               WHERE s.id = $1 AND bc_can_see_item($2, s.user_id, s.rating_visibility)
               FOR SHARE OF s"#,
            id,
            viewer
        )
        .fetch_optional(&mut *conn)
        .await?
        .map(|row| (Some(row.user_id), row.work_id, row.take)),
    };
    Ok(row.map(|(author, work_id, text)| Seen {
        author,
        work_id,
        text,
    }))
}

/// Whether an admin removed `target`'s text on a report (a decision, not a report closed on
/// arrival). Only a tombstoned post or an emptied take can still be read after that.
async fn removed_by_admin(conn: &mut PgConnection, target: TextTarget) -> sqlx::Result<bool> {
    let (post_id, quote_id, shelf_entry_id) = target.columns();
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM reports
                          WHERE target_kind = $1 AND status = 'removed' AND NOT closed_on_arrival
                            AND (post_id = $2 OR quote_id = $3 OR shelf_entry_id = $4))
           AS "removed!""#,
        target.kind(),
        post_id,
        quote_id,
        shelf_entry_id
    )
    .fetch_one(conn)
    .await
}

/// What a report of `target` finds: the text to file it on, or a status of `removed` for one an
/// admin removed already (closed on arrival). 404 for a text missing, hidden or gone, 400 for
/// the reader's own.
async fn reportable(
    conn: &mut PgConnection,
    user_id: i64,
    target: TextTarget,
    lock: bool,
) -> ApiResult<(Seen, &'static str)> {
    let seen = seen(conn, user_id, target, lock)
        .await?
        .ok_or_else(|| target.not_found())?;
    let status = if seen.text.is_some() {
        "open"
    } else if removed_by_admin(conn, target).await? {
        "removed"
    } else {
        return Err(target.not_found());
    };
    if seen.author == Some(user_id) {
        return Err(AppError::bad_request(OWN_TEXT));
    }
    Ok((seen, status))
}

/// The reader's report of `target` with this status, if any.
async fn earlier_report(
    conn: &mut PgConnection,
    user_id: i64,
    target: TextTarget,
    status: &str,
) -> sqlx::Result<Option<i64>> {
    let (post_id, quote_id, shelf_entry_id) = target.columns();
    sqlx::query_scalar!(
        "SELECT id FROM reports
         WHERE created_by = $1 AND target_kind = $2 AND status = $3
           AND (post_id = $4 OR quote_id = $5 OR shelf_entry_id = $6)
         ORDER BY id DESC LIMIT 1",
        user_id,
        target.kind(),
        status,
        post_id,
        quote_id,
        shelf_entry_id
    )
    .fetch_optional(conn)
    .await
}

/// Files the report, or answers with the reader's earlier one (see the module docs). A repeat is
/// answered from a plain read before any lock, as for picture reports (security review, Phase 2
/// Wave 1 #1); a new report takes the reader's turn, the author's row and the text's row, reads
/// again, and keeps the text as it is then.
async fn filed(
    state: &AppState,
    user_id: i64,
    target: TextTarget,
    body: ReportIn,
) -> ApiResult<(StatusCode, Json<ReportCreated>)> {
    let mut conn = state.db.acquire().await?;
    let (seen, status) = reportable(&mut conn, user_id, target, false).await?;
    if let Some(id) = earlier_report(&mut conn, user_id, target, status).await? {
        return Ok(created(id, status));
    }
    let mut tx = conn.begin().await?;
    sqlx::query!(
        "SELECT pg_advisory_xact_lock($1, hashtext($2))",
        REPORT_LOCK_SPACE,
        target.lock_key(user_id)
    )
    .execute(&mut *tx)
    .await?;
    lock_owner(&mut tx, seen.author).await?;
    let (seen, status) = reportable(&mut tx, user_id, target, true).await?;
    if let Some(id) = earlier_report(&mut tx, user_id, target, status).await? {
        tx.rollback().await?;
        return Ok(created(id, status));
    }
    state
        .limits
        .report_user
        .check_and_hit(&format!("u:{user_id}"))
        .map_err(|wait| wait.into_error(msg::SLOW_DOWN))?;
    let (post_id, quote_id, shelf_entry_id) = target.columns();
    let closed = status == "removed";
    let id = sqlx::query_scalar!(
        "INSERT INTO reports (target_kind, post_id, quote_id, shelf_entry_id, work_id,
                              target_user_id, text_snapshot, reason, details, created_by, status,
                              resolved_at, closed_on_arrival)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, CASE WHEN $12 THEN now() END, $12)
         RETURNING id",
        target.kind(),
        post_id,
        quote_id,
        shelf_entry_id,
        seen.work_id,
        seen.author,
        seen.text,
        body.reason,
        body.details.unwrap_or_default(),
        user_id,
        status,
        closed
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(created(id, status))
}

/// What a decision on a text report found, for its notices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided {
    /// The title of the book the text is on, if the book is known.
    pub book: Option<String>,
    /// The author, on a removal (the statement of reasons goes to them).
    pub owner: Option<i64>,
    /// "remove" withdrew a text.
    pub removed: bool,
    /// The text was gone before the decision: its author deleted it (or emptied the take).
    pub gone: bool,
    /// The withdrawn text, shortened, for its author's statement.
    pub excerpt: Option<String>,
}

/// A text report as the decision reads it.
#[derive(Debug, Clone, Copy)]
pub struct TextReport<'a> {
    pub kind: &'a str,
    pub post_id: Option<i64>,
    pub quote_id: Option<i64>,
    pub shelf_entry_id: Option<i64>,
    pub work_id: Option<i64>,
    pub author: Option<i64>,
}

/// Decides a text report inside the admin's transaction (see the module docs): locks the
/// author's row and the text's, resolves every open report of the text with `status`, and on
/// `remove` withdraws the text as it is now. A gone text is 409 [`TEXT_GONE`] on `remove`; on
/// `dismiss` the caller resolves the one report.
pub async fn decide(
    conn: &mut PgConnection,
    report: TextReport<'_>,
    remove: bool,
    admin_id: i64,
) -> ApiResult<Decided> {
    lock_owner(conn, report.author).await?;
    let target = TextTarget::of(
        report.kind,
        report.post_id,
        report.quote_id,
        report.shelf_entry_id,
    );
    let current = match target {
        Some(target) => locked_text(conn, target).await?,
        None => None,
    };
    let book = match report.work_id {
        Some(work_id) => {
            sqlx::query_scalar!("SELECT title FROM works WHERE id = $1", work_id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let (Some(target), Some(text)) = (target, current) else {
        if remove {
            return Err(AppError::conflict(TEXT_GONE));
        }
        return Ok(Decided {
            book,
            owner: None,
            removed: false,
            gone: true,
            excerpt: None,
        });
    };
    // The text's open reports first: a deleted post or quote clears their `post_id` or
    // `quote_id`, and nothing would tie them to this decision afterwards.
    let (post_id, quote_id, shelf_entry_id) = target.columns();
    sqlx::query!(
        "UPDATE reports SET status = $5, resolved_by = $6, resolved_at = now()
         WHERE status = 'open' AND target_kind = $1
           AND (post_id = $2 OR quote_id = $3 OR shelf_entry_id = $4)",
        target.kind(),
        post_id,
        quote_id,
        shelf_entry_id,
        if remove { "removed" } else { "dismissed" },
        admin_id
    )
    .execute(&mut *conn)
    .await?;
    if remove {
        withdraw(conn, target, &text).await?;
    }
    Ok(Decided {
        book,
        owner: report.author.filter(|_| remove),
        removed: remove,
        gone: false,
        excerpt: remove.then(|| truncate_chars(&text, EXCERPT_CHARS).to_owned()),
    })
}

/// The text of `target` as it is now, its row locked `FOR UPDATE`; `None` when its author
/// deleted it (or emptied the take).
async fn locked_text(conn: &mut PgConnection, target: TextTarget) -> sqlx::Result<Option<String>> {
    Ok(match target {
        TextTarget::Post(id) => sqlx::query_scalar!(
            "SELECT CASE WHEN deleted_at IS NULL THEN body END FROM posts WHERE id = $1 FOR UPDATE",
            id
        )
        .fetch_optional(conn)
        .await?
        .flatten(),
        TextTarget::Quote(id) => {
            sqlx::query_scalar!("SELECT body FROM quotes WHERE id = $1 FOR UPDATE", id)
                .fetch_optional(conn)
                .await?
        }
        TextTarget::Take(id) => sqlx::query_scalar!(
            "SELECT nullif(take, '') FROM shelf_entries WHERE id = $1 FOR UPDATE",
            id
        )
        .fetch_optional(conn)
        .await?
        .flatten(),
    })
}

/// "Remove": withdraws the text (locked by [`locked_text`]) wherever it shows.
async fn withdraw(conn: &mut PgConnection, target: TextTarget, text: &str) -> sqlx::Result<()> {
    match target {
        TextTarget::Post(id) => {
            crate::posts::edit::withdraw(conn, id).await?;
        }
        TextTarget::Quote(id) => {
            // Other readers' feeds hide a `quote_added` whose quote is gone; the owner's own
            // feed showed the stored excerpt.
            sqlx::query!(
                "UPDATE events e SET payload = e.payload - 'excerpt'
                 FROM quotes q
                 WHERE q.id = $1 AND e.kind = 'quote_added' AND e.actor_id = q.user_id
                   AND e.payload ->> 'quote_id' = $1::int8::text",
                id
            )
            .execute(&mut *conn)
            .await?;
            sqlx::query!("DELETE FROM quotes WHERE id = $1", id)
                .execute(&mut *conn)
                .await?;
        }
        TextTarget::Take(id) => {
            // A finish's event shows the take it was written with: the reported versions and
            // the current one go from the feeds too. Older takes of the entry stay (nobody
            // reported them).
            sqlx::query!(
                "UPDATE events e SET payload = e.payload - 'take'
                 FROM shelf_entries s
                 WHERE s.id = $1 AND e.kind = 'shelf_finished' AND e.actor_id = s.user_id
                   AND e.work_id = s.work_id
                   AND (e.payload ->> 'take' = $2
                        OR e.payload ->> 'take' IN (SELECT r.text_snapshot FROM reports r
                                                    WHERE r.shelf_entry_id = $1
                                                      AND r.text_snapshot IS NOT NULL))",
                id,
                text
            )
            .execute(&mut *conn)
            .await?;
            sqlx::query!("UPDATE shelf_entries SET take = '' WHERE id = $1", id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// The routes of the readers' text reports; the decisions go through the admin queue
/// (`media::reports::resolve_report`).
pub fn routes() -> utoipa_axum::router::OpenApiRouter<AppState> {
    utoipa_axum::router::OpenApiRouter::new()
        .routes(utoipa_axum::routes!(report_post))
        .routes(utoipa_axum::routes!(report_quote))
        .routes(utoipa_axum::routes!(report_take))
}
