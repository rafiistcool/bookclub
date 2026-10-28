//! Shelf entries: the shelf list, add, change and remove (endpoint-map §7,
//! docs/phase1/spec/shelf.md §1.1, §1.2, §1.7, §1.8 and §2).
//!
//! Writes run in one transaction under the user's row lock: the entry's new state is computed
//! with the `ordering` rules, written in one statement (the status CHECKs see a consistent row),
//! and the other entries of the touched columns are renumbered. The shelf events (§2.6) are
//! recorded in the same transaction.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use time::{Date, OffsetDateTime};
use utoipa::ToSchema;

use super::favorites::{FavoriteOut, favorite_outs};
use super::layout::remove_from_layout;
use super::ordering::{
    ShelfSlot, StatusFields, apply_finish_fields, apply_progress, apply_reading_dates,
    column_order, shelf_order,
};
use super::{OwnerQuery, lock_user, resolve_owner};
use crate::AppState;
use crate::activity::{EventKind, NewEvent, hide_finished_ratings, record};
use crate::auth::extract::CurrentUser;
use crate::catalog::shared::books::{
    BookRef, book_outs, edition_outs, require_edition_of, require_work,
};
use crate::dto::{BookOut, EditionOut, ShelfStatus, UserSummary, Visibility};
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiPath, ApiQuery, INVALID_VALUE, Patch, Validate, Validator};
use crate::text::{char_len, py_strip};
use crate::tz;

pub const ALREADY_ON_SHELF: &str = "Already on your shelf";
pub const NOT_ON_SHELF: &str = "Book not on your shelf";
pub const NOTHING_TO_UPDATE: &str = "Nothing to update";
pub const TAKE_TOO_LONG: &str = "Keep the take to 140 characters";
pub const REASON_TOO_LONG: &str = "Keep the reason to 200 characters";
const MAX_TAKE_CHARS: usize = 140;
const MAX_REASON_CHARS: usize = 200;

/// One entry of a shelf. For anyone but the owner, an `only_me` rating is redacted: `rating`
/// null, `take` empty and `rating_visibility` null.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ShelfItemOut {
    pub id: i64,
    pub status: ShelfStatus,
    /// 0-based order inside the (user, status) column.
    pub position: i32,
    /// The owner's manual order across every status; null until the shelf is first arranged.
    pub shelf_position: Option<i32>,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    /// The work, with the entry's edition as context (its title, pages and cover win).
    pub book: BookOut,
    /// The edition the owner reads or owns.
    pub edition: Option<EditionOut>,
    /// 1 to 5, only for finished books.
    pub rating: Option<i32>,
    /// Only the owner sees it; null for everyone else.
    pub rating_visibility: Option<Visibility>,
    /// At most 140 characters, only for finished books; `""` when there is none.
    pub take: String,
    /// At most 200 characters, only for books not finished; `""` when there is none.
    pub dnf_reason: String,
    /// Percent read, only while reading.
    pub progress: Option<i32>,
}

/// A user's whole shelf and favourites.
#[derive(Debug, Serialize, ToSchema)]
pub struct ShelfListOut {
    /// The owner's time zone, for showing reading dates.
    pub timezone: String,
    /// The owner of the shelf.
    pub user: UserSummary,
    /// Ordered by status (alphabetically), position, id.
    pub items: Vec<ShelfItemOut>,
    /// The owner's favourites, by position.
    pub favorites: Vec<FavoriteOut>,
}

/// A book to put on the caller's shelf. Book metadata comes from the catalog only.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ShelfAddIn {
    /// The work; a merged work resolves to the work it was merged into.
    pub work_id: i64,
    /// The edition the caller reads or owns, an edition of that work.
    #[serde(default)]
    pub edition_id: Option<i64>,
    /// Default `want_to_read`.
    #[serde(default = "want_to_read")]
    pub status: ShelfStatus,
    /// 1 to 5; kept only for `finished`.
    #[serde(default)]
    pub rating: Option<i32>,
    /// Who sees the rating and take; default `audience`.
    #[serde(default)]
    pub rating_visibility: Option<Visibility>,
    /// Trimmed, at most 140 characters; kept only for `finished`.
    #[serde(default)]
    pub take: String,
    /// Trimmed, at most 200 characters; kept only for `did_not_finish`.
    #[serde(default)]
    pub dnf_reason: String,
    /// 0 to 100; kept only for `currently_reading`.
    #[serde(default)]
    pub progress: Option<i32>,
}

