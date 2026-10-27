//! Diary entries and pick discussions in one table (#44b, owner POSTS): threads, reactions and
//! feeds (endpoint-map §8, docs/phase1/spec/diary.md §1.1–§1.6, docs/phase1/spec/club.md
//! §1.5–§1.11, conventions §6.3).
//!
//! A post is either a **diary entry** (`club_id` NULL: personal, `audience` or `only_me`) or a
//! **club post** (`club_id` and `pick_id` set: the pick discussion, for that club's members).
//! Replies are one level deep and inherit their root's work and scope (`posts_parent_fk`).
//!
//! - `out`: [`out::PostOut`] and the loaders every endpoint shares. Every read goes through
//!   `bc_visible_posts`; `author_rating` through `bc_visible_shelf`; reaction names through
//!   `bc_visible_owners`.
//! - `write`: the insert shared by both scopes (position snapshot, parent checks, the
//!   closed-pick guard, `note_posted`).
//! - `diary`: a work's thread, writing an entry or a reply, and the diary feed.
//! - `picks`: a club pick's thread and writing a club post.
//! - `edit`: `PATCH`, `DELETE` and the reaction toggle on `/api/posts/{post_id}`, each through
//!   `visibility::post_access` (404 for a missing, tombstoned or invisible post).

pub mod diary;
pub mod edit;
pub mod out;
pub mod picks;
pub mod write;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::AppState;
use crate::extract::Validator;
use crate::text::{char_len, py_strip};

/// Body rule: empty after the strip.
pub const WRITE_SOMETHING: &str = "Write a short take, quote, or meeting note";
/// Body rule: more than 1000 characters after the strip.
pub const BODY_TOO_LONG: &str = "Keep it to 1000 characters";
/// A reply whose parent is missing, invisible or on another work (or another club or pick).
pub const NOT_ON_THIS_BOOK: &str = "That entry is not on this book";
/// A reply to a reply.
pub const REPLY_TO_ENTRY: &str = "Reply to the entry itself, not to a reply";
/// A reply to a tombstone.
pub const ENTRY_DELETED: &str = "That entry was deleted";
pub const EDIT_OWN: &str = "You can only edit your own entries";
pub const DELETE_OWN: &str = "You can only delete your own entries";
pub const NOTHING_TO_UPDATE: &str = "Nothing to update";
pub const PICK_A_REACTION: &str = "Pick one of the club reactions";
/// `only_me` on anything but the root of a diary entry (new in Phase 1).
pub const CANNOT_BE_PRIVATE: &str = "Replies and club posts cannot be private";
pub const NO_PICK_YET: &str = "No club pick yet";
pub const NO_PICK_WITH_ID: &str = "No club pick with that id";
pub const PICK_CLOSED: &str = "That pick is closed";
pub const MILESTONE_NOT_ON_PICK: &str = "That milestone is not on this pick";

const MAX_BODY_CHARS: usize = 1000;

/// The six reactions, compared by exact code points after the strip (`post_reactions.emoji`
/// CHECK): red heart with VS16, thumbs up, tears of joy, open mouth, fire, books.
pub const REACTIONS: [&str; 6] = [
    "\u{2764}\u{fe0f}",
    "\u{1f44d}",
    "\u{1f602}",
    "\u{1f62e}",
    "\u{1f525}",
    "\u{1f4da}",
];

/// The post routes.
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(diary::work_thread, diary::write_entry))
        .routes(routes!(diary::feed))
        .routes(routes!(edit::update_post, edit::delete_post))
        .routes(routes!(edit::react))
        .routes(routes!(
            picks::current_pick_thread,
            picks::write_current_pick
        ))
        .routes(routes!(picks::pick_thread, picks::write_pick))
}

/// The stripped body, recording the legacy messages when it is empty or too long.
fn checked_body(raw: &str, v: &mut Validator) -> String {
    let body = py_strip(raw).to_owned();
    if body.is_empty() {
        v.fail(WRITE_SOMETHING);
    } else if char_len(&body) > MAX_BODY_CHARS {
        v.fail(BODY_TOO_LONG);
    }
    body
}

/// `spoiler_upto`: a percentage, 0 to 100.
fn check_spoiler(value: Option<i64>, v: &mut Validator) {
    v.range_i64(value, 0, 100);
}

/// A checked `spoiler_upto` as stored (the range check ran in `Validate`).
fn spoiler(value: Option<i64>) -> Option<i32> {
    value.and_then(|value| i32::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_are_stripped_and_counted_in_characters() {
        let mut v = Validator::new();
        assert_eq!(checked_body("  Loved it \u{1f}", &mut v), "Loved it");
        assert!(v.messages().is_empty());
        assert_eq!(
            checked_body(&"é".repeat(1000), &mut v).chars().count(),
            1000
        );
        assert!(v.messages().is_empty());
        checked_body(&"é".repeat(1001), &mut v);
        checked_body(" \t ", &mut v);
        assert_eq!(v.messages(), [BODY_TOO_LONG, WRITE_SOMETHING]);
    }

    #[test]
    fn the_reaction_set_matches_the_schema() {
        assert!(REACTIONS.contains(&"❤️"));
        assert!(!REACTIONS.contains(&"\u{2764}"));
        assert!(REACTIONS.contains(&"🔥"));
        assert_eq!(spoiler(Some(40)), Some(40));
        assert_eq!(spoiler(None), None);
    }
}
