//! Language services: highlighting, bundled parsers, completion, spelling, and LSP.

#[path = "languages/complete.rs"]
pub mod complete;
#[path = "highlighting/highlight.rs"]
pub mod highlight;
pub mod index;
#[path = "detection/language.rs"]
pub mod language;
#[path = "languages/lsp/mod.rs"]
pub mod lsp;
#[path = "languages/manifest.rs"]
pub mod manifest;
#[path = "languages/registry.rs"]
pub mod registry;
#[path = "detection/related.rs"]
pub mod related;
#[path = "languages/signature.rs"]
pub mod signature;
#[path = "languages/spell.rs"]
pub mod spell;
#[path = "grammars/treesitter.rs"]
pub mod treesitter;

#[cfg(test)]
#[path = "languages/bundled_tests.rs"]
mod bundled_tests;
#[cfg(test)]
mod fixtures;
