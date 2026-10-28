//! `GET /api/books/search`, `/trending` and `/subjects/{subject}` (endpoint-map §4,
//! docs/phase1/spec/catalog.md §1.1–§1.3, §2.3–§2.8).
//!
//! Search runs, in order: the length and short-query checks; the empty box serves trending; an
//! ISBN-shaped query goes through the ISBN lookup chain (`CatalogLookup::resolve_isbn`); else
//! the local catalog (full-text and trigram, page 1 only) and Open Library with the legacy
//! candidate rules. Remote hits are persisted as stubs, local hits come first, duplicates (by
//! work and ISBN) are dropped and the page is capped at `limit`.

use std::sync::LazyLock;

use axum::Json;
use axum::extract::State;
use regex::Regex;
use serde::Deserialize;
use sqlx::PgConnection;
use utoipa::IntoParams;

use super::cache::Fetch;
use super::hits::{HitRef, dedupe, persist_docs, prefetch_covers, search_hits};
use super::isbn_lookup::resolve_isbn;
use super::openlibrary::{OlDoc, Sort};
use super::shared::isbn::{looks_like_isbn, parse_isbn};
use super::{
    BROWSE_UNAVAILABLE, INVALID_SORT, INVALID_SUBJECT, NO_SUCH_SUBJECT, QUERY_TOO_SHORT,
    SEARCH_TOO_LONG, SEARCH_UNAVAILABLE, browse_error, upstream_turn,
};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::dto::SearchPage;
use crate::error::{ApiResult, AppError, ErrorBody};
use crate::extract::{ApiPath, ApiQuery, Validate, Validator};
use crate::text::{casefold, char_len, collapse_ws, py_strip};

/// Longest search, in characters after whitespace is collapsed.
pub const MAX_QUERY_CHARS: usize = 200;
/// Shortest text query without a subject (an ISBN is always allowed).
pub const MIN_QUERY_CHARS: usize = 3;
/// The local catalog is searched for needles of at least this many characters.
pub const MIN_LOCAL_CHARS: usize = 2;
pub const MAX_LIMIT: i64 = 40;
/// Highest page of a search or a subject. Every page is its own upstream call and cache entry,
/// so an open-ended page number bypassed the cache at will; Open Library pages that deep are
/// not worth reading anyway.
pub const MAX_PAGE: i64 = 100;

static TITLE_BY_AUTHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(.+?)\s+by\s+(.+)$").unwrap());

fn first_page() -> i32 {
    1
}

fn search_limit() -> i32 {
    24
}

fn browse_limit() -> i32 {
    12
}

/// `GET /api/books/search` parameters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchParams {
    /// The search box; whitespace runs collapse to one space; at most 200 characters.
    #[serde(default)]
    pub q: String,
    /// An Open Library subject key (`science_fiction`); an invalid one is ignored.
    #[serde(default)]
    pub subject: String,
    /// `readinglog`, `new`, `title` or `relevance` (default: relevance with a query, else
    /// readinglog).
    #[serde(default)]
    pub sort: Option<String>,
    /// 1 to 100.
    #[serde(default = "first_page")]
    pub page: i32,
    /// 1 to 40, default 24.
    #[serde(default = "search_limit")]
    pub limit: i32,
}

impl Validate for SearchParams {
    fn validate(&mut self, v: &mut Validator) {
        if let Some(sort) = &self.sort {
            v.check(Sort::parse(sort).is_some(), INVALID_SORT);
        }
        v.range_i64(Some(self.page.into()), 1, MAX_PAGE);
        v.range_i64(Some(self.limit.into()), 1, MAX_LIMIT);
    }
}

/// `GET /api/books/trending` parameters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TrendingParams {
    /// 1 to 40, default 12.
    #[serde(default = "browse_limit")]
    pub limit: i32,
}

