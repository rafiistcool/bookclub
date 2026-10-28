//! Legacy `physical_shelf_layouts.layout_json` → `physical_shelf_layouts.layout`
//! (legacy-mapping.md §5).
//!
//! The stored JSON is read the way the layout endpoint reads a stored layout: with the current
//! `PhysicalShelfLayout` schema (docs/phase1/spec/shelf.md §1.5), missing fields filled with their
//! defaults (`lights` false, `decorations` {}, spine `display` "spine", `color`/`width` null) and
//! colours lowercased. Before that, shelf entries merged away are re-mapped to their winner
//! (`merge_rules::remap_layout`); after it, entry ids the user does not own are pruned from the
//! rows and spines. A layout that is not JSON, or that the schema refuses, becomes the default.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::catalog::shared::merge_rules::remap_layout;

static DECORATION_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^decor:[a-zA-Z0-9_-]{1,64}$").unwrap());
static SPINE_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[1-9][0-9]{0,18}$").unwrap());
static COLOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#[0-9a-fA-F]{6}$").unwrap());

const MATERIALS: [&str; 4] = ["oak", "walnut", "white", "black"];
const DISPLAYS: [&str; 3] = ["spine", "flat", "cover"];
const DECORATION_KINDS: [&str; 6] = ["plant", "vase", "candle", "frame", "bookend", "globe"];

/// The layout a user without a stored one sees.
pub(crate) fn default_layout() -> Value {
    json!({"rows": [[], [], []], "material": "oak", "lights": false, "spines": {},
           "decorations": {}})
}

/// A legacy layout ready to store: normalised, re-mapped and pruned.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    pub layout: Value,
    /// Entry ids removed from the rows and spine keys because the user does not own them.
    pub pruned: usize,
}

/// Normalises one legacy layout. `merges` lists the user's (loser, winner) shelf entry pairs,
/// `owned` the user's imported entry ids. `Err` names why the layout does not fit the current
/// schema (the caller stores the default instead).
pub(crate) fn normalise_layout(
    raw: &str,
    merges: &[(i64, i64)],
    owned: &HashSet<i64>,
) -> Result<Layout, String> {
    let mut value: Value = serde_json::from_str(raw).map_err(|e| format!("not JSON ({e})"))?;
    if !value.is_object() {
        return Err("not a JSON object".into());
    }
    for (loser, winner) in merges {
        remap_layout(&mut value, *loser, *winner);
    }
    let Value::Object(obj) = value else {
        unreachable!("checked above");
    };
    let mut pruned = 0;

    let rows = match obj.get("rows") {
        None => vec![Vec::new(), Vec::new(), Vec::new()],
        Some(rows) => read_rows(rows)?,
    };
    let material = match obj.get("material") {
        None => "oak".to_owned(),
        Some(Value::String(m)) if MATERIALS.contains(&m.as_str()) => m.clone(),
        Some(other) => return Err(format!("material {other} is not one of {MATERIALS:?}")),
    };
    let lights = match obj.get("lights") {
        None => false,
        Some(v) => lax_bool(v).ok_or_else(|| format!("lights {v} is not a boolean"))?,
    };
    let spines = match obj.get("spines") {
        None => Map::new(),
        Some(spines) => read_spines(spines)?,
    };
    let decorations = match obj.get("decorations") {
        None => Map::new(),
        Some(decorations) => read_decorations(decorations)?,
    };

    // The `rows` field validator and the model validator.
    let items: Vec<&Value> = rows.iter().flatten().collect();
    if items.len() > 2100 {
        return Err("At most 2100 items fit on the physical shelf".into());
    }
    if items.iter().filter(|item| item.is_i64()).count() > 2000 {
        return Err("At most 2000 books fit on the physical shelf".into());
    }
    let mut seen = HashSet::new();
    if !items.iter().all(|item| seen.insert(item.to_string())) {
        return Err("Each item can only appear once on the physical shelf".into());
    }
    let placed: HashSet<&str> = items.iter().filter_map(|item| item.as_str()).collect();
    let declared: HashSet<&str> = decorations.keys().map(String::as_str).collect();
    if placed != declared {
        return Err("Each decoration must appear exactly once on the physical shelf".into());
    }

    // Read-time pruning: entries the user does not own leave the rows and spines.
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let kept: Vec<Value> = row
                .into_iter()
                .filter(|item| match item.as_i64() {
                    Some(id) if !owned.contains(&id) => {
                        pruned += 1;
                        false
                    }
                    _ => true,
                })
                .collect();
            Value::Array(kept)
        })
        .collect();
    let spines: Map<String, Value> = spines
        .into_iter()
        .filter(|(key, _)| {
            let keep = key.parse::<i64>().is_ok_and(|id| owned.contains(&id));
            if !keep {
                pruned += 1;
            }
            keep
        })
        .collect();

    Ok(Layout {
        layout: json!({
            "rows": rows,
            "material": material,
            "lights": lights,
            "spines": spines,
            "decorations": decorations,
        }),
        pruned,
    })
}

