//! Request extractors that answer every rejection with the API's 400 JSON shape
//! (docs/phase1/conventions.md §4.3–§4.6). Never use `axum::Json`, `Query` or `Path` for input.

use std::collections::HashMap;
use std::fmt;

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, PathRejection};
use axum::extract::{
    FromRequest, FromRequestParts, Multipart, Path, Query, RawPathParams, Request,
};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use serde::de::{self, DeserializeOwned, Deserializer, Visitor};
use serde::{Deserialize, Serialize};

use crate::error::{ApiResult, AppError, msg};

/// pydantic's message for a missing field or a missing body.
pub const FIELD_REQUIRED: &str = "Field required";
/// The body is not valid JSON.
pub const JSON_DECODE_ERROR: &str = "JSON decode error";
/// The body is not a JSON object, or was not sent as `application/json`.
pub const NOT_AN_OBJECT: &str =
    "Input should be a valid dictionary or object to extract fields from";
pub const NOT_A_STRING: &str = "Input should be a valid string";
pub const NOT_AN_INTEGER: &str = "Input should be a valid integer";
pub const NOT_A_BOOLEAN: &str = "Input should be a valid boolean";
pub const NOT_A_LIST: &str = "Input should be a valid list";
/// Also the answer to a string value with a NUL character (U+0000) in a body, query or path:
/// Postgres `text` cannot store it.
pub const INVALID_VALUE: &str = "Invalid value";
/// A query or path value that should be an integer.
pub const INTEGER_PARSE: &str =
    "Input should be a valid integer, unable to parse string as an integer";
/// A query value that should be a boolean.
pub const BOOLEAN_PARSE: &str = "Input should be a valid boolean, unable to interpret input";
/// A query value that should be a number.
pub const NUMBER_PARSE: &str = "Input should be a valid number, unable to parse string as a number";
/// A body over the route's `DefaultBodyLimit` (1 MiB unless the route raises it).
pub const BODY_TOO_LARGE: &str = "Request body is too large";

/// Normalises a request value in place and collects every custom validation message, in field
/// declaration order. The default checks nothing.
pub trait Validate {
    fn validate(&mut self, _v: &mut Validator) {}
}

/// Collects validation messages; a non-empty list becomes one 400 joined by `"; "`.
#[derive(Debug, Default)]
pub struct Validator {
    messages: Vec<String>,
}

impl Validator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a message.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.messages.push(message.into());
    }

    /// Records `message` unless `ok`.
    pub fn check(&mut self, ok: bool, message: &str) {
        if !ok {
            self.fail(message);
        }
    }

    /// pydantic's `ge`/`le` bounds with its wording. `None` passes.
    pub fn range_i64(&mut self, value: Option<i64>, min: i64, max: i64) {
        match value {
            Some(v) if v < min => {
                self.fail(format!("Input should be greater than or equal to {min}"))
            }
            Some(v) if v > max => self.fail(format!("Input should be less than or equal to {max}")),
            _ => {}
        }
    }

    /// The messages recorded so far.
    pub fn messages(&self) -> &[String] {
        &self.messages
    }

    /// `Ok` when nothing failed, else 400 with every message joined by `"; "`.
    pub fn into_result(self) -> ApiResult<()> {
        if self.messages.is_empty() {
            Ok(())
        } else {
            Err(AppError::bad_request(self.messages.join("; ")))
        }
    }
}

fn validated<T: Validate>(mut value: T) -> ApiResult<T> {
    let mut validator = Validator::new();
    value.validate(&mut validator);
    validator.into_result()?;
    Ok(value)
}

/// A JSON body: `Content-Type: application/json`, a JSON object, deserialised strictly and then
/// validated. The only JSON input extractor.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_json = has_json_content_type(req.headers());
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(body_rejection)?;
        let value = parse_json_body(is_json, &bytes)?;
        Ok(ApiJson(validated(value)?))
    }
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
}

fn body_rejection(rejection: BytesRejection) -> AppError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::payload_too_large(BODY_TOO_LARGE)
    } else {
        AppError::bad_request(FIELD_REQUIRED)
    }
}