impl Validate for TrendingParams {
    fn validate(&mut self, v: &mut Validator) {
        v.range_i64(Some(self.limit.into()), 1, MAX_LIMIT);
    }
}

/// `GET /api/books/subjects/{subject}` parameters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SubjectParams {
    /// 1 to 100.
    #[serde(default = "first_page")]
    pub page: i32,
    /// 1 to 40, default 12.
    #[serde(default = "browse_limit")]
    pub limit: i32,
}

impl Validate for SubjectParams {
    fn validate(&mut self, v: &mut Validator) {
        v.range_i64(Some(self.page.into()), 1, MAX_PAGE);
        v.range_i64(Some(self.limit.into()), 1, MAX_LIMIT);
    }
}

/// An Open Library subject key: `"_".join(subject.lower().split())`, valid only as
/// `[a-z0-9_]+`; anything else is `""`.
pub fn normalize_subject(subject: &str) -> String {
    let key = collapse_ws(&subject.to_lowercase()).replace(' ', "_");
    let valid = key
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if valid { key } else { String::new() }
}

fn open_library_q(query: &str, subject: &str) -> String {
    match (query.is_empty(), subject.is_empty()) {
        (false, false) => format!("{query} subject_key:{subject}"),
        (true, false) => format!("subject_key:{subject}"),
        _ => query.to_owned(),
    }
}

/// The Open Library `q` values for a text search (spec §2.5): the raw query first, then for
/// `"X by Y"` a fielded title/author query, else `title:` and `author:` fallbacks. Each carries
/// ` subject_key:{subject}` when a subject is set.
pub fn search_candidates(query: &str, subject: &str) -> Vec<String> {
    let primary = open_library_q(query, subject);
    if query.is_empty() {
        return vec![primary];
    }
    let with_subject = |q: String| {
        if subject.is_empty() {
            q
        } else {
            format!("{q} subject_key:{subject}")
        }
    };
    if let Some(caps) = TITLE_BY_AUTHOR.captures(query) {
        let title = py_strip(&caps[1]);
        let author = py_strip(&caps[2]);
        // Raw query first: "Stand by Me" must not become title:"Stand" author:"Me" before the
        // plain query, or junk fielded hits would win.
        return vec![
            primary,
            with_subject(format!("title:\"{title}\" author:\"{author}\"")),
        ];
    }
    vec![
        primary,
        with_subject(format!("title:{query}")),
        with_subject(format!("author:{query}")),
    ]
}

