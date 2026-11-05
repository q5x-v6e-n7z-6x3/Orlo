//! Shared JSON command format, keybindings and recorded editor macros.

use crate::editor::*;
use gpui::{prelude::*, Action, App, Div, KeyBinding};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

use orlo_config::keybindings::arguments;
pub use orlo_config::keybindings::{keymap_path, Command};

/// Build the UI action named by a serialized configuration command.
pub trait CommandAction {
    fn build(&self, cx: &App) -> Result<Box<dyn Action>, String>;
}

impl CommandAction for Command {
    fn build(&self, cx: &App) -> Result<Box<dyn Action>, String> {
        if self.command == "insert" || self.command == InsertText::name_for_type() {
            let action = serde_json::from_value::<InsertText>(self.args.clone()).map_err(|e| e.to_string())?;
            if action.replace.is_some_and(|[start, end]| start > end) {
                return Err("Replacement start must not follow its end".into());
            }
            return Ok(Box::new(action));
        }
        macro_rules! parameterized {
            ($($ty:ty),* $(,)?) => {$(
                if self.command == <$ty>::name_for_type() {
                    return serde_json::from_value::<$ty>(self.args.clone())
                        .map(|a| Box::new(a) as Box<dyn Action>).map_err(|e| e.to_string());
                }
            )*};
        }
        macro_rules! build_editor_type {
            (parameter, record, $ty:ident) => {
                parameterized!($ty);
            };
            (parameter, build, $ty:ident) => {
                parameterized!($ty);
            };
            ($kind:ident, $policy:ident, $ty:ident) => {};
        }
        macro_rules! build_editor_types {
            ($($ty:ident [$kind:ident, $policy:ident] => $handler:ident $(($mode:ident => $method:ident))?,)*) => {
                $(build_editor_type!($kind, $policy, $ty);)*
            };
        }
        editor_actions!(build_editor_types);
        parameterized!(
            crate::workspace::SelectTab,
            crate::workspace::SetRuler,
            crate::workspace::SetLineNumberMode,
            crate::workspace::SetTabSize,
            crate::workspace::SetIndentStyle,
            crate::workspace::SetSyntax,
            crate::workspace::SetLineEnding,
            crate::workspace::NextLineEnding,
            crate::workspace::FocusGroup,
            crate::workspace::MoveToGroup,
            crate::workspace::FocusNeighbour,
            crate::workspace::MoveToNeighbour,
            crate::workspace::AffixLines,
            crate::workspace::SetWrapColumn,
            crate::workspace::RunPackageCommand,
            crate::workspace::RunConfiguredTask,
            crate::workspace::PackageOperation
        );
        cx.build_action(&self.command, Some(self.args.clone())).map_err(|e| e.to_string())
    }
}

