//! Creating, listing, reading, renaming and deleting clubs (endpoint-map §3).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::PgConnection;
use unicode_normalization::UnicodeNormalization;
use utoipa::ToSchema;

use super::extract::{ClubAdmin, ClubMember, ClubOwner, Role};
use super::lock_club;
use super::out::{ClubList, ClubOut, club_out, club_outs};
use crate::AppState;
use crate::auth::extract::{CurrentUser, VerifiedUser};
use crate::auth::random;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, Validate, Validator};
use crate::text::{char_len, collapse_ws, py_strip};
use crate::tz;

/// A user may have created at most this many existing clubs.
pub const MAX_CLUBS_CREATED: i64 = 10;
pub const TOO_MANY_CLUBS: &str = "You can create at most 10 clubs";
pub const NAME_NEEDED: &str = "Give the club a name";
pub const NAME_TOO_LONG: &str = "Keep the club name to 80 characters";
const MAX_NAME_CHARS: usize = 80;
/// The slugified name is cut to this many characters before the random suffix.
const MAX_SLUG_BASE: usize = 50;
/// Slug suffix alphabet: the random part keeps slugs from being guessed from a name.
const SLUG_ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const SLUG_ATTEMPTS: usize = 8;

/// A new club.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClubIn {
    /// 1 to 80 characters; surrounding whitespace is removed and inner runs collapse to one
    /// space.
    pub name: String,
    /// IANA zone for the club's times (exact spelling; an old name such as `Asia/Calcutta` is
    /// stored as the current one, `Asia/Kolkata`); default: the creator's zone.
    #[serde(default)]
    pub timezone: Option<String>,
}

impl Validate for ClubIn {
    fn validate(&mut self, v: &mut Validator) {
        self.name = club_name(&self.name, v);
        validate_zone(&mut self.timezone, v);
    }
}

/// Club settings to change; `null` or absent leaves a value as it is.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClubPatchIn {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
}

impl Validate for ClubPatchIn {
    fn validate(&mut self, v: &mut Validator) {
        if let Some(name) = &mut self.name {
            *name = club_name(name, v);
        }
        validate_zone(&mut self.timezone, v);
    }
}

/// The normalised name, recording `Give the club a name` or `Keep the club name to 80
/// characters`.
fn club_name(raw: &str, v: &mut Validator) -> String {
    let name = collapse_ws(raw);
    if name.is_empty() {
        v.fail(NAME_NEEDED);
    } else if char_len(&name) > MAX_NAME_CHARS {
        v.fail(NAME_TOO_LONG);
    }
    name
}

/// Validates the zone and stores an old name (`Asia/Calcutta`) as the current one
/// (`Asia/Kolkata`).
fn validate_zone(zone: &mut Option<String>, v: &mut Validator) {
    if let Some(zone) = zone {
        match tz::canonical_zone_name(py_strip(zone)) {
            Ok(name) => *zone = name,
            Err(_) => v.fail(msg::INVALID_TIME_ZONE),
        }
    }
}

