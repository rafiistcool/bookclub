//! Bearer tokens and the session list (endpoint-map §2, docs/phase1/conventions.md §5.2):
//! `POST /api/auth/tokens`, `GET /api/auth/sessions`, `DELETE /api/auth/sessions/{session_id}`.

use axum::Json;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::extract::CurrentUser;
use super::login::{authenticate, confirm};
use super::rules;
use super::session::{self, Presented, SessionKind};
use crate::AppState;
use crate::client_ip::ClientIp;
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::{ApiJson, ApiPath, Validate, Validator};

const NO_SUCH_SESSION: &str = "No such session";

/// Credentials for a new bearer token.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenIn {
    /// A username, or a verified email address.
    pub username: String,
    pub password: String,
    /// Shown in the session list, e.g. the device name; cut to 200 characters.
    pub label: String,
}

impl Validate for TokenIn {
    fn validate(&mut self, _v: &mut Validator) {
        self.username = rules::normalize_username(&self.username);
    }
}

/// One signed-in browser or app.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionOut {
    pub id: i64,
    pub kind: SessionKind,
    /// The browser's User-Agent for cookies, the app's label for bearer tokens.
    pub label: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Updated at most once an hour.
    #[serde(with = "time::serde::rfc3339")]
    pub last_used_at: OffsetDateTime,
    /// The session that authenticated this request.
    pub current: bool,
}

/// A new bearer token; the token is shown only here.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenOut {
    /// `bc_…`, for `Authorization: Bearer`.
    pub token: String,
    /// Its session (`current` is false: this request did not use it).
    pub session: SessionOut,
}

/// The user's live sessions.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionList {
    pub items: Vec<SessionOut>,
}

/// Creates a bearer token for the apps (Phase 4). Same limits and errors as login. It expires
/// after 90 days without use.
#[utoipa::path(
    post,
    path = "/api/auth/tokens",
    operation_id = "auth_create_token",
    tag = "auth",
    request_body = TokenIn,
    responses(
        (status = 201, body = TokenOut, description = "The new token"),
        (status = 400, body = ErrorBody, description = "Malformed body"),
        (status = 401, body = ErrorBody, description = "`Wrong username or password`"),
        (status = 429, body = ErrorBody, description = "`Too many attempts. Wait a few minutes.`")
    )
)]
pub async fn create_token(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<TokenIn>,
) -> ApiResult<(StatusCode, Json<TokenOut>)> {
    let account = authenticate(&state, ip, &body.username, &body.password).await?;
    let mut tx = state.db.begin().await?;
    confirm(&mut tx, &account).await?;
    let new_session =
        session::create(&mut tx, account.user_id, SessionKind::Bearer, &body.label).await?;
    tx.commit().await?;
    let out = TokenOut {
        token: new_session.secret,
        session: SessionOut {
            id: new_session.id,
            kind: SessionKind::Bearer,
            label: new_session.label,
            created_at: new_session.created_at,
            last_used_at: new_session.last_used_at,
            current: false,
        },
    };
    Ok((StatusCode::CREATED, Json(out)))
}

/// The caller's live cookie sessions and bearer tokens, most recently used first.
#[utoipa::path(
    get,
    path = "/api/auth/sessions",
    operation_id = "auth_list_sessions",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    responses(
        (status = 200, body = SessionList, description = "Live sessions"),
        (status = 401, body = ErrorBody, description = "`Not signed in`")
    )
)]
pub async fn list_sessions(
    user: CurrentUser,
    State(state): State<AppState>,
) -> ApiResult<Json<SessionList>> {
    let rows = sqlx::query!(
        r#"SELECT id, kind AS "kind: SessionKind", label, created_at, last_used_at
           FROM sessions
           WHERE user_id = $1 AND expires_at > now()
           ORDER BY last_used_at DESC, id DESC"#,
        user.id
    )
    .fetch_all(&state.db)
    .await?;
    let items = rows
        .into_iter()
        .map(|row| SessionOut {
            current: row.id == user.session_id,
            id: row.id,
            kind: row.kind,
            label: row.label,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        })
        .collect();
    Ok(Json(SessionList { items }))
}

/// Signs one session or token out. Revoking the current cookie session logs out (the cookie is
/// cleared).
#[utoipa::path(
    delete,
    path = "/api/auth/sessions/{session_id}",
    operation_id = "auth_revoke_session",
    tag = "auth",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(("session_id" = i64, Path, description = "A session id from the list")),
    responses(
        (status = 204, description = "Revoked"),
        (status = 400, body = ErrorBody, description = "The id is not an integer"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such session` (also another user's)")
    )
)]
pub async fn revoke_session(
    user: CurrentUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiPath(session_id): ApiPath<i64>,
) -> ApiResult<Response> {
    let deleted = sqlx::query_scalar!(
        "DELETE FROM sessions WHERE id = $1 AND user_id = $2 RETURNING id",
        session_id,
        user.id
    )
    .fetch_optional(&state.db)
    .await?;
    if deleted.is_none() {
        return Err(AppError::not_found(NO_SUCH_SESSION));
    }
    let by_cookie = matches!(session::presented(&headers), Presented::Cookie(_));
    if session_id == user.session_id && by_cookie {
        let cookie = session::clear_cookie(&state.config);
        return Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, cookie)]).into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
