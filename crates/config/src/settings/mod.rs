//! Orlo's Sublime-style settings file: `Preferences.sublime-settings` in Application Support,
//! JSON with comments and trailing commas, Sublime's key names.

pub mod editorconfig;
pub mod modelines;
pub mod selector;
mod server;
mod terminal;
pub mod unicode;

pub use terminal::{TerminalProfile, TerminalSettings};

pub use server::ServerConfig;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use syntect::highlighting::ScopeSelectors;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Indentation {
    pub size: Option<usize>,
    pub spaces: bool,
}

/// Filename ordering in the explorer, independent of document preferences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerSort {
    #[default]
    Lexical,
    Natural,
}

/// Labels for physical source lines in an editor gutter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineNumberMode {
    #[default]
    Absolute,
    Relative,
}

/// Inclusive maximum severity for inline language-server messages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error = 1,
    #[default]
    Warning = 2,
    Information = 3,
    Hint = 4,
}

/// Which caret origins may navigate beyond a logical line end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualSpace {
    #[default]
    Off,
    Rectangular,
    All,
}

impl VirtualSpace {
    pub fn allows(self, rectangular: bool) -> bool {
        self == Self::All || self == Self::Rectangular && rectangular
    }

    pub fn enabled(self) -> bool {
        self != Self::Off
    }
}

impl From<bool> for VirtualSpace {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::All
        } else {
            Self::Off
        }
    }
}

impl<'de> Deserialize<'de> for VirtualSpace {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Policy;
        impl serde::de::Visitor<'_> for Policy {
            type Value = VirtualSpace;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("off, rectangular, all, or a legacy Boolean")
            }
            fn visit_bool<E: serde::de::Error>(self, enabled: bool) -> Result<Self::Value, E> {
                Ok(enabled.into())
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "off" => Ok(VirtualSpace::Off),
                    "rectangular" => Ok(VirtualSpace::Rectangular),
                    "all" => Ok(VirtualSpace::All),
                    _ => Err(E::unknown_variant(value, &["off", "rectangular", "all"])),
                }
            }
        }
        deserializer.deserialize_any(Policy)
    }
}

/// Appearance assets and distraction-free presentation belong to the application.
pub const APPLICATION_SETTINGS: &[&str] = &[
    "theme",
    "color_scheme",
    "distraction_free",
    "lsp_enabled",
    "language_servers",
    "formatters",
    "terminal",
    "explorer_sort",
    "explorer_sticky_ancestors",
    "fold_buttons",
    "fade_fold_buttons",
    "draw_white_space",
    "drag_text",
    "auto_complete",
    "auto_complete_min_chars",
    "show_definitions",
    "index_files",
    "index_exclude_patterns",
];

/// Preferences the editor resolves for each file on its own.
pub const FILE_SETTINGS: &[&str] = &[
    "font_face",
    "font_size",
    "tab_size",
    "translate_tabs_to_spaces",
    "word_wrap",
    "wrap_width",
    "rulers",
    "show_minimap",
    "line_number_mode",
    "cursor_column_highlight",
    "detect_indentation",
    "modelines",
    "spell_check",
    "spelling_selector",
    "animation_enabled",
    "trim_trailing_white_space_on_save",
    "remove_trailing_blank_lines_on_save",
    "ensure_newline_at_eof_on_save",
    "format_on_save",
    "format_on_save_timeout_ms",
    "file_autosave",
    "file_autosave_delay_ms",
    "virtual_space",
    "selection_occurrences",
    "inline_diagnostics",
    "inline_diagnostics_severity",
    "unicode_warnings",
    "unicode_warning_characters",
    "unicode_warning_ranges",
];

pub const DEFAULT_SELECTOR: &str = "markup.raw, source string.quoted - punctuation - meta.preprocessor.include, source comment - source comment.block.preprocessor, -(constant, entity, keyword, punctuation, storage, support, variable, markup.underline.link, meta.tag)";

/// Unknown keys are ignored, so a Sublime preferences file can be pasted in.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub terminal: TerminalSettings,
    pub explorer_sort: ExplorerSort,
    /// Pin scrolled-out parent folders above the visible tree rows.
    pub explorer_sticky_ancestors: bool,
    /// Local stdio language servers; absent entries use the bundled command defaults.
    pub lsp_enabled: bool,
    pub language_servers: std::collections::BTreeMap<String, ServerConfig>,
    /// Language id to a formatter's arguments, over the language's own; `[]` turns it off.
    #[serde(deserialize_with = "server::formatters")]
    pub formatters: std::collections::BTreeMap<String, Vec<String>>,
    pub font_face: String,
    pub font_size: f32,
    #[serde(deserialize_with = "whole")]
    pub tab_size: usize,
    /// On by default in Orlo; Sublime's default is false.
    pub translate_tabs_to_spaces: bool,
    /// Allow editable file carets beyond logical line ends without inserting spaces.
    pub virtual_space: VirtualSpace,
    pub word_wrap: WordWrap,
    /// Wrap at most this many display columns; 0 wraps at the view's width.
    #[serde(deserialize_with = "whole")]
    pub wrap_width: usize,
    #[serde(deserialize_with = "rulers")]
    pub rulers: Vec<usize>,
    /// An Orlo key: Sublime keeps the minimap per window.
    pub show_minimap: bool,
    pub line_number_mode: LineNumberMode,
    /// Low-emphasis bands at the current caret display columns.
    pub cursor_column_highlight: bool,
    pub theme: String,
    pub color_scheme: String,
    pub detect_indentation: bool,
    /// Read only the documented data-only Vim/Emacs file preferences.
    pub modelines: bool,
    pub overrides: Vec<ScopedSettings>,
    /// Explicit global/scoped indentation beats automatic detection.
    #[serde(skip)]
    pub indentation_explicit: [bool; 2],
    pub trim_trailing_white_space_on_save: Trim,
    /// An Orlo key: Sublime has no such setting.
    pub remove_trailing_blank_lines_on_save: bool,
    pub ensure_newline_at_eof_on_save: bool,
    /// Ask the document's language server to format before save-time cleanup.
    pub format_on_save: bool,
    /// Positive whole milliseconds for save formatting, including waiting for the service.
    #[serde(deserialize_with = "positive_milliseconds")]
    pub format_on_save_timeout_ms: u32,
    /// Save eligible existing files after their configured idle delay.
    pub file_autosave: bool,
    /// Positive whole milliseconds without edits before automatic Save (default two seconds).
    #[serde(deserialize_with = "positive_milliseconds")]
    pub file_autosave_delay_ms: u32,
    /// Overlay warnings without changing text shaping.
    pub unicode_warnings: bool,
    /// Passively mark other literal matches of one single-line selection.
    pub selection_occurrences: bool,
    /// Show one diagnostic after each visible physical line end.
    pub inline_diagnostics: bool,
    pub inline_diagnostics_severity: DiagnosticSeverity,
    pub unicode_warning_characters: unicode::WarningCharacters,
    pub unicode_warning_ranges: unicode::WarningRanges,
    /// Sublime's key: the side bar slides and Page Up/Down scroll smoothly; macOS Reduce
    /// Motion turns them off too.
    pub animation_enabled: bool,
    /// What View > Enter Distraction Free Mode shows; Orlo's key, in place of Sublime's
    /// separate Distraction Free settings file.
    pub distraction_free: DistractionFree,
    /// ▾ beside the lines that head a foldable block, ▸ beside folded ones.
    pub fold_buttons: bool,
    /// ▾ only while the pointer is over the gutter.
    pub fade_fold_buttons: bool,
    /// Where white space is drawn.
    pub draw_white_space: WhiteSpace,
    /// A press on selected text drags it, instead of starting a new selection.
    pub drag_text: bool,
    /// The completion list opens as a word is typed.
    pub auto_complete: bool,
    /// An Orlo key: how many letters of the word open it.
    #[serde(deserialize_with = "whole")]
    pub auto_complete_min_chars: usize,
    /// Hover over a code identifier for indexed definitions and documentation.
    pub show_definitions: bool,
    /// The folders' files go in the project index (open buffers do either way).
    pub index_files: bool,
    /// Files the index leaves out, read as the project's file exclude patterns are.
    pub index_exclude_patterns: Vec<String>,
    pub spell_check: bool,
    /// Preserve bounded private versions before file overwrites.
    pub file_history: bool,
    pub spelling_selector: String,
}

