//! Keeps docs/phase1/parity-checklist.md (#49) from going stale. It fails when:
//!
//! - a Python test function in `backend/tests/test_*.py` has no checklist line, or a section's
//!   count is not the number of test functions in its file;
//! - a line is not checked off (`- [x]`), carries no status, or a `ported`, `changed` or
//!   `replaced` line names no Rust test, or a `dropped` line gives no reason;
//! - a `/// parity: <file>::<test>` comment in `server/src` or `server/tests` sits on a Rust test
//!   that the line of that Python test does not name;
//! - the file names a Rust test (`` `tests/x.rs::name` `` or `` `src/…/x.rs::name` ``) that
//!   does not exist, or a function that is not a test (no `#[test]`, `#[sqlx::test]` or
//!   `#[tokio::test]` attribute);
//! - a parametrised Python test's line does not give its case count `(N cases)`, or N is not
//!   the product of the lengths of its `parametrize` lists; a line of a test without
//!   `parametrize` gives a case count;
//! - a row of the summary table disagrees with its section: the tests, the cases (the sum of
//!   the lines' N, 1 for a line without one) and the four statuses; or the total row is not the
//!   sum of the rows, or the opening paragraph's file, function and case counts are not the
//!   table's;
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
    /// The product of the lengths of its `parametrize` lists; 1 without one.
    cases: usize,
}

struct Line {
    checked: bool,
    status: Option<&'static str>,
    rust: Vec<String>,
    /// N of a leading `(N cases)`.
    cases: Option<usize>,
    rest: String,
    raw: String,
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// A Python fragment without its comments and with each string literal replaced by `s`, so
/// brackets and commas are code. Enough for decorators, which hold no triple-quoted strings.
fn python_code(text: &str) -> String {
    let mut code = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '#' => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        code.push(n);
                        break;
                    }
                }
            }
            '"' | '\'' => {
                while let Some(n) = chars.next() {
                    if n == '\\' {
                        chars.next();
                    } else if n == c {
                        break;
                    }
                }
                code.push('s');
            }
            _ => code.push(c),
        }
    }
    code
}

/// How many brackets `code` leaves open.
fn open_brackets(code: &str) -> i64 {
    code.chars()
        .map(|c| match c {
            '(' | '[' | '{' => 1,
            ')' | ']' | '}' => -1,
            _ => 0,
        })
        .sum()
}

/// The comma-separated items directly inside the first bracket of `code`, each with the text
/// of its own level (nested brackets left out).
fn bracket_items(code: &str) -> Vec<(String, String)> {
    let mut depth = 0;
    let mut items = vec![(String::new(), String::new())];
    for c in code.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            ',' if depth == 1 => {
                items.push((String::new(), String::new()));
                continue;
            }
            _ => {}
        }
        if depth >= 1 {
            let (full, level) = items.last_mut().unwrap();
            full.push(c);
            if depth == 1 {
                level.push(c);
            }
        }
    }
    items.retain(|(full, _)| !full.trim().is_empty());
    items
}

