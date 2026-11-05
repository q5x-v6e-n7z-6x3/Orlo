//! PTY sessions and terminal state, independent of presentation and application runtimes.

pub mod commands;
pub mod configuration;
pub mod emulation;
pub mod input;
pub mod links;
pub mod mouse;
pub mod output;
pub mod processes;
pub mod pty;
pub mod search;
pub mod sessions;
pub mod shell;

pub use configuration::TerminalConfiguration;
pub use emulation::{Color, Emulator, Screen};
pub use input::{key_bytes, paste_bytes, Key, Modifiers, PtyCommand};
pub use output::{DrainStatus, PtyEvent};
pub use portable_pty::CommandBuilder;
pub use pty::PtySession;
pub use sessions::{CellPosition, Selection, TerminalSession};