/// Distraction-free mode's own presentation. Toggling an element inside the mode
/// changes it for that stay only.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct DistractionFree {
    /// Enter macOS full screen with it (and leave it with it).
    pub full_screen: bool,
    pub show_sidebar: bool,
    pub show_tabs: bool,
    pub show_status_bar: bool,
    pub show_minimap: bool,
    pub show_panel: bool,
    pub line_numbers: bool,
    pub word_wrap: bool,
    #[serde(deserialize_with = "whole")]
    pub wrap_width: usize,
    /// The text column in the middle of the window.
    pub draw_centered: bool,
}

impl Default for DistractionFree {
    fn default() -> Self {
        Self {
            full_screen: true,
            show_sidebar: false,
            show_tabs: false,
            show_status_bar: false,
            show_minimap: false,
            show_panel: false,
            line_numbers: false,
            word_wrap: true,
            wrap_width: 80,
            draw_centered: true,
        }
    }
}

/// The widest `wrap_width` taken; more is clamped.
pub const MAX_WRAP_WIDTH: usize = 1000;

impl Default for Settings {
    fn default() -> Self {
        Self {
            lsp_enabled: true,
            terminal: TerminalSettings::default(),
            explorer_sort: ExplorerSort::Lexical,
            explorer_sticky_ancestors: false,
            language_servers: Default::default(),
            formatters: Default::default(),
            font_face: "Menlo".into(),
            font_size: 12.,
            tab_size: 4,
            translate_tabs_to_spaces: true,
            virtual_space: VirtualSpace::Off,
            word_wrap: WordWrap::Auto,
            wrap_width: 0,
            rulers: Vec::new(),
            show_minimap: true,
            line_number_mode: LineNumberMode::Absolute,
            cursor_column_highlight: false,
            theme: "auto".into(),
            color_scheme: "Mariana".into(),
            detect_indentation: true,
            modelines: false,
            overrides: Vec::new(),
            indentation_explicit: [false; 2],
            trim_trailing_white_space_on_save: Trim::None,
            remove_trailing_blank_lines_on_save: false,
            ensure_newline_at_eof_on_save: false,
            format_on_save: false,
            format_on_save_timeout_ms: 5000,
            file_autosave: false,
            file_autosave_delay_ms: 2000,
            // Trojan Source: bidi controls and invisible characters in comments and strings
            unicode_warnings: true,
            selection_occurrences: false,
            inline_diagnostics: false,
            inline_diagnostics_severity: DiagnosticSeverity::Warning,
            unicode_warning_characters: unicode::default_characters(),
            unicode_warning_ranges: Default::default(),
            animation_enabled: true,
            distraction_free: DistractionFree::default(),
            fold_buttons: true,
            fade_fold_buttons: true,
            draw_white_space: WhiteSpace::SELECTION,
            drag_text: true,
            auto_complete: true,
            auto_complete_min_chars: 2,
            show_definitions: true,
            index_files: true,
            index_exclude_patterns: vec!["*.log".into()],
            spell_check: false,
            file_history: false,
            spelling_selector: DEFAULT_SELECTOR.into(),
        }
    }
}

impl Settings {
    /// Global preferences, followed by matching overrides in file order. The workspace
    /// applies project, EditorConfig, modelines, window and explicit buffer/view choices afterwards.
    pub fn for_file(&self, path: Option<&Path>, language: &str, scope: &str) -> Self {
        let mut out = self.clone();
        let stack: Vec<_> = scope.split_whitespace().filter_map(|s| syntect::parsing::Scope::new(s).ok()).collect();
        for rule in &self.overrides {
            if rule.language.as_ref().is_some_and(|s| !s.eq_ignore_ascii_case(language))
                || rule
                    .path
                    .as_ref()
                    .is_some_and(|s| path.is_none_or(|p| !crate::editorconfig::matches(s, &p.to_string_lossy())))
                || rule.scope.as_ref().is_some_and(|s| {
                    ScopeSelectors::from_str(s).map_or(true, |selectors| selectors.does_match(&stack).is_none())
                })
            {
                continue;
            }
            out = out.overlay(&rule.settings).expect("scoped settings were validated");
        }
        out
    }

    /// Merge a partial layer without resetting preferences the layer does not mention.
    pub fn overlay(&self, values: &serde_json::Map<String, serde_json::Value>) -> Result<Self, String> {
        let mut merged = serde_json::to_value(self).map_err(|e| e.to_string())?;
        for (key, value) in values {
            merged[key] = value.clone();
        }
        let mut out: Self = serde_json::from_value(merged).map_err(|e| e.to_string())?;
        out.indentation_explicit = [
            self.indentation_explicit[0] || values.contains_key("tab_size"),
            self.indentation_explicit[1] || values.contains_key("translate_tabs_to_spaces"),
        ];
        out.normalize();
        Ok(out)
    }

    fn normalize(&mut self) {
        self.tab_size = self.tab_size.clamp(1, 16);
        self.font_size = self.font_size.clamp(6., 72.);
        self.wrap_width = self.wrap_width.min(MAX_WRAP_WIDTH);
        self.distraction_free.wrap_width = self.distraction_free.wrap_width.min(MAX_WRAP_WIDTH);
        if self.font_face.trim().is_empty() {
            self.font_face = Settings::default().font_face;
        }
    }
    /// Whether the chrome is Default Dark: `"auto"`, and a theme Orlo does not have, follow
    /// macOS as Sublime's `"auto"` does.
    pub fn dark(&self, macos_dark: bool) -> bool {
        match self.theme.as_str() {
            "Default.sublime-theme" => false,
            "Default Dark.sublime-theme" => true,
            _ => macos_dark,
        }
    }

    pub fn cleanup(&self) -> Cleanup {
        Cleanup {
            trim: self.trim_trailing_white_space_on_save,
            remove_trailing_blank_lines: self.remove_trailing_blank_lines_on_save,
            ensure_newline: self.ensure_newline_at_eof_on_save,
            remove_final_newline: false,
        }
    }
}

/// A rule selects a language name, a path glob and/or a syntax scope selector; all supplied
/// selectors must match. A settings object is a partial layer, never a second set of defaults.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScopedSettings {
    pub language: Option<String>,
    #[serde(default, deserialize_with = "path_pattern")]
    pub path: Option<String>,
    #[serde(default, deserialize_with = "scope_selector")]
    pub scope: Option<String>,
    #[serde(deserialize_with = "scoped_values")]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

/// An optional string, rejected when `check` finds it malformed.
fn checked<'de, D: Deserializer<'de>, E: std::fmt::Display>(
    d: D,
    check: impl FnOnce(&str) -> Result<(), E>,
) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(d)?;
    value.as_deref().map_or(Ok(()), check).map_err(D::Error::custom)?;
    Ok(value)
}

fn path_pattern<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    checked(d, crate::editorconfig::validate_pattern)
}

fn scope_selector<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    checked(d, |s| ScopeSelectors::from_str(s).map(drop))
}

/// Whether `key` alone, set to `value`, deserializes.
fn usable(key: &str, value: &serde_json::Value) -> Result<(), serde_json::Error> {
    serde_json::from_value::<Settings>(serde_json::json!({ key: value })).map(drop)
}

fn scoped_values<'de, D: Deserializer<'de>>(d: D) -> Result<serde_json::Map<String, serde_json::Value>, D::Error> {
    let map = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
    for (key, value) in &map {
        if key == "overrides" {
            return Err(D::Error::custom("scoped settings cannot contain overrides"));
        }
        if APPLICATION_SETTINGS.contains(&key.as_str()) {
            return Err(D::Error::custom(format!("{key:?} is application-wide; set it in global preferences")));
        }
        if !FILE_SETTINGS.contains(&key.as_str()) {
            return Err(D::Error::custom(format!("unsupported scoped preference {key:?}")));
        }
        usable(key, value).map_err(D::Error::custom)?;
    }
    Ok(map)
}

/// What a save tidies, all off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cleanup {
    pub trim: Trim,
    pub remove_trailing_blank_lines: bool,
    pub ensure_newline: bool,
    /// EditorConfig's explicit `insert_final_newline = false`; the global false leaves text.
    pub remove_final_newline: bool,
}

