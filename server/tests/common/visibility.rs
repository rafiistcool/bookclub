//! Template for `tests/<pkg>_visibility.rs` (#46, docs/phase1/conventions.md §6.5).
//!
//! Every Wave 2 package that serves personal data covers each kind it serves (shelf, layout,
//! favourites, stats, diary, quotes, ratings in book detail, pick readers and activity) across
//! the viewer types of [`Viewers`] and both item visibilities:
//!
//! ```ignore
//! mod common;
//! use common::visibility::{Viewers, assert_hidden_like_unknown};
//! use common::*;
//!
//! #[sqlx::test]
//! async fn shelf_is_visible_to_the_audience_only(db: PgPool) {
//!     let app = TestApp::new(db).await;
//!     let v = Viewers::new(&app).await;
//!     // … the owner shelves a book with an only_me rating …
//!     for viewer in v.audience() {
//!         let res = viewer.get("/api/shelf?username=owner").await;
//!         res.assert_status(200);
//!         // the entry is there, its rating is null
//!     }
//!     for viewer in v.strangers() {
//!         assert_hidden_like_unknown(viewer, |name| format!("/api/shelf?username={name}"), "owner").await;
//!     }
//!     // the owner sees the rating
//! }
//! ```

use serde_json::Value;

use super::{TestApp, TestClient, TestClub};

/// The owner of personal data and one viewer of each type, with fixed usernames.
pub struct Viewers {
    /// `owner`: the data belongs to them.
    pub owner: TestClient,
    /// `comember`: shares the club `shared` with the owner (the audience).
    pub co_member: TestClient,
    /// `otherclub`: a member of `other`, a club the owner is not in.
    pub other_club: TestClient,
    /// `stranger`: in no club.
    pub stranger: TestClient,
    /// `former`: was in `shared` with the owner and left.
    pub former: TestClient,
    /// The owner's club, with `comember` as a member.
    pub shared: TestClub,
    /// `otherclub`'s club, owned by `otherclub`, with `comember` as a member too.
    pub other: TestClub,
}

/// The username every unknown-name check uses.
pub const UNKNOWN: &str = "nobody_here";

impl Viewers {
    /// Creates the five users and the two clubs. `comember` is also in `other`, so the
    /// audience is not transitive: `otherclub` shares a club with `comember` but not with the
    /// owner.
    pub async fn new(app: &TestApp) -> Self {
        let owner = app.user("owner").await;
        let co_member = app.user("comember").await;
        let other_club = app.user("otherclub").await;
        let stranger = app.user("stranger").await;
        let former = app.user("former").await;
        let shared = app.club(&owner, "Shared").await;
        app.join(&shared, &co_member, "member").await;
        app.join(&shared, &former, "member").await;
        app.exec(&format!(
            "DELETE FROM club_memberships WHERE club_id = {} AND user_id = {}",
            shared.id,
            former.id()
        ))
        .await;
        let other = app.club(&other_club, "Other").await;
        app.join(&other, &co_member, "member").await;
        Viewers {
            owner,
            co_member,
            other_club,
            stranger,
            former,
            shared,
            other,
        }
    }

    /// The viewers in the owner's audience: they see `audience` items, never `only_me` ones.
    pub fn audience(&self) -> Vec<&TestClient> {
        vec![&self.co_member]
    }

    /// The viewers who see nothing of the owner's: a member of another club, a stranger and a
    /// former co-member.
    pub fn strangers(&self) -> Vec<&TestClient> {
        vec![&self.other_club, &self.stranger, &self.former]
    }
}

/// Asserts that `viewer` gets the same answer (status and body) for the owner as for an
/// unknown username, and that it is a 404: strangers and unknown names are indistinguishable.
pub async fn assert_hidden_like_unknown(
    viewer: &TestClient,
    path_for: impl Fn(&str) -> String,
    owner: &str,
) {
    let hidden = viewer.get(&path_for(owner)).await;
    let unknown = viewer.get(&path_for(UNKNOWN)).await;
    assert_eq!(
        hidden.status.as_u16(),
        404,
        "{} sees {owner}: {}",
        viewer.username(),
        hidden.text()
    );
    assert_eq!(hidden.status, unknown.status);
    assert_eq!(
        serde_json::from_slice::<Value>(&hidden.body).ok(),
        serde_json::from_slice::<Value>(&unknown.body).ok(),
        "a stranger's answer differs from an unknown name's"
    );
}
