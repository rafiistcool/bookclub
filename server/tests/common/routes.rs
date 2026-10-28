//! Every documented operation, for the generic route-wide guard tests (docs/phase1/conventions.md
//! §17.7): `auth_required.rs`, `admin_guard.rs` and `clubs_non_member.rs` walk the OpenAPI
//! document, so they cover Wave 2 routes as soon as they are merged.

use axum::http::Method;
use serde_json::Value;

use super::{TestClient, TestResponse};

const METHODS: [(&str, Method); 5] = [
    ("get", Method::GET),
    ("post", Method::POST),
    ("put", Method::PUT),
    ("patch", Method::PATCH),
    ("delete", Method::DELETE),
];

/// One operation of the OpenAPI document.
#[derive(Debug, Clone)]
pub struct Operation {
    pub method: Method,
    /// The path template, e.g. `/api/clubs/{slug}/invites/{invite_id}`.
    pub path: String,
    /// The path parameters in order, each with whether its schema is an integer.
    pub params: Vec<(String, bool)>,
    /// The operation documents a request body.
    pub has_body: bool,
    /// The first tag: the owning module (`clubs`, `auth`, …).
    pub tag: String,
}

impl Operation {
    /// `"GET /api/clubs"`.
    pub fn label(&self) -> String {
        format!("{} {}", self.method, self.path)
    }

    /// The path with every `{name}` replaced by `value(name, is_integer)`.
    pub fn fill(&self, value: impl Fn(&str, bool) -> String) -> String {
        let mut path = self.path.clone();
        for (name, integer) in &self.params {
            path = path.replace(&format!("{{{name}}}"), &value(name, *integer));
        }
        assert!(
            !path.contains('{'),
            "{}: a path parameter is not documented",
            self.label()
        );
        path
    }

    /// [`Operation::fill`] with the dummy values of §17.7: `slug` for `{slug}`, `1` for
    /// integer parameters and `x` for every other string.
    pub fn fill_dummy(&self, slug: &str) -> String {
        self.fill(|name, integer| match (name, integer) {
            ("slug", _) => slug.to_owned(),
            (_, true) => "1".to_owned(),
            (_, false) => "x".to_owned(),
        })
    }

    /// Sends the operation to `path` as `client`, with `body` as JSON when the operation takes
    /// a body (the guards answer before any body is read, so `{}` does for them).
    pub async fn send(&self, client: &TestClient, path: &str, body: &Value) -> TestResponse {
        if self.has_body {
            let bytes = serde_json::to_vec(body).unwrap();
            client
                .request(self.method.clone(), path, Some("application/json"), bytes)
                .await
        } else {
            client
                .request(self.method.clone(), path, None, Vec::new())
                .await
        }
    }
}

/// Every operation of `bookclub_server::openapi()`, sorted by path and method.
pub fn operations() -> Vec<Operation> {
    let doc = serde_json::to_value(bookclub_server::openapi()).unwrap();
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().expect("paths") {
        for (key, method) in &METHODS {
            let Some(operation) = item.get(*key) else {
                continue;
            };
            let params = operation["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|param| param["in"] == "path")
                .map(|param| {
                    let name = param["name"].as_str().expect("parameter name").to_owned();
                    (name, param["schema"]["type"] == "integer")
                })
                .collect();
            out.push(Operation {
                method: method.clone(),
                path: path.clone(),
                params,
                has_body: operation.get("requestBody").is_some(),
                tag: operation["tags"][0].as_str().unwrap_or_default().to_owned(),
            });
        }
    }
    out
}

/// The status and the JSON body (`null` when the body is not JSON), for assertions that name
/// the operation.
pub fn outcome(res: &TestResponse) -> (u16, Value) {
    (
        res.status.as_u16(),
        serde_json::from_slice(&res.body).unwrap_or(Value::Null),
    )
}

/// The operation `method path`, if the document has it.
pub fn find(method: &Method, path: &str) -> Option<Operation> {
    operations()
        .into_iter()
        .find(|op| op.method == *method && op.path == path)
}
