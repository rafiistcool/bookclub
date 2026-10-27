//! The OpenAPI document keeps the contract the frontend's generated types rely on (#47).
//!
//! `frontend/scripts/gen-api-types.mjs` turns the document into `frontend/src/api/generated.ts`,
//! and CI fails when the committed file is stale. These tests catch the server-side half: a shared
//! DTO of endpoint-map.md §0 that was renamed or reshaped without the design changing with it, a
//! response field the generated types would wrongly call always present, and a document whose
//! bytes change from run to run (which would make the CI check flap).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn document() -> Value {
    serde_json::to_value(bookclub_server::openapi()).unwrap()
}

fn schemas(doc: &Value) -> &serde_json::Map<String, Value> {
    doc["components"]["schemas"]
        .as_object()
        .expect("components.schemas")
}

/// The markdown section whose heading line starts with `heading` (e.g. `"## 0. "`), up to the next
/// heading of the same or a higher level outside a code block.
fn section(markdown: &str, heading: &str) -> String {
    let level = heading.chars().take_while(|&c| c == '#').count();
    let mut lines = markdown
        .lines()
        .skip_while(|line| !line.starts_with(heading));
    let first = lines
        .next()
        .unwrap_or_else(|| panic!("no section {heading:?}"));
    let mut out = vec![first];
    let mut in_code = false;
    for line in lines {
        if line.starts_with("```") {
            in_code = !in_code;
        }
        let hashes = line.chars().take_while(|&c| c == '#').count();
        if !in_code && hashes > 0 && hashes <= level && line[hashes..].starts_with(' ') {
            break;
        }
        out.push(line);
    }
    out.join("\n")
}

/// One field of a §0 object, e.g. `isbn13|null`, `source: "openlibrary"|"dnb"` or
/// `contributors: [{name, role}]`.
#[derive(Debug)]
struct Field {
    name: String,
    /// The fields of an inline `[{…}]` (an array of objects).
    items: Option<Vec<Field>>,
    /// The string literals of a `"a"|"b"` value. They are compared only when the document gives
    /// the field an enum (`"YYYY-MM-DD"` is a format, `"owner"|"admin"` a documented string).
    literals: Vec<String>,
}

/// One DTO of endpoint-map.md §0.
#[derive(Debug)]
enum Dto {
    Object { name: String, fields: Vec<Field> },
    Enum { name: String, values: Vec<String> },
}

impl Dto {
    fn name(&self) -> &str {
        match self {
            Dto::Object { name, .. } | Dto::Enum { name, .. } => name,
        }
    }
}

fn string_literals(text: &str) -> Vec<String> {
    regex::Regex::new(r#""([^"]*)""#)
        .unwrap()
        .captures_iter(text)
        .map(|captures| captures[1].to_owned())
        .collect()
}

/// Splits `text` at the commas outside brackets and quotes.
fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quoted, mut start) = (0i32, false, 0);
    for (at, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '{' | '[' | '(' if !quoted => depth += 1,
            '}' | ']' | ')' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                parts.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn parse_fields(body: &str) -> Vec<Field> {
    let name = regex::Regex::new(r"^([a-z_][a-z0-9_]*)").unwrap();
    let nested = regex::Regex::new(r"(?s)^:\s*\[\s*\{(.*)\}\s*\]").unwrap();
    split_top_level(body)
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let field = name
                .captures(part)
                .unwrap_or_else(|| panic!("§0: cannot read the field {part:?}"))[1]
                .to_owned();
            let rest = part[field.len()..].trim_start();
            let items = nested
                .captures(rest)
                .map(|captures| parse_fields(&captures[1]));
            let literals = if items.is_some() {
                Vec::new()
            } else {
                string_literals(rest)
            };
            Field {
                name: field,
                items,
                literals,
            }
        })
        .collect()
}