/// The clubs the caller belongs to, newest join first, each with the caller's role and the
/// current pick.
#[utoipa::path(
    get,
    path = "/api/clubs",
    operation_id = "clubs_list",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = ClubList, description = "The caller's clubs"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn list_clubs(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<ClubList>> {
    let items = club_outs(&mut *state.db.acquire().await?, user.id, None).await?;
    Ok(Json(ClubList { items }))
}

/// Creates a club with the caller as its owner. Needs a verified email address. The slug is the
/// slugified name plus `-` and four random characters, and never changes.
#[utoipa::path(
    post,
    path = "/api/clubs",
    operation_id = "clubs_create",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = ClubIn,
    responses(
        (status = 201, body = ClubOut, description = "The new club"),
        (status = 400, body = ErrorBody, description = "`Give the club a name`, `Keep the club name to 80 characters`, `That is not a valid time zone`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Verify your email address first`"),
        (status = 409, body = ErrorBody, description = "`You can create at most 10 clubs`")
    )
)]
pub async fn create_club(
    VerifiedUser(user): VerifiedUser,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ClubIn>,
) -> ApiResult<(StatusCode, Json<ClubOut>)> {
    let timezone = body.timezone.unwrap_or_else(|| user.timezone.clone());
    let mut tx = state.db.begin().await?;
    // The user row lock serialises two creations by one user, so the count cannot be raced.
    sqlx::query_scalar!("SELECT id FROM users WHERE id = $1 FOR UPDATE", user.id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    let created = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM clubs WHERE created_by = $1"#,
        user.id
    )
    .fetch_one(&mut *tx)
    .await?;
    if created >= MAX_CLUBS_CREATED {
        return Err(AppError::conflict(TOO_MANY_CLUBS));
    }
    let club_id = insert_club(&mut tx, &body.name, &timezone, user.id).await?;
    sqlx::query!(
        "INSERT INTO club_memberships (club_id, user_id, role) VALUES ($1, $2, 'owner')",
        club_id,
        user.id
    )
    .execute(&mut *tx)
    .await?;
    let out = club_out(&mut tx, user.id, club_id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// Inserts the club under a fresh slug, drawing a new suffix when the slug is taken.
async fn insert_club(
    conn: &mut PgConnection,
    name: &str,
    timezone: &str,
    created_by: i64,
) -> ApiResult<i64> {
    let base = slug_base(name);
    for _ in 0..SLUG_ATTEMPTS {
        let slug = format!("{base}-{}", slug_suffix());
        let id = sqlx::query_scalar!(
            "INSERT INTO clubs (slug, name, timezone, created_by) VALUES ($1, $2, $3, $4)
             ON CONFLICT (slug) DO NOTHING
             RETURNING id",
            slug,
            name,
            timezone,
            created_by
        )
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(id) = id {
            return Ok(id);
        }
    }
    Err(AppError::internal(format!(
        "no free slug for {base:?} after {SLUG_ATTEMPTS} attempts"
    )))
}

/// The name as a slug: German umlauts and ß transliterated, other accents dropped, every run of
/// other characters one `-`, cut to 50 characters; `club` when nothing is left. The legacy
/// importer slugs its club with this and `slug_suffix` too.
pub fn slug_base(name: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in name.to_lowercase().chars() {
        let spelled: Option<String> = match c {
            'ä' => Some("ae".into()),
            'ö' => Some("oe".into()),
            'ü' => Some("ue".into()),
            'ß' => Some("ss".into()),
            c if c.is_ascii_alphanumeric() => Some(c.to_string()),
            c => {
                let base: String = c.nfkd().filter(char::is_ascii_alphanumeric).collect();
                (!base.is_empty()).then_some(base.to_lowercase())
            }
        };
        match spelled {
            Some(part) => {
                if pending_dash && !slug.is_empty() {
                    slug.push('-');
                }
                pending_dash = false;
                slug.push_str(&part);
            }
            None => pending_dash = true,
        }
    }
    let cut: String = slug.chars().take(MAX_SLUG_BASE).collect();
    let cut = cut.trim_end_matches('-');
    if cut.is_empty() {
        "club".to_owned()
    } else {
        cut.to_owned()
    }
}

/// Four random characters from `[a-z0-9]`.
pub(crate) fn slug_suffix() -> String {
    let mut bytes = [0u8; 4];
    random::fill(&mut bytes);
    bytes
        .iter()
        .map(|b| SLUG_ALPHABET[usize::from(*b) % SLUG_ALPHABET.len()] as char)
        .collect()
}

/// The club, with the caller's role and the current pick.
#[utoipa::path(
    get,
    path = "/api/clubs/{slug}",
    operation_id = "clubs_get",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 200, body = ClubOut, description = "The club"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such club` (also for a non-member)")
    )
)]
pub async fn get_club(
    member: ClubMember,
    State(state): State<AppState>,
) -> ApiResult<Json<ClubOut>> {
    let out = club_out(
        &mut *state.db.acquire().await?,
        member.user.id,
        member.club.id,
    )
    .await?;
    Ok(Json(out))
}

/// Renames the club or changes its time zone (admins). The slug stays.
#[utoipa::path(
    patch,
    path = "/api/clubs/{slug}",
    operation_id = "clubs_update",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    request_body = ClubPatchIn,
    responses(
        (status = 200, body = ClubOut, description = "The club"),
        (status = 400, body = ErrorBody, description = "`Give the club a name`, `Keep the club name to 80 characters`, `That is not a valid time zone`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only club admins can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`")
    )
)]
pub async fn update_club(
    ClubAdmin(member): ClubAdmin,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ClubPatchIn>,
) -> ApiResult<Json<ClubOut>> {
    let mut conn = state.db.acquire().await?;
    // The role check is part of the statement: an admin demoted since the extractor read the
    // role changes nothing.
    let updated = sqlx::query!(
        "UPDATE clubs SET name = coalesce($3, name), timezone = coalesce($4, timezone)
         WHERE id = $1
           AND EXISTS (SELECT 1 FROM club_memberships
                       WHERE club_id = $1 AND user_id = $2 AND role IN ('owner', 'admin'))",
        member.club.id,
        member.user.id,
        body.name,
        body.timezone
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(refusal(&mut conn, member.user.id, member.club.id, Role::Admin).await?);
    }
    Ok(Json(
        club_out(&mut conn, member.user.id, member.club.id).await?,
    ))
}

