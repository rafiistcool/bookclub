//! Club access (docs/phase1/conventions.md §5.3). Frozen: every `/api/clubs/{slug}/…` handler
//! takes one of the extractors, and club rows reached any other way go through `member_by_slug`
//! or `role_in`.

use std::collections::HashMap;

use axum::extract::rejection::RawPathParamsRejection;
use axum::extract::{FromRequestParts, RawPathParams};
use axum::http::request::Parts;
use sqlx::PgConnection;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::dto::ClubRef;
use crate::error::{ApiResult, AppError, msg};

/// A member's role in a club, ordered `Member < Admin < Owner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Role {
    Member,
    Admin,
    Owner,
}

impl Role {
    /// The stored and serialised name: `member`, `admin` or `owner`.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Member => "member",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }
}

/// The club of a `/api/clubs/{slug}/…` request.
#[derive(Debug, Clone)]
pub struct ClubCtx {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub timezone: String,
}

/// A signed-in member of the club in the path; 404 `No such club` for a missing club and for a
/// non-member alike.
#[derive(Debug, Clone)]
pub struct ClubMember {
    pub user: CurrentUser,
    pub club: ClubCtx,
    pub role: Role,
}

/// A member with role admin or owner; else 403 `Only club admins can do that`.
#[derive(Debug, Clone)]
pub struct ClubAdmin(pub ClubMember);

/// The club's owner; else 403 `Only the club owner can do that`.
#[derive(Debug, Clone)]
pub struct ClubOwner(pub ClubMember);

impl FromRequestParts<AppState> for ClubMember {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        let slug = path_slug(parts).await?;
        let mut conn = state.db.acquire().await?;
        member_by_slug(&mut conn, &user, &slug).await
    }
}

/// The `{slug}` path parameter. When any parameter of the path is not valid UTF-8, axum reads
/// none of them: such a path names no club the caller can see, so 404 `No such club`.
async fn path_slug(parts: &mut Parts) -> ApiResult<String> {
    let params = match RawPathParams::from_request_parts(parts, &()).await {
        Ok(params) => params,
        Err(RawPathParamsRejection::InvalidUtf8InPathParam(_)) => {
            return Err(AppError::not_found(msg::NO_SUCH_CLUB));
        }
        Err(rejection) => return Err(AppError::internal(rejection.body_text())),
    };
    params
        .iter()
        .find(|(key, _)| *key == "slug")
        .map(|(_, value)| value.to_owned())
        .ok_or_else(|| AppError::internal("club extractor on a route without {slug}"))
}

impl FromRequestParts<AppState> for ClubAdmin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let member = ClubMember::from_request_parts(parts, state).await?;
        if member.role >= Role::Admin {
            Ok(ClubAdmin(member))
        } else {
            Err(AppError::forbidden(msg::ONLY_ADMINS))
        }
    }
}

impl FromRequestParts<AppState> for ClubOwner {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let member = ClubMember::from_request_parts(parts, state).await?;
        if member.role == Role::Owner {
            Ok(ClubOwner(member))
        } else {
            Err(AppError::forbidden(msg::ONLY_OWNER))
        }
    }
}

/// The caller's membership of the club with this slug. 404 `No such club` for an unknown slug or
/// a non-member. Also for `?club=<slug>` query parameters, which `RawPathParams` cannot read.
/// Wave 1 step 4 implements it.
pub async fn member_by_slug(
    _conn: &mut PgConnection,
    _user: &CurrentUser,
    _slug: &str,
) -> ApiResult<ClubMember> {
    Err(AppError::not_found(msg::NO_SUCH_CLUB))
}

/// The user's role in a club, `None` for a non-member: for club rows reached by a global id.
/// Wave 1 step 4 implements it.
pub async fn role_in(
    _conn: &mut PgConnection,
    _user_id: i64,
    _club_id: i64,
) -> sqlx::Result<Option<Role>> {
    Ok(None)
}

/// The `ClubRef` (slug, name) of each club id, for DTO annotation. Wave 1 step 4 implements it.
pub async fn club_refs(
    _conn: &mut PgConnection,
    _club_ids: &[i64],
) -> sqlx::Result<HashMap<i64, ClubRef>> {
    Ok(HashMap::new())
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    #[test]
    fn roles_are_ordered() {
        assert!(Role::Member < Role::Admin && Role::Admin < Role::Owner);
        assert_eq!(Role::Owner.as_str(), "owner");
    }

    async fn slug_of(request: Request) -> ApiResult<String> {
        let (mut parts, _) = request.into_parts();
        path_slug(&mut parts).await
    }

    async fn call(path: &str) -> (StatusCode, String) {
        let app = Router::new().route("/api/clubs/{slug}/picks/{pick_id}", get(slug_of));
        let request = Request::get(path).body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_slug_is_read_percent_decoded() {
        assert_eq!(
            call("/api/clubs/thursday-readers/picks/1").await,
            (StatusCode::OK, "thursday-readers".to_owned())
        );
        assert_eq!(
            call("/api/clubs/a%20b/picks/1").await,
            (StatusCode::OK, "a b".to_owned())
        );
    }

    #[tokio::test]
    async fn an_undecodable_path_parameter_is_no_such_club() {
        for path in [
            "/api/clubs/%ff/picks/1",
            "/api/clubs/thursday-readers/picks/%ff",
        ] {
            assert_eq!(
                call(path).await,
                (
                    StatusCode::NOT_FOUND,
                    r#"{"detail":"No such club"}"#.to_owned()
                ),
                "{path}"
            );
        }
    }
}
