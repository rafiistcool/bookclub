//! What the two admin queues share (#54): the `?status=&page=` query of `GET /api/admin/reports`
//! and `GET /api/admin/edit-requests`, and the overdue threshold.

use serde::Deserialize;
use utoipa::IntoParams;

use crate::extract::{Validate, Validator};

/// Items per page of either queue.
pub const PAGE_SIZE: i64 = 50;
/// The highest page number accepted (50,000 items deep).
pub const MAX_PAGE: i64 = 1000;
/// Hours after which an open report or request is overdue: the decision target of the takedown
/// procedure (work plan D4, 72 h until Q12 is answered).
pub const TARGET_HOURS: i64 = 72;
/// The pydantic enum message for an unknown `status`.
pub const STATUS_CHOICES: &str = "Input should be 'open' or 'resolved'";

/// Which part of a queue to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct QueueQuery {
    /// `open` (the default): waiting for a decision, oldest first. `resolved`: decided, the
    /// newest decision first.
    #[serde(default = "default_status")]
    #[param(value_type = Option<String>)]
    pub status: String,
    /// 1-based, 50 items a page; default 1.
    #[serde(default = "first_page")]
    #[param(value_type = Option<i64>)]
    pub page: i64,
}

fn default_status() -> String {
    "open".to_owned()
}

fn first_page() -> i64 {
    1
}

impl Validate for QueueQuery {
    fn validate(&mut self, v: &mut Validator) {
        v.check(
            matches!(self.status.as_str(), "open" | "resolved"),
            STATUS_CHOICES,
        );
        v.range_i64(Some(self.page), 1, MAX_PAGE);
    }
}

impl QueueQuery {
    /// The decided part of the queue is asked for.
    pub fn resolved(&self) -> bool {
        self.status == "resolved"
    }

    /// The rows to skip before this page.
    pub fn offset(&self) -> i64 {
        (self.page - 1) * PAGE_SIZE
    }

    /// The `LIMIT` of a page query: one more than a page, so a full page tells whether more
    /// follow ([`QueueQuery::split`]).
    pub fn limit(&self) -> i64 {
        PAGE_SIZE + 1
    }

    /// Cuts the extra row of [`QueueQuery::limit`]: the page's ids, and whether more follow.
    pub fn split(&self, mut ids: Vec<i64>) -> (Vec<i64>, bool) {
        let more = ids.len() as i64 > PAGE_SIZE;
        ids.truncate(PAGE_SIZE as usize);
        (ids, more)
    }
}
