//! The one error shape of the API: JSON `{"detail": "<English>"}` (docs/phase1/conventions.md §4.1).

use std::borrow::Cow;
use std::fmt;

use axum::Json;
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

/// Result type of handlers and operation functions.
pub type ApiResult<T> = Result<T, AppError>;

/// Detail strings shared across modules, copied byte for byte from the specs.
///
/// Every string of endpoint-map §11.1 lives here, whichever package returns it, so
/// `tests/api_error_copy.rs` passes in every worktree. Strings used by one module only are
/// `const`s at the top of that module.
pub mod msg {
    // endpoint-map §11.1: kept verbatim and mapped in frontend/src/i18n/apiErrors.ts.
    pub const NOT_SIGNED_IN: &str = "Not signed in";
    pub const USERNAME_TAKEN: &str = "That username is already taken";
    pub const INVITE_INVALID: &str = "That invite code is not valid";
    pub const TOO_MANY_ATTEMPTS: &str = "Too many attempts. Wait a few minutes.";
    pub const WRONG_CREDENTIALS: &str = "Wrong username or password";
    pub const CURRENT_PASSWORD_WRONG: &str = "Current password is wrong";
    pub const PICK_DIFFERENT_PASSWORD: &str = "Pick a different password";
    pub const PICTURE_TOO_LARGE: &str = "That picture is too large (50 MB max)";
    pub const PICTURE_TOO_MANY_PIXELS: &str = "That picture has too many pixels";
    pub const PICTURE_BAD_TYPE: &str = "Use a JPEG, PNG, WebP, or GIF";
    pub const PICTURE_NOT_AN_IMAGE: &str = "That file is not a picture";
    pub const FAVOURITE_TWICE: &str = "Each book can only be a favourite once";
    pub const AT_MOST_3_FAVOURITES: &str = "At most 3 favourites";
    pub const ALREADY_3_FAVOURITES: &str = "You already have 3 favourites";
    pub const READING_DATE_NEEDS_FINISHED: &str = "Reading date requires a finished book";
    pub const READING_DATE_IN_FUTURE: &str = "Reading date cannot be in the future";
    pub const SEARCH_HISTORY_NOT_FOUND: &str = "Search history entry not found";

    // conventions §4.1: shared across modules.
    pub const NO_SUCH_CLUB: &str = "No such club";
    pub const ONLY_ADMINS: &str = "Only club admins can do that";
    pub const ONLY_OWNER: &str = "Only the club owner can do that";
    pub const SLOW_DOWN: &str = "Slow down a little. Try again later.";
    pub const NO_MEMBER: &str = "No member with that name";
    pub const NO_SUCH_BOOK: &str = "No such book.";
    pub const BOOK_NOT_IN_CATALOG: &str = "That book is not in the catalog";
    pub const NO_ENTRY: &str = "No entry with that id";
    pub const NOT_FOUND: &str = "Not found";

    // Platform strings and other strings more than one module returns.
    pub const INTERNAL: &str = "Internal Server Error";
    pub const METHOD_NOT_ALLOWED: &str = "Method Not Allowed";
    pub const CROSS_SITE_REFUSED: &str = "Cross-site request refused";
    /// 408 from the request deadline (`serve::REQUEST_DEADLINE`).
    pub const REQUEST_TIMEOUT: &str = "The request took too long";
    /// 503 when no password permit freed up within its wait (`auth::password::Busy`).
    pub const SERVER_BUSY: &str = "The server is busy. Try again in a moment.";
    pub const VERIFY_EMAIL_FIRST: &str = "Verify your email address first";
    pub const INVALID_TIME_ZONE: &str = "That is not a valid time zone";
    pub const CLUB_FULL: &str = "This club is full";
    pub const EDITION_OF_ANOTHER_BOOK: &str = "That edition belongs to another book";
    /// Also returned for `closes_at` and `due_at` (docs/phase1/spec/club.md §2.1).
    pub const MEETING_AT_FORMAT: &str = "meeting_at must be a date and time";