/// `false` / `"none"`, `true` / `"all"`, or `"not_on_caret"`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Trim {
    #[default]
    None,
    All,
    NotOnCaret,
}

impl<'de> Deserialize<'de> for Trim {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Bool(false) => Ok(Trim::None),
            serde_json::Value::Bool(true) => Ok(Trim::All),
            serde_json::Value::String(s) if s == "none" => Ok(Trim::None),
            serde_json::Value::String(s) if s == "all" => Ok(Trim::All),
            serde_json::Value::String(s) if s == "not_on_caret" => Ok(Trim::NotOnCaret),
            v => Err(D::Error::custom(format!(
                "trim_trailing_white_space_on_save must be false, true or \"not_on_caret\", not {v}"
            ))),
        }
    }
}

impl Serialize for Trim {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Self::None => "none",
            Self::All => "all",
            Self::NotOnCaret => "not_on_caret",
        })
    }
}

/// JSON `true` / `false` / `"auto"`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WordWrap {
    On,
    Off,
    Auto,
}

impl WordWrap {
    /// `Auto` wraps every syntax that is not source code, as Sublime does.
    pub fn wraps(self, syntax_scope: &str) -> bool {
        match self {
            WordWrap::On => true,
            WordWrap::Off => false,
            WordWrap::Auto => !syntax_scope.starts_with("source."),
        }
    }
}

impl<'de> Deserialize<'de> for WordWrap {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Bool(true) => Ok(WordWrap::On),
            serde_json::Value::Bool(false) => Ok(WordWrap::Off),
            serde_json::Value::String(s) if s == "auto" => Ok(WordWrap::Auto),
            v => Err(D::Error::custom(format!("word_wrap must be true, false or \"auto\", not {v}"))),
        }
    }
}

impl Serialize for WordWrap {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::On => s.serialize_bool(true),
            Self::Off => s.serialize_bool(false),
            Self::Auto => s.serialize_str("auto"),
        }
    }
}

/// Where Sublime's `draw_white_space` marks white space: `"none"`, `"selection"`,
/// `"trailing"`, `"all"`, or an array of them, which is their union.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WhiteSpace {
    /// Inside the selections.
    pub selection: bool,
    /// After a line's last non-white character.
    pub trailing: bool,
    pub all: bool,
}

impl WhiteSpace {
    pub const NONE: Self = Self { selection: false, trailing: false, all: false };
    pub const SELECTION: Self = Self { selection: true, ..Self::NONE };
    pub const ALL: Self = Self { all: true, ..Self::NONE };

    /// Whether any white space can be marked.
    pub fn any(self) -> bool {
        self.selection || self.trailing || self.all
    }
}

impl<'de> Deserialize<'de> for WhiteSpace {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let names = match &value {
            serde_json::Value::Array(names) => names.as_slice(),
            name => std::slice::from_ref(name),
        };
        let mut out = WhiteSpace::NONE;
        for name in names {
            match name.as_str() {
                Some("none") => {}
                Some("selection") => out.selection = true,
                Some("trailing") => out.trailing = true,
                Some("all") => out.all = true,
                _ => {
                    return Err(D::Error::custom(format!(
                        "draw_white_space must be \"none\", \"selection\", \"trailing\", \"all\" \
                         or an array of them, not {value}"
                    )))
                }
            }
        }
        Ok(out)
    }
}

impl Serialize for WhiteSpace {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let names = [(self.selection, "selection"), (self.trailing, "trailing"), (self.all, "all")];
        s.collect_seq(names.iter().filter(|n| n.0).map(|n| n.1))
    }
}

/// `~/Library/Application Support/Orlo/Preferences.sublime-settings`
pub fn path() -> PathBuf {
    orlo_utils::paths::support_dir().join("Preferences.sublime-settings")
}

/// Whether `p` is the settings file, however it is spelled.
pub fn is_path(p: &Path) -> bool {
    let me = path();
    p == me || matches!((p.canonicalize(), me.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// Sublime's JSON dialect. Text that is not a JSON object is an error naming its line and
/// column; a key whose value Orlo cannot use keeps its default, and the second value names
/// those keys and their lines, for an alert.
pub fn parse(text: &str) -> Result<(Settings, Option<String>), String> {
    use serde_json::Value;
    // editors on Windows save JSON with a byte order mark, which serde_json rejects
    let json = strip_json(text.strip_prefix('\u{feff}').unwrap_or(text));
    // an emptied file means the defaults, as `{}` does
    if json.trim().is_empty() {
        return Ok((Settings::default(), None));
    }
    let Value::Object(map) = serde_json::from_str::<Value>(&json).map_err(|e| e.to_string())? else {
        return Err("The settings file must be one JSON object, { … }.".into());
    };
    let mut bad = Vec::new();
    let mut good = serde_json::Map::new();
    for (k, mut v) in map {
        if k == "language_servers" {
            if let Some(servers) = v.as_object_mut() {
                servers.retain(|family, value| match serde_json::from_value::<ServerConfig>(value.clone()) {
                    Ok(_) => true,
                    Err(e) => {
                        bad.push(format!("language_servers.{family} on line {}: {e}", line_of(&json, &k)));
                        false
                    }
                });
            }
        }
        if k == "formatters" {
            if let Some(formatters) = v.as_object_mut() {
                formatters.retain(|language, value| {
                    match serde_json::from_value::<server::FormatterCommand>(value.clone()) {
                        Ok(_) => true,
                        Err(e) => {
                            bad.push(format!("formatters.{language} on line {}: {e}", line_of(&json, &k)));
                            false
                        }
                    }
                });
            }
        }
        match usable(&k, &v) {
            Ok(()) => {
                good.insert(k, v);
            }
            Err(e) => bad.push(format!("\"{k}\" on line {}: {e}", line_of(&json, &k))),
        }
    }
    let s = Settings::default().overlay(&good)?;
    let warning = (!bad.is_empty()).then(|| format!("These keep their defaults:\n{}", bad.join("\n")));
    Ok((s, warning))
}

/// The 1-based line `"key":` is on.
pub fn line_of(json: &str, key: &str) -> usize {
    let quoted = format!("\"{key}\"");
    json.lines().position(|l| l.contains(&quoted)).map_or(1, |i| i + 1)
}

/// A whole number, also written as `4.0` (Sublime takes it).
fn whole<'de, D: Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    let n = f64::deserialize(d)?;
    match n >= 0. && n.fract() == 0. {
        true => Ok(n as usize),
        false => Err(D::Error::custom(format!("expected a whole number, not {n}"))),
    }
}

/// Whole numeric milliseconds, including JSON's `2000.0`, without truncation/saturation.
fn positive_milliseconds<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let n = f64::deserialize(d)?;
    if (1. ..=u32::MAX as f64).contains(&n) && n.fract() == 0. {
        Ok(n as u32)
    } else {
        Err(D::Error::custom("expected whole milliseconds in 1..=4294967295"))
    }
}

/// Sublime's rulers: columns, or `[column, "style"]` pairs (the style is not drawn).
fn rulers<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<usize>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Ruler {
        Column(f64),
        Styled(f64, serde::de::IgnoredAny),
    }
    Vec::<Ruler>::deserialize(d)?
        .into_iter()
        .map(|r| match r {
            Ruler::Column(c) | Ruler::Styled(c, _) if c >= 0. => Ok(c as usize),
            _ => Err(D::Error::custom("a ruler is a column")),
        })
        .collect()
}

/// Blank out `//` and `/* */` comments outside strings, and commas before `}` or `]`; line
/// breaks stay, so parse errors point at the right line.
pub fn strip_json(text: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut comma = None;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                comma = None;
                out.push(c);
                while let Some(c) = chars.next() {
                    out.push(c);
                    match c {
                        '\\' => out.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                while chars.next_if(|&c| c != '\n').is_some() {
                    out.push(' ');
                }
                out.push(' ');
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let start = out.len();
                out.extend([' ', ' ']);
                let mut prev = ' ';
                let mut closed = false;
                for c in chars.by_ref() {
                    out.push(if c == '\n' { '\n' } else { ' ' });
                    if prev == '*' && c == '/' {
                        closed = true;
                        break;
                    }
                    prev = c;
                }
                if !closed {
                    // Leave an invalid token at the opening delimiter, so serde reports
                    // its location instead of silently applying a partially written file.
                    out[start] = '/';
                }
            }
            '}' | ']' => {
                if let Some(i) = comma.take() {
                    out[i] = ' ';
                }
                out.push(c);
            }
            ',' => {
                comma = Some(out.len());
                out.push(c);
            }
            c => {
                if !c.is_whitespace() {
                    comma = None;
                }
                out.push(c);
            }
        }
    }
    out.into_iter().collect()
}

