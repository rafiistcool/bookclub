//! Frozen media interfaces shared by every package (docs/phase1/conventions.md §8).
//!
//! Wave 1 step 2 adds `NewMedia`, `register`, `is_removed_hash` and `MemMediaStore`.

pub mod store;
pub mod urls;

pub use store::{FsMediaStore, MediaStore};