fn want_to_read() -> ShelfStatus {
    ShelfStatus::WantToRead
}

impl Validate for ShelfAddIn {
    fn validate(&mut self, v: &mut Validator) {
        v.range_i64(self.rating.map(i64::from), 1, 5);
        self.take = checked_text(&self.take, MAX_TAKE_CHARS, TAKE_TOO_LONG, v);
        self.dnf_reason = checked_text(&self.dnf_reason, MAX_REASON_CHARS, REASON_TOO_LONG, v);
        v.range_i64(self.progress.map(i64::from), 0, 100);
    }
}

/// Changes to one of the caller's entries. `status`, `position`, `shelf_position` and
/// `rating_visibility` treat null as absent; for the other fields null clears the value and an
/// absent key keeps it.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ShelfPatchIn {
    /// The reading date of a finished book (`YYYY-MM-DD`, not in the future in the owner's
    /// zone), stored as noon local time; null clears it.
    #[serde(default)]
    #[schema(value_type = Option<Date>)]
    pub finished_on: Patch<Date>,
    #[serde(default)]
    pub status: Option<ShelfStatus>,
    /// The position inside the status column (clamped).
    #[serde(default)]
    pub position: Option<i64>,
    /// The position in the free order across every status (clamped). Alone it changes nothing
    /// else, not even `updated_at`.
    #[serde(default)]
    pub shelf_position: Option<i64>,
    #[serde(default)]
    #[schema(value_type = Option<i32>)]
    pub rating: Patch<i32>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub take: Patch<String>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub dnf_reason: Patch<String>,
    #[serde(default)]
    #[schema(value_type = Option<i32>)]
    pub progress: Patch<i32>,
    /// An edition of the entry's work; null clears it.
    #[serde(default)]
    #[schema(value_type = Option<i64>)]
    pub edition_id: Patch<i64>,
    #[serde(default)]
    pub rating_visibility: Option<Visibility>,
}

impl Validate for ShelfPatchIn {
    fn validate(&mut self, v: &mut Validator) {
        if let Patch::Value(date) = self.finished_on {
            v.check(tz::personal_date_in_range(date), INVALID_VALUE);
        }
        v.range_i64(self.position, 0, i64::MAX);
        v.range_i64(self.shelf_position, 0, i64::MAX);
        if let Patch::Value(rating) = self.rating {
            v.range_i64(Some(rating.into()), 1, 5);
        }
        if let Patch::Value(take) = &mut self.take {
            *take = checked_text(take, MAX_TAKE_CHARS, TAKE_TOO_LONG, v);
        }
        if let Patch::Value(reason) = &mut self.dnf_reason {
            *reason = checked_text(reason, MAX_REASON_CHARS, REASON_TOO_LONG, v);
        }
        if let Patch::Value(progress) = self.progress {
            v.range_i64(Some(progress.into()), 0, 100);
        }
    }
}

impl ShelfPatchIn {
    /// Nothing to change: the legacy check (`Nothing to update`), which runs before the entry
    /// is looked up.
    fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.position.is_none()
            && self.shelf_position.is_none()
            && self.rating.is_missing()
            && self.take.is_missing()
            && self.dnf_reason.is_missing()
            && self.progress.is_missing()
            && self.finished_on.is_missing()
            && self.edition_id.is_missing()
            && self.rating_visibility.is_none()
    }
}

