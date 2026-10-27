//! [`TestClient`]: requests through the router as one browser or app.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request};
use bookclub_server::auth::SESSION_COOKIE;
use serde_json::Value;

use super::{TestResponse, send};

const BOUNDARY: &str = "bookclub-test-boundary-7MA4YWxkTrZu0gW";

/// One browser or app: a cookie jar for the session cookie (`bookclub_session`, or
/// `__Host-bookclub_session` when the app's cookies are `Secure`), an optional bearer token, an
/// optional client IP (`ConnectInfo`; without one the server sees `0.0.0.0`) and extra headers.
///
/// Clones share the cookie jar: a login through one is seen by the others.
#[derive(Clone)]
pub struct TestClient {
    router: Router,
    cookie_name: &'static str,
    cookie: Arc<Mutex<Option<String>>>,
    bearer: Option<String>,
    ip: Option<IpAddr>,
    headers: Vec<(String, String)>,
    user: Option<(i64, String)>,
}

/// One part of a multipart body.
#[derive(Debug, Clone)]
pub enum Part {
    File {
        name: String,
        filename: String,
        content_type: String,
        bytes: Vec<u8>,
    },
    Text {
        name: String,
        value: String,
    },
}

impl Part {
    /// The `file` part.
    pub fn file(filename: &str, content_type: &str, bytes: Vec<u8>) -> Part {
        Part::File {
            name: "file".to_owned(),
            filename: filename.to_owned(),
            content_type: content_type.to_owned(),
            bytes,
        }
    }

    /// A text field, e.g. the cover's `edition_id`.
    pub fn text(name: &str, value: &str) -> Part {
        Part::Text {
            name: name.to_owned(),
            value: value.to_owned(),
        }
    }
}

impl TestClient {
    /// An anonymous client for any router, e.g. a small router around one extractor. Its
    /// cookie jar holds `bookclub_session` (a DEV_MODE configuration).
    pub fn for_router(router: Router) -> Self {
        Self::new(router, SESSION_COOKIE, None, None)
    }

    pub(super) fn new(
        router: Router,
        cookie_name: &'static str,
        user: Option<(i64, String)>,
        cookie: Option<String>,
    ) -> Self {
        Self {
            router,
            cookie_name,
            cookie: Arc::new(Mutex::new(cookie)),
            bearer: None,
            ip: None,
            headers: Vec::new(),
            user,
        }
    }

    pub async fn get(&self, path: &str) -> TestResponse {
        self.request(Method::GET, path, None, Vec::new()).await
    }

    pub async fn post(&self, path: &str, body: Value) -> TestResponse {
        self.json(Method::POST, path, body).await
    }

    pub async fn put(&self, path: &str, body: Value) -> TestResponse {
        self.json(Method::PUT, path, body).await
    }

    pub async fn patch(&self, path: &str, body: Value) -> TestResponse {
        self.json(Method::PATCH, path, body).await
    }

    pub async fn delete(&self, path: &str) -> TestResponse {
        self.request(Method::DELETE, path, None, Vec::new()).await
    }

    /// `DELETE` with a JSON body (`DELETE /api/auth/me {password}`).
    pub async fn delete_json(&self, path: &str, body: Value) -> TestResponse {
        self.json(Method::DELETE, path, body).await
    }

    async fn json(&self, method: Method, path: &str, body: Value) -> TestResponse {
        let bytes = serde_json::to_vec(&body).expect("JSON body");
        self.request(method, path, Some("application/json"), bytes)
            .await
    }

    /// Any method, any content type, raw body: malformed JSON, wrong content types, HEAD, 405
    /// checks. The response's `Set-Cookie` for the session updates the jar.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> TestResponse {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(content_type) = content_type {
            builder = builder.header("content-type", content_type);
        }
        if let Some(cookie) = self.cookie() {
            builder = builder.header("cookie", format!("{}={cookie}", self.cookie_name));
        }
        if let Some(token) = &self.bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }
        let mut request = builder.body(Body::from(body)).expect("valid request");
        if let Some(ip) = self.ip {
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::new(ip, 40_000)));
        }
        let response = send(self.router.clone(), request).await;
        self.store_cookies(&response);
        response
    }

    pub async fn post_multipart(&self, path: &str, parts: Vec<Part>) -> TestResponse {
        self.multipart(Method::POST, path, parts).await
    }

    pub async fn put_multipart(&self, path: &str, parts: Vec<Part>) -> TestResponse {
        self.multipart(Method::PUT, path, parts).await
    }

    async fn multipart(&self, method: Method, path: &str, parts: Vec<Part>) -> TestResponse {
        let mut body = Vec::new();
        for part in parts {
            body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
            match part {
                Part::File {
                    name,
                    filename,
                    content_type,
                    bytes,
                } => {
                    body.extend_from_slice(
                        format!(
                            "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
                        )
                        .as_bytes(),
                    );
                    body.extend_from_slice(&bytes);
                }
                Part::Text { name, value } => {
                    body.extend_from_slice(
                        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}")
                            .as_bytes(),
                    );
                }
            }
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        let content_type = format!("multipart/form-data; boundary={BOUNDARY}");
        self.request(method, path, Some(&content_type), body).await
    }

    /// The same client seen from `ip` (sent as `ConnectInfo`).
    pub fn with_ip(mut self, ip: IpAddr) -> Self {
        self.ip = Some(ip);
        self
    }

    /// The same client with an extra header on every request (e.g. `origin`).
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// The same client with `Authorization: Bearer <token>` on every request.
    pub fn with_bearer(mut self, token: &str) -> Self {
        self.bearer = Some(token.to_owned());
        self
    }

    /// The user id; panics for an anonymous client.
    pub fn id(&self) -> i64 {
        self.user.as_ref().expect("an anonymous client has no id").0
    }

    /// The username; panics for an anonymous client.
    pub fn username(&self) -> &str {
        &self
            .user
            .as_ref()
            .expect("an anonymous client has no name")
            .1
    }

    /// The session cookie value in the jar.
    pub fn cookie(&self) -> Option<String> {
        self.cookie.lock().unwrap().clone()
    }

    /// Replaces the session cookie in the jar (`None` empties it).
    pub fn set_cookie(&self, value: Option<&str>) {
        *self.cookie.lock().unwrap() = value.map(str::to_owned);
    }

    /// Like a browser: a `Set-Cookie` for the session stores its value, an empty value or
    /// `Max-Age=0` deletes it.
    fn store_cookies(&self, response: &TestResponse) {
        let prefix = format!("{}=", self.cookie_name);
        for header in response.set_cookies() {
            let Some(rest) = header.strip_prefix(&prefix) else {
                continue;
            };
            let value = rest.split(';').next().unwrap_or_default();
            let expired = header
                .split(';')
                .any(|attribute| attribute.trim().eq_ignore_ascii_case("Max-Age=0"));
            let mut jar = self.cookie.lock().unwrap();
            *jar = (!value.is_empty() && !expired).then(|| value.to_owned());
        }
    }
}