    // LEG (#53).
    /// 400 when a sign-up or `POST /api/auth/terms` sends no terms version, or another one than
    /// the operator's current terms (`LEGAL_DIR/manifest.json`).
    pub const TERMS_CHANGED: &str = "The terms of use have changed. Reload the page and try again.";

    // ADM2 (#55).
    /// 403 when a suspended account signs in or creates a bearer token, after its password
    /// matched (`auth::login::confirm`).
    pub const ACCOUNT_SUSPENDED: &str = "This account is suspended";

    // FRIENDS (#57).
    /// 404 for a username no account has: the admin's suspension routes, a friend request and a
    /// block (docs/phase3/work-plan.md §4.9).
    pub const NO_ACCOUNT: &str = "No account with that name";
    /// 400 when a friend request or a block names the caller.
    pub const SELF_ACCOUNT: &str = "That is your own account";
    /// 409 when the caller sends a friend request to an account they blocked.
    pub const YOU_BLOCKED: &str = "You blocked this account";
    /// 404 when there is no pending request the caller may accept, decline or cancel.
    pub const NO_FRIEND_REQUEST: &str = "No such friend request";
    /// 404 when the caller is not friends with that name: removal and friend invites.
    pub const NO_FRIEND: &str = "No friend with that name";
}

/// An error response. Every non-2xx answer of the API is one of these.
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub detail: Cow<'static, str>,
    /// Extra top-level fields, e.g. `item` next to 409 `Already on your shelf`.
    pub extra: Option<Map<String, Value>>,
    /// Seconds for the `Retry-After` header (429).
    pub retry_after: Option<u64>,
}

/// The documented error body. `extra` fields of an [`AppError`] are not part of the schema.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub detail: String,
}

impl AppError {
    /// An error with any status and detail.
    pub fn new(status: StatusCode, detail: impl Into<Cow<'static, str>>) -> Self {
        Self {
            status,
            detail: detail.into(),
            extra: None,
            retry_after: None,
        }
    }

    /// 400 with `detail`.
    pub fn bad_request(detail: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, detail)
    }

    /// 401 `Not signed in`.
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, msg::NOT_SIGNED_IN)
    }

    /// 403 with `detail`: the caller can see the thing but may not do this.
    pub fn forbidden(detail: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::FORBIDDEN, detail)
    }

    /// 404 with `detail`: missing, or not visible to the caller.
    pub fn not_found(detail: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::NOT_FOUND, detail)
    }

    /// 409 with `detail`.
    pub fn conflict(detail: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::CONFLICT, detail)
    }

    /// 413 with `detail`.
    pub fn payload_too_large(detail: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, detail)
    }

    /// 429 with `detail`, and `Retry-After` when `retry_after` is set.
    pub fn too_many(detail: impl Into<Cow<'static, str>>, retry_after: Option<u64>) -> Self {
        Self {
            retry_after,
            ..Self::new(StatusCode::TOO_MANY_REQUESTS, detail)
        }
    }

    /// 502 with `detail`: an upstream catalog or image source failed.
    pub fn bad_gateway(detail: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, detail)
    }

    /// 500 `Internal Server Error`. Logs `error` at ERROR; the client never sees it.
    pub fn internal(error: impl fmt::Display) -> Self {
        tracing::error!(%error, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, msg::INTERNAL)
    }

    /// Adds an extra top-level field to the body. `detail` cannot be overridden.
    pub fn with(mut self, key: &str, value: impl Serialize) -> Self {
        let value = serde_json::to_value(value).unwrap_or(Value::Null);
        self.extra
            .get_or_insert_with(Map::new)
            .insert(key.to_owned(), value);
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status.as_u16(), self.detail)
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let mut body = Map::new();
        body.insert("detail".to_owned(), Value::String(self.detail.into_owned()));
        for (key, value) in self.extra.into_iter().flatten() {
            if key != "detail" {
                body.insert(key, value);
            }
        }
        let mut response = (self.status, Json(Value::Object(body))).into_response();
        if let Some(seconds) = self.retry_after {
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::internal(error)
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        match error.downcast_ref::<crate::auth::password::Busy>() {
            Some(busy) => (*busy).into(),
            None => Self::internal(format!("{error:#}")),
        }
    }
}

