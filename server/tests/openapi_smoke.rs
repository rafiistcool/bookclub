//! The OpenAPI document is well-formed (docs/phase1/conventions.md §14). Generic: it covers every
//! package's routes once they merge.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// The shared DTOs of endpoint-map §0, plus `ErrorBody`.
const SHARED_DTOS: [&str; 14] = [
    "BookOut",
    "EditionOut",
    "ContributorOut",
    "SearchHit",
    "SearchPage",
    "UserOut",
    "UserSummary",
    "ClubRef",
    "ClubSummary",
    "ShelfStatus",
    "Visibility",
    "CatalogSource",
    "ReactionOut",
    "ErrorBody",
];

fn document() -> Value {
    serde_json::to_value(bookclub_server::openapi()).unwrap()
}

/// `(path, method, operation)` for every operation in the document.
fn operations(doc: &Value) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().expect("paths") {
        for method in METHODS {
            if let Some(operation) = item.get(method) {
                out.push((path.clone(), method.to_owned(), operation.clone()));
            }
        }
    }
    out
}

#[test]
fn operation_ids_are_unique_and_namespaced() {
    let doc = document();
    let pattern = regex::Regex::new("^[a-z]+_[a-z0-9_]+$").unwrap();
    let mut seen = HashSet::new();
    let operations = operations(&doc);
    assert!(!operations.is_empty());
    for (path, method, operation) in operations {
        let id = operation["operationId"]
            .as_str()
            .unwrap_or_else(|| panic!("{method} {path} has no operationId"));
        assert!(
            pattern.is_match(id),
            "{method} {path}: operationId {id:?} is not <module>_<action>"
        );
        assert!(seen.insert(id.to_owned()), "operationId {id} is used twice");
    }
}

#[test]
fn every_operation_has_a_tag_and_responses() {
    let doc = document();
    for (path, method, operation) in operations(&doc) {
        let tags = operation["tags"].as_array().map_or(0, Vec::len);
        assert!(tags > 0, "{method} {path} has no tag");
        let responses = operation["responses"].as_object().map_or(0, |r| r.len());
        assert!(responses > 0, "{method} {path} documents no responses");
    }
}

#[test]
fn every_shared_dto_is_in_the_document() {
    let doc = document();
    let schemas = doc["components"]["schemas"].as_object().expect("schemas");
    for name in SHARED_DTOS {
        assert!(
            schemas.contains_key(name),
            "{name} is missing from components.schemas"
        );
    }
    let schemes = doc["components"]["securitySchemes"]
        .as_object()
        .expect("securitySchemes");
    assert!(schemes.contains_key("bookclub_session") && schemes.contains_key("bearer"));
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// utoipa silently keeps only one of two same-named schemas, so every `ToSchema` type name must
/// be unique crate-wide.
#[test]
fn to_schema_names_are_unique() {
    let derive = regex::Regex::new(r"#\[derive\(([^)]*)\)\]").unwrap();
    let item = regex::Regex::new(r"\b(?:struct|enum)\s+([A-Za-z0-9_]+)").unwrap();
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );

    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for found in derive.captures_iter(&text) {
            if !found[1].contains("ToSchema") {
                continue;
            }
            let rest = &text[found.get(0).unwrap().end()..];
            let name = item
                .captures(rest)
                .expect("a struct or enum after the derive")[1]
                .to_owned();
            names
                .entry(name)
                .or_default()
                .push(file.display().to_string());
        }
    }
    for shared in SHARED_DTOS {
        assert!(names.contains_key(shared), "the scan missed {shared}");
    }
    let duplicates: Vec<_> = names.iter().filter(|(_, files)| files.len() > 1).collect();
    assert!(
        duplicates.is_empty(),
        "ToSchema names defined twice: {duplicates:?}"
    );
}

/// `bookclub-server openapi` prints the document without a database or configuration.
#[test]
fn the_cli_prints_the_document() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bookclub-server"))
        .arg("openapi")
        .env_clear()
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(printed, document());
}
