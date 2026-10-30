//! The cover search (#74, owner COVER; docs/phase2/notes/cover.md): every catalog and imported
//! book gets a real cover. A book a reader added (`works.source = 'user'`) is never searched,
//! and Google covers stay out (docs/plan.md, "Catalog"): the sources are Open Library (fetched
//! by cover id, served from our domain) and pictures people upload.
//!
//! - [`matching`]: the matching rule, pure functions.
//! - [`search`]: one search of one work (facts, remote, apply), shared by every caller.
//! - [`dead`]: Open Library pictures that failed, fetched again and, when gone, cleared.
//! - [`job`]: the background search every 10 minutes, with its breaker.
//! - [`admin`]: the admin's suggestions, the books without a cover and "Search now".
pub mod admin;
pub mod dead;
pub mod job;
pub mod matching;
pub mod search;