/// The JSON body steps of §4.3 for already-buffered bytes. An empty body is `Field required`
/// whatever the content type (parity: FastAPI checks for a body first).
fn parse_json_body<T: DeserializeOwned>(is_json: bool, bytes: &[u8]) -> ApiResult<T> {
    if bytes.is_empty() {
        return Err(AppError::bad_request(FIELD_REQUIRED));
    }
    if !is_json {
        return Err(AppError::bad_request(NOT_AN_OBJECT));
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| AppError::bad_request(JSON_DECODE_ERROR))?;
    if !value.is_object() {
        return Err(AppError::bad_request(NOT_AN_OBJECT));
    }
    if has_nul(&value) {
        return Err(AppError::bad_request(INVALID_VALUE));
    }
    serde_path_to_error::deserialize(value)
        .map_err(|error| AppError::bad_request(json_message(&error.into_inner().to_string())))
}

/// Whether any string value in `value` contains U+0000, which no `text` column accepts (it
/// would turn the request into a 500).
fn has_nul(value: &serde_json::Value) -> bool {
    use serde_json::Value;
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(has_nul),
        Value::Object(fields) => fields.values().any(has_nul),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// Maps serde_json's message for the first deserialisation error to pydantic's wording.
fn json_message(serde_message: &str) -> &'static str {
    if serde_message.starts_with("missing field ") {
        return FIELD_REQUIRED;
    }
    let Some(rest) = serde_message.strip_prefix("invalid type: ") else {
        return INVALID_VALUE;
    };
    let expected = rest
        .rsplit_once(", expected ")
        .map_or("", |(_, expected)| expected);
    match expected {
        "a string" => NOT_A_STRING,
        "a boolean" => NOT_A_BOOLEAN,
        "a sequence" => NOT_A_LIST,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
        | "usize" => NOT_AN_INTEGER,
        _ => INVALID_VALUE,
    }
}

/// Query parameters, deserialised like `axum::extract::Query`, rejections mapped to 400, then
/// validated. Use [`lax_bool`] for boolean parameters. A value with a NUL character (`%00`) is
/// 400 `Invalid value`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Ok(Query(pairs)) = Query::<Vec<(String, String)>>::try_from_uri(&parts.uri)
            && pairs
                .iter()
                .any(|(key, value)| key.contains('\0') || value.contains('\0'))
        {
            return Err(AppError::bad_request(INVALID_VALUE));
        }
        let Query(value) = Query::<T>::try_from_uri(&parts.uri)
            .map_err(|rejection| AppError::bad_request(query_message(&rejection.body_text())))?;
        Ok(ApiQuery(validated(value)?))
    }
}

/// Maps a query-string deserialisation message to pydantic's wording.
fn query_message(text: &str) -> &'static str {
    const INTEGER_ERRORS: [&str; 4] = [
        "invalid digit found in string",
        "cannot parse integer from empty string",
        "number too large to fit in target type",
        "number too small to fit in target type",
    ];
    if text.contains(BOOLEAN_PARSE) || text.contains("provided string was not `true` or `false`") {
        BOOLEAN_PARSE
    } else if text.contains("missing field") {
        FIELD_REQUIRED
    } else if INTEGER_ERRORS.iter().any(|message| text.contains(message)) {
        INTEGER_PARSE
    } else if text.contains("invalid float literal") {
        NUMBER_PARSE
    } else {
        INVALID_VALUE
    }
}

/// Path parameters. A non-numeric value for an integer parameter is 400; a numeric value that
/// overflows its type is 404 `Not found` (no such row can exist). A value with a NUL character
/// (`%00`) is 400 `Invalid value`, like one that is not UTF-8.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let value = match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(value)) => value,
            Err(rejection) => return Err(path_rejection(rejection)),
        };
        if let Ok(raw) = RawPathParams::from_request_parts(parts, state).await
            && raw.iter().any(|(_, value)| value.contains('\0'))
        {
            return Err(AppError::bad_request(INVALID_VALUE));
        }
        Ok(ApiPath(value))
    }
}

fn path_rejection(rejection: PathRejection) -> AppError {
    use axum::extract::path::ErrorKind;

    let PathRejection::FailedToDeserializePathParams(error) = rejection else {
        return AppError::internal(rejection.body_text());
    };
    match error.kind() {
        ErrorKind::ParseErrorAtKey {
            value,
            expected_type,
            ..
        }
        | ErrorKind::ParseErrorAtIndex {
            value,
            expected_type,
            ..
        }
        | ErrorKind::ParseError {
            value,
            expected_type,
        } => integer_path_error(value, expected_type),
        ErrorKind::InvalidUtf8InPathParam { .. }
        | ErrorKind::DeserializeError { .. }
        | ErrorKind::Message(_) => AppError::bad_request(INVALID_VALUE),
        _ => AppError::internal(error.body_text()),
    }
}

