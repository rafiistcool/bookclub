//! #41 acceptance: no Google Books code or data under `server/` (legacy-mapping.md §0). Scans
//! `server/src`, `server/tests` and `server/migrations`, plus the image (the root `Dockerfile`)
//! and the compose files, for Google Books API hosts, cover hosts and settings, except the
//! importer (code that recognises and drops Google fields) and the synthetic legacy fixture.

use std::fs;
use std::path::{Path, PathBuf};

/// Exempt subtrees, relative to `server/`.
const CARVE_OUTS: [&str; 2] = ["src/importer", "tests/fixtures/legacy"];

/// Single lines that may mention a Google host because they assert it is refused, as
/// (file, trimmed line). Each is a W1 test of the outbound redirect guard.
const ALLOWED_LINES: [(&str, &str); 1] = [(
    "src/outbound.rs",
    "\"https://books.google.com/books/content?id=x\",",
)];

/// What Google Books code or data would contain (matched case-insensitively).
fn needles() -> Vec<String> {
    [
        "googleapis",
        "books.google",
        "googleusercontent",
        "google_books_api_key",
        "x-goog-api-key",
        "volumeinfo",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn no_google_books_code_or_data_in_the_server() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut all = Vec::new();
    for dir in ["src", "tests", "migrations"] {
        files(&root.join(dir), &mut all);
    }
    // Relative to `server/`. These must exist: a renamed file would otherwise drop out unseen.
    for extra in [
        "Cargo.toml",
        "compose.yaml",
        "../Dockerfile",
        "../deploy/hosted/compose.yaml",
    ] {
        let path = root.join(extra);
        assert!(path.is_file(), "{extra} is missing; update the scan list");
        all.push(path);
    }
    let needles = needles();
    let mut scanned = 0;
    let mut found = Vec::new();
    for file in all {
        let relative = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // This scanner names what it looks for.
        if relative == file!() || CARVE_OUTS.iter().any(|exempt| relative.starts_with(exempt)) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue; // binary fixtures
        };
        scanned += 1;
        for (number, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            if !needles.iter().any(|needle| lower.contains(needle.as_str())) {
                continue;
            }
            if ALLOWED_LINES.contains(&(relative.as_str(), line.trim())) {
                continue;
            }
            found.push(format!("{relative}:{}: {}", number + 1, line.trim()));
        }
    }
    assert!(scanned > 50, "only {scanned} files scanned");
    assert!(
        found.is_empty(),
        "Google Books code or data:\n{}",
        found.join("\n")
    );
}

/// The scan itself works: it finds a planted needle.
#[test]
fn the_needles_match_google_hosts() {
    let needles = needles();
    for sample in [
        "https://www.googleapis.com/books/v1/volumes",
        "https://books.google.com/books/content?id=x",
        "GOOGLE_BOOKS_API_KEY=x",
        "X-Goog-Api-Key: secret",
        "\"volumeInfo\": {}",
    ] {
        let lower = sample.to_lowercase();
        assert!(
            needles.iter().any(|needle| lower.contains(needle.as_str())),
            "{sample}"
        );
    }
}