/// 1 to 12 rows of strict positive integers (entry ids) and decoration ids.
fn read_rows(rows: &Value) -> Result<Vec<Vec<Value>>, String> {
    let rows = rows.as_array().ok_or("rows is not a list")?;
    if rows.is_empty() || rows.len() > 12 {
        return Err(format!("{} rows (1 to 12 fit)", rows.len()));
    }
    rows.iter()
        .map(|row| {
            let row = row.as_array().ok_or("a row is not a list")?;
            row.iter()
                .map(|item| match item {
                    Value::Number(n) if n.as_i64().is_some_and(|id| id > 0) => Ok(item.clone()),
                    Value::String(id) if DECORATION_ID.is_match(id) => Ok(item.clone()),
                    other => Err(format!(
                        "{other} is neither a shelf entry id nor a decoration"
                    )),
                })
                .collect()
        })
        .collect()
}

/// Spine appearances with every key filled: `color` (lowercased) and `width` may be null,
/// `display` defaults to "spine".
fn read_spines(spines: &Value) -> Result<Map<String, Value>, String> {
    let spines = spines.as_object().ok_or("spines is not an object")?;
    if spines.len() > 2000 {
        return Err(format!("{} spines (at most 2000)", spines.len()));
    }
    let mut out = Map::new();
    for (key, spine) in spines {
        if !SPINE_KEY.is_match(key) {
            return Err("Spine appearances must refer to shelf entry IDs".into());
        }
        let spine = spine
            .as_object()
            .ok_or_else(|| format!("spine {key} is not an object"))?;
        let color = match spine.get("color") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(c)) if COLOR.is_match(c) => Value::String(c.to_lowercase()),
            Some(other) => return Err(format!("spine {key}: colour {other} is not #rrggbb")),
        };
        let width = match spine.get("width") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::Number(n)) if n.as_i64().is_some_and(|w| (26..=64).contains(&w)) => {
                Value::Number(n.clone())
            }
            Some(other) => return Err(format!("spine {key}: width {other} is not 26..64")),
        };
        let display = match spine.get("display") {
            None => "spine".to_owned(),
            Some(Value::String(d)) if DISPLAYS.contains(&d.as_str()) => d.clone(),
            Some(other) => return Err(format!("spine {key}: display {other} is unknown")),
        };
        out.insert(
            key.clone(),
            json!({"color": color, "width": width, "display": display}),
        );
    }
    Ok(out)
}

/// Decorations: a known kind and a `#rrggbb` colour (lowercased), each.
fn read_decorations(decorations: &Value) -> Result<Map<String, Value>, String> {
    let decorations = decorations
        .as_object()
        .ok_or("decorations is not an object")?;
    if decorations.len() > 100 {
        return Err(format!("{} decorations (at most 100)", decorations.len()));
    }
    let mut out = Map::new();
    for (key, decoration) in decorations {
        if !DECORATION_ID.is_match(key) {
            return Err(format!("{key:?} is not a decoration id"));
        }
        let kind = decoration.get("kind").and_then(Value::as_str);
        let color = decoration.get("color").and_then(Value::as_str);
        match (kind, color) {
            (Some(kind), Some(color))
                if DECORATION_KINDS.contains(&kind) && COLOR.is_match(color) =>
            {
                out.insert(
                    key.clone(),
                    json!({"kind": kind, "color": color.to_lowercase()}),
                );
            }
            _ => {
                return Err(format!(
                    "decoration {key} needs a known kind and a #rrggbb colour"
                ));
            }
        }
    }
    Ok(out)
}

