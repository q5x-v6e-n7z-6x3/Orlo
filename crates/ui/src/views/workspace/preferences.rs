//! Buffer preferences: detection < configured global/scoped preferences < project <
//! EditorConfig < modelines < explicit window/buffer/view choices. Detection never edits existing text.

use super::*;
use crate::config::editorconfig::{self, Properties};
use crate::config::settings::{modelines, Indentation};

/// Avoid allocating or walking an oversized line, even when it contains a marker.
pub(crate) fn scan_modelines(rope: &ropey::Rope) -> modelines::Properties {
    // Ropey's terminal empty display row is not another physical source record.
    let terminated = rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n';
    modelines::scan(rope.len_lines() - usize::from(terminated), |i| {
        let line = rope.line(i);
        (line.len_bytes() <= modelines::MAX_LINE_BYTES).then(|| line.to_string())
    })
}

/// Resolve data against the existing catalog only; never treat a declaration as a path or command.
fn modeline_language(
    value: &str,
    registry: &crate::languages::registry::Registry,
    syntaxes: &[String],
) -> Option<(String, String)> {
    type Def = std::sync::Arc<crate::languages::registry::LanguageDef>;
    // a language's id or name, else the id its server knows it by (`shellscript`)
    let language = |value: &str| {
        let languages = registry.languages();
        let named = |l: &&Def| l.id().eq_ignore_ascii_case(value) || l.name().eq_ignore_ascii_case(value);
        let lsp = |l: &&Def| l.manifest.lsp.as_ref().is_some_and(|p| p.language_id.eq_ignore_ascii_case(value));
        let l = languages.iter().find(named).or_else(|| languages.iter().find(lsp))?;
        let syntax =
            syntaxes.iter().find(|name| name.as_str() == l.name()).cloned().unwrap_or_else(|| "Plain Text".into());
        Some((l.id().into(), syntax))
    };
    if let Some(found) = language(value) {
        return Some(found);
    }
    let value = value.strip_suffix("-mode").unwrap_or(value);
    let alias = match value.to_ascii_lowercase().as_str() {
        "sh" | "shell" => "bash",
        "js" => "javascript",
        "ts" => "typescript",
        "c++" => "cpp",
        "c#" | "cs" => "csharp",
        "text" | "plain" | "plaintext" => "Plain Text",
        _ => value,
    };
    language(alias).or_else(|| {
        syntaxes.iter().find(|name| name.eq_ignore_ascii_case(alias)).map(|name| (name.clone(), name.clone()))
    })
}

/// The indentation the text uses, guessed from its first 2,000 lines (blank ones skipped): tabs
/// give the style only, never a tab width. `None` when the lines do not say.
pub fn infer(rope: &ropey::Rope) -> Option<Indentation> {
    let (mut tabs, mut spaces) = (0, 0);
    let mut votes = [0usize; 17];
    let mut previous = 0;
    for line in rope.lines().take(2_000) {
        let mut count = 0;
        let mut tab = false;
        let mut nonblank = false;
        for c in line.chars().take(512) {
            match c {
                ' ' => count += 1,
                '\t' => tab = true,
                '\n' | '\r' => break,
                _ => {
                    nonblank = true;
                    break;
                }
            }
        }
        if !nonblank {
            continue;
        }
        if tab {
            tabs += 1;
            previous = 0;
            continue;
        }
        if count > 0 {
            spaces += 1;
            if count <= 16 {
                votes[count] += 1;
            }
        }
        let difference = count.abs_diff(previous);
        if (1..=16).contains(&difference) {
            votes[difference] += 2;
        }
        previous = count;
    }
    if tabs >= 2 && tabs > spaces {
        Some(Indentation { size: None, spaces: false })
    } else if spaces >= 2 && spaces >= tabs {
        let width = (1..=16).max_by_key(|&n| (votes[n], std::cmp::Reverse(n)))?;
        (votes[width] > 0).then_some(Indentation { size: Some(width), spaces: true })
    } else {
        None
    }
}

impl Workspace {
    pub(crate) fn settings_for_path(&self, e: &Editor, path: Option<&Path>, cx: &App) -> Settings {
        self.settings_for_path_with_modeline(e, path, true, cx)
    }

    /// The registered language of `e` saved at `path` (when one is), and the name and scope that
    /// file preferences match.
    pub(crate) fn file_language(
        &self,
        e: &Editor,
        path: Option<&Path>,
        declared: bool,
        cx: &App,
    ) -> (Option<std::sync::Arc<crate::languages::registry::LanguageDef>>, String, String) {
        let detected;
        let use_declared = declared && path == e.doc(cx).path.as_deref() && e.highlighter.language_override().is_none();
        let choice = use_declared.then_some(e.modeline_language.as_deref()).flatten();
        let highlighter = if path != e.doc(cx).path.as_deref()
            || (!declared && e.modeline_language.is_some() && e.highlighter.language_override().is_none())
        {
            detected = Highlighter::detect(path, &editor::first_line(&e.doc(cx).rope));
            &detected
        } else {
            &e.highlighter
        };
        let registry = crate::languages::registry::registry();
        let manifest = if let Some(explicit) = highlighter.language_override() {
            registry.get(explicit)
        } else if let Some(choice) = choice {
            registry.get(choice)
        } else if highlighter.language_override().is_none() && !registry.languages().is_empty() {
            let head = e
                .doc(cx)
                .rope
                .slice(..e.doc(cx).rope.len_chars().min(crate::languages::registry::CONTENT_BYTES))
                .to_string();
            registry.detect(path, &head)
        } else {
            None
        };
        let (language, scope) = manifest.map_or_else(
            || (highlighter.syntax_name().to_string(), highlighter.scope()),
            |l| (l.name().to_string(), l.manifest.scope.clone()),
        );
        (manifest.cloned(), language, scope)
    }

    fn settings_for_path_with_modeline(&self, e: &Editor, path: Option<&Path>, declared: bool, cx: &App) -> Settings {
        let (manifest, language, scope) = self.file_language(e, path, declared, cx);
        let language = language.as_str();
        let mut s = cx.global::<Settings>().for_file(path, language, &scope);
        if let Some(project) = &self.project {
            // Global rules have already run. Only this project's own rules run after its layer.
            s.overrides.clear();
            s = s
                .overlay(&project.settings.preferences)
                .expect("project preferences were validated")
                .for_file(path, language, &scope);
        }
        // The language's defaults stand in for Orlo's where no layer set indentation; detection,
        // EditorConfig and modelines still apply over them.
        if let Some(indent) = manifest.as_ref().and_then(|l| l.manifest.indent.as_ref()) {
            let [size_set, spaces_set] = s.indentation_explicit;
            if let Some(size) = indent.tab_size.filter(|_| !size_set) {
                s.tab_size = size;
            }
            if let Some(spaces) = indent.translate_tabs_to_spaces.filter(|_| !spaces_set) {
                s.translate_tabs_to_spaces = spaces;
            }
        }
        s
    }

