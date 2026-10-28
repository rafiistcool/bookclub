//! The legal paperwork stays true to the code (#53): `docs/legal/vvt.md` records every table
//! with personal data, `docs/legal/processors.md` gives every outbound host a verdict,
//! `docs/legal/takedown.md` names only endpoints that exist (or that ADM and ADM2 are building),
//! and every file the four documents cite exists, with its line numbers inside the file.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::test_config;
use regex::Regex;
use serde_json::Value;
use sqlx::PgPool;

const LEGAL_DOCS: [&str; 4] = [
    "docs/legal/privacy-facts.md",
    "docs/legal/vvt.md",
    "docs/legal/processors.md",
    "docs/legal/takedown.md",
];

/// Endpoints `takedown.md` names that Wave 1 and 2 packages are building (work plan §2.1 rule
/// 5). The integrator removes each one once its package has merged.
const PLANNED: [&str; 7] = [
    "PATCH /api/admin/works/{work_id}",              // ADM
    "PATCH /api/admin/editions/{edition_id}",        // ADM
    "GET /api/admin/summary",                        // ADM
    "POST /api/books/works/{work_id}/reports",       // ADM2
    "POST /api/books/editions/{edition_id}/reports", // ADM2
    "POST /api/admin/users/{username}/suspend",      // ADM2
    "POST /api/admin/users/{username}/unsuspend",    // ADM2
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn repo_file(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// Every table with a foreign key to `users`, and `users` itself, appears in `vvt.md` as
/// `` `table` ``. A new table with personal data fails here until the record names it.
#[sqlx::test]
async fn every_table_with_personal_data_is_in_the_record(db: PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT conrelid::regclass::text FROM pg_constraint
         WHERE contype = 'f' AND confrelid = 'users'::regclass
         ORDER BY 1",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert!(tables.len() >= 20, "too few tables: {tables:?}");
    let vvt = repo_file("docs/legal/vvt.md");
    let missing: Vec<String> = tables
        .into_iter()
        .chain(["users".to_owned()])
        .filter(|table| !vvt.contains(&format!("`{table}`")))
        .collect();
    assert!(missing.is_empty(), "vvt.md does not record {missing:?}");
}

/// Every host the server calls (the default upstream URLs of `config.rs` and the redirect
/// domains of `outbound.rs`) has a line in `processors.md` that names it and gives a verdict.
#[test]
fn every_outbound_host_has_a_verdict() {
    let config = test_config(&[]);
    let mut hosts: BTreeSet<String> = [
        &config.openlibrary_url,
        &config.openlibrary_covers_url,
        &config.dnb_sru_url,
        &config.dnb_oai_url,
    ]
    .into_iter()
    .map(|url| url.host_str().unwrap().to_owned())
    .collect();
    let outbound = repo_file("server/src/outbound.rs");
    let redirects = Regex::new(r"REDIRECT_DOMAINS: \[&str; \d+\] = \[([^\]]*)\]")
        .unwrap()
        .captures(&outbound)
        .expect("REDIRECT_DOMAINS in outbound.rs");
    let quoted = Regex::new(r#""([^"]+)""#).unwrap();
    hosts.extend(
        quoted
            .captures_iter(&redirects[1])
            .map(|captures| captures[1].to_owned()),
    );
    assert!(hosts.len() >= 4, "{hosts:?}");

    let processors = repo_file("docs/legal/processors.md");
    for host in &hosts {
        let verdict = processors.lines().any(|line| {
            line.contains(&format!("`{host}`")) && line.to_lowercase().contains("processor")
        });
        assert!(
            verdict,
            "processors.md has no line naming `{host}` with a verdict"
        );
    }
}

/// Every `METHOD /path` that `takedown.md` names is an operation of the OpenAPI document, or a
/// planned one; every app file it names exists.
#[test]
fn every_removal_path_points_to_an_endpoint_or_a_screen() {
    let doc: Value = serde_json::to_value(bookclub_server::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    let operations: BTreeSet<String> = paths
        .iter()
        .flat_map(|(path, item)| {
            item.as_object()
                .unwrap()
                .keys()
                .map(move |method| format!("{} {path}", method.to_uppercase()))
        })
        .collect();

    let takedown = repo_file("docs/legal/takedown.md");
    let named = Regex::new(r"`(GET|POST|PUT|PATCH|DELETE) (/[^`\s]+)`").unwrap();
    let mut found = 0;
    for captures in named.captures_iter(&takedown) {
        let operation = format!("{} {}", &captures[1], &captures[2]);
        found += 1;
        assert!(
            operations.contains(&operation) || PLANNED.contains(&operation.as_str()),
            "takedown.md names {operation}, which is neither an endpoint nor planned"
        );
    }
    assert!(found >= 15, "only {found} endpoints found in takedown.md");

    let screens = Regex::new(r"`(frontend/src/[^`]+\.vue)`").unwrap();
    let screens: Vec<String> = screens
        .captures_iter(&takedown)
        .map(|captures| captures[1].to_owned())
        .collect();
    assert!(screens.len() >= 3, "{screens:?}");
    for screen in screens {
        assert!(
            repo_root().join(&screen).is_file(),
            "takedown.md names {screen}, which does not exist"
        );
    }
}

/// Every `server/…`, `frontend/…` or `docs/…` path the legal documents cite exists, and a
/// `:line` or `:from-to` suffix stays inside the file. Globs and templates are skipped.
#[test]
fn every_cited_file_exists() {
    let cited = Regex::new(r"`((?:server|frontend|docs)/[^`:\s]+)(?::([0-9][0-9,-]*))?`").unwrap();
    let number = Regex::new(r"[0-9]+").unwrap();
    let mut checked = 0;
    for doc in LEGAL_DOCS {
        let text = repo_file(doc);
        for captures in cited.captures_iter(&text) {
            let relative = &captures[1];
            if relative.contains(['*', '{', '<']) {
                continue;
            }
            let path = repo_root().join(relative);
            assert!(
                path.exists(),
                "{doc} cites {relative}, which does not exist"
            );
            checked += 1;
            let Some(lines) = captures.get(2) else {
                continue;
            };
            let count = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{doc}: cannot read {relative}: {error}"))
                .lines()
                .count();
            for line in number.find_iter(lines.as_str()) {
                let line: usize = line.as_str().parse().unwrap();
                assert!(
                    (1..=count).contains(&line),
                    "{doc} cites {relative}:{line}, but the file has {count} lines"
                );
            }
        }
    }
    assert!(checked >= 40, "only {checked} citations checked");
}