/// The trimmed text, recording `too_long` when it has more than `max` characters.
fn checked_text(raw: &str, max: usize, too_long: &str, v: &mut Validator) -> String {
    let text = py_strip(raw).to_owned();
    v.check(char_len(&text) <= max, too_long);
    text
}

/// A user's shelf (`?username=` for someone in the caller's audience), with their favourites.
#[utoipa::path(
    get,
    path = "/api/shelf",
    operation_id = "shelf_list",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(OwnerQuery),
    responses(
        (status = 200, body = ShelfListOut, description = "The shelf"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No member with that name` (also for users outside the caller's audience)")
    )
)]
pub async fn list_shelf(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<OwnerQuery>,
) -> ApiResult<Json<ShelfListOut>> {
    let mut conn = state.db.acquire().await?;
    let owner = resolve_owner(&mut conn, &user, &query).await?;
    let timezone = sqlx::query_scalar!("SELECT timezone FROM users WHERE id = $1", owner.id)
        .fetch_one(&mut *conn)
        .await?;
    let items = item_outs(&mut conn, user.id, owner.id, None).await?;
    let favorites = favorite_outs(&mut conn, owner.id).await?;
    Ok(Json(ShelfListOut {
        timezone,
        user: owner.into(),
        items,
        favorites,
    }))
}

/// Puts a book on the caller's shelf, at the top of its status column. 409 `Already on your
/// shelf` carries the existing entry as `item`.
#[utoipa::path(
    post,
    path = "/api/shelf",
    operation_id = "shelf_add",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = ShelfAddIn,
    responses(
        (status = 201, body = ShelfItemOut, description = "The new entry"),
        (status = 400, body = ErrorBody, description = "Validation, `Keep the take to 140 characters`, `Keep the reason to 200 characters`, `That edition belongs to another book`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such book.`"),
        (status = 409, body = ErrorBody, description = "`Already on your shelf`, with the existing entry as `item`")
    )
)]
pub async fn add_to_shelf(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ShelfAddIn>,
) -> ApiResult<(StatusCode, Json<ShelfItemOut>)> {
    let mut tx = state.db.begin().await?;
    let work_id = require_work(&mut tx, body.work_id).await?;
    if let Some(edition_id) = body.edition_id {
        require_edition_of(&mut tx, work_id, edition_id).await?;
    }
    lock_user(&mut tx, user.id).await?;
    let Some(id) = add_entry(&mut tx, user.id, work_id, &body).await? else {
        // The entry existed (or a merge made one meanwhile): answer with it, change nothing.
        drop(tx);
        return Err(already_on_shelf(&state, user.id, work_id).await);
    };
    let item = item_out(&mut tx, user.id, id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(item)))
}

/// Inserts the entry at the top of its column and records its event; `None` when the user
/// already has an entry for the work.
async fn add_entry(
    conn: &mut PgConnection,
    user_id: i64,
    work_id: i64,
    body: &ShelfAddIn,
) -> ApiResult<Option<i64>> {
    let status = body.status;
    let now = OffsetDateTime::now_utc();
    let mut fields = StatusFields::empty();
    apply_finish_fields(
        &mut fields,
        status,
        body.rating,
        Some(&body.take),
        Some(&body.dnf_reason),
    );
    apply_progress(&mut fields, status, body.progress);
    apply_reading_dates(&mut fields, status, false, now);
    let entry = NewEntry {
        user_id,
        work_id,
        edition_id: body.edition_id,
        status,
        rating_visibility: body.rating_visibility.unwrap_or(Visibility::Audience),
        fields: &fields,
        now,
    };
    let Some(id) = insert_entry(conn, &entry).await? else {
        return Ok(None);
    };
    status_event(
        conn,
        user_id,
        work_id,
        status,
        &fields,
        entry.rating_visibility,
        true,
    )
    .await?;
    Ok(Some(id))
}

/// A new shelf entry with its fields already computed (the add and the Goodreads import).
pub(super) struct NewEntry<'a> {
    pub user_id: i64,
    pub work_id: i64,
    pub edition_id: Option<i64>,
    pub status: ShelfStatus,
    pub rating_visibility: Visibility,
    pub fields: &'a StatusFields,
    /// The entry's `updated_at`.
    pub now: OffsetDateTime,
}