    pub(crate) fn editor_settings(&self, e: &Editor, cx: &App) -> Settings {
        self.settings_for_path(e, e.doc(cx).path.as_deref(), cx)
    }

    /// `e`'s display: its file's settings, with the window's menu choices over them.
    pub(crate) fn view_for_editor(&self, e: &Editor, cx: &App) -> ViewSettings {
        let s = self.editor_settings(e, cx);
        let mut view = self.effective_view(cx);
        view.font_face = s.font_face.into();
        view.font_size = self.overrides.font_size.unwrap_or(s.font_size);
        view.minimap = self.overrides.minimap.unwrap_or(s.show_minimap);
        view.line_number_mode = self.overrides.line_number_mode.unwrap_or(s.line_number_mode);
        view.cursor_column_highlight = self.overrides.cursor_column_highlight.unwrap_or(s.cursor_column_highlight);
        let modeline = e.modelines_enabled.then_some(&e.modelines);
        let margin = modeline.and_then(|p| p.margin);
        let rulers = margin.map(|n| if n == 0 { Vec::new() } else { vec![n] }).unwrap_or(s.rulers);
        view.rulers = self.overrides.rulers.clone().unwrap_or(rulers);
        view.unicode_warnings = s.unicode_warnings;
        view.selection_occurrences = s.selection_occurrences;
        view.inline_diagnostics = s.inline_diagnostics;
        view.inline_diagnostics_severity = s.inline_diagnostics_severity;
        view.unicode_warning_characters = s.unicode_warning_characters;
        view.unicode_warning_ranges = s.unicode_warning_ranges;
        view.spell_check = s.spell_check;
        view.spelling_selector = s.spelling_selector;
        view.animate = s.animation_enabled && !cx.try_global::<view::ReduceMotion>().is_some_and(|r| r.0);
        if self.distraction_free.is_none() {
            view.word_wrap = modeline.map_or(s.word_wrap, |p| p.wrapping(s.word_wrap));
            view.wrap_width = margin.unwrap_or(s.wrap_width);
        }
        view
    }

    /// New/opened files take their repository's save format without rewriting their text.
    pub(crate) fn prepare_editor(&self, e: &mut Editor, cx: &mut Context<Editor>) -> Vec<String> {
        let inferred = infer(&e.doc(cx).rope);
        e.update_doc(cx, |d| d.inferred_indentation = inferred);
        let (policy, warnings) = editorconfig::load(e.doc(cx).path.as_deref());
        self.apply_editor_properties(e, &policy, cx);
        e.update_doc(cx, |d| {
            if !d.unsaved() && !d.edits_blocked() {
                if let Some(ending) = policy.line_ending {
                    d.line_ending = ending;
                }
                if let Some(bom) = policy.bom {
                    d.bom = bom;
                }
                // still clean: the next save writes the file that way, unasked
                d.mark_saved();
            }
        });
        warnings
    }

    /// Live settings/project/syntax changes preserve explicit buffer and view choices.
    pub(crate) fn apply_editor_preferences(&self, e: &mut Editor, cx: &mut Context<Editor>) {
        let (policy, _) = editorconfig::load(e.doc(cx).path.as_deref());
        self.apply_editor_properties(e, &policy, cx);
    }

    pub(crate) fn apply_editor_properties(&self, e: &mut Editor, policy: &Properties, cx: &mut Context<Editor>) {
        self.derive_modelines(e, cx);
        let settings = self.editor_settings(e, cx);
        let guess = settings.detect_indentation.then_some(e.doc(cx).inferred_indentation).flatten();
        // Explicit settings beat detection; modelines and EditorConfig overlay supported fields.
        let [size_set, spaces_set] = settings.indentation_explicit;
        let size = (guess.filter(|_| !size_set).and_then(|g| g.size)).unwrap_or(settings.tab_size);
        let guessed_spaces = guess.filter(|_| !spaces_set).map(|g| g.spaces);
        let spaces =
            e.modelines.spaces.or(policy.spaces).or(guessed_spaces).unwrap_or(settings.translate_tabs_to_spaces);
        let width = e.modelines.tab_width.or(policy.tab_width).or(policy.indent_size).unwrap_or(size);
        let indent = e.modelines.indent_size.or((policy.indent_is_tab).then_some(width)).or(policy.indent_size);
        e.hold_top(cx);
        e.update_doc(cx, |d| {
            d.history_enabled = settings.file_history;
            if !d.indent_override[0] {
                d.tab_size = width;
                d.indent_size = indent;
            }
            if !d.indent_override[1] {
                d.translate_tabs = spaces;
            }
        });
        e.set_virtual_space(settings.virtual_space, cx);
        e.view = self.view_for_editor(e, cx);
        e.clear_stale_occurrences(cx);
    }

    /// Opt-in is anchored before the declared language; other scoped settings use its result.
    pub(crate) fn derive_modelines(&self, e: &mut Editor, cx: &mut Context<Editor>) {
        e.modelines_enabled = self.settings_for_path_with_modeline(e, e.doc(cx).path.as_deref(), false, cx).modelines;
        e.modelines = if e.modelines_enabled { scan_modelines(&e.doc(cx).rope) } else { Default::default() };
        let language = if e.highlighter.language_override().is_none() && !e.modelines.languages.is_empty() {
            let registry = crate::languages::registry::registry();
            let syntaxes = Highlighter::syntax_names();
            e.modelines.languages.iter().rev().find_map(|value| modeline_language(value, &registry, &syntaxes))
        } else {
            None
        };
        let identity = language.as_ref().map(|(id, _)| id.clone());
        if identity != e.modeline_language
            || language.as_ref().is_some_and(|(_, name)| e.highlighter.syntax_name() != name)
        {
            e.modeline_language = identity;
            if e.highlighter.language_override().is_none() {
                e.highlighter = match language {
                    Some((_, name)) => Highlighter::declared(&name),
                    None => Highlighter::detect(e.doc(cx).path.as_deref(), &editor::first_line(&e.doc(cx).rope)),
                };
            }
            e.invalidate_spelling();
            e.close_completion(cx);
            e.language_version = None;
            e.diagnostics.clear();
            e.semantic_spans.clear();
            e.lsp_status.clear();
            e.signature_triggers = Default::default();
            self.invalidate_language_requests(cx);
        }
    }