pub fn bindings(text: &str, cx: &App) -> Result<Vec<KeyBinding>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Binding {
        keys: Vec<String>,
        command: Option<String>,
        #[serde(default = "arguments")]
        args: Value,
        context: Option<String>,
    }
    let entries: Vec<Binding> =
        serde_json::from_str(&orlo_config::settings::strip_json(text)).map_err(|e| e.to_string())?;
    entries
        .into_iter()
        .enumerate()
        .map(|(i, b)| {
            let make = || {
                if b.keys.is_empty() || b.keys.iter().any(|k| k.trim().is_empty()) {
                    return Err("keys must contain at least one nonempty keystroke".into());
                }
                let action = match b.command {
                    None => Box::new(gpui::NoAction) as Box<dyn Action>,
                    Some(command) => Command { command, args: b.args }.build(cx)?,
                };
                let context = b
                    .context
                    .as_deref()
                    .map(gpui::KeyBindingContextPredicate::parse)
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .map(std::rc::Rc::new);
                KeyBinding::load(&b.keys.join(" "), action, context, false, None, &gpui::DummyKeyboardMapper)
                    .map_err(|e| e.to_string())
            };
            make().map_err(|e| format!("Keybinding {}: {e}", i + 1))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct InsertText {
    pub text: String,
    /// Character offsets from each caret, for OS text replacements such as accent selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<[isize; 2]>,
}

/// Rebuilding starts with the defaults, so deleted user bindings disappear too.
/// Orlo's own key bindings, later sets winning over earlier ones. Startup binds them before the
/// user's keymap is read, so a broken keymap still leaves them; `install` binds them again.
pub fn bind_defaults(cx: &mut App) {
    cx.bind_keys(crate::editor::key_bindings());
    cx.bind_keys(crate::workspace::key_bindings());
    cx.bind_keys(crate::chrome::quick_panel::key_bindings());
    cx.bind_keys(crate::chrome::find_panel::key_bindings());
    cx.bind_keys(crate::searching::find_in_files::key_bindings());
    cx.bind_keys(crate::workspace::results_key_bindings());
    cx.bind_keys(crate::workspace::playground_key_bindings());
    cx.bind_keys(crate::integrations::terminal::key_bindings());
    cx.bind_keys(crate::workspace::git_key_bindings());
    cx.bind_keys(crate::workspace::comparison_key_bindings());
    cx.bind_keys(crate::workspace::history_key_bindings());
    cx.bind_keys(crate::workspace::discarded::key_bindings());
}

pub fn install(cx: &mut App) -> Result<(), String> {
    let user = match std::fs::read_to_string(keymap_path()) {
        Ok(text) => bindings(&text, cx)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    cx.clear_key_bindings();
    bind_defaults(cx);
    cx.bind_keys(user);
    Ok(())
}

#[derive(Default)]
pub struct Recorder {
    pub recording: bool,
    pub playing: bool,
    pub commands: Vec<Command>,
    pub last: Vec<Command>,
}
impl gpui::Global for Recorder {}

pub const MAX_STEPS: usize = 10_000;

pub fn record(command: Command, cx: &mut App) {
    let r = cx.default_global::<Recorder>();
    // past the cap steps are dropped: recording stays on, so the toggle still stops it
    if r.recording && !r.playing && r.commands.len() < MAX_STEPS {
        r.commands.push(command);
    }
}

pub fn record_text(text: &str, replace: Option<[isize; 2]>, cx: &mut App) {
    if !text.is_empty() {
        record(
            Command {
                command: InsertText::name_for_type().into(),
                args: serde_json::json!({"text":text,"replace":replace}),
            },
            cx,
        );
    }
}

/// Capture on the editor itself excludes the palette and other form inputs.
pub fn capture(mut d: Div) -> Div {
    macro_rules! plain { ($($ty:ty),* $(,)?) => {$(
        d = d.capture_action(|a:&$ty,_,cx| record(Command {
            command:a.name().into(),args:arguments() },cx));
    )*}; }
    macro_rules! params { ($($ty:ty),* $(,)?) => {$(
        d = d.capture_action(|a:&$ty,_,cx| {
            if let Ok(args)=serde_json::to_value(a) {
                record(Command { command:a.name().into(),args },cx);
            }
        });
    )*}; }
    macro_rules! record_editor_type {
        (unit, record, $ty:ident) => {
            plain!($ty);
        };
        ($kind:ident, record, $ty:ident) => {
            params!($ty);
        };
        ($kind:ident, $policy:ident, $ty:ident) => {};
    }
    macro_rules! record_editor_types {
        ($($ty:ident [$kind:ident, $policy:ident] => $handler:ident $(($mode:ident => $method:ident))?,)*) => {
            $(record_editor_type!($kind, $policy, $ty);)*
        };
    }
    editor_actions!(record_editor_types);
    d
}

pub fn macro_path(name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > 100
        || name.starts_with('.')
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
    {
        return Err("Use a macro name of 1–100 characters without path separators".into());
    }
    Ok(orlo_utils::paths::support_dir().join("Macros").join(format!("{name}.json")))
}

pub fn parse_macro(text: &str, cx: &App) -> Result<Vec<Command>, String> {
    let commands: Vec<Command> =
        serde_json::from_str(&orlo_config::settings::strip_json(text)).map_err(|e| e.to_string())?;
    if commands.is_empty() || commands.len() > MAX_STEPS {
        return Err(format!("A macro must contain 1–{MAX_STEPS} editor commands"));
    }
    for command in &commands {
        if command.command != "insert" && !command.command.starts_with("editor::") {
            return Err("Macros accept editor commands only".into());
        }
        command.build(cx)?;
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn keybindings_keep_context_chords_and_arguments(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let parsed = bindings(
                r#"[
                // a context-dependent chord
                {"keys":["cmd-k","cmd-i"],"command":"insert","args":{"text":"é"},"context":"Editor && !single_line"},
            ]"#,
                cx,
            )
            .unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].keystrokes().len(), 2);
            assert!(parsed[0].predicate().is_some());
        });
    }

    #[gpui::test]
    fn recording_past_the_step_cap_keeps_recording_and_drops_the_rest(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.default_global::<Recorder>().recording = true;
            for i in 0..=MAX_STEPS {
                record_text(&i.to_string(), None, cx);
            }
            let r = cx.global::<Recorder>();
            assert!(r.recording);
            assert_eq!(r.commands.len(), MAX_STEPS);
            assert!(r.last.is_empty());
        });
    }

    #[test]
    fn macro_names_are_limited_in_characters_not_bytes() {
        assert!(macro_path(&"é".repeat(100)).is_ok());
        assert!(macro_path(&"é".repeat(101)).is_err());
    }

    #[gpui::test]
    fn bad_keybinding_is_rejected_before_installing_any(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert!(bindings(r#"[{"keys":["cmd-j"],"command":"editor::DuplicateLine"},{"keys":["cmd-k"],"command":"not_a_command"}]"#, cx).is_err());
            assert!(bindings(r#"[{"keys":[],"command":"editor::Copy"}]"#, cx).is_err());
            assert!(bindings(r#"[{"keys":["cmd-j"],"command":"editor::Copy","context":"("}]"#, cx).is_err());
        });
    }
}
