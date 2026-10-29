//! Closed-beta sign-up invites minted by an instance admin (`POST/GET /api/admin/invites`,
//! endpoint-map §3) or by the CLI (`bookclub-server beta-invite`), and revoked by an instance
//! admin (`DELETE /api/admin/invites/{invite_id}`). They have no club: they only unlock sign-up
//! while `SIGNUP_MODE=invite`.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::PgConnection;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::auth::extract::InstanceAdmin;
use crate::clubs::invites::{NO_SUCH_INVITE, insert_invite};
use crate::clubs::out::{InviteList, InviteOut, invite_out, invite_outs};
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, Validate, Validator};

/// The expiry of an invite minted without limits (`{}` or `beta-invite` without flags), which
/// is then single-use.
const DEFAULT_DAYS: i32 = 30;

/// Instance-admin routes; every path starts with `/api/admin/`.
pub fn admin_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_signup_invites, create_signup_invite))
        .routes(routes!(revoke_signup_invite))
}

/// A new sign-up invite. Without either field it is single-use and expires in 30 days.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SignupInviteIn {
    /// 1 to 100 sign-ups; absent: no use limit (unless `expires_in_days` is absent too).
    #[serde(default)]
    pub max_uses: Option<i64>,
    /// 1 to 90 days; absent: no expiry (unless `max_uses` is absent too).
    #[serde(default)]
    pub expires_in_days: Option<i64>,
}

impl Validate for SignupInviteIn {
    fn validate(&mut self, v: &mut Validator) {
        v.range_i64(self.max_uses, 1, 100);
        v.range_i64(self.expires_in_days, 1, 90);
    }
}

/// Inserts a sign-up invite and returns its id. `max_uses` 1..=100 and `expires_in_days`
/// 1..=90 are the caller's to check; with neither, the invite is single-use for 30 days.
pub async fn mint_signup_invite(
    conn: &mut PgConnection,
    created_by: Option<i64>,
    max_uses: Option<i32>,
    expires_in_days: Option<i32>,
) -> sqlx::Result<i64> {
    let (max_uses, expires_in_days) = match (max_uses, expires_in_days) {
        (None, None) => (Some(1), Some(DEFAULT_DAYS)),
        limits => limits,
    };
    insert_invite(conn, None, created_by, max_uses, expires_in_days).await
}

/// Every sign-up invite, newest first.
#[utoipa::path(
    get,
    path = "/api/admin/invites",
    operation_id = "admin_list_invites",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = InviteList, description = "The sign-up invites"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (not an instance admin)")
    )
)]
pub async fn list_signup_invites(
    InstanceAdmin(_admin): InstanceAdmin,
    State(state): State<AppState>,
) -> ApiResult<Json<InviteList>> {
    let mut conn = state.db.acquire().await?;
    let items = invite_outs(&mut conn, &state.config, None, None).await?;
    Ok(Json(InviteList { items }))
}

/// Mints a closed-beta sign-up invite for someone who wants no club.
#[utoipa::path(
    post,
    path = "/api/admin/invites",
    operation_id = "admin_create_invite",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    request_body = SignupInviteIn,
    responses(
        (status = 201, body = InviteOut, description = "The new sign-up invite"),
        (status = 400, body = ErrorBody, description = "A limit out of range"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`Not found` (not an instance admin)")
    )
)]
pub async fn create_signup_invite(
    InstanceAdmin(admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiJson(body): ApiJson<SignupInviteIn>,
) -> ApiResult<(StatusCode, Json<InviteOut>)> {
    let mut conn = state.db.acquire().await?;
    // Validated to 1..=100 and 1..=90.
    let id = mint_signup_invite(
        &mut conn,
        Some(admin.id),
        body.max_uses.map(|n| n as i32),
        body.expires_in_days.map(|n| n as i32),
    )
    .await?;
    let out = invite_out(&mut conn, &state.config, None, id).await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// Revokes a sign-up invite: it no longer unlocks sign-up. Revoking twice is harmless. Club
/// invites are their club admins' to revoke (`DELETE /api/clubs/{slug}/invites/{invite_id}`).
#[utoipa::path(
    delete,
    path = "/api/admin/invites/{invite_id}",
    operation_id = "admin_revoke_invite",
    tag = "admin",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("invite_id" = i64, Path, description = "The sign-up invite's id")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 400, body = ErrorBody, description = "The id is not an integer"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such invite` (also a club's invite); `Not found` for everyone but instance admins")
    )
)]
pub async fn revoke_signup_invite(
    InstanceAdmin(_admin): InstanceAdmin,
    State(state): State<AppState>,
    ApiPath(invite_id): ApiPath<i64>,
) -> ApiResult<StatusCode> {
    let mut conn = state.db.acquire().await?;
    sqlx::query_scalar!(
        "UPDATE club_invites SET revoked_at = coalesce(revoked_at, now())
         WHERE club_id IS NULL AND id = $1
         RETURNING id",
        invite_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| AppError::not_found(NO_SUCH_INVITE))?;
    Ok(StatusCode::NO_CONTENT)
}
