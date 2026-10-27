//! The physical shelf layout: one per user, the shelf entries and decorations arranged on up
//! to 12 rows (endpoint-map §7, docs/phase1/spec/shelf.md §1.4, §1.5 and §6 risks 10–13).
//!
//! The PUT body is checked like the legacy pydantic model: row items are strict (a positive
//! integer is a shelf entry, a `decor:<id>` string a decoration; booleans, floats and numeric
//! strings are refused), colours are lowercased, and the custom messages are verbatim. The
//! stored JSON is the normalised layout. Reads prune entries the owner no longer has and fall
//! back to the default layout when a stored value fails the schema, never a 500.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use axum::Json;
use axum::extract::State;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;
use utoipa::ToSchema;

use super::{OwnerQuery, lock_user, resolve_owner};
use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::auth::rules::normalize_username;
use crate::error::{ApiResult, AppError, ErrorBody, msg};
use crate::extract::{ApiJson, ApiQuery, Validate, Validator, parse_lax_bool};

pub const TOO_MANY_ITEMS: &str = "At most 2100 items fit on the physical shelf";
pub const TOO_MANY_BOOKS: &str = "At most 2000 books fit on the physical shelf";
pub const ITEM_TWICE: &str = "Each item can only appear once on the physical shelf";
pub const SPINE_KEYS: &str = "Spine appearances must refer to shelf entry IDs";
pub const DECORATIONS_PLACED_ONCE: &str =
    "Each decoration must appear exactly once on the physical shelf";
pub const BOOKS_NOT_ON_SHELF: &str = "Physical shelf books must be on your shelf";

const MAX_ROWS: usize = 12;
const MAX_ITEMS: usize = 2100;
const MAX_BOOKS: usize = 2000;
const MAX_SPINES: usize = 2000;
const MAX_DECORATIONS: usize = 100;
const MIN_WIDTH: i64 = 26;
const MAX_WIDTH: i64 = 64;
const COLOR_PATTERN: &str = "String should match pattern '^#[0-9a-fA-F]{6}$'";
const DECORATION_PATTERN: &str = "String should match pattern '^decor:[a-zA-Z0-9_-]{1,64}$'";

/// One place on a row: a shelf entry id (a positive integer) or a decoration id
/// (`decor:` plus 1 to 64 of `A-Z a-z 0-9 _ -`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(untagged)]
pub enum PhysicalShelfItem {
    Entry(i64),
    Decoration(String),
}

impl<'de> Deserialize<'de> for PhysicalShelfItem {
    /// Strict, like pydantic's `strict=True`: only JSON integers and strings.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ItemVisitor;

        impl Visitor<'_> for ItemVisitor {
            type Value = PhysicalShelfItem;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a shelf entry id or a decoration id")
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(PhysicalShelfItem::Entry(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                i64::try_from(value)
                    .map(PhysicalShelfItem::Entry)
                    .map_err(|_| E::invalid_value(de::Unexpected::Unsigned(value), &self))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(PhysicalShelfItem::Decoration(value.to_owned()))
            }
        }

        deserializer.deserialize_any(ItemVisitor)
    }
}

/// The wood of the shelf.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShelfMaterial {
    #[default]
    Oak,
    Walnut,
    White,
    Black,
}

/// How a book stands: spine out, lying flat, or facing out with its cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpineDisplay {
    #[default]
    Spine,
    Flat,
    Cover,
}

/// A decoration's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecorationKind {
    Plant,
    Vase,
    Candle,
    Frame,
    Bookend,
    Globe,
}

/// How one book looks on the shelf. A missing colour or width follows the book (its cover
/// colour and page count).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PhysicalSpine {
    /// `#rrggbb`, stored lowercase.
    #[serde(default)]
    pub color: Option<String>,
    /// 26 to 64.
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub display: SpineDisplay,
}

/// A decoration on the shelf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PhysicalDecoration {
    pub kind: DecorationKind,
    /// `#rrggbb`, stored lowercase.
    pub color: String,
}