/// The DTOs of the `text` block in endpoint-map.md §0. Every line that starts in column 0 must
/// open a DTO (`Name {…}`, `Name (note) {…}` or `Name = "a"|"b"`), so a format the parser does not
/// know fails here instead of being skipped.
fn section_0_dtos() -> Vec<Dto> {
    let map = repo_file("docs/phase1/endpoint-map.md");
    let section_0 = section(&map, "## 0. ");
    let block = section_0
        .split("```text\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("§0 has a ```text block");
    let text: String = block
        .lines()
        .map(|line| line.split("//").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n");

    let head = regex::Regex::new(r"^([A-Z][A-Za-z0-9]*)\s*(?:\([^)]*\)\s*)?([{=])").unwrap();
    let mut dtos = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let line_end = text[at..].find('\n').map_or(text.len(), |n| at + n);
        let line = &text[at..line_end];
        if line.trim().is_empty() || line.starts_with(char::is_whitespace) {
            at = line_end + 1;
            continue;
        }
        let captures = head
            .captures(line)
            .unwrap_or_else(|| panic!("§0: cannot read the line {line:?}"));
        let name = captures[1].to_owned();
        let open = at + captures.get(2).unwrap().start();
        if &captures[2] == "=" {
            let values = string_literals(&text[open + 1..line_end]);
            assert!(!values.is_empty(), "§0: {name} lists no values");
            dtos.push(Dto::Enum { name, values });
            at = line_end + 1;
            continue;
        }
        let mut depth = 0;
        let close = text[open..]
            .char_indices()
            .find(|&(_, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })
            .map(|(n, _)| open + n)
            .unwrap_or_else(|| panic!("§0: {name} has no closing brace"));
        let fields = parse_fields(&text[open + 1..close]);
        assert!(!fields.is_empty(), "§0: {name} lists no fields");
        dtos.push(Dto::Object { name, fields });
        at = text[close..]
            .find('\n')
            .map_or(text.len(), |n| close + n + 1);
    }
    dtos
}

/// The schema behind `schema`: it follows `$ref`s and drops the `null` member of a nullable
/// `oneOf`/`anyOf`.
fn resolve<'a>(doc: &'a Value, schema: &'a Value) -> &'a Value {
    if let Some(reference) = schema["$ref"].as_str() {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("unexpected $ref {reference}"));
        return resolve(doc, &schemas(doc)[name]);
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(members) = schema[key].as_array() {
            let not_null: Vec<_> = members.iter().filter(|m| m["type"] != "null").collect();
            if let [only] = not_null[..] {
                return resolve(doc, only);
            }
        }
    }
    schema
}

fn string_set<'a>(values: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    values.into_iter().map(str::to_owned).collect()
}

/// Compares the fields endpoint-map.md lists for `what` with the properties of `schema`; returns
/// one line per difference.
fn compare_fields(doc: &Value, what: &str, fields: &[Field], schema: &Value) -> Vec<String> {
    let schema = resolve(doc, schema);
    let Some(properties) = schema["properties"].as_object() else {
        return vec![format!("{what}: the document's schema has no properties")];
    };
    let mut problems = Vec::new();
    let listed = string_set(fields.iter().map(|f| f.name.as_str()));
    let documented = string_set(properties.keys().map(String::as_str));
    for missing in listed.difference(&documented) {
        problems.push(format!("{what}.{missing} is in §0 but not in the document"));
    }
    for extra in documented.difference(&listed) {
        problems.push(format!("{what}.{extra} is in the document but not in §0"));
    }
    for field in fields {
        let Some(property) = properties.get(&field.name) else {
            continue;
        };
        let path = format!("{what}.{}", field.name);
        if let Some(items) = &field.items {
            let array = resolve(doc, property);
            problems.extend(compare_fields(doc, &path, items, &array["items"]));
        }
        if let Some(values) = resolve(doc, property)["enum"].as_array()
            && !field.literals.is_empty()
        {
            let listed = string_set(field.literals.iter().map(String::as_str));
            let documented = string_set(values.iter().filter_map(Value::as_str));
            if listed != documented {
                problems.push(format!(
                    "{path}: §0 lists {listed:?}, the document {documented:?}"
                ));
            }
        }
    }
    problems
}

/// Every DTO name endpoint-map.md §0 lists is a schema of the document, so a renamed DTO fails
/// here before the frontend's imports break.
#[test]
fn every_section_0_dto_is_in_the_document() {
    let doc = document();
    let dtos = section_0_dtos();
    let names: BTreeSet<&str> = dtos.iter().map(Dto::name).collect();
    // The parser read the whole block, not just a prefix of it.
    for expected in [
        "BookOut",
        "EditionOut",
        "UserOut",
        "ShelfStatus",
        "ReactionOut",
    ] {
        assert!(names.contains(expected), "§0 parse missed {expected}");
    }
    assert!(dtos.len() >= 11, "§0 parse found only {names:?}");

    let missing: Vec<_> = names
        .iter()
        .filter(|name| !schemas(&doc).contains_key(**name))
        .collect();
    assert!(
        missing.is_empty(),
        "§0 DTOs missing from components.schemas: {missing:?}"
    );
}

/// Every §0 object has exactly the fields §0 lists, including the inline `[{…}]` of
/// `EditionOut.contributors`, and every field §0 gives string literals for has those enum values.
#[test]
fn section_0_objects_match_the_document() {
    let doc = document();
    let mut problems = Vec::new();
    let mut objects = 0;
    for dto in section_0_dtos() {
        if let Dto::Object { name, fields } = dto {
            objects += 1;
            let schema = schemas(&doc)
                .get(&name)
                .unwrap_or_else(|| panic!("{name} is missing"));
            problems.extend(compare_fields(&doc, &name, &fields, schema));
        }
    }
    assert!(objects >= 9, "§0 parse found only {objects} objects");
    assert!(
        problems.is_empty(),
        "§0 and the document differ:\n{}",
        problems.join("\n")
    );
}

/// Every §0 enum (`ShelfStatus = "want_to_read"|…`) has exactly the values §0 lists.
#[test]
fn section_0_enums_match_the_document() {
    let doc = document();
    let enums: BTreeMap<String, Vec<String>> = section_0_dtos()
        .into_iter()
        .filter_map(|dto| match dto {
            Dto::Enum { name, values } => Some((name, values)),
            Dto::Object { .. } => None,
        })
        .collect();
    assert!(
        enums.contains_key("ShelfStatus") && enums.contains_key("Visibility"),
        "§0 parse found only the enums {enums:?}"
    );
    for (name, values) in enums {
        let documented = schemas(&doc)[&name]["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("{name} is not an enum in the document"));
        assert_eq!(
            string_set(values.iter().map(String::as_str)),
            string_set(documented.iter().filter_map(Value::as_str)),
            "{name}"
        );
    }
}

/// conventions.md §14 names the schemas `openapi::ApiDoc` registers, among them the two §0 only
/// describes inline (`ContributorOut`, `CatalogSource`) and `ErrorBody`; the frontend imports
/// them by these names.
#[test]
fn every_schema_conventions_14_registers_is_in_the_document() {
    let doc = document();
    let conventions = repo_file("docs/phase1/conventions.md");
    let list = regex::Regex::new(r"components\(schemas\(([^)]*)\)\)")
        .unwrap()
        .captures(&section(&conventions, "## 14. "))
        .expect("§14 lists `components(schemas(…))`")[1]
        .to_owned();
    let names: Vec<&str> = list.split(',').map(str::trim).collect();
    assert!(
        names.contains(&"ContributorOut") && names.contains(&"ErrorBody"),
        "§14 parse found only {names:?}"
    );
    let missing: Vec<_> = names
        .iter()
        .filter(|name| !schemas(&doc).contains_key(**name))
        .collect();
    assert!(
        missing.is_empty(),
        "§14 schemas missing from the document: {missing:?}"
    );
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

/// The generated types mark every response field required (`x: T | null`), because the server
/// sends each one, `null` when empty (endpoint-map.md). That holds only while no `ToSchema` type
/// lets serde skip a field, so this source scan forbids `skip_serializing` and
/// `skip_serializing_if` in them. Serde attributes sit inside their item, so each occurrence
/// belongs to the nearest `struct` or `enum` before it.
#[test]
fn schema_types_never_skip_a_field() {
    let derive = regex::Regex::new(r"#\[derive\(([^)]*)\)\]").unwrap();
    let item = regex::Regex::new(r"\b(?:struct|enum)\s+([A-Za-z0-9_]+)").unwrap();
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );

    let mut scanned = 0;
    let mut skipping = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let mut schema_types = BTreeSet::new();
        for found in derive.captures_iter(&text) {
            if found[1].contains("ToSchema") {
                let rest = &text[found.get(0).unwrap().end()..];
                let name = &item
                    .captures(rest)
                    .expect("a struct or enum after the derive")[1];
                schema_types.insert(name.to_owned());
            }
        }
        scanned += schema_types.len();
        for (at, _) in text.match_indices("skip_serializing") {
            let owner = item
                .captures_iter(&text[..at])
                .last()
                .map(|captures| captures[1].to_owned());
            if let Some(owner) = owner.filter(|owner| schema_types.contains(owner)) {
                skipping.push(format!("{owner} in {}", file.display()));
            }
        }
    }
    assert!(
        scanned >= 14,
        "the scan found only {scanned} ToSchema types"
    );
    assert!(
        skipping.is_empty(),
        "these ToSchema types skip a field when serialising, but the generated frontend types \
         (frontend/scripts/gen-api-types.mjs) treat every response field as present: {skipping:?}"
    );
}

/// `bookclub-server openapi` prints the same bytes on every run: no map with a random order feeds
/// the document. Otherwise the regenerated `generated.ts` could differ from the committed one
/// without any change, and the CI check would fail at random.
#[test]
fn the_cli_prints_the_same_bytes_every_time() {
    let run = || {
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
        output.stdout
    };
    let first = run();
    for _ in 0..3 {
        assert!(
            run() == first,
            "two runs of `bookclub-server openapi` differ"
        );
    }
    assert_eq!(
        String::from_utf8(first).unwrap().trim_end(),
        bookclub_server::openapi().to_pretty_json().unwrap(),
        "the CLI prints the document `openapi()` builds"
    );
}
