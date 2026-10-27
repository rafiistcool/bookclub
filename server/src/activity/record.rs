//! Writing activity events (docs/phase1/conventions.md §15). Frozen; the body comes in Wave 1
//! step 2.

use serde_json::{Map, Value};
use sqlx::PgConnection;

/// The legacy event kinds (docs/phase1/spec/club.md §2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    MemberJoined,
    PickSet,
    PickCleared,
    NotePosted,
    Reaction,
    Nominated,
    Voted,
    VoteClosed,
    MilestoneAdded,
    ShelfAdded,
    ShelfMoved,
    ShelfFinished,
    ShelfDnf,
    ShelfRemoved,
    Progress,
    QuoteAdded,
}

impl EventKind {
    /// Personal events have no club (`club_id` NULL) and are joined into club feeds at read time.
    pub fn is_personal(self) -> bool {
        matches!(
            self,
            EventKind::ShelfAdded
                | EventKind::ShelfMoved
                | EventKind::ShelfFinished
                | EventKind::ShelfDnf
                | EventKind::ShelfRemoved
                | EventKind::Progress
                | EventKind::QuoteAdded
        )
    }

    /// The stored `events.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::MemberJoined => "member_joined",
            EventKind::PickSet => "pick_set",
            EventKind::PickCleared => "pick_cleared",
            EventKind::NotePosted => "note_posted",
            EventKind::Reaction => "reaction",
            EventKind::Nominated => "nominated",
            EventKind::Voted => "voted",
            EventKind::VoteClosed => "vote_closed",
            EventKind::MilestoneAdded => "milestone_added",
            EventKind::ShelfAdded => "shelf_added",
            EventKind::ShelfMoved => "shelf_moved",
            EventKind::ShelfFinished => "shelf_finished",
            EventKind::ShelfDnf => "shelf_dnf",
            EventKind::ShelfRemoved => "shelf_removed",
            EventKind::Progress => "progress",
            EventKind::QuoteAdded => "quote_added",
        }
    }
}

/// An event to record.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub club_id: Option<i64>,
    pub actor_id: Option<i64>,
    pub kind: EventKind,
    pub work_id: Option<i64>,
    pub pick_id: Option<i64>,
    pub payload: Map<String, Value>,
}

/// Records `ev` in the caller's transaction and returns its id. Drops null payload values;
/// asserts `club_id.is_none() == kind.is_personal()`. Wave 1 step 2 implements it.
pub async fn record(_conn: &mut PgConnection, ev: NewEvent) -> sqlx::Result<i64> {
    Err(sqlx::Error::Protocol(format!(
        "activity::record is not implemented yet ({})",
        ev.kind.as_str()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_kinds_match_the_schema_check() {
        let personal: Vec<&str> = [
            EventKind::MemberJoined,
            EventKind::PickSet,
            EventKind::PickCleared,
            EventKind::NotePosted,
            EventKind::Reaction,
            EventKind::Nominated,
            EventKind::Voted,
            EventKind::VoteClosed,
            EventKind::MilestoneAdded,
            EventKind::ShelfAdded,
            EventKind::ShelfMoved,
            EventKind::ShelfFinished,
            EventKind::ShelfDnf,
            EventKind::ShelfRemoved,
            EventKind::Progress,
            EventKind::QuoteAdded,
        ]
        .into_iter()
        .filter(|kind| kind.is_personal())
        .map(EventKind::as_str)
        .collect();
        // events_scope in 0001_init.sql.
        assert_eq!(
            personal,
            [
                "shelf_added",
                "shelf_moved",
                "shelf_finished",
                "shelf_dnf",
                "shelf_removed",
                "progress",
                "quote_added"
            ]
        );
    }
}
