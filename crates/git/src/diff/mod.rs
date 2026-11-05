//! Git patches, line changes, and merge conflict text.

pub mod changes;
mod conflicts;
mod patch;
mod versions;

pub use conflicts::{conflicts, has_conflict_markers, resolve_conflict, ConflictBlock, ConflictChoice};
pub use patch::diff;
pub use versions::{head_text, versions, Versions};