/// Deletes the club and everything in it (owner only): picks, threads, votes, invites, events.
#[utoipa::path(
    delete,
    path = "/api/clubs/{slug}",
    operation_id = "clubs_delete",
    tag = "clubs",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("slug" = String, Path, description = "The club's slug")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 403, body = ErrorBody, description = "`Only the club owner can do that`"),
        (status = 404, body = ErrorBody, description = "`No such club`")
    )
)]
pub async fn delete_club(
    ClubOwner(member): ClubOwner,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    if !lock_club(&mut tx, member.club.id).await? {
        return Err(AppError::not_found(msg::NO_SUCH_CLUB));
    }
    let deleted = sqlx::query!(
        "DELETE FROM clubs
         WHERE id = $1
           AND EXISTS (SELECT 1 FROM club_memberships
                       WHERE club_id = $1 AND user_id = $2 AND role = 'owner')",
        member.club.id,
        member.user.id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(refusal(&mut tx, member.user.id, member.club.id, Role::Owner).await?);
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The error for a caller who lost the `needed` role since the extractor checked it: 404 `No
/// such club` when they are no longer a member, else the 403 for that role.
pub async fn refusal(
    conn: &mut PgConnection,
    user_id: i64,
    club_id: i64,
    needed: Role,
) -> sqlx::Result<AppError> {
    Ok(
        match super::extract::role_in(conn, user_id, club_id).await? {
            None => AppError::not_found(msg::NO_SUCH_CLUB),
            Some(_) if needed == Role::Owner => AppError::forbidden(msg::ONLY_OWNER),
            Some(_) => AppError::forbidden(msg::ONLY_ADMINS),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_readable_slugs() {
        assert_eq!(slug_base("Thursday Readers!"), "thursday-readers");
        assert_eq!(
            slug_base("  Bücherwürmer & Große Leser "),
            "buecherwuermer-grosse-leser"
        );
        assert_eq!(slug_base("Café Noir"), "cafe-noir");
        assert_eq!(slug_base("!!!"), "club");
        assert_eq!(slug_base("読書会"), "club");
        assert_eq!(slug_base("a--b__c"), "a-b-c");
        assert_eq!(slug_base(&"x".repeat(100)).len(), MAX_SLUG_BASE);
        assert_eq!(slug_base(&format!("{} y", "x".repeat(49))), "x".repeat(49));
    }

    #[test]
    fn slugs_match_the_schema() {
        let pattern = regex::Regex::new("^[a-z0-9](?:[a-z0-9-]{1,62}[a-z0-9])$").unwrap();
        for name in [
            "Thursday Readers",
            "!!!",
            "ab",
            "x",
            &"long name ".repeat(20),
        ] {
            let slug = format!("{}-{}", slug_base(name), slug_suffix());
            assert!(pattern.is_match(&slug), "{slug}");
        }
    }

    #[test]
    fn names_are_collapsed_and_bounded() {
        let mut v = Validator::new();
        assert_eq!(
            club_name("  Thursday \n  Readers ", &mut v),
            "Thursday Readers"
        );
        assert!(v.messages().is_empty());
        club_name(" \t ", &mut v);
        club_name(&"ä".repeat(81), &mut v);
        assert!(club_name(&"ä".repeat(80), &mut Validator::new()).len() > 80);
        assert_eq!(v.messages(), [NAME_NEEDED, NAME_TOO_LONG]);
    }
}
