//! Keeps docs/phase1/parity-checklist.md (#49) from going stale. It fails when:
//!
//! - a Python test function in `backend/tests/test_*.py` has no checklist line, or a section's
//!   count is not the number of test functions in its file;
//! - a line is not checked off (`- [x]`), carries no status, or a `ported`, `changed` or
//!   `replaced` line names no Rust test, or a `dropped` line gives no reason;
//! - a `/// parity: <file>::<test>` comment in `server/src` or `server/tests` sits on a Rust test
//!   that the line of that Python test does not name;
//! - the file names a Rust test (`` `tests/x.rs::name` `` or `` `src/…/x.rs::name` ``) that
//!   does not exist;
//! - a parametrised Python test's line does not give its case count;
//! - a row of the summary table disagrees with its section (test count and statuses);
//! - a Python test listed in `docs/phase1/parity-notes.md` (the reasoned drops) has no line.
//!
//! Lines whose Python test no longer exists are allowed: #48 deletes
//! `test_api_error_copy.py`, and the cutover (#52) deletes `backend/` (then delete this test,
//! and the checklist stays as the record).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

const STATUSES: [&str; 4] = ["ported", "changed", "replaced", "dropped"];

struct PyTest {
    name: String,
    parametrised: bool,
}

struct Line {
    checked: bool,
    status: Option<&'static str>,
    rust: Vec<String>,
    rest: String,
    raw: String,
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Test functions per Python file, in file order.
fn python_tests() -> BTreeMap<String, Vec<PyTest>> {
    let dir = repo().join("backend/tests");
    assert!(
        dir.is_dir(),
        "{} is gone: after the cutover (#52), delete this test",
        dir.display()
    );
    let def = Regex::new(r"^(?:async\s+)?def\s+(test\w*)\s*\(").unwrap();
    let nested = Regex::new(r"^\s+(?:async\s+)?def\s+test\w*\s*\(").unwrap();
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with("test_") && name.ends_with(".py")) {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let mut tests = Vec::new();
        let mut decorators: Vec<&str> = Vec::new();
        for line in text.lines() {
            assert!(
                !line.starts_with("class Test") && !nested.is_match(line),
                "{name}: test classes and nested test functions need a checker update: {line}"
            );
            if line.starts_with('@') {
                decorators.push(line);
                continue;
            }
            if let Some(found) = def.captures(line) {
                tests.push(PyTest {
                    name: found[1].to_owned(),
                    parametrised: decorators.iter().any(|d| d.contains("parametrize")),
                });
            }
            // A statement at column 0 ends the decorators; the continuation lines of a
            // multi-line decorator are indented or close a bracket.
            if line
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
            {
                decorators.clear();
            }
        }
        files.insert(name, tests);
    }
    files
}

fn checklist_text() -> String {
    fs::read_to_string(repo().join("docs/phase1/parity-checklist.md")).unwrap()
}

/// Every Rust test the checklist names, anywhere in the file.
fn all_references(text: &str) -> BTreeSet<String> {
    let rust = Regex::new(r"`((?:src|tests)/[\w/]+\.rs)::(\w+)`").unwrap();
    rust.captures_iter(text)
        .map(|r| format!("{}::{}", &r[1], &r[2]))
        .collect()
}

/// The summary table: file → [tests, cases, ported, changed, replaced, dropped].
fn summary(text: &str) -> BTreeMap<String, [usize; 6]> {
    let row =
        Regex::new(r"^\| (?:`(test_\w+\.py)`|\*\*(Total)\*\*) \|((?: \**\d+\** \|){6})$").unwrap();
    let mut rows = BTreeMap::new();
    for line in text.lines() {
        if let Some(found) = row.captures(line) {
            let name = found.get(1).or(found.get(2)).unwrap().as_str().to_owned();
            let numbers: Vec<usize> = found[3]
                .split('|')
                .map(|cell| cell.trim().trim_matches('*'))
                .filter(|cell| !cell.is_empty())
                .map(|cell| cell.parse().unwrap())
                .collect();
            rows.insert(name, numbers.try_into().unwrap());
        }
    }
    rows
}

/// Checklist lines per Python file, and each section's stated count.
fn checklist(
    text: &str,
) -> (
    BTreeMap<String, BTreeMap<String, Line>>,
    BTreeMap<String, usize>,
) {
    let section = Regex::new(r"^## (test_\w+\.py) \((\d+)\)$").unwrap();
    let item = Regex::new(r"^- \[([ x])\] `(test\w+)`(.*)$").unwrap();
    let rust = Regex::new(r"`((?:src|tests)/[\w/]+\.rs)::(\w+)`").unwrap();
    let mut sections: BTreeMap<String, BTreeMap<String, Line>> = BTreeMap::new();
    let mut counts = BTreeMap::new();
    let mut current: Option<String> = None;
    for raw in text.lines() {
        if raw.starts_with("## ") {
            current = section.captures(raw).map(|found| {
                counts.insert(found[1].to_owned(), found[2].parse().unwrap());
                found[1].to_owned()
            });
            continue;
        }
        let (Some(file), Some(found)) = (&current, item.captures(raw)) else {
            continue;
        };
        let rest = found[3].to_owned();
        let status = STATUSES
            .into_iter()
            .find(|status| rest.contains(&format!("**{status}**")));
        let line = Line {
            checked: &found[1] == "x",
            status,
            rust: rust
                .captures_iter(&rest)
                .map(|r| format!("{}::{}", &r[1], &r[2]))
                .collect(),
            rest,
            raw: raw.to_owned(),
        };
        let previous = sections
            .entry(file.clone())
            .or_default()
            .insert(found[2].to_owned(), line);
        assert!(previous.is_none(), "{file}: two lines for {}", &found[2]);
    }
    (sections, counts)
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// `(python file, python test) → {rust path::fn}` from the `/// parity:` comments.
fn parity_comments() -> BTreeMap<(String, String), BTreeSet<String>> {
    let server = repo().join("server");
    let comment = Regex::new(r"^\s*///\s*parity:\s*(test_\w+\.py)::(\w+)").unwrap();
    let function = Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)").unwrap();
    let mut files = Vec::new();
    rust_files(&server.join("src"), &mut files);
    rust_files(&server.join("tests"), &mut files);
    let mut found: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for path in files {
        let relative = path
            .strip_prefix(&server)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).unwrap();
        let mut pending: Vec<(String, String)> = Vec::new();
        for line in text.lines() {
            if let Some(c) = comment.captures(line) {
                pending.push((c[1].to_owned(), c[2].to_owned()));
            } else if let Some(f) = function.captures(line) {
                for key in pending.drain(..) {
                    found
                        .entry(key)
                        .or_default()
                        .insert(format!("{relative}::{}", &f[1]));
                }
            }
        }
        assert!(
            pending.is_empty(),
            "{relative}: a parity comment is not followed by a function"
        );
    }
    found
}