impl From<crate::auth::password::Busy> for AppError {
    /// 503 `The server is busy. Try again in a moment.`: every password permit stayed taken.
    fn from(_: crate::auth::password::Busy) -> Self {
        tracing::warn!("a password operation gave up waiting for a permit");
        Self::new(StatusCode::SERVICE_UNAVAILABLE, msg::SERVER_BUSY)
    }
}

/// True when `error` is a unique violation (SQLSTATE 23505) of the named constraint or
/// unique index, so a race can be mapped to its documented 409 or 400.
pub fn is_unique_violation(error: &sqlx::Error, constraint: &str) -> bool {
    error.as_database_error().is_some_and(|db| {
        db.code().as_deref() == Some("23505") && db.constraint() == Some(constraint)
    })
}

/// True when Postgres aborted the transaction to break a deadlock (SQLSTATE 40P01). A caller
/// that holds one transaction across several catalog ingests (HAR's page) can deadlock with a
/// concurrent one and retries the whole transaction (conventions.md §7.2).
pub fn is_deadlock(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|db| db.code().as_deref() == Some("40P01"))
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn body_of(error: AppError) -> (StatusCode, axum::http::HeaderMap, Value) {
        let response = error.into_response();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn renders_detail_as_json() {
        let (status, headers, body) = body_of(AppError::not_found(msg::NO_SUCH_CLUB)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(body, serde_json::json!({ "detail": "No such club" }));
    }

    #[tokio::test]
    async fn extra_fields_sit_next_to_detail() {
        let error = AppError::conflict("Already on your shelf")
            .with("item", serde_json::json!({ "id": 7 }))
            .with("detail", "ignored");
        let (status, _, body) = body_of(error).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body,
            serde_json::json!({ "detail": "Already on your shelf", "item": { "id": 7 } })
        );
    }

    #[tokio::test]
    async fn too_many_sets_retry_after() {
        let (status, headers, body) = body_of(AppError::too_many(msg::SLOW_DOWN, Some(42))).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(headers["retry-after"], "42");
        assert_eq!(body["detail"], msg::SLOW_DOWN);
        let (_, headers, _) = body_of(AppError::too_many(msg::SLOW_DOWN, None)).await;
        assert!(headers.get("retry-after").is_none());
    }

    #[tokio::test]
    async fn internal_errors_hide_the_cause() {
        let (status, _, body) = body_of(AppError::from(sqlx::Error::RowNotFound)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body,
            serde_json::json!({ "detail": "Internal Server Error" })
        );
        let (status, _, body) = body_of(AppError::from(anyhow::anyhow!("secret path"))).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["detail"], "Internal Server Error");
    }

    #[test]
    fn constructors_pick_their_status() {
        assert_eq!(AppError::bad_request("x").status, StatusCode::BAD_REQUEST);
        assert_eq!(AppError::unauthorized().status, StatusCode::UNAUTHORIZED);
        assert_eq!(AppError::unauthorized().detail, "Not signed in");
        assert_eq!(AppError::forbidden("x").status, StatusCode::FORBIDDEN);
        assert_eq!(
            AppError::payload_too_large("x").status,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(AppError::bad_gateway("x").status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn strings_keep_their_exact_bytes() {
        assert_eq!(msg::NO_SUCH_BOOK, "No such book.");
        assert_eq!(
            msg::TOO_MANY_ATTEMPTS,
            "Too many attempts. Wait a few minutes."
        );
        assert_eq!(msg::SLOW_DOWN, "Slow down a little. Try again later.");
        assert_eq!(
            msg::PICTURE_TOO_LARGE,
            "That picture is too large (50 MB max)"
        );
    }

    #[test]
    fn other_errors_are_not_unique_violations() {
        assert!(!is_unique_violation(
            &sqlx::Error::RowNotFound,
            "users_username_key"
        ));
    }
}
