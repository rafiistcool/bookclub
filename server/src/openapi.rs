//! The OpenAPI document (docs/phase1/conventions.md §14): `GET /api/openapi.json` and
//! `bookclub-server openapi`. #47 generates `frontend/src/api/generated.ts` from it.

use std::sync::OnceLock;

use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::auth::SESSION_COOKIE;
use crate::dto::{
    BookOut, CatalogSource, ClubRef, ClubSummary, ContributorOut, EditionOut, ReactionOut,
    SearchHit, SearchPage, ShelfStatus, UserOut, UserSummary, Visibility,
};
use crate::error::ErrorBody;

/// Security scheme names for `#[utoipa::path(security(...))]`.
pub const COOKIE_SCHEME: &str = "bookclub_session";
pub const BEARER_SCHEME: &str = "bearer";

/// The document's root: info, security schemes and every shared DTO of endpoint-map §0, so they
/// are in the document whether or not a merged handler uses them yet.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Bookclub API",
        description = "The HTTP API of the hosted Bookclub service. Errors are JSON `{\"detail\": \"…\"}`."
    ),
    modifiers(&SecuritySchemes),
    components(schemas(
        BookOut,
        EditionOut,
        ContributorOut,
        SearchHit,
        SearchPage,
        UserOut,
        UserSummary,
        ClubRef,
        ClubSummary,
        ShelfStatus,
        Visibility,
        CatalogSource,
        ReactionOut,
        ErrorBody
    )),
    tags((name = "platform", description = "Health and the API document"))
)]
pub struct ApiDoc;

/// Registers the session cookie and bearer token schemes.
struct SecuritySchemes;

impl Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            COOKIE_SCHEME,
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                SESSION_COOKIE,
                "The session cookie. Named `__Host-bookclub_session` when it is `Secure` \
                 (`COOKIE_SECURE`, the production default).",
            ))),
        );
        components.add_security_scheme(
            BEARER_SCHEME,
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("bc_<token>")
                    .build(),
            ),
        );
    }
}

/// `GET /api/openapi.json`.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(openapi_json))
}

/// The OpenAPI document of every route. It is the API contract and contains no secrets.
#[utoipa::path(
    get,
    path = "/api/openapi.json",
    operation_id = "platform_openapi",
    tag = "platform",
    responses((status = 200, description = "The OpenAPI document", content_type = "application/json"))
)]
async fn openapi_json() -> impl IntoResponse {
    static DOCUMENT: OnceLock<String> = OnceLock::new();
    let json = DOCUMENT.get_or_init(|| {
        crate::openapi()
            .to_json()
            .expect("the OpenAPI document serialises")
    });
    ([(CONTENT_TYPE, "application/json")], json.as_str())
}
