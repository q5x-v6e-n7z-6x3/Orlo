//! GPUI presentation for Orlo's editor, workspaces, panels, and native UI.

// Typed UI actions that only forward constants and window/context arguments.
macro_rules! forward_actions {
    ($($(#[$attr:meta])* $vis:vis $name:ident($action:ty, $window:pat, $cx:pat) => $target:ident($($args:expr),* $(,)?);)*) => {
        $($(#[$attr])* $vis fn $name(&mut self, _: &$action, $window: &mut gpui::Window, $cx: &mut gpui::Context<Self>) {
            self.$target($($args),*);
        })*
    };
}

// Parameterized actions keep their public fields and optional JSON support together.
macro_rules! parameter_actions {
    ($namespace:ident, $kind:ident; $($(#[$attr:meta])* $name:ident {
        $($(#[$field_attr:meta])* $field:ident: $ty:ty),* $(,)?
    })*) => {
        $(parameter_actions!(@$kind $namespace; $(#[$attr])* pub struct $name {
            $($(#[$field_attr])* pub $field: $ty),*
        });)*
    };
    (@serializable $namespace:ident; $definition:item) => {
        parameter_actions!(@plain $namespace;
            #[derive(serde::Serialize, serde::Deserialize)] $definition);
    };
    (@plain $namespace:ident; $definition:item) => {
        #[derive(Clone, PartialEq, Debug, gpui::Action)]
        #[action(namespace = $namespace, no_json)]
        $definition
    };
}

pub mod bars;
pub mod components;
pub mod documents;
pub mod icons;
pub mod keybindings;
pub mod languages;
pub mod layout;
pub mod overlays;
pub mod panels;
pub mod platform;
pub mod styling;
pub mod views;

pub use views::{editor, workspace};

pub mod app {
    pub use crate::platform::{macos, perf};
    pub use orlo_workspace::{cli, handoff, session};
}

pub mod chrome {
    pub use crate::components::widgets as ui;
    pub use crate::icons;
    pub use crate::layout;
    pub use crate::overlays::quick_panel;
    pub use crate::panels::explorer as sidebar;
    pub use crate::panels::search::find_panel;
    pub use crate::styling as theme;
    pub use orlo_utils::fuzzy;
}

pub mod config {
    pub use crate::keybindings as customization;
    pub use orlo_config::{editorconfig, settings};
    pub mod automation {
        pub use orlo_config::automation::*;
        pub use orlo_editor::commands::FilterSnapshot;
    }
    pub mod project {
        pub use orlo_project::project::*;
        pub use orlo_workspace::sessions::WorkspaceFile;
    }
}

pub mod editing {
    pub use crate::components::{clipboard_history, html};
    pub use orlo_editor::{columns, convert, csv, display, encoding, fold, json, snippet, stats, text, transform};
}

pub mod files {
    pub use orlo_project::{comparison, file_history, fsops, watch};
}

pub mod integrations {
    pub use crate::panels::terminal::emulator as terminal;
    pub use orlo_git::{changes, git};
}

pub mod searching {
    pub use crate::panels::search::find_in_files;
    pub use orlo_editor::searching::{extract, pattern, pipeline, results, saved, search};
}

#[cfg(test)]
pub(crate) mod testing;
