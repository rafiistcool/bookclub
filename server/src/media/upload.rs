//! Reading an uploaded picture (avatars and cover photos): the legacy order of checks of
//! docs/phase1/spec/auth.md §1.6 and conventions.md §4.6.
//!
//! 1. A `Content-Length` over 52,432,896 bytes (50 MiB plus 4 KiB of multipart overhead) is 413
//!    `That picture is too large (50 MB max)` before any of the body is read.
//! 2. A body that is not multipart, or has no `file` part with a file name, is 400 `Use a JPEG,
//!    PNG, WebP, or GIF`.
//! 3. A `file` part over 50 MiB is 400 `That picture is too large (50 MB max)` (a 400 here, as in
//!    the legacy app).
//!
//! The pipeline (`MediaService::process`) then decides whether the bytes are a picture.

use axum::extract::multipart::MultipartError;
use axum::http::header::CONTENT_LENGTH;
use axum::http::{HeaderMap, StatusCode};
use bytes::{Bytes, BytesMut};

use super::pipeline::{MAX_UPLOAD_BYTES, PictureError};
use crate::error::{ApiResult, AppError, msg};
use crate::extract::{ApiMultipart, FormFields};
use crate::ratelimit::{GatePass, RateLimits};

/// Multipart overhead allowed on top of [`MAX_UPLOAD_BYTES`] in `Content-Length`.
pub const UPLOAD_LENGTH_SLACK: u64 = 4096;
/// The body limit of the upload routes (conventions §4.6).
pub const UPLOAD_BODY_LIMIT: usize = 50 * 1024 * 1024 + 64 * 1024;
/// Longest text field of an upload form (`edition_id`).
const MAX_TEXT_FIELD: usize = 1024;

/// The multipart form of an avatar upload (OpenAPI only).
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub struct PictureUpload {
    /// A JPEG, PNG, WebP or GIF of at most 50 MB and 25 megapixels.
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
}

/// The multipart form of a cover photo (OpenAPI only).
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
pub struct CoverUpload {
    /// A JPEG, PNG, WebP or GIF of at most 50 MB and 25 megapixels.
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
    /// The edition the photo shows (an edition of the work in the path); without it, the work.
    edition_id: Option<i64>,
}

/// True when `Content-Length` already announces more than the picture limit plus slack. A
/// missing or unparsable header passes (the body limit still applies).
pub fn oversize_content_length(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .is_some_and(|length| length > MAX_UPLOAD_BYTES as u64 + UPLOAD_LENGTH_SLACK)
}

/// The account's one picture upload at a time (`RateLimits::upload_gate`), taken before the
/// body is read; 429 `Slow down a little. Try again later.` while another is still running, so
/// one account cannot hold many 50 MiB bodies in memory or queue many decodes at once.
pub fn upload_turn(limits: &RateLimits, user_id: i64) -> ApiResult<GatePass> {
    limits
        .upload_gate
        .try_enter(&format!("u:{user_id}"))
        .ok_or_else(|| AppError::too_many(msg::SLOW_DOWN, None))
}

/// The picture's bytes and the form's text fields.
pub async fn read_picture(
    headers: &HeaderMap,
    multipart: Result<ApiMultipart, AppError>,
) -> ApiResult<(Bytes, FormFields)> {
    if oversize_content_length(headers) {
        return Err(AppError::payload_too_large(msg::PICTURE_TOO_LARGE));
    }
    let Ok(ApiMultipart(mut multipart)) = multipart else {
        return Err(AppError::bad_request(msg::PICTURE_BAD_TYPE));
    };
    let mut file = None;
    let mut fields = FormFields::default();
    while let Some(mut field) = multipart.next_field().await.map_err(refused)? {
        let name = field.name().unwrap_or_default().to_owned();
        if name == "file" && field.file_name().is_some() {
            let mut bytes = BytesMut::new();
            while let Some(chunk) = field.chunk().await.map_err(refused)? {
                if bytes.len() + chunk.len() > MAX_UPLOAD_BYTES {
                    return Err(AppError::bad_request(msg::PICTURE_TOO_LARGE));
                }
                bytes.extend_from_slice(&chunk);
            }
            file = Some(bytes.freeze());
        } else {
            let mut value = BytesMut::new();
            while let Some(chunk) = field.chunk().await.map_err(refused)? {
                if value.len() + chunk.len() > MAX_TEXT_FIELD {
                    return Err(AppError::bad_request(msg::PICTURE_BAD_TYPE));
                }
                value.extend_from_slice(&chunk);
            }
            fields
                .0
                .insert(name, String::from_utf8_lossy(&value).into_owned());
        }
    }
    let file = file.ok_or_else(|| AppError::bad_request(msg::PICTURE_BAD_TYPE))?;
    Ok((file, fields))
}

/// A body that broke off or passed the route's body limit while it was read.
fn refused(error: MultipartError) -> AppError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        AppError::bad_request(msg::PICTURE_TOO_LARGE)
    } else {
        AppError::bad_request(msg::PICTURE_BAD_TYPE)
    }
}

/// The 400 of a refused picture, or a 500 when the encoder failed.
pub fn picture_error(error: PictureError) -> AppError {
    match error.detail() {
        Some(detail) => AppError::bad_request(detail),
        None => AppError::internal(format!("picture pipeline: {error:?}")),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    /// parity: test_avatars.py::test_oversize_content_length_is_rejected_before_read (the unit half)
    #[test]
    fn content_length_over_the_limit_plus_slack_is_oversize() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_LENGTH, HeaderValue::from_str(value).unwrap());
            oversize_content_length(&headers)
        };
        assert!(!oversize_content_length(&HeaderMap::new()));
        assert!(!with("100"));
        assert!(!with("52432896"));
        assert!(with("52432897"));
        assert!(!with("not a number"));
    }
}
