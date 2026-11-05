//! Project roots, document storage, filesystem operations, and disk watchers.

pub mod directories;
pub mod documents;
pub mod explorer;
pub mod filesystem;
pub mod paths;
pub mod watchers;

pub use directories::project;
pub use documents::{comparison, encoding, file_history, io};
pub use filesystem as fsops;
pub use watchers as watch;