/// What Settings opens when the file does not exist yet: every key with its default.
pub const TEMPLATE: &str = r#"// Settings in here override Orlo's defaults. Uncomment a line to change it;
// saving this file applies it to every window.
{
	// "font_face": "Menlo",
	// "font_size": 12,
	// "tab_size": 4,
	// "translate_tabs_to_spaces": true,
	// "virtual_space": "off",   // "rectangular" or "all"; materialize padding only with text
	// "word_wrap": "auto",        // true, false, or "auto": wrap everything but source code
	// "wrap_width": 0,            // 0 wraps at the view's width; N wraps at most N columns
	// "rulers": [],               // columns, e.g. [80, 100]
	// "show_minimap": true,
	// "explorer_sort": "lexical", // or "natural": chapter-2 before chapter-10
	// "explorer_sticky_ancestors": false, // keep scrolled-out parent folders visible
	// "theme": "auto",            // follows macOS; or "Default.sublime-theme", "Default Dark.sublime-theme"
	// "color_scheme": "Mariana", // built in, or a user .tmTheme
	// "detect_indentation": true,
	// "modelines": false,      // limited declarative Vim/Emacs/Kate file preferences
	// "lsp_enabled": true,
	// "language_servers": {},   // server family: command argument array and initialization_options
	// "formatters": {},        // language id: argument array (stdin to stdout) for Format Document; [] turns one off
	// "terminal": {"shell": null, "arguments": null, "shell_integration": false, "scrollback": 10000, "profiles": {}, "default_profile": null}, // new sessions
	// "overrides": [{ "language": "Rust", "path": "**/tests/*.rs", "scope": "source.rust", "settings": { "tab_size": 2 } }],
	// "trim_trailing_white_space_on_save": false,   // true or "all"; or "not_on_caret": spare the caret's line
	// "file_autosave": false,                     // existing files; separate from recovery/history
	// "file_autosave_delay_ms": 2000,              // whole milliseconds, 1..=4294967295
	// "format_on_save": false,                    // LSP document formatting before save cleanup
	// "format_on_save_timeout_ms": 5000,          // positive whole milliseconds; failure leaves edits unsaved
	// "remove_trailing_blank_lines_on_save": false, // Orlo's own key
	// "ensure_newline_at_eof_on_save": false,
	// "animation_enabled": true,  // side bar slide, smooth Page Up/Down (off with Reduce Motion)
	// "fold_buttons": true,       // ▾ in the gutter beside foldable lines, ▸ beside folded ones
	// "fade_fold_buttons": true,  // ▾ only while the pointer is over the gutter
	// "line_number_mode": "absolute", // or "relative": source-line distance from the primary caret
	// "cursor_column_highlight": false, // faint bands at each caret display column
	// "unicode_warnings": true, // warning overlays; Edit > Character Info gives descriptions
	// "selection_occurrences": false, // other literal, case-sensitive matches of one single-line selection
	// "inline_diagnostics": false, // overlay one message after a visible source line end
	// "inline_diagnostics_severity": "warning", // error, warning, information, hint (inclusive)
	// "unicode_warning_characters": {"\u00a0": {"glyph": "!", "description": "Non-breaking space"}}, // replaces the default list
	// "unicode_warning_ranges": {"U+E000..U+E00F": {"glyph": "?", "description": "Private markers"}},
	// "draw_white_space": "selection", // "none", "selection", "trailing", "all", or an array of them
	// "drag_text": true,          // drag selected text to move it (option at the release copies)
	// "auto_complete": true,      // the completion list opens as a word is typed (ctrl-space opens it any time)
	// "auto_complete_min_chars": 2, // Orlo's key: the letters typed before it opens
	// "show_definitions": true,   // preview indexed definitions after hovering for 500 ms
	// "index_files": true,        // index the folders' symbols and words (open files are indexed either way)
	// "index_exclude_patterns": ["*.log"], // files the index leaves out, as .gitignore lines from each folder
	"file_history": false,      // opt-in private versions before saves
	// "spell_check": false,      // View > Spell Check (F6) overrides this in each view
	// "spelling_selector": "markup.raw, source string.quoted - punctuation - meta.preprocessor.include, source comment - source comment.block.preprocessor, -(constant, entity, keyword, punctuation, storage, support, variable, markup.underline.link, meta.tag)",
	// View > Enter Distraction Free Mode (ctrl-cmd-shift-f) shows:
	// "distraction_free": {
	// 	"full_screen": true, "show_sidebar": false, "show_tabs": false, "show_status_bar": false,
	// 	"show_minimap": false, "show_panel": false, "line_numbers": false,
	// 	"word_wrap": true, "wrap_width": 80, "draw_centered": true,
	// },
}
"#;