/// Inserts the entry at the top of its status column and renumbers the column (`place_item`
/// at 0); `None` when the user already has an entry for the work. Records no event. The caller
/// holds the user's lock.
pub(super) async fn insert_entry(
    conn: &mut PgConnection,
    entry: &NewEntry<'_>,
) -> sqlx::Result<Option<i64>> {
    let fields = entry.fields;
    let id = sqlx::query_scalar!(
        "INSERT INTO shelf_entries (user_id, work_id, edition_id, status, position, rating,
                                    rating_visibility, take, dnf_reason, progress, updated_at,
                                    started_at, finished_at)
         VALUES ($1, $2, $3, $4, 0, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT ON CONSTRAINT shelf_entries_user_work_key DO NOTHING
         RETURNING id",
        entry.user_id,
        entry.work_id,
        entry.edition_id,
        status_text(entry.status),
        fields.rating,
        visibility_text(entry.rating_visibility),
        fields.take,
        fields.dnf_reason,
        fields.progress,
        entry.now,
        fields.started_at,
        fields.finished_at
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(id) = id else {
        return Ok(None);
    };
    let siblings = column_ids(conn, entry.user_id, entry.status, id).await?;
    write_positions(conn, &column_order(&siblings, id, 0)).await?;
    Ok(Some(id))
}

/// 409 `Already on your shelf` with the caller's entry for the work as `item`.
async fn already_on_shelf(state: &AppState, user_id: i64, work_id: i64) -> AppError {
    let existing = async {
        let mut conn = state.db.acquire().await?;
        let id = sqlx::query_scalar!(
            "SELECT id FROM shelf_entries WHERE user_id = $1 AND work_id = $2",
            user_id,
            work_id
        )
        .fetch_optional(&mut *conn)
        .await?;
        match id {
            Some(id) => item_out(&mut conn, user_id, id).await.map(Some),
            None => Ok(None),
        }
    }
    .await;
    match existing {
        Ok(Some(item)) => AppError::conflict(ALREADY_ON_SHELF).with("item", item),
        Ok(None) => AppError::conflict(ALREADY_ON_SHELF),
        Err(error) => error,
    }
}

/// Changes one of the caller's entries (docs/phase1/spec/shelf.md §1.7, in its order of
/// checks: `Nothing to update` before the 404, the reading-date checks before any change).
#[utoipa::path(
    patch,
    path = "/api/shelf/{entry_id}",
    operation_id = "shelf_update",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("entry_id" = i64, Path, description = "The shelf entry")),
    request_body = ShelfPatchIn,
    responses(
        (status = 200, body = ShelfItemOut, description = "The changed entry"),
        (status = 400, body = ErrorBody, description = "Validation, `Nothing to update`, `Reading date requires a finished book`, `Reading date cannot be in the future`, `That edition belongs to another book`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Book not on your shelf` (also another user's entry)")
    )
)]
pub async fn update_entry(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(entry_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ShelfPatchIn>,
) -> ApiResult<Json<ShelfItemOut>> {
    if body.is_empty() {
        return Err(AppError::bad_request(NOTHING_TO_UPDATE));
    }
    let zone = tz::zone_or_utc(&user.timezone);
    let mut tx = state.db.begin().await?;
    lock_user(&mut tx, user.id).await?;
    let entry = load_entry(&mut tx, user.id, entry_id)
        .await?
        .ok_or_else(|| AppError::not_found(NOT_ON_SHELF))?;
    patch_entry(&mut tx, entry, body, &zone).await?;
    let item = item_out(&mut tx, user.id, entry_id).await?;
    tx.commit().await?;
    Ok(Json(item))
}

/// The PATCH steps after the entry was found (docs/phase1/spec/shelf.md §1.7 steps 3–10).
async fn patch_entry(
    conn: &mut PgConnection,
    mut entry: Entry,
    body: ShelfPatchIn,
    zone: &jiff::tz::TimeZone,
) -> ApiResult<()> {
    let previous_status = entry.status;
    let previous_progress = entry.fields.progress;
    let status = body.status.unwrap_or(entry.status);
    let position = body.position.unwrap_or(entry.position.into());

    if let Some(finished_on) = body.finished_on.as_option() {
        if status != ShelfStatus::Finished {
            return Err(AppError::bad_request(msg::READING_DATE_NEEDS_FINISHED));
        }
        if finished_on.is_some_and(|date| *date > tz::today(zone)) {
            return Err(AppError::bad_request(msg::READING_DATE_IN_FUTURE));
        }
    }
    if let Patch::Value(edition_id) = body.edition_id {
        require_edition_of(conn, entry.work_id, edition_id).await?;
    }

    if body.status.is_some()
        || !body.rating.is_missing()
        || !body.take.is_missing()
        || !body.dnf_reason.is_missing()
    {
        let rating = keep_or(body.rating, entry.fields.rating);
        let take = keep_or(body.take, Some(entry.fields.take.clone()));
        let dnf_reason = keep_or(body.dnf_reason, Some(entry.fields.dnf_reason.clone()));
        apply_finish_fields(
            &mut entry.fields,
            status,
            rating,
            take.as_deref(),
            dnf_reason.as_deref(),
        );
    }
    let progress_sent = !body.progress.is_missing();
    if body.status.is_some() || progress_sent {
        let progress = keep_or(body.progress, entry.fields.progress);
        apply_progress(&mut entry.fields, status, progress);
    }
    if let Some(edition_id) = body.edition_id.into_option() {
        entry.edition_id = edition_id;
    }
    if let Some(visibility) = body.rating_visibility {
        entry.rating_visibility = visibility;
    }

    if let Some(shelf_position) = body.shelf_position {
        place_on_shelf(conn, &mut entry, shelf_position).await?;
    }
    if body.shelf_position.is_none() || body.status.is_some() || body.position.is_some() {
        place_item(conn, &mut entry, status, position).await?;
    }
    if let Some(finished_on) = body.finished_on.into_option() {
        entry.fields.finished_at = finished_on.map(|date| tz::noon_local_utc(date, zone));
        // A completion entered after the fact must not precede the recorded start.
        if let (Some(started), Some(finished)) = (entry.fields.started_at, entry.fields.finished_at)
            && started > finished
        {
            entry.fields.started_at = Some(finished);
        }
    }
    save_entry(conn, &entry).await?;
    if entry.rating_visibility == Visibility::OnlyMe {
        // A rating or take the earlier events show while it was audience is private now.
        hide_finished_ratings(conn, Some(entry.user_id), Some(&[entry.work_id])).await?;
    }

    if entry.status != previous_status {
        status_event(
            conn,
            entry.user_id,
            entry.work_id,
            entry.status,
            &entry.fields,
            entry.rating_visibility,
            false,
        )
        .await?;
    } else if entry.status == ShelfStatus::CurrentlyReading
        && progress_sent
        && entry.fields.progress.is_some()
        && entry.fields.progress != previous_progress
    {
        record_event(
            conn,
            entry.user_id,
            entry.work_id,
            EventKind::Progress,
            json!({ "progress": entry.fields.progress }),
        )
        .await?;
    }
    Ok(())
}

/// The value a PATCH field leaves: the current one when absent, else the sent one (null clears).
fn keep_or<T>(field: Patch<T>, current: Option<T>) -> Option<T> {
    field.into_option().unwrap_or(current)
}

/// Removes one of the caller's entries: records `shelf_removed`, prunes the entry from the
/// stored layout and renumbers its column, in one transaction.
#[utoipa::path(
    delete,
    path = "/api/shelf/{entry_id}",
    operation_id = "shelf_remove",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("entry_id" = i64, Path, description = "The shelf entry")),
    responses(
        (status = 204, description = "Removed"),
        (status = 400, body = ErrorBody, description = "A non-numeric id"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Book not on your shelf` (also another user's entry)")
    )
)]
pub async fn remove_entry(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(entry_id): ApiPath<i64>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    lock_user(&mut tx, user.id).await?;
    let entry = load_entry(&mut tx, user.id, entry_id)
        .await?
        .ok_or_else(|| AppError::not_found(NOT_ON_SHELF))?;
    record_event(
        &mut tx,
        user.id,
        entry.work_id,
        EventKind::ShelfRemoved,
        json!({ "status": entry.status }),
    )
    .await?;
    remove_from_layout(&mut tx, user.id, entry_id).await?;
    sqlx::query!("DELETE FROM shelf_entries WHERE id = $1", entry_id)
        .execute(&mut *tx)
        .await?;
    // The finished events keep no rating or take: adding the work again (audience by default)
    // must not bring back what this entry showed or hid.
    hide_finished_ratings(&mut tx, Some(user.id), Some(&[entry.work_id])).await?;
    let leftovers = column_ids(&mut tx, user.id, entry.status, entry_id).await?;
    write_positions(&mut tx, &leftovers).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// One of the caller's entries as the write paths change it.
#[derive(Debug)]
struct Entry {
    id: i64,
    user_id: i64,
    work_id: i64,
    edition_id: Option<i64>,
    status: ShelfStatus,
    position: i32,
    shelf_position: Option<i32>,
    rating_visibility: Visibility,
    updated_at: OffsetDateTime,
    fields: StatusFields,
}

/// The caller's entry `entry_id`; `None` when it is missing or someone else's.
async fn load_entry(
    conn: &mut PgConnection,
    user_id: i64,
    entry_id: i64,
) -> sqlx::Result<Option<Entry>> {
    let row = sqlx::query!(
        r#"SELECT id, user_id, work_id, edition_id, status AS "status: ShelfStatus", position,
                  shelf_position, rating, rating_visibility AS "rating_visibility: Visibility",
                  take, dnf_reason, progress, updated_at, started_at, finished_at
           FROM shelf_entries
           WHERE id = $1 AND user_id = $2"#,
        entry_id,
        user_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| Entry {
        id: row.id,
        user_id: row.user_id,
        work_id: row.work_id,
        edition_id: row.edition_id,
        status: row.status,
        position: row.position,
        shelf_position: row.shelf_position,
        rating_visibility: row.rating_visibility,
        updated_at: row.updated_at,
        fields: StatusFields {
            rating: row.rating,
            take: row.take,
            dnf_reason: row.dnf_reason,
            progress: row.progress,
            started_at: row.started_at,
            finished_at: row.finished_at,
        },
    }))
}

/// Writes every field of the entry in one statement, so the status CHECKs see the new row.
async fn save_entry(conn: &mut PgConnection, entry: &Entry) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE shelf_entries
         SET edition_id = $2, status = $3, position = $4, shelf_position = $5, rating = $6,
             rating_visibility = $7, take = $8, dnf_reason = $9, progress = $10, updated_at = $11,
             started_at = $12, finished_at = $13
         WHERE id = $1",
        entry.id,
        entry.edition_id,
        status_text(entry.status),
        entry.position,
        entry.shelf_position,
        entry.fields.rating,
        visibility_text(entry.rating_visibility),
        entry.fields.take,
        entry.fields.dnf_reason,
        entry.fields.progress,
        entry.updated_at,
        entry.fields.started_at,
        entry.fields.finished_at
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// `place_item`: moves the entry into `status` at `position` (clamped), renumbers the target
/// column and, after a status change, the column it left. Bumps `updated_at`, and stamps the
/// reading dates when the status changed.
async fn place_item(
    conn: &mut PgConnection,
    entry: &mut Entry,
    status: ShelfStatus,
    position: i64,
) -> sqlx::Result<()> {
    let source = entry.status;
    let siblings = column_ids(conn, entry.user_id, status, entry.id).await?;
    let order = column_order(&siblings, entry.id, position);
    entry.position = index_of(&order, entry.id);
    entry.status = status;
    let now = OffsetDateTime::now_utc();
    entry.updated_at = now;
    if source != status {
        apply_reading_dates(&mut entry.fields, status, true, now);
    }
    write_positions(conn, &order).await?;
    if source != status {
        let leftovers = column_ids(conn, entry.user_id, source, entry.id).await?;
        write_positions(conn, &leftovers).await?;
    }
    Ok(())
}

/// `place_on_shelf`: moves the entry to `position` in the free order and renumbers every entry
/// of the user. Touches nothing else.
async fn place_on_shelf(
    conn: &mut PgConnection,
    entry: &mut Entry,
    position: i64,
) -> sqlx::Result<()> {
    let slots = sqlx::query_as!(
        ShelfSlot,
        r#"SELECT id, status AS "status: ShelfStatus", position, shelf_position
           FROM shelf_entries WHERE user_id = $1"#,
        entry.user_id
    )
    .fetch_all(&mut *conn)
    .await?;
    let order = shelf_order(&slots, entry.id, position);
    entry.shelf_position = Some(index_of(&order, entry.id));
    let ranks: Vec<i32> = (0..order.len()).map(rank).collect();
    sqlx::query!(
        "UPDATE shelf_entries AS s SET shelf_position = v.rank
         FROM unnest($1::int8[], $2::int4[]) AS v (id, rank)
         WHERE s.id = v.id AND s.shelf_position IS DISTINCT FROM v.rank",
        &order,
        &ranks
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The ids of the user's `status` column without `except`, by `(position, id)`.
async fn column_ids(
    conn: &mut PgConnection,
    user_id: i64,
    status: ShelfStatus,
    except: i64,
) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        "SELECT id FROM shelf_entries
         WHERE user_id = $1 AND status = $2 AND id <> $3
         ORDER BY position, id",
        user_id,
        status_text(status),
        except
    )
    .fetch_all(conn)
    .await
}

/// Sets each entry's `position` to its index in `order`.
async fn write_positions(conn: &mut PgConnection, order: &[i64]) -> sqlx::Result<()> {
    let positions: Vec<i32> = (0..order.len()).map(rank).collect();
    sqlx::query!(
        "UPDATE shelf_entries AS s SET position = v.position
         FROM unnest($1::int8[], $2::int4[]) AS v (id, position)
         WHERE s.id = v.id AND s.position <> v.position",
        order,
        &positions
    )
    .execute(conn)
    .await?;
    Ok(())
}

fn index_of(order: &[i64], id: i64) -> i32 {
    rank(order.iter().position(|&other| other == id).unwrap_or(0))
}

/// An index as a stored position (a shelf never has 2^31 entries).
fn rank(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
}

/// The stored text of a status.
pub fn status_text(status: ShelfStatus) -> &'static str {
    match status {
        ShelfStatus::WantToRead => "want_to_read",
        ShelfStatus::CurrentlyReading => "currently_reading",
        ShelfStatus::Finished => "finished",
        ShelfStatus::DidNotFinish => "did_not_finish",
    }
}

/// The stored text of a visibility.
pub fn visibility_text(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Audience => "audience",
        Visibility::OnlyMe => "only_me",
    }
}

/// The event of an entry that landed in `status` (added, or moved by a PATCH): `shelf_finished`
/// `{rating?, take?}`, `shelf_dnf` `{reason?}`, else `shelf_added` or `shelf_moved` `{status}`.
/// An only-me rating and take never enter the payload, which outlives the entry and its
/// visibility (`activity::hide_finished_ratings`).
async fn status_event(
    conn: &mut PgConnection,
    user_id: i64,
    work_id: i64,
    status: ShelfStatus,
    fields: &StatusFields,
    rating_visibility: Visibility,
    added: bool,
) -> sqlx::Result<()> {
    let non_empty = |text: &str| (!text.is_empty()).then(|| text.to_owned());
    let shared = rating_visibility == Visibility::Audience;
    let (kind, payload) = match status {
        ShelfStatus::Finished => (
            EventKind::ShelfFinished,
            json!({
                "rating": fields.rating.filter(|_| shared),
                "take": non_empty(&fields.take).filter(|_| shared),
            }),
        ),
        ShelfStatus::DidNotFinish => (
            EventKind::ShelfDnf,
            json!({ "reason": non_empty(&fields.dnf_reason) }),
        ),
        _ if added => (EventKind::ShelfAdded, json!({ "status": status })),
        _ => (EventKind::ShelfMoved, json!({ "status": status })),
    };
    record_event(conn, user_id, work_id, kind, payload).await
}

/// Records a personal shelf event (null payload values are dropped by `activity::record`).
async fn record_event(
    conn: &mut PgConnection,
    user_id: i64,
    work_id: i64,
    kind: EventKind,
    payload: Value,
) -> sqlx::Result<()> {
    let payload = match payload {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    record(
        conn,
        NewEvent {
            club_id: None,
            actor_id: Some(user_id),
            kind,
            work_id: Some(work_id),
            pick_id: None,
            payload,
        },
    )
    .await?;
    Ok(())
}

/// One of the owner's entries as the owner sees it.
pub async fn item_out(
    conn: &mut PgConnection,
    owner: i64,
    entry_id: i64,
) -> ApiResult<ShelfItemOut> {
    item_outs(conn, owner, owner, Some(&[entry_id]))
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found(NOT_ON_SHELF))
}

/// The owner's entries as `viewer` sees them (read through `bc_visible_shelf`, so another
/// viewer gets `only_me` ratings redacted and an empty list for an owner they cannot see),
/// ordered by status text, position and id. `only` restricts the entries. Entries whose work
/// is missing are skipped.
pub async fn item_outs(
    conn: &mut PgConnection,
    viewer: i64,
    owner: i64,
    only: Option<&[i64]>,
) -> sqlx::Result<Vec<ShelfItemOut>> {
    let rows = sqlx::query!(
        r#"SELECT s.id AS "id!", s.work_id AS "work_id!", s.edition_id,
                  s.status AS "status!: ShelfStatus", s."position" AS "position!",
                  s.shelf_position, s.rating, s.rating_visibility AS "rating_visibility: Visibility",
                  s.take AS "take!", s.dnf_reason AS "dnf_reason!", s.progress,
                  s.updated_at AS "updated_at!", s.started_at, s.finished_at
           FROM bc_visible_shelf($1) AS s
           WHERE s.user_id = $2 AND ($3::int8[] IS NULL OR s.id = ANY($3))
           ORDER BY s.status COLLATE "C", s."position", s.id"#,
        viewer,
        owner,
        only as Option<&[i64]>
    )
    .fetch_all(&mut *conn)
    .await?;
    let refs: Vec<BookRef> = rows
        .iter()
        .map(|row| BookRef {
            work_id: row.work_id,
            edition_id: row.edition_id,
        })
        .collect();
    let mut books = book_outs(conn, &refs).await?;
    let edition_ids: Vec<i64> = rows.iter().filter_map(|row| row.edition_id).collect();
    let mut editions = edition_outs(conn, &edition_ids).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let book = books.remove(&BookRef {
                work_id: row.work_id,
                edition_id: row.edition_id,
            })?;
            Some(ShelfItemOut {
                id: row.id,
                status: row.status,
                position: row.position,
                shelf_position: row.shelf_position,
                updated_at: row.updated_at,
                started_at: row.started_at,
                finished_at: row.finished_at,
                book,
                edition: row.edition_id.and_then(|id| editions.remove(&id)),
                rating: row.rating,
                rating_visibility: row.rating_visibility,
                take: row.take,
                dnf_reason: row.dnf_reason,
                progress: row.progress,
            })
        })
        .collect())
}
