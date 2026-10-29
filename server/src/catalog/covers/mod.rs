//! The cover search (#74, owner COVER; docs/phase2/notes/cover.md): every catalog and imported
//! book gets a real cover. A book a reader added (`works.source = 'user'`) is never searched.
//!
//! - [`matching`]: the matching rule, pure functions.
pub mod matching;