fn integer_path_error(value: &str, expected_type: &str) -> AppError {
    let is_integer_type = matches!(
        expected_type,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
    );
    if !is_integer_type {
        return AppError::bad_request(INVALID_VALUE);
    }
    let digits = value.strip_prefix(['-', '+']).unwrap_or(value);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        AppError::not_found(msg::NOT_FOUND)
    } else {
        AppError::bad_request(INTEGER_PARSE)
    }
}

/// A multipart upload. Its own rejection (not multipart at all) is 400 `Field required`.
pub struct ApiMultipart(pub Multipart);

/// The `file` part of an upload.
#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub filename: String,
    pub content_type: Option<String>,
    pub bytes: Bytes,
}

/// The other text fields of an upload, e.g. the cover's `edition_id`.
#[derive(Debug, Clone, Default)]
pub struct FormFields(pub HashMap<String, String>);

impl<S> FromRequest<S> for ApiMultipart
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Multipart::from_request(req, state)
            .await
            .map(ApiMultipart)
            .map_err(|_| AppError::bad_request(FIELD_REQUIRED))
    }
}

impl ApiMultipart {
    /// Reads every part. Returns the `file` part plus the text fields. A missing `file` part, a
    /// malformed body or a part over `max_bytes` → 400 with `bad_upload` (the route's documented
    /// string, e.g. `error::msg::PICTURE_TOO_LARGE` for images).
    pub async fn file(
        self,
        max_bytes: usize,
        bad_upload: &'static str,
    ) -> ApiResult<(UploadedFile, FormFields)> {
        let ApiMultipart(mut multipart) = self;
        let bad = || AppError::bad_request(bad_upload);
        let mut file = None;
        let mut fields = FormFields::default();
        while let Some(mut field) = multipart.next_field().await.map_err(|_| bad())? {
            let name = field.name().unwrap_or_default().to_owned();
            if name == "file" {
                let filename = field.file_name().unwrap_or_default().to_owned();
                let content_type = field.content_type().map(str::to_owned);
                let mut bytes = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(|_| bad())? {
                    if bytes.len() + chunk.len() > max_bytes {
                        return Err(bad());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                file = Some(UploadedFile {
                    filename,
                    content_type,
                    bytes: Bytes::from(bytes),
                });
            } else {
                let value = field.text().await.map_err(|_| bad())?;
                fields.0.insert(name, value);
            }
        }
        Ok((file.ok_or_else(bad)?, fields))
    }
}

/// A PATCH field that tells "absent" from `null`. Use with `#[serde(default)]` and document it
/// with `#[schema(value_type = Option<T>)]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Patch<T> {
    /// The key was not in the body: leave the value alone.
    #[default]
    Missing,
    /// The key was `null`: clear the value.
    Null,
    /// The key had a value: set it.
    Value(T),
}

impl<T> Patch<T> {
    pub fn is_missing(&self) -> bool {
        matches!(self, Patch::Missing)
    }

    /// `None` when missing, `Some(None)` for null, `Some(Some(v))` for a value.
    pub fn into_option(self) -> Option<Option<T>> {
        match self {
            Patch::Missing => None,
            Patch::Null => Some(None),
            Patch::Value(value) => Some(Some(value)),
        }
    }

    /// Borrowing form of [`Patch::into_option`].
    pub fn as_option(&self) -> Option<Option<&T>> {
        match self {
            Patch::Missing => None,
            Patch::Null => Some(None),
            Patch::Value(value) => Some(Some(value)),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            None => Patch::Null,
            Some(value) => Patch::Value(value),
        })
    }
}

impl<T: Serialize> Serialize for Patch<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Patch::Missing | Patch::Null => serializer.serialize_none(),
            Patch::Value(value) => serializer.serialize_some(value),
        }
    }
}

/// Parses a boolean the way pydantic does in query strings: `true/false/1/0/yes/no/on/off/t/f/y/n`
/// in any case.
pub fn parse_lax_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" | "t" | "y" => Some(true),
        "false" | "0" | "no" | "off" | "f" | "n" => Some(false),
        _ => None,
    }
}