    /// A current-text scan has no delayed result. Ordinary edits do not reload repository
    /// policy, resize views or mutate document metadata unless supported directives changed.
    pub(crate) fn refresh_modelines(&self, document: u64, cx: &mut Context<Self>) {
        for editor in self.views.tabs() {
            let e = editor.read(cx);
            if e.doc(cx).id() != document || !e.modelines_enabled {
                continue;
            }
            if scan_modelines(&e.doc(cx).rope) != e.modelines {
                editor.update(cx, |e, cx| {
                    self.apply_editor_preferences(e, cx);
                    cx.notify();
                });
            }
        }
    }

    pub(crate) fn cleanup_for_path(
        &self,
        e: &Editor,
        path: Option<&Path>,
        cx: &App,
    ) -> (settings::Cleanup, Properties, Vec<String>) {
        let mut cleanup = self.settings_for_path(e, path, cx).cleanup();
        let (policy, warnings) = editorconfig::load(path);
        if let Some(trim) = policy.trim {
            cleanup.trim = if trim { settings::Trim::All } else { settings::Trim::None };
        }
        if let Some(newline) = policy.final_newline {
            cleanup.ensure_newline = newline;
            cleanup.remove_final_newline = !newline;
        }
        (cleanup, policy, warnings)
    }

    pub(crate) fn guess_indentation(&mut self, _: &GuessIndentation, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let Some(guess) = infer(&editor.read(cx).doc(cx).rope) else {
            return self.status_message("Not enough consistent indentation to guess", cx);
        };
        editor.update(cx, |e, cx| {
            e.hold_top(cx);
            e.update_doc(cx, |d| {
                if let Some(size) = guess.size {
                    d.tab_size = size;
                }
                d.indent_size = None;
                d.translate_tabs = guess.spaces;
                d.inferred_indentation = Some(guess);
                d.indent_override = [true; 2];
            });
            cx.notify();
        });
        self.status_message(
            if guess.spaces {
                format!("Indentation: {} spaces", guess.size.unwrap())
            } else {
                "Indentation: tabs (tab width kept)".into()
            },
            cx,
        );
        self.refresh_menus(cx);
        autosave(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{text, window};
    use super::*;
    use gpui::TestAppContext;

    fn dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("orlo-preferences-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn indentation_inference_has_evidence_and_does_not_invent_a_tab_width() {
        for (text, want) in [
            ("fn f() {\n  a();\n  if x {\n    b();\n  }\n}", Some(Indentation { size: Some(2), spaces: true })),
            ("x\n    a\n        b\n    c\n", Some(Indentation { size: Some(4), spaces: true })),
            ("x\n\ta\n\t\tb\n", Some(Indentation { size: None, spaces: false })),
            ("no\nindentation\n", None),
            ("one\n  indented\n", None),
            ("\n    \n\t\n", None),
        ] {
            assert_eq!(infer(&ropey::Rope::from_str(text)), want, "{text:?}");
        }
    }

    #[test]
    fn independent_indent_and_tab_widths_preserve_columns_and_undo() {
        let mut state = EditorState::from_text("x");
        state.doc.tab_size = 3;
        state.doc.indent_size = Some(4);
        state.doc.translate_tabs = false;
        state.indent();
        assert_eq!(state.text(), "\t x");
        state.indent();
        assert_eq!(state.text(), "\t\t  x");
        state.unindent();
        assert_eq!(state.text(), "\t x");
        state.undo();
        assert_eq!(state.text(), "\t\t  x");
        let mut space = EditorState::from_text("\ttext");
        space.doc.tab_size = 8;
        space.doc.indent_size = Some(2);
        space.set_cursor(1);
        space.tab();
        assert_eq!(space.text(), "\t  text");
    }

    #[test]
    fn editorconfig_explicit_no_final_newline_is_undoable_and_empty_stays_empty() {
        let mut state = EditorState::from_text("x\n\n");
        assert!(state.cleanup_for_save(&settings::Cleanup { remove_final_newline: true, ..Default::default() }));
        assert_eq!(state.text(), "x");
        state.undo();
        assert_eq!(state.text(), "x\n\n");
        let mut empty = EditorState::new();
        assert!(!empty.cleanup_for_save(&settings::Cleanup { ensure_newline: true, ..Default::default() }));
    }

    #[gpui::test]
    fn project_preferences_and_project_scopes_override_global_scopes(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("project-layers");
        let path = dir.join("example.rs");
        std::fs::write(dir.join(".editorconfig"), "root=true").unwrap();
        std::fs::write(&path, "fn main() {}\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(
                r#"{"overrides":[{"language":"Rust","settings":{"font_size":30,"tab_size":8}}]}"#,
                window,
                cx,
            );
            let (project, warnings) = crate::config::project::Project::parse(
                &dir.join("Example.sublime-project"),
                r#"{
                "settings": {
                    "font_size": 20,
                    "trim_trailing_white_space_on_save": true,
                    "overrides": [{"language":"Rust","settings":{"font_size":22,"tab_size":3}}]
                }
            }"#,
            )
            .unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            ws.project = Some(project);
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            let e = editor.read(cx);
            assert_eq!((e.view.font_size, e.doc(cx).tab_size), (22., 3));
            assert_eq!(ws.editor_settings(e, cx).cleanup().trim, settings::Trim::All);
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn a_languages_indent_applies_only_where_nothing_else_sets_indentation(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("language-indent");
        let languages = session::support_dir().join("Languages");
        std::fs::create_dir_all(languages.join("indented")).unwrap();
        std::fs::write(
            languages.join("indented/language.json"),
            r#"{"schema":1,"id":"indented","name":"Indented","scope":"source.indented",
            "files":{"extensions":["orloindent"]},"indent":{"tab_size":2,"translate_tabs_to_spaces":false}}"#,
        )
        .unwrap();
        let open = |ws: &mut Workspace, name: &str, text: &str, window: &mut Window, cx: &mut Context<Workspace>| {
            let path = dir.join(name).with_extension("orloindent");
            std::fs::write(&path, text).unwrap();
            ws.open_path(path, window, cx);
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            (e.doc(cx).tab_size, e.doc(cx).translate_tabs)
        };
        let flat = "one\ntwo\n";
        ws.update_in(cx, |ws, window, cx| {
            ws.reload_customizations(&ReloadCustomizations, window, cx);
            std::fs::write(dir.join(".editorconfig"), "root=true").unwrap();
            ws.apply_settings("{}", window, cx);
            assert_eq!(open(ws, "default", flat, window, cx), (2, false), "nothing set: the language's");
            assert_eq!(
                open(ws, "detected", "a\n    b\n    c\n        d\n", window, cx),
                (4, true),
                "the text's own indentation beats a language default"
            );
            ws.apply_settings(r#"{"tab_size":4}"#, window, cx);
            assert_eq!(open(ws, "global-size", flat, window, cx), (4, false), "global tab_size");
            ws.apply_settings(r#"{"translate_tabs_to_spaces":true}"#, window, cx);
            assert_eq!(open(ws, "global-spaces", flat, window, cx), (2, true), "global translate_tabs_to_spaces");
            ws.apply_settings(
                r#"{"overrides":[{"language":"Indented","settings":{"tab_size":6,"translate_tabs_to_spaces":true}}]}"#,
                window,
                cx,
            );
            assert_eq!(open(ws, "override", flat, window, cx), (6, true), "overrides");
            ws.apply_settings("{}", window, cx);
            let (project, warnings) = crate::config::project::Project::parse(
                &dir.join("Indent.sublime-project"),
                r#"{"settings":{"tab_size":3}}"#,
            )
            .unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            ws.project = Some(project);
            assert_eq!(open(ws, "project", flat, window, cx), (3, false), "project");
            ws.project = None;
            std::fs::write(dir.join(".editorconfig"), "root=true\n[*]\nindent_style=space\nindent_size=8").unwrap();
            assert_eq!(open(ws, "editorconfig", flat, window, cx), (8, true), "EditorConfig");
            std::fs::write(dir.join(".editorconfig"), "root=true").unwrap();
            ws.apply_settings(r#"{"modelines":true}"#, window, cx);
            assert_eq!(open(ws, "modeline", "# vim: set ts=5 et:\none\n", window, cx), (5, true), "modeline");
            ws.apply_settings("{}", window, cx);
            assert_eq!(open(ws, "again", flat, window, cx), (2, false));
        });
        std::fs::remove_dir_all(languages.join("indented")).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn restored_unsaved_metadata_survives_conflicting_editorconfig(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("restored-format");
        std::fs::write(dir.join(".editorconfig"), "root=true\n[*]\nend_of_line=lf\ncharset=utf-8").unwrap();
        ws.update(cx, |ws, cx| {
            let unsaved = session::Unsaved {
                text: "restored draft\n".into(),
                line_ending: "\r\n".into(),
                bom: true,
                ..Default::default()
            };
            let editor = ws.new_editor(unsaved.state(&dir.join("draft.txt")).unwrap(), cx);
            let e = editor.read(cx);
            assert_eq!((e.doc(cx).line_ending, e.doc(cx).bom), ("\r\n", true));
            assert!(e.doc(cx).dirty);
            assert_eq!(e.text(cx), "restored draft\n");
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn file_preferences_apply_on_open_and_keep_explicit_buffer_choices(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("layers");
        std::fs::write(dir.join(".editorconfig"), "root=true\n[*.rs]\nindent_style=space\nindent_size=2\ntab_width=8")
            .unwrap();
        let path = dir.join("test.rs");
        let original = "fn a() {\n    one();\n    two();\n}\n";
        std::fs::write(&path, original).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(r#"{"overrides":[{"scope":"source.rust","settings":{"word_wrap":true,"wrap_width":72,"rulers":[72]}}]}"#, window, cx);
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            let e = editor.read(cx);
            assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), (8, 2, true));
            assert_eq!((e.view.word_wrap, e.view.wrap_width, e.view.rulers.clone()), (settings::WordWrap::On, 72, vec![72]));
            assert_eq!(e.text(cx), original);
            assert!(!e.doc(cx).dirty);
            ws.set_tab_size(&SetTabSize { size: 3 }, window, cx);
            ws.set_indent_style(&SetIndentStyle { to_spaces: false }, window, cx);
            ws.apply_settings(r#"{"tab_size":16,"translate_tabs_to_spaces":true,"font_size":18}"#, window, cx);
            ws.apply_view(cx);
            let e = editor.read(cx);
            assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), (3, 3, false));
            assert_eq!(e.text(cx), original);
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn guess_indentation_updates_siblings_without_rewriting_text(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let original = "root\n  a\n    b\n  c";
        ws.update_in(cx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| e.set_text(original, cx));
            ws.set_tab_size(&SetTabSize { size: 8 }, window, cx);
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.guess_indentation(&GuessIndentation, window, cx);
            for editor in ws.views.tabs() {
                let e = editor.read(cx);
                assert_eq!((e.doc(cx).tab_size, e.doc(cx).translate_tabs), (2, true));
                assert_eq!(e.text(cx), original);
            }
        });
    }

    #[gpui::test]
    fn save_uses_the_buffers_scoped_cleanup_and_editorconfig_format(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("save");
        let path = dir.join("file.txt");
        std::fs::write(
            dir.join(".editorconfig"),
            "root=true\n[*.txt]\nend_of_line=crlf\ncharset=utf-8-bom\ninsert_final_newline=false",
        )
        .unwrap();
        std::fs::write(&path, "a  \nb  \n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(
                r#"{"overrides":[{"path":"*.txt","settings":{"trim_trailing_white_space_on_save":true}}]}"#,
                window,
                cx,
            );
            ws.open_path(path.clone(), window, cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "a  \nb  \n");
            ws.save(&Save, window, cx);
            assert_eq!(std::fs::read(&path).unwrap(), b"\xef\xbb\xbfa\nb");
            ws.set_line_ending(&SetLineEnding { ending: "\r\n" }, window, cx);
            ws.save(&Save, window, cx);
            assert_eq!(std::fs::read(&path).unwrap(), b"\xef\xbb\xbfa\r\nb");
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).bytes_as_saved(None).unwrap(), b"\xef\xbb\xbfa\nb");
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "a  \nb  \n");
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn save_as_uses_destination_editorconfig_and_cancel_preserves_text(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("save-as");
        let destination = dir.join("new.txt");
        std::fs::write(dir.join(".editorconfig"), "root=true\n[*.txt]\ncharset=utf-8-bom\nend_of_line=crlf\ntrim_trailing_whitespace=true\ninsert_final_newline=true").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.set_text("a  \nb  ", cx));
            ws.save_as(&SaveAs, window, cx);
        });
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert_eq!(text(&ws, cx), "a  \nb  ");
        ws.update_in(cx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
        cx.simulate_new_path_selection(|_| Some(destination.clone()));
        cx.run_until_parked();
        assert_eq!(std::fs::read(&destination).unwrap(), b"\xef\xbb\xbfa\nb\r\n");
        ws.update_in(cx, |ws, window, cx| {
            ws.set_line_ending(&SetLineEnding { ending: "\r\n" }, window, cx);
            ws.save(&Save, window, cx);
        });
        assert_eq!(std::fs::read(&destination).unwrap(), b"\xef\xbb\xbfa\r\nb\r\n");
        ws.update_in(cx, |ws, window, cx| {
            ws.set_line_ending(&SetLineEnding { ending: "\n" }, window, cx);
            ws.save(&Save, window, cx);
        });
        assert_eq!(std::fs::read(&destination).unwrap(), b"\xef\xbb\xbfa\nb\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore]
    fn bench_modeline_scan_is_bounded_on_large_files_and_giant_lines() {
        use crate::testing::bench::time;
        let ordinary =
            ropey::Rope::from_str(&format!("vim: set ts=8 sw=2 wrap tw=80:\n{}", "猫 let x = 1;\n".repeat(100_000)));
        time("modeline scan, 100k Unicode lines", 100, 1., || {
            assert_eq!(scan_modelines(&ordinary).tab_width, Some(8));
        });
        let line = format!("{} vim: set ts=8:", " ".repeat(4080));
        assert_eq!(line.len(), 4095);
        let edges = format!("{line}\n").repeat(5);
        let maximum = ropey::Rope::from_str(&format!("{edges}{}{edges}{line}", "x\n".repeat(100_000)));
        time("modeline scan, maximum edge payload", 100, 1., || {
            assert_eq!(scan_modelines(&maximum).tab_width, Some(8));
        });
        let giant = ropey::Rope::from_str(&format!("{} vim: set ts=8:", "猫".repeat(1_000_000)));
        time("modeline scan, 3 MB line rejected", 100, 1., || {
            assert_eq!(scan_modelines(&giant), modelines::Properties::default());
        });
    }

    #[gpui::test]
    fn virtual_space_global_project_and_scoped_policies_apply_to_existing_views(cx: &mut TestAppContext) {
        use crate::editing::text::Selection;
        use settings::VirtualSpace;
        let (ws, cx) = window(cx);
        let directory = dir("virtual-space-policy-layers");
        let path = directory.join("plain.txt");
        std::fs::write(&path, "short\nlonger").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(r#"{"virtual_space":true}"#, window, cx);
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).state.virtual_space, VirtualSpace::All);
            let (project, warnings) = crate::config::project::Project::parse(
                &directory.join("test.sublime-project"),
                r#"{"settings":{"virtual_space":"rectangular"}}"#,
            )
            .unwrap();
            assert!(warnings.is_empty());
            ws.project = Some(project);
            ws.apply_view(cx);
            assert_eq!(editor.read(cx).state.virtual_space, VirtualSpace::Rectangular);
            ws.apply_settings(
                r#"{"virtual_space":"all","overrides":[{"language":"Plain Text","settings":{"virtual_space":"off"}}]}"#,
                window,
                cx,
            );
            ws.apply_view(cx);
            assert_eq!(
                editor.read(cx).state.virtual_space,
                VirtualSpace::Rectangular,
                "project preference wins over global/scoped settings"
            );
            ws.project = None;
            ws.apply_view(cx);
            assert_eq!(editor.read(cx).state.virtual_space, VirtualSpace::Off);
            ws.apply_settings(r#"{"virtual_space":"all"}"#, window, cx);
            ws.apply_view(cx);
            assert_eq!(editor.read(cx).state.virtual_space, VirtualSpace::All);
            editor.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::from_points((5, 4), (5, 4))]));
            ws.apply_settings(r#"{"virtual_space":"rectangular"}"#, window, cx);
            ws.apply_view(cx);
            assert_eq!(editor.read(cx).state.selections, [Selection::caret(5)]);
            assert_eq!(editor.read(cx).text(cx), "short\nlonger");
            assert!(!editor.read(cx).doc(cx).unsaved());
        });
    }

    #[gpui::test]
    fn modelines_layer_over_detection_project_and_editorconfig_but_under_manual_choices(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("modeline-layers");
        let path = dir.join("example.rs");
        let original = "// 猫 vim: set ts=8 sw=3 noet wrap tw=72:\nfn f() {\n  a();\n  b();\n}\n";
        std::fs::write(dir.join(".editorconfig"), "root=true\n[*]\nindent_size=2\ntab_width=6\nindent_style=space")
            .unwrap();
        std::fs::write(&path, original).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            let (project, warnings) = crate::config::project::Project::parse(&dir.join("test.sublime-project"),
                r#"{"settings":{"modelines":true,"tab_size":4,"word_wrap":false,"wrap_width":99,"rulers":[99],"virtual_space":true,"format_on_save":true}}"#).unwrap();
            assert!(warnings.is_empty());
            ws.project = Some(project);
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            let e = editor.read(cx);
            assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), (8,3,false));
            assert_eq!((e.view.word_wrap, e.view.wrap_width, e.view.rulers.clone()), (settings::WordWrap::On,72,vec![72]));
            assert!(e.state.virtual_space.enabled() && ws.editor_settings(e, cx).format_on_save);
            assert_eq!(e.text(cx), original);
            assert!(!e.doc(cx).dirty);
            std::fs::write(dir.join(".editorconfig"), "root=true").unwrap();
            ws.apply_view(cx);
            let e = editor.read(cx);
            assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), (8,3,false));
            ws.set_tab_size(&SetTabSize { size: 5 }, window, cx);
            ws.set_indent_style(&SetIndentStyle { to_spaces: false }, window, cx);
            editor.update(cx, |e, _| { e.wrap_override = Some(false); e.wrap_width_override = Some(40); });
            ws.apply_view(cx);
            let e = editor.read(cx);
            assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), (5,5,false));
            assert_eq!((e.wraps(), e.wrap_cols()), (false,40));
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn expanded_modelines_language_forms_are_derived_and_scoped_enablement_is_stable(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("expanded-modeline-language");
        let path = dir.join("example.rs");
        std::fs::write(dir.join(".editorconfig"), "root=true").unwrap();
        std::fs::write(&path, "// vim: set ft=python:\npass\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(r#"{"overrides":[{"language":"Rust","settings":{"modelines":true}},{"language":"Python","settings":{"modelines":false,"tab_size":3}}]}"#, window, cx);
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).highlighter.syntax_name(), "Python");
            assert_eq!(editor.read(cx).highlighter.language_override(), None);
            for declaration in ["// vim: set filetype=python:", "// vi: setlocal syntax=python:", "# -*- mode: python; -*-", "# -*- python -*-", "// kate: syntax Python;", "// vim: set ft=python ft=not-a-language:"] {
                editor.update(cx, |e, cx| e.set_text(declaration, cx));
                for _ in 0..3 { ws.apply_view(cx); }
                let e = editor.read(cx);
                assert_eq!(e.highlighter.syntax_name(), "Python", "{declaration}");
                assert_eq!(e.doc(cx).tab_size, 3);
                assert_eq!(e.highlighter.language_override(), None);
            }
            ws.apply_settings(r#"{"overrides":[{"language":"Rust","settings":{"modelines":false}},{"language":"Python","settings":{"modelines":true}}]}"#, window, cx);
            for _ in 0..3 { ws.apply_view(cx); }
            assert_eq!(editor.read(cx).highlighter.syntax_name(), "Rust", "a disabled anchor cannot enable itself through a declaration");
            ws.set_syntax(&SetSyntax { name: "C++".into() }, window, cx);
            assert_eq!(editor.read(cx).highlighter.syntax_name(), "C++");
            assert_eq!(editor.read(cx).highlighter.language_override(), Some("C++"));
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn modeline_language_aliases_use_existing_catalog_identities() {
        use crate::languages::{language::Language, treesitter};
        let registry = crate::languages::registry::registry();
        let syntaxes = Highlighter::syntax_names();
        let modeline_language = |value: &str| super::modeline_language(value, &registry, &syntaxes);
        for l in Language::ALL {
            for alias in [l.registry_id(), l.id()] {
                assert_eq!(modeline_language(alias), Some((l.registry_id().into(), treesitter::name(l).into())));
            }
        }
        // the bundled languages answer through the registry
        assert!(registry.languages().len() >= Language::ALL.len());
        for l in registry.languages() {
            let upper = l.id().to_uppercase();
            let lsp = l.manifest.lsp.as_ref().map(|lsp| lsp.language_id.as_str());
            // a language whose name the syntax menu does not list yet opens as Plain Text
            let syntax = if syntaxes.iter().any(|s| s == l.name()) { l.name() } else { "Plain Text" };
            for alias in [l.id(), &upper].into_iter().chain(lsp) {
                assert_eq!(modeline_language(alias), Some((l.id().into(), syntax.into())), "{alias}");
            }
        }
        for (alias, id, syntax) in [
            ("RuSt", "rust", "Rust"),
            ("rust-mode", "rust", "Rust"),
            ("c++", "cpp", "C++"),
            ("c++-mode", "cpp", "C++"),
            ("cs", "csharp", "C#"),
            ("c#", "csharp", "C#"),
            ("js", "javascript", "JavaScript"),
            ("ts", "typescript", "TypeScript"),
            ("sh", "bash", "Bourne Again Shell (bash)"),
            ("shell", "bash", "Bourne Again Shell (bash)"),
            ("text", "Plain Text", "Plain Text"),
            ("plain", "Plain Text", "Plain Text"),
            ("plaintext", "Plain Text", "Plain Text"),
            ("JSON", "JSON", "JSON"),
        ] {
            assert_eq!(modeline_language(alias), Some((id.into(), syntax.into())), "{alias}");
        }
        assert!(modeline_language("not-a-language").is_none());
        assert!(modeline_language("../../bin/sh").is_none());
    }

    #[test]
    fn modelines_scan_physical_edge_records_with_or_without_a_final_newline() {
        for ending in ["", "\n"] {
            let mut lines = vec!["ordinary"; 20];
            lines[15] = "vim: set ts=8:";
            assert_eq!(scan_modelines(&ropey::Rope::from_str(&(lines.join("\n") + ending))).tab_width, Some(8));
            lines[15] = "ordinary";
            lines[14] = "vim: set ts=8:";
            assert_eq!(
                scan_modelines(&ropey::Rope::from_str(&(lines.join("\n") + ending))),
                modelines::Properties::default()
            );
        }
        let unicode = "\u{2028}".repeat(10) + "\n\n\n\n\nvim: set ts=8:\n\n\n\n\n\n";
        assert_eq!(scan_modelines(&ropey::Rope::from_str(&unicode)), modelines::Properties::default());
    }

    #[gpui::test]
    fn modeline_fields_overlay_editorconfig_independently_and_keep_save_policy(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let directory = dir("expanded-modeline-field-precedence");
        let path = directory.join("sample.txt");
        std::fs::write(directory.join(".editorconfig"), "root=true\n[*]\ntab_width=6\nindent_size=2\nindent_style=space\nend_of_line=crlf\ncharset=utf-8-bom\ntrim_trailing_whitespace=true\ninsert_final_newline=true").unwrap();
        std::fs::write(&path, "ordinary\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(r#"{"modelines":true,"tab_size":4,"word_wrap":false,"wrap_width":99,"rulers":[99]}"#, window, cx);
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            for (directive, expected) in [
                ("ordinary", (6, 2, true)),
                ("vim: set ts=8:", (8, 2, true)),
                ("vim: set sw=3:", (6, 3, true)),
                ("kate: replace-tabs off;", (6, 2, false)),
                ("-*- tab-width: 7; standard-indent: 1; indent-tabs-mode: t; -*-", (7, 1, false)),
            ] {
                editor.update(cx, |e, cx| e.set_text(directive, cx));
                ws.apply_view(cx);
                let e = editor.read(cx);
                assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), expected, "{directive}");
                assert_eq!((e.doc(cx).line_ending, e.doc(cx).bom), ("\r\n", true));
                let (cleanup, _, warnings) = ws.cleanup_for_path(e, Some(&path), cx);
                assert!(warnings.is_empty());
                assert_eq!(cleanup.trim, settings::Trim::All);
                assert!(cleanup.ensure_newline && !cleanup.remove_final_newline);
            }
            editor.update(cx, |e, cx| {
                e.set_text("kate: tab-width 8; indent-width 3; replace-tabs on; word-wrap on; word-wrap-column 72; syntax python; encoding utf-16;", cx);
                e.wrap_override = Some(false);
                e.wrap_width_override = Some(40);
            });
            ws.set_tab_size(&SetTabSize { size: 5 }, window, cx);
            ws.set_indent_style(&SetIndentStyle { to_spaces: false }, window, cx);
            ws.set_ruler(&SetRuler { col: Some(50) }, window, cx);
            let e = editor.read(cx);
            assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), (5, 5, false));
            assert_eq!((e.wraps(), e.wrap_cols(), e.view.rulers.clone()), (false, 40, vec![50]));
            assert_eq!((e.doc(cx).line_ending, e.doc(cx).bom), ("\r\n", true));
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"ordinary\n");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[gpui::test]
    fn modeline_language_disabled_edits_shared_views_undo_and_restore(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let directory = dir("expanded-modeline-language-lifecycle");
        let path = directory.join("sample.rs");
        let original = "// vim: set ft=python:\npass\n";
        std::fs::write(directory.join(".editorconfig"), "root=true").unwrap();
        std::fs::write(&path, original).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(path.clone(), window, cx);
            let original = ws.active_editor().unwrap();
            assert_eq!(original.read(cx).highlighter.syntax_name(), "Rust");
            original.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("// kate: syntax Go;\npackage main\n");
                })
            });
        });
        cx.run_until_parked();
        let automatic = ws.update_in(cx, |ws, window, cx| {
            let original = ws.active_editor().unwrap();
            assert_eq!(original.read(cx).highlighter.syntax_name(), "Rust", "disabled edits do not parse declarations");
            ws.apply_settings(r#"{"modelines":true}"#, window, cx);
            ws.apply_view(cx);
            assert_eq!(original.read(cx).highlighter.syntax_name(), "Go");
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.set_syntax(&SetSyntax { name: "C++".into() }, window, cx);
            original.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
            original
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            assert_eq!(automatic.read(cx).highlighter.syntax_name(), "Python");
            assert_eq!(automatic.read(cx).highlighter.language_override(), None);
            assert_eq!(ws.active_editor().unwrap().read(cx).highlighter.syntax_name(), "C++");
            let snapshot = ws.capture(window, false, &mut Vec::new(), cx);
            let json = serde_json::to_value(&snapshot).unwrap();
            for group in json["groups"].as_array().unwrap() {
                for view in group["views"].as_array().unwrap() {
                    assert!(
                        !view.as_object().unwrap().keys().any(|key| key.contains("modeline") || key.contains("syntax")),
                        "derived language adds no session field"
                    );
                }
            }
            ws.restore(snapshot, window, cx);
            for editor in ws.views.tabs() {
                assert_eq!(editor.read(cx).highlighter.syntax_name(), "Python");
                assert_eq!(editor.read(cx).highlighter.language_override(), None);
                assert_eq!(editor.read(cx).text(cx), original);
            }
        });
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[gpui::test]
    fn modeline_language_preferences_preserve_full_selection_and_native_composition(cx: &mut TestAppContext) {
        use crate::editing::text::Selection;
        use gpui::EntityInputHandler;
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(r#"{"virtual_space":"all"}"#, window, cx);
            ws.apply_view(cx);
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| {
                e.set_text("// kate: syntax Python;\n猫\n", cx);
                let at = e.doc(cx).len() - 1;
                e.sel(cx).set_cursor(at);
                e.replace_and_mark_text_in_range(None, "に", Some(1..1), window, cx);
                let mut rectangular = Selection::from_points((e.doc(cx).len(), 3), (e.doc(cx).len(), 5));
                rectangular.rectangular = true;
                e.sel(cx).set_selections(vec![Selection::caret(2), rectangular]);
            });
            let (selections, primary, marked, version, text) = {
                let e = editor.read(cx);
                (e.state.selections.clone(), e.state.primary(), e.marked.clone(), e.doc(cx).version, e.text(cx))
            };
            assert!(selections[1].rectangular && selections[1].has_virtual());
            assert!(!marked.is_empty());
            ws.apply_settings(r#"{"modelines":true,"virtual_space":"all"}"#, window, cx);
            ws.apply_view(cx);
            let e = editor.read(cx);
            assert_eq!(e.highlighter.syntax_name(), "Python");
            assert_eq!(
                (e.state.selections.clone(), e.state.primary(), e.marked.clone(), e.doc(cx).version, e.text(cx)),
                (selections, primary, marked, version, text)
            );
        });
    }

    #[gpui::test]
    fn modelines_live_edits_undo_disable_and_shared_view_overrides(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(
                r#"{"modelines":true,"tab_size":4,"word_wrap":false,"wrap_width":99,"rulers":[99]}"#,
                window,
                cx,
            );
            ws.with_active(cx, |e, cx| e.set_text("猫 vim: set ts=8 sw=2 noet wrap tw=72:", cx));
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, _| {
                e.wrap_override = Some(false);
                e.wrap_width_override = Some(40);
            });
        });
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 2);
            for editor in ws.views.tabs() {
                let e = editor.read(cx);
                assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), (8,2,false));
                assert_eq!((e.view.word_wrap, e.view.wrap_width), (settings::WordWrap::On,72));
            }
            ws.with_active(cx, |e, cx| e.edit(cx, |s| { s.set_cursor(s.len()); s.type_text("\n# -*- tab-width: 3; standard-indent: 1; indent-tabs-mode: nil; fill-column: 0; truncate-lines: t; -*-"); }));
        });
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            for editor in ws.views.tabs() {
                let e = editor.read(cx);
                assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width(), e.doc(cx).translate_tabs), (3, 1, true));
                assert_eq!(
                    (e.view.word_wrap, e.view.wrap_width, e.view.rulers.clone()),
                    (settings::WordWrap::Off, 0, vec![])
                );
            }
            let active = ws.active_editor().unwrap();
            assert_eq!((active.read(cx).wrap_override, active.read(cx).wrap_width_override), (Some(false), Some(40)));
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).doc(cx).tab_size, 8);
            ws.apply_settings(
                r#"{"modelines":false,"tab_size":4,"word_wrap":false,"wrap_width":99,"rulers":[99]}"#,
                window,
                cx,
            );
            ws.apply_view(cx);
            for editor in ws.views.tabs() {
                let e = editor.read(cx);
                assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_size, e.doc(cx).translate_tabs), (4, None, true));
                assert_eq!(
                    (e.view.word_wrap, e.view.wrap_width, e.view.rulers.clone()),
                    (settings::WordWrap::Off, 99, vec![99])
                );
            }
        });
    }

    #[gpui::test]
    fn modelines_shared_across_windows_keep_document_manual_and_local_wrap_choices(cx: &mut TestAppContext) {
        let (ws1, ws2, mut cx1, mut cx2) = super::super::ui_tests::two_windows(cx);
        let dir = dir("modeline-windows");
        let path = dir.join("sample.txt");
        std::fs::write(dir.join(".editorconfig"), "root=true").unwrap();
        std::fs::write(&path, "vim: set ts=8 wrap tw=72 ft=python:\n猫").unwrap();
        ws1.update_in(&mut cx1, |ws, window, cx| {
            ws.apply_settings(r#"{"modelines":true}"#, window, cx);
            ws.open_path(path.clone(), window, cx);
        });
        ws2.update_in(&mut cx2, |ws, window, cx| ws.open_path(path.clone(), window, cx));
        let e1 = ws1.update(&mut cx1, |ws, _| ws.active_editor().unwrap());
        let e2 = ws2.update(&mut cx2, |ws, _| ws.active_editor().unwrap());
        e1.read_with(&cx1, |first, cx| assert!(first.same_document(e2.read(cx))));
        e2.update(&mut cx2, |e, _| {
            e.wrap_override = Some(false);
            e.wrap_width_override = Some(40);
        });
        ws2.update_in(&mut cx2, |ws, window, cx| {
            ws.set_tab_size(&SetTabSize { size: 5 }, window, cx);
            ws.set_syntax(&SetSyntax { name: "C++".into() }, window, cx);
        });
        e1.update(&mut cx1, |e, cx| e.set_text("vim: set ts=3 sw=1 wrap tw=80 ft=go:\n猫", cx));
        cx1.run_until_parked();
        for editor in [&e1, &e2] {
            editor.read_with(&cx1, |e, cx| {
                assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_width()), (5, 5));
                assert_eq!(
                    (e.view.word_wrap, e.view.wrap_width, e.view.rulers.clone()),
                    (settings::WordWrap::On, 80, vec![80])
                );
            });
        }
        e1.read_with(&cx1, |e, _| assert_eq!((e.wraps(), e.wrap_cols()), (true, 80)));
        e2.read_with(&cx2, |e, _| assert_eq!((e.wraps(), e.wrap_cols()), (false, 40)));
        e1.read_with(&cx1, |e, _| assert_eq!(e.highlighter.syntax_name(), "Go"));
        e2.read_with(&cx2, |e, _| assert_eq!(e.highlighter.syntax_name(), "C++"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn modelines_save_as_destination_enablement_and_cancel_preserve_bytes(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("modeline-save-as").canonicalize().unwrap();
        let path = dir.join("before.txt");
        let dest = dir.join("after.rs");
        let original = "vim: set ts=8 sw=2 noet wrap tw=72:\n猫 é\n";
        std::fs::write(dir.join(".editorconfig"), "root=true").unwrap();
        std::fs::write(&path, original).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(r#"{"overrides":[{"path":"*.txt","settings":{"modelines":true}}]}"#, window, cx);
            ws.open_path(path.clone(), window, cx);
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.save_as(&SaveAs, window, cx);
        });
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            for editor in ws.views.tabs() {
                assert_eq!(editor.read(cx).doc(cx).tab_size, 8);
            }
            ws.save_as(&SaveAs, window, cx);
        });
        cx.simulate_new_path_selection(|_| Some(dest.clone()));
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            for editor in ws.views.tabs() {
                let e = editor.read(cx);
                assert_eq!(e.doc(cx).path.as_ref(), Some(&dest));
                assert_eq!((e.doc(cx).tab_size, e.doc(cx).indent_size, e.doc(cx).translate_tabs), (4, None, true));
                assert_eq!(e.text(cx), original);
                assert!(!e.doc(cx).dirty);
            }
        });
        assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
        assert_eq!(std::fs::read(&dest).unwrap(), original.as_bytes());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn modelines_default_off_read_only_reload_and_removal_preserve_text(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("modeline-reload");
        let path = dir.join("sample.txt");
        std::fs::write(dir.join(".editorconfig"), "root=true").unwrap();
        let original = "# -*- tab-width: 8; standard-indent: 2; indent-tabs-mode: nil; -*-\n猫\n";
        std::fs::write(&path, original).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).doc(cx).tab_size, 4);
            editor.update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true));
            ws.apply_settings(r#"{"modelines":true}"#, window, cx);
            ws.apply_view(cx);
            let e = editor.read(cx);
            assert_eq!(e.doc(cx).tab_size, 8);
            assert!(e.doc(cx).read_only && !e.doc(cx).dirty);
            assert_eq!(e.text(cx), original);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            editor.update(cx, |e, cx| {
                e.update_doc(cx, |d| d.read_only = false);
                std::fs::write(&path, "vim: set ts=3 sw=1:\n猫\n").unwrap();
                let loaded = Loaded::read(&path).unwrap();
                let changes = loaded.diff(&e.doc(cx).rope);
                e.reload(loaded, &changes, cx);
            });
        });
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).tab_size, 3);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
        });
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).tab_size, 8);
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), original);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.redo()));
        });
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).tab_size, 3);
            ws.with_active(cx, |e, cx| e.set_text("no directive 猫", cx));
        });
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!((editor.read(cx).doc(cx).tab_size, editor.read(cx).doc(cx).indent_size), (4, None));
            assert_eq!(editor.read(cx).text(cx), "no directive 猫");
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[gpui::test]
    fn passive_occurrences_settings_reach_views_and_clear_immediately_on_disable(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| {
                e.set_text("猫 猫", cx);
                e.sel(cx).set_selection(0, 1);
            });
            ws.apply_settings(r#"{"selection_occurrences":true}"#, window, cx);
            ws.apply_view(cx);
            assert!(ws.active_editor().unwrap().read(cx).view.selection_occurrences);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).passive_occurrence_ranges(), std::slice::from_ref(&(2..3)));
            ws.apply_settings(r#"{"selection_occurrences":false}"#, window, cx);
            ws.apply_view(cx);
            assert!(!e.read(cx).view.selection_occurrences);
            assert!(e.read(cx).passive_occurrence_ranges().is_empty());
            assert_eq!(e.read(cx).text(cx), "猫 猫");
        });
    }

    #[gpui::test]
    fn unicode_warning_settings_come_from_preferences_not_projects(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let dir = dir("unicode-warning-preferences");
        let path = dir.join("example.rs");
        std::fs::write(&path, "x\u{a0}y").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.apply_settings(
                r#"{"unicode_warnings":false,"overrides":[{"language":"Rust","settings":{
                "unicode_warning_characters":{"x":{"glyph":"!","description":"global"}},
                "unicode_warning_ranges":{"U+E000":{"glyph":"!","description":"global range"}}
            }}]}"#,
                window,
                cx,
            );
            // a folder may not change the warnings that would catch the text it brings
            let (project, warnings) = crate::config::project::Project::parse(
                &dir.join("test.sublime-project"),
                r#"{
                "settings":{"unicode_warning_characters":{},"overrides":[{"path":"*.rs","settings":{
                    "unicode_warnings":true,"unicode_warning_characters":{"y":{"glyph":"?","description":"project"}},
                    "unicode_warning_ranges":{"U+E001..U+E002":{"glyph":"?","description":"project range"}}
                }}]}
            }"#,
            )
            .unwrap();
            assert_eq!(warnings.iter().filter(|w| w.contains("misleading text")).count(), 4, "{warnings:?}");
            ws.project = Some(project);
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            let view = &editor.read(cx).view;
            assert!(!view.unicode_warnings, "the user's own choice");
            assert_eq!(view.unicode_warning_characters.len(), 1);
            assert_eq!(view.unicode_warning_characters[&'x'].description, "global");
            let unicode = &crate::config::settings::unicode::describe_with_ranges(
                "\u{e000}\u{e001}",
                &view.unicode_warning_characters,
                &view.unicode_warning_ranges,
                true,
            );
            assert!(unicode.contains("U+E000 — global range") && !unicode.contains("project range"), "{unicode}");
        });
        std::fs::remove_dir_all(dir).unwrap();
    }
}
