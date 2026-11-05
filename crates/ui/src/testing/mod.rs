//! Shared source fixtures and release performance checks for the UI test harness.

#[path = "../../../../benches/mod.rs"]
pub(crate) mod bench;

#[path = "../../../syntax/src/fixtures.rs"]
mod fixtures;
pub(crate) use fixtures::{block_on, EDITING_MODEL};

pub(crate) mod gate;
