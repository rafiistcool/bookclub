//! The legal paperwork stays true to the code (#53): `docs/legal/vvt.md` records every table
//! with personal data, `docs/legal/processors.md` gives every outbound host a verdict,
//! `docs/legal/takedown.md` names only endpoints that exist (or that a package is building),
//! and every file the four documents cite exists, with its line numbers inside the file and the
//! names cited in it still there. Files other packages edit are cited by name, not by line.

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

/// Endpoints `takedown.md` names that a package is still building (work plan §2.1 rule 5). The
/// integrator removes each one once its package has merged. ADM's and ADM2's are merged, so
/// every endpoint `takedown.md` names must exist.
const PLANNED: [&str; 0] = [];

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

/// Files other Phase 2 packages edit (work plan §0 and §2: ADM, ADM2, GATE, DEP, CUT, LEG2).
/// A line number into one of them goes stale at that package's merge while
/// [`every_cited_file_exists`] stays green (the files grow), so the documents cite them by name:
/// `` `path` (`symbol`) ``, which [`every_cited_name_exists`] checks. A directory ends in `/`.
const EDITED_ELSEWHERE: [&str; 45] = [
    // ADM
    "server/src/admin/",
    "server/src/catalog/admin.rs",
    "server/src/catalog/requests.rs",
    "server/src/catalog/mod.rs",
    "server/src/media/reports.rs",
    "server/src/media/mod.rs",
    "server/src/email/templates.rs",
    "server/src/jobs.rs",
    "frontend/src/pages/AdminPage.vue",
    "frontend/src/components/admin/",
    "frontend/src/admin.ts",
    "frontend/src/components/EditRequestSheet.vue",
    "frontend/src/pages/BookDetailPage.vue",
    "frontend/src/components/ReportCoverSheet.vue",
    "frontend/src/pages/SettingsPage.vue",
    "server/src/lib.rs",
    // ADM2
    "server/src/auth/extract.rs",
    "server/src/auth/login.rs",
    "server/src/catalog/reports.rs",
    "frontend/src/pages/MemberShelfPage.vue",
    "frontend/src/components/ReportSheet.vue",
    // GATE
    "server/src/spa.rs",
    "frontend/index.html",
    "frontend/public/theme-init.js",
    // DEP
    "server/src/main.rs",
    "server/src/healthcheck.rs",
    "server/src/media/shared/store.rs",
    "server/src/email/mod.rs",
    // CUT
    "server/src/importer/source.rs",
    // LEG2
    "server/src/auth/mod.rs",
    "server/src/auth/deletion.rs",
    "server/tests/jobs.rs",
    // COVER
    "server/src/catalog/openlibrary.rs",
    "server/src/media/service.rs",
    // Phase 3 (SCAN, FRIENDS, MOVE)
    "server/src/ratelimit.rs",
    "server/src/auth/export.rs",
    "server/src/auth/mail.rs",
    "server/src/auth/reset.rs",
    "server/src/config.rs",
    "server/src/catalog/isbn_lookup.rs",
    "server/src/clubs/",
    "server/src/activity/feed.rs",
    "server/src/visibility.rs",
    "server/src/error.rs",
    "server/src/posts/out.rs",
];

/// No `path:line` citation points into a file another package edits ([`EDITED_ELSEWHERE`]).
#[test]
fn files_other_packages_edit_are_cited_by_name() {
    let by_line = Regex::new(r"`((?:server|frontend|docs)/[^`:\s]+):[0-9]").unwrap();
    for doc in LEGAL_DOCS {
        for captures in by_line.captures_iter(&repo_file(doc)) {
            let relative = &captures[1];
            let edited = EDITED_ELSEWHERE.iter().any(|file| {
                relative == *file || (file.ends_with('/') && relative.starts_with(file))
            });
            assert!(
                !edited,
                "{doc} cites {relative} by line, but another package edits it: cite a name \
                 instead, `{relative}` (`symbol`)"
            );
        }
    }
}

/// `` `path` (`name`, …) `` cites names in a file: each must still be there. In a Rust file a
/// snake_case name must be a function and a SCREAMING_CASE name a constant; any other name
/// (a type, an i18n key, a table) must occur as a word. In a directory (`dir/`) each name is an
/// entry.
#[test]
fn every_cited_name_exists() {
    let cited =
        Regex::new(r"`((?:server|frontend|docs)/[^`:\s]+)` \(((?:`[^`]+`(?:, )?)+)\)").unwrap();
    let quoted = Regex::new(r"`([^`]+)`").unwrap();
    let mut checked = 0;
    for doc in LEGAL_DOCS {
        let text = repo_file(doc);
        for captures in cited.captures_iter(&text) {
            let relative = &captures[1];
            let names = quoted
                .captures_iter(&captures[2])
                .map(|name| name[1].to_owned());
            if relative.ends_with('/') {
                for entry in names {
                    assert!(
                        repo_root().join(relative).join(&entry).exists(),
                        "{doc} cites {entry} in {relative}, which does not exist"
                    );
                    checked += 1;
                }
                continue;
            }
            let source = repo_file(relative);
            for name in names {
                assert!(
                    names_it(relative, &source, &name),
                    "{doc} cites {name} in {relative}, which it no longer has"
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 30, "only {checked} names checked");
}

/// Whether `source` (the file `relative`) has `name` in the sense of [`every_cited_name_exists`].
fn names_it(relative: &str, source: &str, name: &str) -> bool {
    let escaped = regex::escape(name);
    let rust = relative.ends_with(".rs");
    let snake_case = name
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_'));
    let screaming_case = name
        .bytes()
        .all(|b| matches!(b, b'A'..=b'Z' | b'0'..=b'9' | b'_'));
    let pattern = if rust && snake_case {
        format!(r"\bfn {escaped}\b")
    } else if rust && screaming_case {
        format!(r"\b(?:const|static) {escaped}\b")
    } else {
        format!(r"(?:^|[^\w.-]){escaped}(?:[^\w-]|$)")
    };
    Regex::new(&pattern).unwrap().is_match(source)
}