/// A user's physical shelf. Every field is optional on input; `{}` is the default layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PhysicalShelfLayout {
    /// 1 to 12 rows of shelf entry ids and decoration ids; at most 2100 items, 2000 of them
    /// books, each placed once.
    #[serde(default = "default_rows")]
    pub rows: Vec<Vec<PhysicalShelfItem>>,
    #[serde(default)]
    pub material: ShelfMaterial,
    #[serde(default, deserialize_with = "lax_bool")]
    pub lights: bool,
    /// Keyed by shelf entry id (as a string); at most 2000.
    #[serde(default)]
    pub spines: BTreeMap<String, PhysicalSpine>,
    /// Keyed by decoration id; exactly the decorations placed on the rows, at most 100.
    #[serde(default)]
    pub decorations: BTreeMap<String, PhysicalDecoration>,
}

fn default_rows() -> Vec<Vec<PhysicalShelfItem>> {
    vec![Vec::new(), Vec::new(), Vec::new()]
}

impl Default for PhysicalShelfLayout {
    fn default() -> Self {
        Self {
            rows: default_rows(),
            material: ShelfMaterial::default(),
            lights: false,
            spines: BTreeMap::new(),
            decorations: BTreeMap::new(),
        }
    }
}

/// pydantic's lax boolean: `true`/`false`, `0`/`1` and the strings `parse_lax_bool` accepts.
fn lax_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    struct LaxBool;

    impl Visitor<'_> for LaxBool {
        type Value = bool;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a boolean")
        }

        fn visit_bool<E: de::Error>(self, value: bool) -> Result<bool, E> {
            Ok(value)
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<bool, E> {
            match value {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(E::invalid_value(de::Unexpected::Signed(value), &self)),
            }
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<bool, E> {
            match value {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(E::invalid_value(de::Unexpected::Unsigned(value), &self)),
            }
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<bool, E> {
            parse_lax_bool(value).ok_or_else(|| E::invalid_value(de::Unexpected::Str(value), &self))
        }
    }

    deserializer.deserialize_any(LaxBool)
}

impl Validate for PhysicalShelfLayout {
    /// The legacy field validators in declaration order, then the model validator (only when
    /// the fields passed). Colours are lowercased in place.
    fn validate(&mut self, v: &mut Validator) {
        let before = v.messages().len();
        self.validate_rows(v);
        self.validate_spines(v);
        self.validate_decorations(v);
        if v.messages().len() == before {
            let placed: HashSet<&str> = self.decoration_ids().collect();
            let declared: HashSet<&str> = self.decorations.keys().map(String::as_str).collect();
            v.check(placed == declared, DECORATIONS_PLACED_ONCE);
        }
    }
}

impl PhysicalShelfLayout {
    fn validate_rows(&self, v: &mut Validator) {
        if self.rows.is_empty() {
            v.fail("List should have at least 1 item after validation, not 0");
            return;
        }
        if self.rows.len() > MAX_ROWS {
            v.fail(format!(
                "List should have at most {MAX_ROWS} items after validation, not {}",
                self.rows.len()
            ));
            return;
        }
        let before = v.messages().len();
        for item in self.rows.iter().flatten() {
            match item {
                PhysicalShelfItem::Entry(id) => v.check(*id > 0, "Input should be greater than 0"),
                PhysicalShelfItem::Decoration(id) => {
                    v.check(is_decoration_id(id), DECORATION_PATTERN)
                }
            }
        }
        if v.messages().len() != before {
            return;
        }
        let items: Vec<&PhysicalShelfItem> = self.rows.iter().flatten().collect();
        let books = items
            .iter()
            .filter(|item| matches!(item, PhysicalShelfItem::Entry(_)))
            .count();
        let distinct: HashSet<&&PhysicalShelfItem> = items.iter().collect();
        if items.len() > MAX_ITEMS {
            v.fail(TOO_MANY_ITEMS);
        } else if books > MAX_BOOKS {
            v.fail(TOO_MANY_BOOKS);
        } else if distinct.len() != items.len() {
            v.fail(ITEM_TWICE);
        }
    }

    fn validate_spines(&mut self, v: &mut Validator) {
        if self.spines.len() > MAX_SPINES {
            v.fail(format!(
                "Dictionary should have at most {MAX_SPINES} items after validation, not {}",
                self.spines.len()
            ));
            return;
        }
        let before = v.messages().len();
        for spine in self.spines.values_mut() {
            if let Some(color) = &mut spine.color {
                normalize_color(color, v);
            }
            v.range_i64(spine.width, MIN_WIDTH, MAX_WIDTH);
        }
        if v.messages().len() == before && !self.spines.keys().all(|key| is_spine_key(key)) {
            v.fail(SPINE_KEYS);
        }
    }

    fn validate_decorations(&mut self, v: &mut Validator) {
        if self.decorations.len() > MAX_DECORATIONS {
            v.fail(format!(
                "Dictionary should have at most {MAX_DECORATIONS} items after validation, not {}",
                self.decorations.len()
            ));
            return;
        }
        for (id, decoration) in &mut self.decorations {
            v.check(is_decoration_id(id), DECORATION_PATTERN);
            normalize_color(&mut decoration.color, v);
        }
    }

    /// The decoration ids placed on the rows.
    fn decoration_ids(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().flatten().filter_map(|item| match item {
            PhysicalShelfItem::Decoration(id) => Some(id.as_str()),
            PhysicalShelfItem::Entry(_) => None,
        })
    }

    /// Every shelf entry the layout refers to: the entries on the rows and the spine keys. A
    /// spine key too large for an id gives `None` (it can be no one's entry).
    fn referenced_entries(&self) -> Vec<Option<i64>> {
        let placed = self.rows.iter().flatten().filter_map(|item| match item {
            PhysicalShelfItem::Entry(id) => Some(Some(*id)),
            PhysicalShelfItem::Decoration(_) => None,
        });
        let spines = self.spines.keys().map(|key| key.parse::<i64>().ok());
        placed.chain(spines).collect()
    }

    /// Keeps only the owner's current entries on the rows and in the spines (row boundaries
    /// and decorations stay).
    fn prune(&mut self, owned: &HashSet<i64>) {
        for row in &mut self.rows {
            row.retain(|item| match item {
                PhysicalShelfItem::Entry(id) => owned.contains(id),
                PhysicalShelfItem::Decoration(_) => true,
            });
        }
        self.spines
            .retain(|key, _| key.parse::<i64>().is_ok_and(|id| owned.contains(&id)));
    }
}

/// Checks `#rrggbb` and lowercases it.
fn normalize_color(color: &mut str, v: &mut Validator) {
    let ok = color.len() == 7
        && color.starts_with('#')
        && color[1..].bytes().all(|b| b.is_ascii_hexdigit());
    v.check(ok, COLOR_PATTERN);
    color.make_ascii_lowercase();
}

/// `^decor:[a-zA-Z0-9_-]{1,64}$`.
fn is_decoration_id(id: &str) -> bool {
    id.strip_prefix("decor:").is_some_and(|suffix| {
        (1..=64).contains(&suffix.len())
            && suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}

/// A spine key: `[1-9][0-9]{0,18}` (fully).
fn is_spine_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    (1..=19).contains(&bytes.len())
        && (b'1'..=b'9').contains(&bytes[0])
        && bytes.iter().all(u8::is_ascii_digit)
}

/// A stored layout as the current schema reads it: the default when it does not parse or no
/// longer validates (it never makes a read fail).
fn stored_layout(value: Value) -> PhysicalShelfLayout {
    let Ok(mut layout) = serde_json::from_value::<PhysicalShelfLayout>(value) else {
        return PhysicalShelfLayout::default();
    };
    let mut v = Validator::new();
    layout.validate(&mut v);
    if v.messages().is_empty() {
        layout
    } else {
        PhysicalShelfLayout::default()
    }
}

/// A user's physical shelf (`?username=` for someone in the caller's audience). Entries the
/// owner no longer has are left out; the stored value is not rewritten.
#[utoipa::path(
    get,
    path = "/api/shelf/physical-layout",
    operation_id = "shelf_get_layout",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(OwnerQuery),
    responses(
        (status = 200, body = PhysicalShelfLayout, description = "The layout (the default when none is saved)"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No member with that name` (also for users outside the caller's audience)")
    )
)]
pub async fn get_layout(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<OwnerQuery>,
) -> ApiResult<Json<PhysicalShelfLayout>> {
    let mut conn = state.db.acquire().await?;
    let owner = resolve_owner(&mut conn, &user, &query).await?;
    let stored = sqlx::query_scalar!(
        "SELECT layout FROM physical_shelf_layouts WHERE user_id = $1",
        owner.id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(stored) = stored else {
        return Ok(Json(PhysicalShelfLayout::default()));
    };
    let mut layout = stored_layout(stored);
    let owned = owned_entries(&mut conn, owner.id).await?;
    layout.prune(&owned);
    Ok(Json(layout))
}

/// Replaces the caller's layout and echoes it normalised. Only the owner can save a layout:
/// `?username=` naming anyone else is 404 `No member with that name`. Every entry on the rows
/// and in the spines must be the caller's. Reading status and order are never touched.
#[utoipa::path(
    put,
    path = "/api/shelf/physical-layout",
    operation_id = "shelf_save_layout",
    tag = "shelf",
    security(("bookclub_session" = []), ("bearer" = [])),
    params(OwnerQuery),
    request_body = PhysicalShelfLayout,
    responses(
        (status = 200, body = PhysicalShelfLayout, description = "The saved layout"),
        (status = 400, body = ErrorBody, description = "Validation (`At most 2100 items fit on the physical shelf`, `At most 2000 books fit on the physical shelf`, `Each item can only appear once on the physical shelf`, `Spine appearances must refer to shelf entry IDs`, `Each decoration must appear exactly once on the physical shelf`), `Physical shelf books must be on your shelf`"),
        (status = 401, body = ErrorBody, description = "`Not signed in`"),
        (status = 404, body = ErrorBody, description = "`No member with that name`: `?username=` names someone else")
    )
)]
pub async fn save_layout(
    user: CurrentUser,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<OwnerQuery>,
    ApiJson(layout): ApiJson<PhysicalShelfLayout>,
) -> ApiResult<Json<PhysicalShelfLayout>> {
    if let Some(name) = query.name()
        && normalize_username(name) != user.username
    {
        return Err(AppError::not_found(msg::NO_MEMBER));
    }
    let mut tx = state.db.begin().await?;
    // The lock keeps a concurrent DELETE from removing a referenced entry after the check.
    lock_user(&mut tx, user.id).await?;
    let owned = owned_entries(&mut tx, user.id).await?;
    let all_owned = layout
        .referenced_entries()
        .into_iter()
        .all(|id| id.is_some_and(|id| owned.contains(&id)));
    if !all_owned {
        return Err(AppError::bad_request(BOOKS_NOT_ON_SHELF));
    }
    let stored = serde_json::to_value(&layout).map_err(AppError::internal)?;
    sqlx::query!(
        "INSERT INTO physical_shelf_layouts (user_id, layout) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET layout = EXCLUDED.layout, updated_at = now()",
        user.id,
        stored
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(layout))
}

/// The user's shelf entry ids.
async fn owned_entries(conn: &mut PgConnection, user_id: i64) -> sqlx::Result<HashSet<i64>> {
    let ids = sqlx::query_scalar!("SELECT id FROM shelf_entries WHERE user_id = $1", user_id)
        .fetch_all(conn)
        .await?;
    Ok(ids.into_iter().collect())
}

/// Removes a deleted entry from the user's stored layout (its row places and its spine), in the
/// caller's transaction. Works on the raw JSON, so a layout the schema no longer accepts is
/// cleaned too. Callers hold the user lock.
pub async fn remove_from_layout(
    conn: &mut PgConnection,
    user_id: i64,
    entry_id: i64,
) -> sqlx::Result<()> {
    let stored = sqlx::query_scalar!(
        "SELECT layout FROM physical_shelf_layouts WHERE user_id = $1",
        user_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(mut layout) = stored else {
        return Ok(());
    };
    if strip_entry(&mut layout, entry_id) {
        sqlx::query!(
            "UPDATE physical_shelf_layouts SET layout = $2, updated_at = now() WHERE user_id = $1",
            user_id,
            layout
        )
        .execute(conn)
        .await?;
    }
    Ok(())
}

/// Removes `entry_id` from every row and drops its spine; true when anything changed.
fn strip_entry(layout: &mut Value, entry_id: i64) -> bool {
    let mut changed = false;
    if let Some(rows) = layout.get_mut("rows").and_then(Value::as_array_mut) {
        for row in rows.iter_mut().filter_map(Value::as_array_mut) {
            let before = row.len();
            row.retain(|item| item.as_i64() != Some(entry_id));
            changed |= row.len() != before;
        }
    }
    if let Some(spines) = layout.get_mut("spines").and_then(Value::as_object_mut) {
        changed |= spines.remove(&entry_id.to_string()).is_some();
    }
    changed
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(value: Value) -> Result<PhysicalShelfLayout, String> {
        let mut layout: PhysicalShelfLayout =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        let mut v = Validator::new();
        layout.validate(&mut v);
        match v.into_result() {
            Ok(()) => Ok(layout),
            Err(error) => Err(error.detail.into_owned()),
        }
    }

    fn decorations(n: usize) -> (Vec<Value>, serde_json::Map<String, Value>) {
        let ids: Vec<Value> = (0..n).map(|i| json!(format!("decor:{i}"))).collect();
        let map = (0..n)
            .map(|i| {
                (
                    format!("decor:{i}"),
                    json!({"kind": "plant", "color": "#123456"}),
                )
            })
            .collect();
        (ids, map)
    }

    /// parity: test_physical_shelf.py::test_mixed_layout_limits_books_and_decorations_separately
    #[test]
    fn books_and_decorations_are_limited_separately() {
        let books: Vec<i64> = (1..=2000).collect();
        let (ids, map) = decorations(100);
        let layout = parse(json!({"rows": [books, ids], "decorations": map})).unwrap();
        assert_eq!(layout.rows[0].len(), 2000);
        assert_eq!(layout.decorations.len(), 100);

        let too_many: Vec<i64> = (1..=2001).collect();
        assert_eq!(
            parse(json!({"rows": [too_many]})).unwrap_err(),
            TOO_MANY_BOOKS
        );
        // 2001 books and 100 decorations: the item limit is checked first.
        assert_eq!(
            parse(json!({"rows": [too_many, ids], "decorations": map})).unwrap_err(),
            TOO_MANY_ITEMS
        );
    }

    #[test]
    fn defaults_fill_and_colours_are_lowercased() {
        let layout = parse(json!({
            "spines": {},
            "decorations": {"decor:a": {"kind": "globe", "color": "#ABCDEF"}},
            "rows": [["decor:a"]],
            "unknown": true
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&layout).unwrap(),
            json!({
                "rows": [["decor:a"]],
                "material": "oak",
                "lights": false,
                "spines": {},
                "decorations": {"decor:a": {"kind": "globe", "color": "#abcdef"}}
            })
        );
        assert_eq!(parse(json!({})).unwrap(), PhysicalShelfLayout::default());
    }

    #[test]
    fn lights_accepts_lax_booleans() {
        for (value, expected) in [
            (json!(true), true),
            (json!(0), false),
            (json!(1), true),
            (json!("yes"), true),
            (json!("False"), false),
        ] {
            assert_eq!(parse(json!({"lights": value})).unwrap().lights, expected);
        }
        for value in [json!("sometimes"), json!(2), json!(null), json!(1.5)] {
            assert!(parse(json!({"lights": value})).is_err(), "{value}");
        }
    }

    #[test]
    fn row_items_are_strict() {
        for item in [
            json!(true),
            json!("1"),
            json!(1.5),
            json!(1.0),
            json!(0),
            json!(-3),
        ] {
            assert!(parse(json!({"rows": [[item]]})).is_err(), "{item}");
        }
        assert!(parse(json!({"rows": [[u64::MAX]]})).is_err());
        assert!(parse(json!({"rows": [[1, 2], [3]]})).is_ok());
    }

    #[test]
    fn spine_keys_and_decoration_ids_follow_their_patterns() {
        assert!(is_spine_key("1"));
        assert!(is_spine_key("9223372036854775807"));
        assert!(is_spine_key("9999999999999999999"));
        for key in ["", "0", "01", "foo", "1a", "12345678901234567890", "١"] {
            assert!(!is_spine_key(key), "{key}");
        }
        assert!(is_decoration_id("decor:Aa_0-b"));
        assert!(is_decoration_id(&format!("decor:{}", "a".repeat(64))));
        for id in [
            "decor:",
            "plant",
            "decor:space name",
            "decor:a\n",
            "decor:é",
            &format!("decor:{}", "a".repeat(65)),
        ] {
            assert!(!is_decoration_id(id), "{id}");
        }
    }

    #[test]
    fn a_huge_spine_key_is_no_ones_entry() {
        let layout = parse(json!({"spines": {"9999999999999999999": {}}})).unwrap();
        assert_eq!(layout.referenced_entries(), vec![None]);
    }

    #[test]
    fn stored_layouts_that_fail_the_schema_read_as_the_default() {
        assert_eq!(
            stored_layout(json!({"rows": "nonsense"})),
            PhysicalShelfLayout::default()
        );
        assert_eq!(
            stored_layout(json!({"rows": [["decor:orphan"]]})),
            PhysicalShelfLayout::default()
        );
        // A legacy layout without decorations, lights or display still loads.
        let legacy = stored_layout(json!({
            "rows": [[], [5, 9999], []],
            "material": "walnut",
            "spines": {"5": {"color": "#ABCDEF", "width": 44}}
        }));
        assert_eq!(legacy.material, ShelfMaterial::Walnut);
        assert_eq!(legacy.spines["5"].display, SpineDisplay::Spine);
        assert_eq!(legacy.spines["5"].color.as_deref(), Some("#abcdef"));
    }

    #[test]
    fn pruning_keeps_rows_and_decorations() {
        let mut layout = parse(json!({
            "rows": [["decor:a", 5, 7], [8], []],
            "spines": {"5": {}, "7": {}},
            "decorations": {"decor:a": {"kind": "vase", "color": "#000000"}}
        }))
        .unwrap();
        layout.prune(&HashSet::from([5]));
        assert_eq!(
            serde_json::to_value(&layout).unwrap()["rows"],
            json!([["decor:a", 5], [], []])
        );
        assert_eq!(layout.spines.keys().collect::<Vec<_>>(), ["5"]);
        assert_eq!(layout.decorations.len(), 1);
    }

    #[test]
    fn stripping_an_entry_edits_the_raw_json() {
        let mut layout = json!({
            "rows": [[5, "decor:a"], [], [7, 5]],
            "spines": {"5": {"width": 30}, "7": {}},
            "extra": "kept"
        });
        assert!(strip_entry(&mut layout, 5));
        assert_eq!(
            layout,
            json!({"rows": [["decor:a"], [], [7]], "spines": {"7": {}}, "extra": "kept"})
        );
        assert!(!strip_entry(&mut layout, 5));
        assert!(!strip_entry(&mut json!("not a layout"), 5));
    }
}
