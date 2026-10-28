//! Every English detail string the frontend translates still exists in the server
//! (docs/phase1/conventions.md §17.6; the Rust port of backend/tests/test_api_error_copy.py).

use std::path::{Path, PathBuf};

/// Mapped strings whose endpoints are gone or renamed (endpoint-map §11.2). #48 removed the last
/// ones from apiErrors.ts; a string listed here must still be mapped there.
const LEGACY_ONLY: &[&str] = &[];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn mapped_details() -> Vec<String> {
    let path = manifest_dir().join("../frontend/src/i18n/apiErrors.ts");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let pattern = regex::Regex::new(r#""([^"]+)":\s*"errors\."#).unwrap();
    pattern
        .captures_iter(&text)
        .map(|captures| captures[1].to_owned())
        .collect()
}

fn rust_sources(dir: &Path, out: &mut String) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push_str(&std::fs::read_to_string(&path).unwrap());
            out.push('\n');
        }
    }
}

/// parity: test_api_error_copy.py::test_mapped_english_api_errors_still_exist_in_backend
#[test]
fn mapped_english_api_errors_exist_in_the_server() {
    let details = mapped_details();
    assert!(
        details.len() >= 10,
        "only {} mapped details found",
        details.len()
    );

    let mut sources = String::new();
    rust_sources(&manifest_dir().join("src"), &mut sources);
    let missing: Vec<&String> = details
        .iter()
        .filter(|detail| !LEGACY_ONLY.contains(&detail.as_str()))
        .filter(|detail| !sources.contains(&format!("\"{detail}\"")))
        .collect();
    assert!(
        missing.is_empty(),
        "apiErrors.ts details missing from server/src string literals: {missing:?}"
    );
}

#[test]
fn the_legacy_allow_list_only_names_mapped_strings() {
    let details = mapped_details();
    let stale: Vec<&&str> = LEGACY_ONLY
        .iter()
        .filter(|legacy| !details.iter().any(|detail| detail == *legacy))
        .collect();
    assert!(
        stale.is_empty(),
        "no longer mapped in apiErrors.ts; remove from LEGACY_ONLY: {stale:?}"
    );
}
