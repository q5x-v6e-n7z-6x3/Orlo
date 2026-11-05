//! Initial screen dimensions and bounded terminal buffering.

pub const MAX_INPUT: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalConfiguration {
    pub rows: u16,
    pub cols: u16,
    pub scrollback: usize,
}

impl Default for TerminalConfiguration {
    fn default() -> Self {
        Self { rows: 24, cols: 80, scrollback: 10_000 }
    }
}