/// Pydantic's lax boolean: true/false, 0/1, and the strings it accepts.
fn lax_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match n.as_f64() {
            Some(0.0) => Some(false),
            Some(1.0) => Some(true),
            _ => None,
        },
        Value::String(s) => match s.to_lowercase().as_str() {
            "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
            "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(ids: &[i64]) -> HashSet<i64> {
        ids.iter().copied().collect()
    }

    #[test]
    fn a_legacy_layout_gets_its_defaults_and_loses_foreign_ids() {
        let raw = r##"{"rows": [[1, 2, "decor:plant1"], [19, 20, 99999], []],
                       "material": "walnut",
                       "spines": {"19": {"color": "#A1B2C3", "width": 30},
                                  "20": {"color": "#00ff00", "display": "cover"},
                                  "99999": {"color": "#000000"}},
                       "decorations": {"decor:plant1": {"kind": "plant", "color": "#2E7D32"}},
                       "unknown": 1}"##;
        let out = normalise_layout(raw, &[(20, 19)], &owned(&[1, 2, 19])).unwrap();
        assert_eq!(
            out.layout,
            json!({
                "rows": [[1, 2, "decor:plant1"], [19], []],
                "material": "walnut",
                "lights": false,
                "spines": {"19": {"color": "#a1b2c3", "width": 30, "display": "spine"}},
                "decorations": {"decor:plant1": {"kind": "plant", "color": "#2e7d32"}},
            })
        );
        // 99999 in the rows and in the spines; 20 merged into the placed 19.
        assert_eq!(out.pruned, 2);
    }

    #[test]
    fn an_unplaced_winner_takes_the_loser_place() {
        let out = normalise_layout(
            r#"{"rows": [[7]], "spines": {"7": {"width": 40}}}"#,
            &[(7, 9)],
            &owned(&[9]),
        )
        .unwrap();
        assert_eq!(out.layout["rows"], json!([[9]]));
        assert_eq!(
            out.layout["spines"],
            json!({"9": {"color": null, "width": 40, "display": "spine"}})
        );
        assert_eq!(out.pruned, 0);
    }

    #[test]
    fn an_empty_object_is_the_default() {
        let out = normalise_layout("{}", &[], &owned(&[])).unwrap();
        assert_eq!(out.layout, default_layout());
    }

    #[test]
    fn lax_booleans_are_read() {
        let out = normalise_layout(r#"{"lights": "yes"}"#, &[], &owned(&[])).unwrap();
        assert_eq!(out.layout["lights"], json!(true));
        let out = normalise_layout(r#"{"lights": 0}"#, &[], &owned(&[])).unwrap();
        assert_eq!(out.layout["lights"], json!(false));
    }

    #[test]
    fn layouts_the_schema_refuses_are_errors() {
        for raw in [
            "{not json",
            "[1, 2]",
            r#"{"material": "pine"}"#,
            r#"{"lights": "sometimes"}"#,
            r#"{"rows": []}"#,
            r#"{"rows": [[1.5]]}"#,
            r#"{"rows": [["1"]]}"#,
            r#"{"rows": [[true]]}"#,
            r#"{"rows": [[1, 1]]}"#,
            r#"{"rows": [["decor:x"]]}"#,
            r##"{"rows": [[]], "decorations": {"decor:x": {"kind": "plant", "color": "#000000"}}}"##,
            r##"{"rows": [["decor:x"]], "decorations": {"decor:x": {"kind": "tree", "color": "#000000"}}}"##,
            r#"{"decorations": []}"#,
            r#"{"spines": {"01": {}}}"#,
            r#"{"spines": {"1": {"width": 3}}}"#,
            r#"{"spines": {"1": {"width": 30.0}}}"#,
            r#"{"spines": {"1": {"color": "red"}}}"#,
            r#"{"spines": {"1": {"display": "upside-down"}}}"#,
            r#"{"rows": null}"#,
        ] {
            assert!(
                normalise_layout(raw, &[], &owned(&[1])).is_err(),
                "{raw} was accepted"
            );
        }
        let thirteen = format!(r#"{{"rows": {}}}"#, json!(vec![Vec::<i64>::new(); 13]));
        assert!(normalise_layout(&thirteen, &[], &owned(&[])).is_err());
    }
}