/// Searches the catalog: Open Library with the fallbacks of [`search_candidates`] after a
/// page-1 miss. The fallbacks run together; the first one with hits, in candidate order, wins,
/// and a failing fallback only fails the search when none had hits. `has_more` is false for an
/// empty page, whatever the upstream total.
pub async fn remote_search(
    state: &AppState,
    query: &str,
    subject: &str,
    sort: Sort,
    page: i32,
    limit: i32,
) -> Result<(Vec<OlDoc>, bool), Fetch> {
    let catalog = &state.catalog;
    let lang_en = query.is_empty();
    let candidates = search_candidates(query, subject);
    let (primary, fallbacks) = candidates.split_first().expect("one candidate at least");
    let first = catalog
        .ol_search_page(primary, sort, page, limit, lang_en)
        .await?;
    let (mut docs, mut num_found) = (first.docs, first.num_found);
    if docs.is_empty() && page == 1 && !fallbacks.is_empty() {
        let results = futures::future::join_all(
            fallbacks
                .iter()
                .map(|q| catalog.ol_search_page(q, sort, 1, limit, lang_en)),
        )
        .await;
        let mut failure = None;
        for result in results {
            match result {
                Ok(found) if !found.docs.is_empty() => {
                    (docs, num_found) = (found.docs, found.num_found);
                    failure = None;
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
    }
    let has_more = !docs.is_empty() && i64::from(page) * i64::from(limit) < num_found;
    Ok((docs, has_more))
}

/// Current works whose title, subtitle, authors or edition titles match `query`: full-text
/// (every word) or a case- and accent-insensitive substring (trigram index). Ranked by
/// `ts_rank` plus trigram similarity (by title first for `sort=title`), then id. Merged-away
/// works never match. Needles under 2 characters find nothing.
pub async fn local_hits(
    conn: &mut PgConnection,
    query: &str,
    limit: i32,
    sort: Sort,
) -> sqlx::Result<Vec<HitRef>> {
    if char_len(&casefold(query)) < MIN_LOCAL_CHARS {
        return Ok(Vec::new());
    }
    let ids = sqlx::query_scalar!(
        r#"SELECT w.id
           FROM works w
           WHERE w.merged_into_id IS NULL
             AND (w.search_vector @@ plainto_tsquery('simple'::regconfig, bc_unaccent($1))
                  OR bc_search_text(w.title, w.author_names, w.search_extra)
                     LIKE '%' || replace(replace(replace(lower(bc_unaccent($1)),
                                  '\', '\\'), '%', '\%'), '_', '\_') || '%')
           ORDER BY CASE WHEN $2 THEN lower(w.title) END,
                    ts_rank(w.search_vector, plainto_tsquery('simple'::regconfig, bc_unaccent($1)))
                    + similarity(bc_search_text(w.title, w.author_names, w.search_extra),
                                 lower(bc_unaccent($1))) DESC,
                    w.id
           LIMIT $3"#,
        query,
        sort == Sort::Title,
        i64::from(limit)
    )
    .fetch_all(conn)
    .await?;
    Ok(ids.into_iter().map(HitRef::work).collect())
}

fn empty_page(page: i32) -> SearchPage {
    SearchPage {
        items: Vec::new(),
        page,
        has_more: false,
    }
}

/// The search of `GET /api/books/search` for `viewer`.
pub async fn run_search(
    state: &AppState,
    viewer: i64,
    params: SearchParams,
) -> ApiResult<SearchPage> {
    let query = collapse_ws(&params.q);
    if char_len(&query) > MAX_QUERY_CHARS {
        return Err(AppError::bad_request(SEARCH_TOO_LONG));
    }
    let subject = normalize_subject(&params.subject);
    let (page, limit) = (params.page, params.limit);

    // An empty box has nothing to send upstream: it serves trending (with the search limit).
    if query.is_empty() && subject.is_empty() {
        return trending_page(state, viewer, limit).await;
    }
    let isbn_shaped = looks_like_isbn(&query);
    if !query.is_empty() && char_len(&query) < MIN_QUERY_CHARS && !isbn_shaped && subject.is_empty()
    {
        return Err(AppError::bad_request(QUERY_TOO_SHORT));
    }
    if isbn_shaped {
        return isbn_search(state, viewer, &query, page).await;
    }
    let sort = match params.sort.as_deref() {
        Some(sort) => Sort::parse(sort).unwrap_or(Sort::Relevance),
        None if query.is_empty() => Sort::Readinglog,
        None => Sort::Relevance,
    };

    let local = if !query.is_empty() && page == 1 {
        let mut conn = state.db.acquire().await?;
        local_hits(&mut conn, &query, limit, sort).await?
    } else {
        Vec::new()
    };
    let (docs, has_more) = match remote_search(state, &query, &subject, sort, page, limit).await {
        Ok(found) => found,
        // An upstream failure still serves what the catalog itself knows.
        Err(error) if !local.is_empty() => {
            tracing::warn!(?error, "search falls back to local hits");
            (Vec::new(), false)
        }
        Err(error) => {
            return Err(browse_error(
                &error,
                &state.outbound.ol_api,
                SEARCH_UNAVAILABLE,
                None,
            ));
        }
    };
    let mut conn = state.db.acquire().await?;
    let remote = persist_docs(&mut conn, &state.media_service, &docs).await?;
    let refs = dedupe(local.into_iter().chain(remote), limit as usize);
    let items = search_hits(&mut conn, viewer, &refs).await?;
    Ok(SearchPage {
        items,
        page,
        has_more,
    })
}

/// An ISBN-shaped query: the ISBN lookup chain (local edition, then by registration group DNB
/// and/or Open Library), persisted. A checksum that does not hold can match nothing.
async fn isbn_search(
    state: &AppState,
    viewer: i64,
    query: &str,
    page: i32,
) -> ApiResult<SearchPage> {
    let Some(isbn) = parse_isbn(query) else {
        return Ok(empty_page(page));
    };
    if page > 1 {
        return Ok(empty_page(page));
    }
    let Some(resolved) = resolve_isbn(state, &isbn, false, SEARCH_UNAVAILABLE).await? else {
        return Ok(empty_page(page));
    };
    let mut conn = state.db.acquire().await?;
    prefetch_covers(&mut conn, &state.media_service, &[resolved.work_id]).await?;
    let refs = [HitRef {
        work_id: resolved.work_id,
        isbn: Some(isbn.as_str().to_owned()),
    }];
    let items = search_hits(&mut conn, viewer, &refs).await?;
    Ok(SearchPage {
        items,
        page,
        has_more: false,
    })
}

/// Trending works, persisted and annotated: always page 1 without more.
async fn trending_page(state: &AppState, viewer: i64, limit: i32) -> ApiResult<SearchPage> {
    let docs =
        state.catalog.ol_trending(limit).await.map_err(|error| {
            browse_error(&error, &state.outbound.ol_api, BROWSE_UNAVAILABLE, None)
        })?;
    let mut conn = state.db.acquire().await?;
    let refs = persist_docs(&mut conn, &state.media_service, &docs).await?;
    let refs = dedupe(refs, limit as usize);
    let items = search_hits(&mut conn, viewer, &refs).await?;
    Ok(SearchPage {
        items,
        page: 1,
        has_more: false,
    })
}

/// Searches the catalog: works from this catalog first (page 1), then Open Library, without
/// duplicates and at most `limit` hits. Every remote hit is stored, so every hit has an `id`.
/// An empty box serves trending (`page` 1, `has_more` false); an ISBN goes through the ISBN
/// lookup (local editions, then DNB for 978-3, then Open Library). When Open Library fails but
/// the catalog has hits, those are served with `has_more` false.
#[utoipa::path(
    get,
    path = "/api/books/search",
    operation_id = "catalog_search",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(SearchParams),
    responses(
        (status = 200, body = SearchPage, description = "A page of hits"),
        (status = 400, body = ErrorBody, description = "`Search is too long`, `Need at least 3 \
            characters. Add the author, or paste an ISBN.` (also an upstream 422), or a \
            parameter error"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 429, body = ErrorBody, description = "`The library is busy. Try again in {n} \
            seconds.` or `The library is busy. Wait a few seconds and try again.`; `Slow down a \
            little. Try again later.` (60 catalog requests per account a minute)"),
        (status = 502, body = ErrorBody, description = "`Could not search the library right \
            now. Try again.` (trending: `Could not load the library right now. Try again.`)")
    )
)]
pub async fn search(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<SearchParams>,
) -> ApiResult<Json<SearchPage>> {
    let _turn = upstream_turn(&state, user.id).await?;
    Ok(Json(run_search(&state, user.id, params).await?))
}

/// Open Library's trending works of the day (cached for 18 hours), stored and annotated.
#[utoipa::path(
    get,
    path = "/api/books/trending",
    operation_id = "catalog_trending",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(TrendingParams),
    responses(
        (status = 200, body = SearchPage, description = "Trending works; `page` 1, `has_more` false"),
        (status = 400, body = ErrorBody, description = "A parameter error, or an upstream 422"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 429, body = ErrorBody, description = "The library is busy, or `Slow down a little. Try again later.` (60 catalog requests per account a minute)"),
        (status = 502, body = ErrorBody, description = "`Could not load the library right now. Try again.`")
    )
)]
pub async fn trending(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<TrendingParams>,
) -> ApiResult<Json<SearchPage>> {
    let _turn = upstream_turn(&state, user.id).await?;
    Ok(Json(trending_page(&state, user.id, params.limit).await?))
}

/// One page of an Open Library subject (cached for 18 hours), stored and annotated.
#[utoipa::path(
    get,
    path = "/api/books/subjects/{subject}",
    operation_id = "catalog_subject",
    tag = "catalog",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(
        ("subject" = String, Path, description = "A subject; spaces become `_`, only `[a-z0-9_]` remain valid"),
        SubjectParams
    ),
    responses(
        (status = 200, body = SearchPage, description = "A page of the subject's works"),
        (status = 400, body = ErrorBody, description = "`That is not a valid subject.`, a parameter error, or an upstream 422"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No such subject.`"),
        (status = 429, body = ErrorBody, description = "The library is busy, or `Slow down a little. Try again later.` (60 catalog requests per account a minute)"),
        (status = 502, body = ErrorBody, description = "`Could not load the library right now. Try again.`")
    )
)]
pub async fn subject(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiPath(subject): ApiPath<String>,
    ApiQuery(params): ApiQuery<SubjectParams>,
) -> ApiResult<Json<SearchPage>> {
    let key = normalize_subject(&subject);
    if key.is_empty() {
        return Err(AppError::bad_request(INVALID_SUBJECT));
    }
    let _turn = upstream_turn(&state, user.id).await?;
    let (page, limit) = (params.page, params.limit);
    let offset = (i64::from(page) - 1) * i64::from(limit);
    let (docs, work_count) = state
        .catalog
        .ol_subject(&key, page, limit, offset)
        .await
        .map_err(|error| {
            browse_error(
                &error,
                &state.outbound.ol_api,
                BROWSE_UNAVAILABLE,
                Some(NO_SUCH_SUBJECT),
            )
        })?;
    let mut conn = state.db.acquire().await?;
    let refs = persist_docs(&mut conn, &state.media_service, &docs).await?;
    let refs = dedupe(refs, limit as usize);
    let items = search_hits(&mut conn, user.id, &refs).await?;
    let has_more = !items.is_empty() && offset + i64::from(limit) < work_count;
    Ok(Json(SearchPage {
        items,
        page,
        has_more,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: test_search.py::test_search_candidates_keep_raw_query_before_by_split
    #[test]
    fn candidates_keep_the_raw_query_first() {
        assert_eq!(
            search_candidates("Stand by Me", ""),
            ["Stand by Me", "title:\"Stand\" author:\"Me\""]
        );
        assert_eq!(
            search_candidates("Circe by Madeline Miller", ""),
            [
                "Circe by Madeline Miller",
                "title:\"Circe\" author:\"Madeline Miller\""
            ]
        );
        // The split is lazy: it happens at the first " by ".
        assert_eq!(
            search_candidates("Stand by Me by Ben E. King", "")[1],
            "title:\"Stand\" author:\"Me by Ben E. King\""
        );
        assert_eq!(
            search_candidates("obscurexyz", "fantasy"),
            [
                "obscurexyz subject_key:fantasy",
                "title:obscurexyz subject_key:fantasy",
                "author:obscurexyz subject_key:fantasy"
            ]
        );
        assert_eq!(search_candidates("", "fantasy"), ["subject_key:fantasy"]);
    }

    #[test]
    fn subjects_normalise_or_are_invalid() {
        assert_eq!(normalize_subject("Science  Fiction"), "science_fiction");
        assert_eq!(normalize_subject("fantasy"), "fantasy");
        for invalid in ["", "sci-fi", "Ünicode", "!!!", "   "] {
            assert_eq!(normalize_subject(invalid), "", "{invalid}");
        }
    }
}
