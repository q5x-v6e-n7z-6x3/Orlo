//! Events emitted by a PTY and the result of applying a bounded output batch.

#[derive(Debug)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Error(String),
    Exited(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainStatus {
    pub running: bool,
    pub changed: bool,
}