/// `#[serde(default, deserialize_with = "crate::extract::lax_bool")]` for boolean query
/// parameters (parity for `include_reading`).
pub fn lax_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    struct LaxBool;

    impl Visitor<'_> for LaxBool {
        type Value = bool;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a boolean")
        }

        fn visit_bool<E: de::Error>(self, value: bool) -> Result<bool, E> {
            Ok(value)
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<bool, E> {
            parse_lax_bool(value).ok_or_else(|| E::custom(BOOLEAN_PARSE))
        }
    }

    deserializer.deserialize_any(LaxBool)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use axum::routing::{get, post};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;

    #[derive(Debug, Deserialize, Serialize)]
    struct Body1 {
        name: String,
        #[serde(default)]
        count: Option<i64>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        flag: bool,
    }

    impl Validate for Body1 {
        fn validate(&mut self, v: &mut Validator) {
            self.name = self.name.trim().to_owned();
            v.check(!self.name.is_empty(), "Give it a name");
            v.range_i64(self.count, 1, 50);
        }
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct PatchBody {
        #[serde(default)]
        rating: Patch<i32>,
    }

    impl Validate for PatchBody {}

    #[derive(Debug, Deserialize, Serialize)]
    struct ListQuery {
        #[serde(default)]
        page: Option<i64>,
        #[serde(default, deserialize_with = "lax_bool")]
        include_reading: bool,
    }

    impl Validate for ListQuery {
        fn validate(&mut self, v: &mut Validator) {
            v.range_i64(self.page, 1, 100);
        }
    }

    #[derive(Debug, Deserialize, Serialize)]
    struct NeedsQ {
        q: String,
    }

    impl Validate for NeedsQ {}

    fn app() -> Router {
        Router::new()
            .route("/json", post(|ApiJson(body): ApiJson<Body1>| async move { axum::Json(body) }))
            .route("/patch", post(|ApiJson(body): ApiJson<PatchBody>| async move { axum::Json(body) }))
            .route("/query", get(|ApiQuery(q): ApiQuery<ListQuery>| async move { axum::Json(q) }))
            .route("/needs-q", get(|ApiQuery(q): ApiQuery<NeedsQ>| async move { axum::Json(q) }))
            .route("/items/{id}", get(|ApiPath(id): ApiPath<i64>| async move { id.to_string() }))
            .route(
                "/clubs/{slug}/picks/{pick_id}",
                get(|ApiPath((slug, id)): ApiPath<(String, i64)>| async move { format!("{slug}/{id}") }),
            )
    }

    async fn send(request: HttpRequest<Body>) -> (StatusCode, Value) {
        let response = app().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, value)
    }

    async fn post_raw(path: &str, content_type: Option<&str>, body: &str) -> (StatusCode, Value) {
        let mut request = HttpRequest::post(path);
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        send(request.body(Body::from(body.to_owned())).unwrap()).await
    }

    async fn post_json(body: &str) -> (StatusCode, Value) {
        post_raw("/json", Some("application/json"), body).await
    }

    async fn get_path(path: &str) -> (StatusCode, Value) {
        send(HttpRequest::get(path).body(Body::empty()).unwrap()).await
    }

    fn detail(result: &(StatusCode, Value)) -> &str {
        result.1["detail"].as_str().unwrap_or_default()
    }

    #[tokio::test]
    async fn json_accepts_and_normalises_a_valid_body() {
        let (status, body) = post_json(r#"{"name":"  Ada  ","count":3,"unknown":1}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({"name": "Ada", "count": 3, "tags": [], "flag": false})
        );
        let (status, _) = post_raw(
            "/json",
            Some("application/json; charset=utf-8"),
            r#"{"name":"x"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn json_body_shape_errors() {
        let cases = [
            (Some("application/json"), "", FIELD_REQUIRED),
            (None, "", FIELD_REQUIRED),
            (Some("text/plain"), "", FIELD_REQUIRED),
            (Some("text/plain"), r#"{"name":"x"}"#, NOT_AN_OBJECT),
            (None, r#"{"name":"x"}"#, NOT_AN_OBJECT),
            (Some("application/json"), "{nope", JSON_DECODE_ERROR),
            (Some("application/json"), "[1]", NOT_AN_OBJECT),
            (Some("application/json"), "\"text\"", NOT_AN_OBJECT),
        ];
        for (content_type, body, expected) in cases {
            let result = post_raw("/json", content_type, body).await;
            assert_eq!(
                result.0,
                StatusCode::BAD_REQUEST,
                "{content_type:?} {body:?}"
            );
            assert_eq!(detail(&result), expected, "{content_type:?} {body:?}");
        }
    }

    #[tokio::test]
    async fn json_type_errors_use_pydantic_wording() {
        let cases = [
            (r#"{}"#, FIELD_REQUIRED),
            (r#"{"name":null}"#, NOT_A_STRING),
            (r#"{"name":5}"#, NOT_A_STRING),
            (r#"{"name":"x","count":"5"}"#, NOT_AN_INTEGER),
            (r#"{"name":"x","count":5.5}"#, NOT_AN_INTEGER),
            (r#"{"name":"x","count":5.0}"#, NOT_AN_INTEGER),
            (r#"{"name":"x","count":true}"#, NOT_AN_INTEGER),
            (r#"{"name":"x","flag":"yes"}"#, NOT_A_BOOLEAN),
            (r#"{"name":"x","tags":"a"}"#, NOT_A_LIST),
            (r#"{"name":"x","tags":[1]}"#, NOT_A_STRING),
            (
                r#"{"name":"x","count":99999999999999999999}"#,
                NOT_AN_INTEGER,
            ),
        ];
        for (body, expected) in cases {
            let result = post_json(body).await;
            assert_eq!(result.0, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(detail(&result), expected, "{body}");
        }
    }

    #[tokio::test]
    async fn json_validation_collects_every_message_in_order() {
        let result = post_json(r#"{"name":"   ","count":0}"#).await;
        assert_eq!(result.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            detail(&result),
            "Give it a name; Input should be greater than or equal to 1"
        );
        let result = post_json(r#"{"name":"x","count":51}"#).await;
        assert_eq!(detail(&result), "Input should be less than or equal to 50");
    }

    #[tokio::test]
    async fn json_body_over_the_limit_is_413_json() {
        let big = format!(r#"{{"name":"{}"}}"#, "x".repeat(3 * 1024 * 1024));
        let result = post_json(&big).await;
        assert_eq!(result.0, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(detail(&result), BODY_TOO_LARGE);
    }

    #[tokio::test]
    async fn patch_tells_missing_from_null() {
        let (_, body) = post_raw("/patch", Some("application/json"), "{}").await;
        assert_eq!(body, json!({"rating": null}));
        let parsed: PatchBody = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed.rating, Patch::Missing);
        let parsed: PatchBody = serde_json::from_str(r#"{"rating":null}"#).unwrap();
        assert_eq!(parsed.rating, Patch::Null);
        assert_eq!(parsed.rating.into_option(), Some(None));
        let parsed: PatchBody = serde_json::from_str(r#"{"rating":4}"#).unwrap();
        assert_eq!(parsed.rating, Patch::Value(4));
        assert!(!parsed.rating.is_missing());
        assert_eq!(parsed.rating.as_option(), Some(Some(&4)));
    }

    #[tokio::test]
    async fn query_parses_lax_bools_and_bounds() {
        let (status, body) = get_path("/query?page=2&include_reading=Yes").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"page": 2, "include_reading": true}));
        for value in ["true", "1", "on", "t", "Y", "TRUE"] {
            let (_, body) = get_path(&format!("/query?include_reading={value}")).await;
            assert_eq!(body["include_reading"], true, "{value}");
        }
        for value in ["false", "0", "off", "f", "n", "No"] {
            let (_, body) = get_path(&format!("/query?include_reading={value}")).await;
            assert_eq!(body["include_reading"], false, "{value}");
        }
        let (_, body) = get_path("/query").await;
        assert_eq!(body, json!({"page": null, "include_reading": false}));
    }

    #[tokio::test]
    async fn query_errors_use_pydantic_wording() {
        let cases = [
            ("/query?page=abc", INTEGER_PARSE),
            ("/query?page=", INTEGER_PARSE),
            ("/query?page=1.5", INTEGER_PARSE),
            (
                "/query?page=0",
                "Input should be greater than or equal to 1",
            ),
            (
                "/query?page=101",
                "Input should be less than or equal to 100",
            ),
            ("/query?include_reading=maybe", BOOLEAN_PARSE),
            ("/needs-q", FIELD_REQUIRED),
        ];
        for (path, expected) in cases {
            let result = get_path(path).await;
            assert_eq!(result.0, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(detail(&result), expected, "{path}");
        }
    }

    #[tokio::test]
    async fn path_ids_are_integers() {
        assert_eq!(get_path("/items/42").await, (StatusCode::OK, json!(42)));
        let result = get_path("/items/abc").await;
        assert_eq!(result.0, StatusCode::BAD_REQUEST);
        assert_eq!(detail(&result), INTEGER_PARSE);
        let result = get_path("/items/99999999999999999999").await;
        assert_eq!(result.0, StatusCode::NOT_FOUND);
        assert_eq!(detail(&result), "Not found");
        let result = get_path("/clubs/book-club/picks/x1").await;
        assert_eq!(result.0, StatusCode::BAD_REQUEST);
        assert_eq!(detail(&result), INTEGER_PARSE);
        let (status, _) = get_path("/clubs/book-club/picks/7").await;
        assert_eq!(status, StatusCode::OK);
    }

    /// Postgres `text` cannot store U+0000, so no extractor lets it reach a query.
    #[tokio::test]
    async fn nul_characters_are_invalid_values() {
        for body in [
            r#"{"name":"a\u0000b"}"#,
            r#"{"name":"x","tags":["ok","\u0000"]}"#,
            r#"{"name":"x","unknown":{"deep":"\u0000"}}"#,
        ] {
            let result = post_json(body).await;
            assert_eq!(result.0, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(detail(&result), INVALID_VALUE, "{body}");
        }
        for path in [
            "/needs-q?q=a%00b",
            "/needs-q?q=x&other=%00",
            "/needs-q?q=x&%00=1",
            "/clubs/a%00b/picks/1",
        ] {
            let result = get_path(path).await;
            assert_eq!(result.0, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(detail(&result), INVALID_VALUE, "{path}");
        }
        // An integer parameter keeps its own message.
        let result = get_path("/items/%00").await;
        assert_eq!(detail(&result), INTEGER_PARSE);
        // Other control characters are fine.
        let (status, body) = post_json(r#"{"name":"a\u0001b"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["name"], "a\u{1}b");
    }

    #[test]
    fn lax_bool_vocabulary() {
        assert_eq!(parse_lax_bool("ON"), Some(true));
        assert_eq!(parse_lax_bool("n"), Some(false));
        assert_eq!(parse_lax_bool("2"), None);
        assert_eq!(parse_lax_bool(""), None);
    }

    #[tokio::test]
    async fn multipart_reads_the_file_and_fields() {
        let boundary = "XBOUNDARY";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"edition_id\"\r\n\r\n12\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\n\
             Content-Type: image/png\r\n\r\nPNGDATA\r\n--{boundary}--\r\n"
        );
        let request = HttpRequest::post("/")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let multipart = ApiMultipart::from_request(request, &()).await.unwrap();
        let (file, fields) = multipart.file(1024, msg::PICTURE_TOO_LARGE).await.unwrap();
        assert_eq!(file.filename, "a.png");
        assert_eq!(file.content_type.as_deref(), Some("image/png"));
        assert_eq!(&file.bytes[..], b"PNGDATA");
        assert_eq!(fields.0["edition_id"], "12");
    }

    #[tokio::test]
    async fn multipart_rejections_are_400() {
        let request = HttpRequest::post("/")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let error = ApiMultipart::from_request(request, &())
            .await
            .err()
            .unwrap();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.detail, FIELD_REQUIRED);

        let boundary = "B";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a\"\r\n\r\n\
             0123456789\r\n--{boundary}--\r\n"
        );
        let request = HttpRequest::post("/")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body.clone()))
            .unwrap();
        let multipart = ApiMultipart::from_request(request, &()).await.unwrap();
        let error = multipart.file(5, msg::PICTURE_TOO_LARGE).await.unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.detail, msg::PICTURE_TOO_LARGE);

        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"other\"\r\n\r\nx\r\n--{boundary}--\r\n"
        );
        let request = HttpRequest::post("/")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let multipart = ApiMultipart::from_request(request, &()).await.unwrap();
        let error = multipart.file(5, "Bad CSV").await.unwrap_err();
        assert_eq!(error.detail, "Bad CSV");
    }
}