/// The settings at startup: a missing file gives the defaults, a broken one the defaults and
/// the error to show.
pub fn load() -> (Settings, Option<String>) {
    match std::fs::read_to_string(path()) {
        Ok(text) => match parse(&text) {
            Ok(s) => s,
            Err(e) => (Settings::default(), Some(e)),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
        Err(e) => (Settings::default(), Some(e.to_string())),
    }
}

impl gpui::Global for Settings {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n29_explorer_preferences_default_and_overlay_without_document_scopes() {
        let defaults = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(defaults["explorer_sort"], "lexical");
        assert_eq!(defaults["explorer_sticky_ancestors"], false);
        let s = ok(r#"{"explorer_sort":"natural","explorer_sticky_ancestors":true}"#);
        let layer = serde_json::json!({"font_size":19}).as_object().unwrap().clone();
        let merged = serde_json::to_value(s.overlay(&layer).unwrap()).unwrap();
        assert_eq!(merged["explorer_sort"], "natural");
        assert_eq!(merged["explorer_sticky_ancestors"], true);
        for key in ["explorer_sort", "explorer_sticky_ancestors"] {
            assert!(APPLICATION_SETTINGS.contains(&key));
        }
        warned(r#"{"explorer_sort":"random","font_size":19}"#, "explorer_sort");
        warned(r#"{"explorer_sticky_ancestors":"yes"}"#, "explorer_sticky_ancestors");
        warned(r#"{"overrides":[{"language":"Rust","settings":{"explorer_sort":"natural"}}]}"#, "overrides");
        warned(r#"{"overrides":[{"path":"*.rs","settings":{"explorer_sticky_ancestors":true}}]}"#, "overrides");
    }

    #[test]
    fn unicode_warnings_are_on_by_default() {
        assert!(ok("{}").unicode_warnings);
    }

    #[test]
    fn unicode_warning_ranges_survive_preferences_and_scope_resolution() {
        let s = ok(r#"{"unicode_warning_ranges":{"U+0300..U+036F":{"glyph":"?","description":"Combining marks"}},
            "overrides":[{"scope":"source.rust","settings":{"unicode_warnings":true,
            "unicode_warning_ranges":{"U+E000":{"glyph":"!","description":"Private marker"}}}}]}"#);
        let plain = serde_json::to_value(s.for_file(None, "Plain Text", "text.plain")).unwrap();
        assert_eq!(plain["unicode_warning_ranges"]["U+0300..U+036F"]["description"], "Combining marks");
        let rust = serde_json::to_value(s.for_file(None, "Rust", "source.rust")).unwrap();
        assert_eq!(rust["unicode_warning_ranges"]["U+E000"]["description"], "Private marker");
        assert_eq!(rust["unicode_warning_ranges"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn unicode_warning_ranges_reject_bad_keys_and_descriptors_recoverably() {
        for key in [
            "U+301",
            "u+0301",
            "0301",
            "U+00301 ",
            "U+ZZZZ",
            "U+110000",
            "U+D800",
            "U+DFFF",
            "U+E000..U+D7FF",
            "U+D7FF..U+E000",
            "U+0300...U+036F",
            "U+0300..",
            "U+0300..U+036F..U+0400",
        ] {
            let json = serde_json::json!({"unicode_warnings":true,"font_size":19,
                "unicode_warning_ranges":{key:{"glyph":"!","description":"bad"}}})
            .to_string();
            let (s, _) = warned(&json, "unicode_warning_ranges");
            assert!(s.unicode_warning_ranges.is_empty(), "{key}");
            assert!(s.unicode_warnings);
            assert_eq!(s.font_size, 19.);
            assert_eq!(s.unicode_warning_characters, unicode::default_characters());
        }
        for value in [
            r#"{"glyph":"!!","description":"bad"}"#,
            r#"{"glyph":"\t","description":"bad"}"#,
            r#"{"glyph":" ","description":"bad"}"#,
            r#"{"glyph":"!"}"#,
            r#"{"glyph":"!","description":4}"#,
        ] {
            let json = format!("{{\"unicode_warning_ranges\":{{\"U+E000\":{value}}}}}");
            assert!(warned(&json, "unicode_warning_ranges").0.unicode_warning_ranges.is_empty());
        }
        let (s, _) = warned(
            r#"{"unicode_warnings":true,"overrides":[{"language":"Rust","settings":{
            "unicode_warning_ranges":{"U+D800":{"glyph":"!","description":"invalid scalar"}}}}]}"#,
            "overrides",
        );
        assert!(s.unicode_warnings);
        assert!(s.for_file(None, "Rust", "source.rust").unicode_warning_ranges.is_empty());
    }

    #[test]
    fn unicode_warning_ranges_report_equivalent_keys_without_losing_other_preferences() {
        for alias in ["U+00e000", "U+E000..U+E000"] {
            for reverse in [false, true] {
                let first = r#""U+E000":{"glyph":"!","description":"first"}"#;
                let second = format!(r#""{alias}":{{"glyph":"?","description":"second"}}"#);
                let ranges = if reverse { format!("{{{second},{first}}}") } else { format!("{{{first},{second}}}") };
                let text = format!(r#"{{"font_size":19,"unicode_warnings":true,"unicode_warning_ranges":{ranges}}}"#);
                let (settings, error) = warned(&text, "duplicate warning range");
                assert!(error.contains("unicode_warning_ranges"));
                assert!(settings.unicode_warning_ranges.is_empty());
                assert!(settings.unicode_warnings);
                assert_eq!(settings.font_size, 19.);
                let scoped = format!(
                    r#"{{"font_size":19,"overrides":[{{"language":"Rust","settings":{{"unicode_warning_ranges":{ranges}}}}}]}}"#
                );
                let (settings, _) = warned(&scoped, "duplicate warning range");
                assert!(settings.for_file(None, "Rust", "source.rust").unicode_warning_ranges.is_empty());
                assert_eq!(settings.font_size, 19.);
            }
        }
    }

    #[test]
    fn unicode_warnings_defaults_round_trip_and_replace_scoped_lists() {
        let defaults = ok("{}");
        assert!(defaults.unicode_warnings);
        let warnings = &defaults.unicode_warning_characters;
        assert_eq!(warnings.len(), 21);
        for ch in ['\0', '\u{a0}', '\u{200b}', '\u{202e}', '\u{2067}', '\u{feff}'] {
            assert_eq!(warnings[&ch].glyph, '!');
            assert!(!warnings[&ch].description.is_empty());
        }
        for ch in " \t\r\ne\u{301}👩\u{200d}👧\u{fe0f}\u{e0100}\u{e0067}\u{200c}日本語العربية".chars() {
            assert!(!warnings.contains_key(&ch), "legitimate scalar U+{:04X}", ch as u32);
        }
        let round_trip = ok(&serde_json::to_string(&defaults).unwrap());
        assert_eq!(round_trip.unicode_warning_characters, *warnings);
        let scoped = ok(r#"{"unicode_warnings":false,"overrides":[{"language":"Rust","settings":{
            "unicode_warnings":true,"unicode_warning_characters":{"\u200d":{"glyph":"?","description":"Explicit joiner"}}
        }}]}"#);
        let rust = scoped.for_file(None, "Rust", "source.rust");
        assert!(rust.unicode_warnings);
        assert_eq!(rust.unicode_warning_characters.len(), 1);
        assert_eq!(rust.unicode_warning_characters[&'\u{200d}'].glyph, '?');
        assert!(!scoped.for_file(None, "Python", "source.python").unicode_warnings);
        assert!(ok(r#"{"unicode_warning_characters":{}}"#).unicode_warning_characters.is_empty());
    }

    #[test]
    fn unicode_warnings_reject_malformed_entries_and_describe_disabled_marks() {
        for value in [
            r#"{"ab":{"glyph":"!","description":"bad key"}}"#,
            r#"{"x":{"glyph":"!!","description":"bad glyph"}}"#,
            r#"{"x":{"glyph":"\n","description":"control"}}"#,
            r#"{"x":{"glyph":" ","description":"space"}}"#,
            r#"{"x":{"glyph":"!"}}"#,
            r#"{"x":{"glyph":"!","description":false}}"#,
            "[]",
            "null",
        ] {
            let text = format!("{{\"unicode_warning_characters\":{value}}}");
            let (s, _) = warned(&text, "unicode_warning_characters");
            assert_eq!(s.unicode_warning_characters, unicode::default_characters());
        }
        // a value it cannot use leaves the default: on
        let (s, _) = warned(r#"{"unicode_warnings":"yes"}"#, "unicode_warnings");
        assert!(s.unicode_warnings);
        let report = unicode::describe("\u{a0}", &s.unicode_warning_characters, false);
        assert!(report.contains("marks disabled"));
        assert!(report.contains("U+00A0 — Non-breaking space"));
        assert!(unicode::describe("e\u{301}👩\u{200d}👧", &s.unicode_warning_characters, true).is_empty());
    }

    #[test]
    fn modelines_are_opt_in_scoped_typed_and_backward_compatible() {
        assert_eq!(serde_json::to_value(ok("{}")).unwrap()["modelines"], false);
        let s = ok(
            r#"{"modelines":true,"virtual_space":true,"format_on_save":true,"unicode_warnings":false,"overrides":[{"language":"Rust","settings":{"modelines":false}}]}"#,
        );
        assert_eq!(serde_json::to_value(s.for_file(None, "Rust", "source.rust")).unwrap()["modelines"], false);
        assert_eq!(serde_json::to_value(s.for_file(None, "Python", "source.python")).unwrap()["modelines"], true);
        assert_eq!(serde_json::to_value(warned(r#"{"modelines":"yes"}"#, "modelines").0).unwrap()["modelines"], false);
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
        assert!(s.virtual_space.enabled() && s.format_on_save && !s.unicode_warnings);
    }

    #[test]
    fn virtual_space_is_opt_in_scoped_and_type_checked() {
        assert_eq!(ok("{}").virtual_space, VirtualSpace::Off);
        let settings =
            ok(r#"{"virtual_space":true,"overrides":[{"language":"Rust","settings":{"virtual_space":false}}]}"#);
        assert_eq!(settings.for_file(None, "Rust", "source.rust").virtual_space, VirtualSpace::Off);
        assert_eq!(settings.for_file(None, "Python", "source.python").virtual_space, VirtualSpace::All);
        assert_eq!(warned(r#"{"virtual_space":"yes"}"#, "virtual_space").0.virtual_space, VirtualSpace::Off);
        let encoded = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&encoded).unwrap().virtual_space, settings.virtual_space);
    }

    #[test]
    fn virtual_space_policies_preserve_legacy_and_scoped_settings() {
        let value = |text: &str| serde_json::to_value(ok(text)).unwrap()["virtual_space"].clone();
        assert_eq!(value("{}"), "off");
        assert_eq!(value(r#"{"virtual_space":false}"#), "off");
        assert_eq!(value(r#"{"virtual_space":true}"#), "all");
        for policy in ["off", "rectangular", "all"] {
            assert_eq!(value(&format!(r#"{{"virtual_space":"{policy}"}}"#)), policy);
        }
        let s = ok(
            r#"{"virtual_space":"all","overrides":[{"language":"Rust","settings":{"virtual_space":"rectangular"}}]}"#,
        );
        assert_eq!(
            serde_json::to_value(s.for_file(None, "Rust", "source.rust")).unwrap()["virtual_space"],
            "rectangular"
        );
        assert_eq!(serde_json::to_value(s.for_file(None, "Python", "source.python")).unwrap()["virtual_space"], "all");
        for value in ["null", "3", "[]", r#""yes""#] {
            assert_eq!(
                serde_json::to_value(warned(&format!(r#"{{"virtual_space":{value}}}"#), "virtual_space").0).unwrap()
                    ["virtual_space"],
                "off"
            );
        }
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
    }

    #[test]
    fn u07_cursor_columns_are_off_by_default_scoped_and_validated() {
        let value = |s: &Settings| serde_json::to_value(s).unwrap()["cursor_column_highlight"].clone();
        assert_eq!(value(&ok("{}")), false);
        let s = ok(
            r#"{"cursor_column_highlight":true,"overrides":[{"language":"Rust","settings":{"cursor_column_highlight":false}}]}"#,
        );
        assert_eq!(value(&s), true);
        assert_eq!(value(&s.for_file(None, "Rust", "source.rust")), false);
        assert_eq!(value(&s.for_file(None, "Python", "source.python")), true);
        for invalid in ["null", "1", "[]", r#""yes""#] {
            assert_eq!(
                value(&warned(&format!(r#"{{"cursor_column_highlight":{invalid}}}"#), "cursor_column_highlight").0),
                false
            );
        }
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
    }

    #[test]
    fn u06_line_number_mode_defaults_scopes_and_rejects_invalid_values() {
        let value = |s: &Settings| serde_json::to_value(s).unwrap()["line_number_mode"].clone();
        assert_eq!(value(&ok("{}")), "absolute");
        let s = ok(
            r#"{"line_number_mode":"relative","overrides":[{"language":"Rust","settings":{"line_number_mode":"absolute"}}]}"#,
        );
        assert_eq!(value(&s), "relative");
        assert_eq!(value(&s.for_file(None, "Rust", "source.rust")), "absolute");
        assert_eq!(value(&s.for_file(None, "Python", "source.python")), "relative");
        for invalid in ["null", "true", "1", "[]", r#""modal""#] {
            let (s, _) = warned(&format!(r#"{{"line_number_mode":{invalid}}}"#), "line_number_mode");
            assert_eq!(value(&s), "absolute");
        }
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
    }

    #[test]
    fn inline_diagnostics_is_opt_in_with_scoped_severity() {
        let settings = ok(
            r#"{"overrides":[{"language":"Rust","settings":{"inline_diagnostics":true,"inline_diagnostics_severity":"error"}}]}"#,
        );
        let value = |s: &Settings| serde_json::to_value(s).unwrap();
        assert_eq!(value(&settings)["inline_diagnostics"], false);
        assert_eq!(value(&settings)["inline_diagnostics_severity"], "warning");
        let rust = value(&settings.for_file(None, "Rust", "source.rust"));
        assert_eq!(rust["inline_diagnostics"], true);
        assert_eq!(rust["inline_diagnostics_severity"], "error");
        assert_eq!(value(&settings.for_file(None, "Python", "source.python"))["inline_diagnostics"], false);
        for severity in ["error", "warning", "information", "hint"] {
            assert_eq!(
                value(&ok(&format!(r#"{{"inline_diagnostics_severity":"{severity}"}}"#)))
                    ["inline_diagnostics_severity"],
                severity
            );
        }
        for invalid in ["null", "1", r#""all""#] {
            let (s, _) =
                warned(&format!(r#"{{"inline_diagnostics_severity":{invalid}}}"#), "inline_diagnostics_severity");
            assert_eq!(value(&s)["inline_diagnostics_severity"], "warning");
        }
        assert!(!value(&warned(r#"{"inline_diagnostics":"yes"}"#, "inline_diagnostics").0)["inline_diagnostics"]
            .as_bool()
            .unwrap());
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&settings).unwrap()).unwrap(), settings);
    }

    #[test]
    fn selection_occurrences_is_opt_in_and_scoped() {
        let settings = ok(r#"{"overrides":[{"language":"Rust","settings":{"selection_occurrences":true}}]}"#);
        let enabled = |s: &Settings| serde_json::to_value(s).unwrap()["selection_occurrences"].clone();
        assert_eq!(enabled(&settings), false);
        assert_eq!(enabled(&settings.for_file(None, "Rust", "source.rust")), true);
        assert_eq!(enabled(&settings.for_file(None, "Python", "source.python")), false);
        let (settings, _) = warned(r#"{"selection_occurrences":"yes"}"#, "selection_occurrences");
        assert_eq!(enabled(&settings), false);
    }

    #[test]
    fn file_autosave_is_opt_in_and_scoped() {
        let settings = ok(r#"{"overrides":[{"language":"Rust","settings":{"file_autosave":true}}]}"#);
        let enabled = |s: &Settings| serde_json::to_value(s).unwrap()["file_autosave"].clone();
        assert_eq!(enabled(&settings), false);
        assert_eq!(enabled(&settings.for_file(None, "Rust", "source.rust")), true);
        assert_eq!(enabled(&settings.for_file(None, "Python", "source.python")), false);
        let (settings, _) = warned(r#"{"file_autosave":"yes"}"#, "file_autosave");
        assert_eq!(enabled(&settings), false);
    }

    #[test]
    fn file_autosave_delay_defaults_validates_and_resolves_scopes() {
        let delay = |s: &Settings| serde_json::to_value(s).unwrap()["file_autosave_delay_ms"].clone();
        assert_eq!(delay(&ok("{}")), 2000);
        for (value, expected) in [("1", 1u64), ("250.0", 250), ("4294967295", 4294967295)] {
            let settings = ok(&format!(r#"{{"file_autosave_delay_ms":{value}}}"#));
            assert_eq!(delay(&settings), expected);
            let encoded = serde_json::to_string(&settings).unwrap();
            assert_eq!(delay(&serde_json::from_str::<Settings>(&encoded).unwrap()), delay(&settings));
        }
        for value in ["0", "-1", "1.5", "4294967296", "1e100", "true", "null", "[]", r#""250""#] {
            let (settings, _) = warned(
                &format!(r#"{{"file_autosave":true,"file_autosave_delay_ms":{value}}}"#),
                "file_autosave_delay_ms",
            );
            assert!(settings.file_autosave);
            assert_eq!(delay(&settings), 2000);
            let scoped = format!(
                r#"{{"file_autosave_delay_ms":750,"overrides":[{{"language":"Rust","settings":{{"file_autosave_delay_ms":{value}}}}}]}}"#
            );
            let (settings, _) = warned(&scoped, "overrides");
            assert_eq!(delay(&settings.for_file(None, "Rust", "source.rust")), 750);
        }
        let settings = ok(
            r#"{"file_autosave_delay_ms":750,"overrides":[{"language":"Rust","path":"*.rs","scope":"source.rust","settings":{"file_autosave":true,"file_autosave_delay_ms":125}}]}"#,
        );
        let rust = settings.for_file(Some(Path::new("example.rs")), "Rust", "source.rust");
        assert!(rust.file_autosave);
        assert_eq!(delay(&rust), 125);
        assert_eq!(delay(&settings.for_file(Some(Path::new("example.txt")), "Rust", "source.rust")), 750);
    }

    #[test]
    fn format_on_save_is_opt_in_and_can_be_scoped() {
        let settings = ok(r#"{"overrides":[{"language":"Rust","settings":{"format_on_save":true}}]}"#);
        let enabled = |settings| serde_json::to_value(settings).unwrap()["format_on_save"].clone();
        assert_eq!(enabled(Settings::default()), false);
        assert_eq!(enabled(settings.for_file(None, "Rust", "source.rust")), true);
        assert_eq!(enabled(settings.for_file(None, "Python", "source.python")), false);
        let (settings, _) = warned(r#"{"format_on_save":"yes"}"#, "format_on_save");
        assert_eq!(enabled(settings), false);
    }

    #[test]
    fn format_on_save_deadline_is_validated_scoped_and_persisted() {
        let timeout = |s: &Settings| serde_json::to_value(s).unwrap()["format_on_save_timeout_ms"].clone();
        assert_eq!(timeout(&Settings::default()), 5000);
        let s = ok(
            r#"{"format_on_save_timeout_ms":750,"overrides":[{"language":"Rust","settings":{"format_on_save_timeout_ms":125}}]}"#,
        );
        assert_eq!(timeout(&s.for_file(None, "Rust", "source.rust")), 125);
        assert_eq!(timeout(&s.for_file(None, "Python", "source.python")), 750);
        let encoded = serde_json::to_string(&s).unwrap();
        assert_eq!(timeout(&serde_json::from_str::<Settings>(&encoded).unwrap()), 750);
        for v in ["0", "-1", "1.5", "4294967296", "true", "null", r#""250""#] {
            let (s, _) = warned(&format!(r#"{{"format_on_save_timeout_ms":{v}}}"#), "format_on_save_timeout_ms");
            assert_eq!(timeout(&s), 5000);
        }
    }

    #[test]
    fn spelling_defaults_and_file_overrides() {
        let defaults = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(defaults["spell_check"], false);
        assert_eq!(defaults["spelling_selector"], "markup.raw, source string.quoted - punctuation - meta.preprocessor.include, source comment - source comment.block.preprocessor, -(constant, entity, keyword, punctuation, storage, support, variable, markup.underline.link, meta.tag)");
        let settings =
            ok(r#"{"overrides":[{"language":"Rust","settings":{"spell_check":true,"spelling_selector":"comment"}}]}"#);
        let rust = settings.for_file(None, "Rust", "source.rust");
        let rust = serde_json::to_value(rust).unwrap();
        assert_eq!(rust["spell_check"], true);
        assert_eq!(rust["spelling_selector"], "comment");
    }

    fn ok(text: &str) -> Settings {
        let (s, warning) = parse(text).unwrap();
        assert_eq!(warning, None, "{text}");
        s
    }

    /// The settings and the warning parsing `text` gives, which must contain `named`.
    fn warned(text: &str, named: &str) -> (Settings, String) {
        let (s, warning) = parse(text).unwrap();
        let warning = warning.unwrap_or_else(|| panic!("{text} was accepted"));
        assert!(warning.contains(named), "{warning}");
        (s, warning)
    }

    /// `key` is application-wide and listed in the template.
    fn application_key(key: &str) {
        assert!(TEMPLATE.contains(&format!("\"{key}\"")), "{key}");
        assert!(APPLICATION_SETTINGS.contains(&key), "{key}");
    }

    #[test]
    fn a_byte_order_mark_is_not_a_settings_error() {
        assert_eq!(ok("\u{feff}{\"tab_size\": 2}").tab_size, 2);
    }

    #[test]
    fn an_empty_or_comment_only_file_is_all_defaults() {
        assert_eq!(ok(""), Settings::default());
        assert_eq!(ok("  // nothing yet\n"), Settings::default());
    }

    #[test]
    fn theme_follows_macos_unless_named() {
        let dark = |v: &str, os| ok(&format!("{{\"theme\": {v}}}")).dark(os);
        assert!(!dark("\"auto\"", false) && dark("\"auto\"", true));
        assert!(!dark("\"Default.sublime-theme\"", true));
        assert!(dark("\"Default Dark.sublime-theme\"", false));
        assert!(dark("\"Adaptive.sublime-theme\"", true), "unknown: as auto");
    }

    #[test]
    fn empty_object_is_all_defaults() {
        assert_eq!(ok("{}"), Settings::default());
    }

    #[test]
    fn unterminated_block_comments_are_errors() {
        for text in ["/*", "/* unfinished", "{} /* unfinished", "{\"tab_size\": 2} /*"] {
            assert!(parse(text).is_err(), "accepted malformed settings: {text:?}");
        }
        assert_eq!(ok("/* complete */"), Settings::default());
    }

    #[test]
    fn comments_and_trailing_commas_are_allowed() {
        let s = ok("// c\n{ \"tab_size\": 2, /* x */ \"rulers\": [80, 100,], }\n");
        assert_eq!((s.tab_size, s.rulers.clone()), (2, vec![80, 100]));
    }

    #[test]
    fn slashes_inside_strings_survive() {
        assert_eq!(ok(r#"{"font_face": "A//B"}"#).font_face, "A//B");
        assert_eq!(ok(r#"{"font_face": "A\"/*B"}"#).font_face, "A\"/*B");
    }

    #[test]
    fn word_wrap_accepts_bool_and_auto() {
        let ww = |v: &str| parse(&format!("{{\"word_wrap\": {v}}}")).map(|s| s.0.word_wrap);
        assert_eq!(ww("true"), Ok(WordWrap::On));
        assert_eq!(ww("false"), Ok(WordWrap::Off));
        assert_eq!(ww("\"auto\""), Ok(WordWrap::Auto));
        let (s, _) = warned("{\"word_wrap\": \"x\"}", "word_wrap");
        assert_eq!(s.word_wrap, WordWrap::Auto);
    }

    #[test]
    fn a_value_orlo_cannot_use_keeps_its_default_and_is_named_with_its_line() {
        let text = "{\n\"font_size\": \"big\",\n\"tab_size\": 2\n}";
        let (s, _) = warned(text, "\"font_size\" on line 2");
        assert_eq!((s.font_size, s.tab_size), (12., 2), "the rest still applies");
        // comments keep line numbers
        warned("{\n  /* a\n  note */ \"tab_size\": \"x\"\n}", "line 3");
        // broken JSON is still an error with its place
        assert!(parse("{\n\"tab_size\": 2 3\n}").unwrap_err().contains("line 2"));
    }

    #[test]
    fn values_sublime_takes_are_taken() {
        let s = ok(r#"{"rulers": [[80, "solid"], 100], "tab_size": 4.0, "font_face": ""}"#);
        assert_eq!((s.rulers, s.tab_size, s.font_face.as_str()), (vec![80, 100], 4, "Menlo"));
    }

    #[test]
    fn template_parses_to_defaults() {
        assert_eq!(ok(TEMPLATE), Settings::default());
    }

    #[test]
    fn save_cleanup_settings_take_what_sublime_takes_and_name_a_bad_value() {
        let trim = |v: &str| ok(&format!("{{\"trim_trailing_white_space_on_save\": {v}}}"));
        for (v, want) in [
            ("false", Trim::None),
            ("\"none\"", Trim::None),
            ("true", Trim::All),
            ("\"all\"", Trim::All),
            ("\"not_on_caret\"", Trim::NotOnCaret),
        ] {
            assert_eq!(trim(v).trim_trailing_white_space_on_save, want, "{v}");
        }
        let s = ok(r#"{"remove_trailing_blank_lines_on_save": true, "ensure_newline_at_eof_on_save": true}"#);
        assert_eq!(
            s.cleanup(),
            Cleanup {
                trim: Trim::None,
                remove_trailing_blank_lines: true,
                ensure_newline: true,
                remove_final_newline: false,
            }
        );
        let (s, _) = warned(
            "{\n\"trim_trailing_white_space_on_save\": \"caret\"\n}",
            "\"trim_trailing_white_space_on_save\" on line 2",
        );
        assert_eq!(s.cleanup(), Cleanup::default(), "off, as before");
        warned(r#"{"ensure_newline_at_eof_on_save": "yes"}"#, "ensure_newline_at_eof_on_save");
        // all three are off by default and listed in the template
        assert_eq!(Settings::default().cleanup(), Cleanup::default());
        for key in [
            "trim_trailing_white_space_on_save",
            "remove_trailing_blank_lines_on_save",
            "ensure_newline_at_eof_on_save",
        ] {
            assert!(TEMPLATE.contains(key), "{key}");
        }
    }

    #[test]
    fn formatters_are_application_commands_keyed_by_language_and_bad_entries_keep_their_siblings() {
        application_key("formatters");
        assert!(Settings::default().formatters.is_empty());
        let s = ok(r#"{"formatters": {"nim": ["nph", "-"], "go": []}}"#);
        assert_eq!(s.formatters["nim"], ["nph", "-"]);
        assert!(s.formatters["go"].is_empty(), "[] turns a language's formatter off");
        for bad in [r#"[""]"#, r#"["  "]"#, r#"["nph", "a\u0000b"]"#, r#""nph -""#] {
            let (s, warning) =
                warned(&format!(r#"{{"formatters": {{"nim": ["nph"], "pony": {bad}}}}}"#), "formatters.pony");
            assert_eq!(s.formatters.keys().collect::<Vec<_>>(), ["nim"], "{bad}");
            assert!(!warning.contains("formatters.nim"), "{warning}");
        }
        warned(r#"{"formatters": ["nph"]}"#, "formatters");
        let scoped = r#"{"overrides":[{"language":"Nim","settings":{"formatters":{"nim":["nph"]}}}]}"#;
        let (s, _) = warned(scoped, "application-wide");
        assert!(s.overrides.is_empty());
    }

    #[test]
    fn fold_buttons_are_on_by_default_and_take_booleans() {
        let s = Settings::default();
        assert!(s.fold_buttons && s.fade_fold_buttons);
        let s = ok(r#"{"fold_buttons": false, "fade_fold_buttons": false}"#);
        assert!(!s.fold_buttons && !s.fade_fold_buttons);
        let (s, _) = warned(r#"{"fold_buttons": "yes"}"#, "\"fold_buttons\" on line 1");
        assert!(s.fold_buttons, "a bad value keeps the default");
        application_key("fold_buttons");
        application_key("fade_fold_buttons");
    }

    #[test]
    fn auto_complete_is_on_after_two_letters_by_default() {
        let s = Settings::default();
        assert!(s.auto_complete);
        assert_eq!(s.auto_complete_min_chars, 2);
        let s = ok(r#"{"auto_complete": false, "auto_complete_min_chars": 4}"#);
        assert!(!s.auto_complete);
        assert_eq!(s.auto_complete_min_chars, 4);
        let (s, _) = warned(r#"{"auto_complete_min_chars": 1.5}"#, "\"auto_complete_min_chars\" on line 1");
        assert_eq!(s.auto_complete_min_chars, 2, "a bad value keeps the default");
        application_key("auto_complete");
        application_key("auto_complete_min_chars");
    }

    #[test]
    fn the_index_is_on_and_leaves_out_log_files_by_default() {
        let s = Settings::default();
        assert!(s.index_files);
        assert_eq!(s.index_exclude_patterns, ["*.log"], "Sublime's default");
        let s = ok(r#"{"index_files": false, "index_exclude_patterns": ["gen/**", "*.min.js"]}"#);
        assert!(!s.index_files);
        assert_eq!(s.index_exclude_patterns, ["gen/**", "*.min.js"]);
        let (s, _) = warned(r#"{"index_exclude_patterns": "*.log"}"#, "\"index_exclude_patterns\" on line 1");
        assert_eq!(s.index_exclude_patterns, ["*.log"], "a bad value keeps the default");
        application_key("index_files");
        application_key("index_exclude_patterns");
    }

    #[test]
    fn drag_text_is_on_by_default_and_takes_a_boolean() {
        assert!(Settings::default().drag_text);
        assert!(!ok(r#"{"drag_text": false}"#).drag_text);
        let (s, _) = warned(r#"{"drag_text": "yes"}"#, "\"drag_text\" on line 1");
        assert!(s.drag_text, "a bad value keeps the default");
        application_key("drag_text");
    }

    #[test]
    fn draw_white_space_takes_a_name_or_an_array_of_them() {
        assert_eq!(Settings::default().draw_white_space, WhiteSpace::SELECTION);
        let ws = |v: &str| ok(&format!("{{\"draw_white_space\": {v}}}")).draw_white_space;
        assert_eq!(ws("\"none\""), WhiteSpace::NONE);
        assert_eq!(ws("\"selection\""), WhiteSpace::SELECTION);
        assert_eq!(ws("\"trailing\""), WhiteSpace { trailing: true, ..WhiteSpace::NONE });
        assert_eq!(ws("\"all\""), WhiteSpace::ALL);
        // an array is the union of its names
        let both = WhiteSpace { selection: true, trailing: true, all: false };
        assert_eq!(ws(r#"["selection", "trailing"]"#), both);
        assert_eq!(ws(r#"["trailing", "selection", "trailing"]"#), both);
        assert_eq!(ws(r#"["none"]"#), WhiteSpace::NONE);
        assert_eq!(ws("[]"), WhiteSpace::NONE);
        assert!(ws(r#"["selection", "all"]"#).all);
        // a value Orlo cannot use keeps the default, and the warning lists the ones it can
        for bad in ["\"everything\"", "true", "7", r#"["all", 3]"#, r#"["al"]"#] {
            let text = format!("{{\n\"draw_white_space\": {bad}\n}}");
            let (s, warning) = warned(&text, "\"draw_white_space\" on line 2");
            assert_eq!(s.draw_white_space, WhiteSpace::SELECTION, "{bad}");
            assert!(warning.contains("\"trailing\""), "{warning}");
        }
        application_key("draw_white_space");
    }

    #[test]
    fn draw_white_space_survives_the_merge_of_scoped_settings() {
        // `overlay` writes the settings out as JSON and reads them back
        for ws in [
            WhiteSpace::NONE,
            WhiteSpace::SELECTION,
            WhiteSpace::ALL,
            WhiteSpace { selection: true, trailing: true, all: false },
        ] {
            let s = Settings { draw_white_space: ws, ..Settings::default() };
            let merged = s.overlay(&serde_json::Map::new()).unwrap();
            assert_eq!(merged.draw_white_space, ws);
        }
    }

    #[test]
    fn auto_wraps_everything_but_source_code() {
        assert!(WordWrap::Auto.wraps("text.plain"));
        assert!(WordWrap::Auto.wraps("text.html.markdown"));
        assert!(!WordWrap::Auto.wraps("source.rust"));
        assert!(WordWrap::On.wraps("source.rust"));
        assert!(!WordWrap::Off.wraps("text.plain"));
    }

    #[test]
    fn tab_size_and_font_size_are_clamped() {
        let s = ok(r#"{"tab_size": 0, "font_size": 500}"#);
        assert_eq!((s.tab_size, s.font_size), (1, 72.));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        assert_eq!(ok(r#"{"color_scheme": "x", "tab_size": 3}"#).tab_size, 3);
    }

    #[test]
    fn scoped_preferences_match_language_path_and_syntax_and_keep_unspecified_values() {
        let s = ok(r#"{
            "font_size": 18,
            "overrides": [
                {"language": "Rust", "settings": {"tab_size": 2, "word_wrap": false}},
                {"scope": "source.rust", "path": "**/tests/*.rs", "settings": {"tab_size": 3}},
                {"path": "*.md", "settings": {"word_wrap": true}}
            ]
        }"#);
        let at = |path: &str, language, scope| s.for_file(Some(Path::new(path)), language, scope);
        let rust = at("/app/tests/one.rs", "Rust", "source.rust");
        assert_eq!((rust.tab_size, rust.font_size, rust.word_wrap), (3, 18., WordWrap::Off));
        assert_eq!(at("/app/main.rs", "Rust", "source.rust").tab_size, 2);
        assert_eq!(at("/app/tests/one.py", "Python", "source.python").tab_size, 4);
        let markdown = at("/app/notes.md", "Markdown", "text.html.markdown");
        assert_eq!(markdown.word_wrap, WordWrap::On);
        assert_eq!(s.for_file(None, "Rust", "source.rust").tab_size, 2);
    }

    #[test]
    fn invalid_scoped_preferences_are_visible_and_do_not_replace_global_preferences() {
        let (s, _) = warned(
            r#"{"tab_size": 8, "overrides": [{"language": "Rust", "settings": {"tab_size": "wrong"}}]}"#,
            "overrides",
        );
        assert_eq!(s.for_file(None, "Rust", "source.rust").tab_size, 8);
    }

    #[test]
    fn application_appearance_is_rejected_in_scoped_preferences() {
        for key in APPLICATION_SETTINGS {
            let text = serde_json::json!({
                "overrides": [{"scope":"source.rust", "settings": {(*key): "example"}}]
            });
            let (settings, warning) = warned(&text.to_string(), "application-wide");
            assert!(warning.contains(key), "{warning}");
            assert!(settings.overrides.is_empty());
        }
    }
}