/// The number of values of one `@pytest.mark.parametrize(names, values, …)` decorator.
fn parametrize_cases(at: &str, decorator: &str) -> usize {
    let code = python_code(decorator);
    let call = &code[code.find('(').unwrap_or(code.len())..];
    let arguments = bracket_items(call);
    let values = arguments.get(1).map(|(full, _)| full.trim_start());
    let Some(values) = values.filter(|values| values.starts_with(['[', '('])) else {
        panic!("{at}: parametrize values that are not a list literal need a checker update");
    };
    let items = bracket_items(values);
    assert!(
        !items.iter().any(|(_, level)| level.contains(" for ")),
        "{at}: a parametrize comprehension needs a checker update"
    );
    items.len()
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
        // The decorators above the next statement at column 0, each with all its lines.
        let mut decorators: Vec<String> = Vec::new();
        let mut open = 0;
        for line in text.lines() {
            assert!(
                !line.starts_with("class Test") && !nested.is_match(line),
                "{name}: test classes and nested test functions need a checker update: {line}"
            );
            if open > 0 {
                let decorator = decorators.last_mut().unwrap();
                decorator.push('\n');
                decorator.push_str(line);
                open = open_brackets(&python_code(decorator));
                continue;
            }
            if line.starts_with('@') {
                open = open_brackets(&python_code(line));
                decorators.push(line.to_owned());
                continue;
            }
            if let Some(found) = def.captures(line) {
                let at = format!("{name}::{}", &found[1]);
                let lists: Vec<usize> = decorators
                    .iter()
                    .filter(|d| d.starts_with("@pytest.mark.parametrize"))
                    .map(|d| parametrize_cases(&at, d))
                    .collect();
                assert!(
                    lists.len()
                        == decorators
                            .iter()
                            .filter(|d| d.contains("parametrize"))
                            .count(),
                    "{at}: a parametrize decorator not spelled `@pytest.mark.parametrize` needs \
                     a checker update"
                );
                tests.push(PyTest {
                    name: found[1].to_owned(),
                    parametrised: !lists.is_empty(),
                    cases: lists.iter().product(),
                });
            }
            // A statement at column 0 ends the decorators.
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
    let cases = Regex::new(r"^ \((\d+) cases\)").unwrap();
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
            cases: cases.captures(&rest).map(|n| n[1].parse().unwrap()),
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

/// Why `reference` is not a Rust test, or `None` when it is one: a function of that name in
/// that file whose attributes, above it past doc comments, include a test attribute.
fn not_a_rust_test(reference: &str) -> Option<&'static str> {
    let (file, name) = reference.split_once("::").unwrap();
    let Ok(text) = fs::read_to_string(repo().join("server").join(file)) else {
        return Some("does not exist");
    };
    let function = Regex::new(&format!(r"^\s*(?:async\s+)?fn\s+{name}\s*\(")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut found = false;
    for (at, line) in lines.iter().enumerate() {
        if !function.is_match(line) {
            continue;
        }
        found = true;
        let is_test = lines[..at]
            .iter()
            .rev()
            .map(|line| line.trim())
            .take_while(|line| line.is_empty() || line.starts_with("//") || line.starts_with("#["))
            .any(|line| {
                line == "#[test]"
                    || line.starts_with("#[sqlx::test")
                    || line.starts_with("#[tokio::test")
            });
        if is_test {
            return None;
        }
    }
    Some(if found {
        "is not a test"
    } else {
        "does not exist"
    })
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
                Some(line) => match (test.parametrised, line.cases) {
                    (true, None) => problems.push(format!(
                        "{file}::{}: parametrised, give the case count `(N cases)`: {}",
                        test.name, line.raw
                    )),
                    (true, Some(n)) if n != test.cases => problems.push(format!(
                        "{file}::{}: the line says {n} cases, its parametrize lists give {}",
                        test.name, test.cases
                    )),
                    (false, Some(n)) => problems.push(format!(
                        "{file}::{}: not parametrised, but the line says {n} cases",
                        test.name
                    )),
                    _ => {}
                },
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
        if let Some(why) = not_a_rust_test(&reference) {
            problems.push(format!("`{reference}` {why}"));
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
            expected[1] += line.cases.unwrap_or(1);
            if let Some(index) = STATUSES.iter().position(|s| Some(*s) == line.status) {
                expected[2 + index] += 1;
            }
        }
        match table.get(file) {
            None => problems.push(format!("summary: no row for {file}")),
            Some(row) => {
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
    let opening =
        Regex::new(r"(\d+) files, (\d+) functions, (\d+) cases\b.*\bAll (\d+) are\.").unwrap();
    let stated: Option<[usize; 4]> = opening
        .captures(&text)
        .map(|found| [1, 2, 3, 4].map(|group| found[group].parse().unwrap()));
    let files = table.len() - usize::from(table.contains_key("Total"));
    if stated != Some([files, total[0], total[1], total[0]]) {
        problems.push(format!(
            "the opening paragraph should say \"{files} files, {tests} functions, {cases} cases\" \
             and \"All {tests} are.\"",
            tests = total[0],
            cases = total[1]
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