fn rust_test_exists(reference: &str) -> bool {
    let (file, name) = reference.split_once("::").unwrap();
    let Ok(text) = fs::read_to_string(repo().join("server").join(file)) else {
        return false;
    };
    let function = Regex::new(&format!(r"(?m)^\s*(?:async\s+)?fn\s+{name}\s*\(")).unwrap();
    function.is_match(&text)
}

#[test]
fn the_parity_checklist_covers_every_python_test() {
    let python = python_tests();
    let text = checklist_text();
    let (lines, counts) = checklist(&text);
    let mut problems = Vec::new();

    for (file, tests) in &python {
        match counts.get(file) {
            None => problems.push(format!("{file}: no section `## {file} (N)`")),
            Some(&count) if count != tests.len() => problems.push(format!(
                "{file}: the section says {count} tests, the file has {}",
                tests.len()
            )),
            Some(_) => {}
        }
        for test in tests {
            match lines.get(file).and_then(|section| section.get(&test.name)) {
                None => problems.push(format!("{file}::{}: no checklist line", test.name)),
                Some(line) if test.parametrised && !line.rest.contains(" cases)") => {
                    problems.push(format!(
                        "{file}::{}: parametrised, give the case count: {}",
                        test.name, line.raw
                    ))
                }
                Some(_) => {}
            }
        }
    }

    for (file, section) in &lines {
        for (name, line) in section {
            let at = format!("{file}::{name}");
            if !line.checked {
                problems.push(format!("{at}: not checked off"));
            }
            match line.status {
                None => problems.push(format!(
                    "{at}: no status (one of {})",
                    STATUSES.map(|s| format!("**{s}**")).join(", ")
                )),
                Some("dropped") => {
                    let reason = line.rest.split("**dropped**").nth(1).unwrap_or("");
                    if reason.trim_start_matches([':', ' ', '.']).len() < 10 {
                        problems.push(format!("{at}: a dropped test needs its reason"));
                    }
                }
                Some(_) if line.rust.is_empty() => {
                    problems.push(format!("{at}: names no Rust test"));
                }
                Some(_) => {}
            }
        }
    }
    for reference in all_references(&text) {
        if !rust_test_exists(&reference) {
            problems.push(format!("`{reference}` does not exist"));
        }
    }

    let notes = fs::read_to_string(repo().join("docs/phase1/parity-notes.md")).unwrap();
    let noted = Regex::new(r"(?m)^- `(test_\w+\.py)::(\w+)`").unwrap();
    for found in noted.captures_iter(&notes) {
        if lines
            .get(&found[1])
            .is_none_or(|section| !section.contains_key(&found[2]))
        {
            problems.push(format!(
                "{}::{}: in parity-notes.md, but the checklist has no line",
                &found[1], &found[2]
            ));
        }
    }

    let table = summary(&text);
    let mut total = [0usize; 6];
    for (file, section) in &lines {
        let mut expected = [section.len(), 0, 0, 0, 0, 0];
        for line in section.values() {
            if let Some(index) = STATUSES.iter().position(|s| Some(*s) == line.status) {
                expected[2 + index] += 1;
            }
        }
        match table.get(file) {
            None => problems.push(format!("summary: no row for {file}")),
            Some(row) => {
                expected[1] = row[1];
                if *row != expected {
                    problems.push(format!(
                        "summary: {file} says {row:?}, the section has {expected:?} \
                         (tests, cases, ported, changed, replaced, dropped)"
                    ));
                }
                for (sum, value) in total.iter_mut().zip(row) {
                    *sum += value;
                }
            }
        }
    }
    if table.get("Total") != Some(&total) {
        problems.push(format!(
            "summary: the total row should be {total:?}, it is {:?}",
            table.get("Total")
        ));
    }

    for ((file, name), rust) in parity_comments() {
        let at = format!("{file}::{name}");
        let Some(line) = lines.get(&file).and_then(|section| section.get(&name)) else {
            problems.push(format!(
                "{at}: a parity comment names it, but the checklist has no line"
            ));
            continue;
        };
        for reference in rust {
            if !line.rust.contains(&reference) {
                problems.push(format!(
                    "{at}: the line does not name `{reference}`, which carries its parity comment"
                ));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "docs/phase1/parity-checklist.md is out of date:\n{}",
        problems.join("\n")
    );
}
