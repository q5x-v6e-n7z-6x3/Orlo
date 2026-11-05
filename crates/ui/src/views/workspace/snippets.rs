//! Tools > Snippets: Insert Snippet's list, New Snippet's template, and the choice between
//! snippets that share the trigger before the carets.

use super::*;
use crate::editing::snippet;
use crate::editor::{ExpandSnippet, SnippetStore};
use std::sync::Arc;

#[derive(Clone, PartialEq, Eq)]
struct TemplateLanguage {
    id: String,
    name: String,
}

pub(crate) struct TemplateChoice {
    snippet: Arc<snippet::Snippet>,
    language: Option<TemplateLanguage>,
}

fn template_languages() -> Vec<TemplateLanguage> {
    let registry = crate::languages::registry::registry();
    let mut languages: Vec<_> = registry
        .languages()
        .iter()
        .filter(|l| !l.manifest.hidden)
        .map(|l| TemplateLanguage { id: l.id().into(), name: l.name().into() })
        .collect();
    for name in Highlighter::syntax_names() {
        if !languages.iter().any(|l| l.name == name) {
            languages.push(TemplateLanguage { id: name.clone(), name });
        }
    }
    languages.sort_by_cached_key(|l| (l.name.to_lowercase(), l.id.clone()));
    languages
}

impl Workspace {
    pub(crate) fn new_from_template(&mut self, _: &NewFromTemplate, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            if let Err(error) = self.selection_set_admission(&editor, cx) {
                return self.status_message(error, cx);
            }
        }
        editor::refresh_snippets(window, cx);
        let items: Vec<_> = cx
            .global::<SnippetStore>()
            .0
            .snippets()
            .iter()
            .map(|s| quick_panel::Item {
                label: editor::snippet_title(s),
                detail: s.path.strip_prefix(snippet::dir()).unwrap_or(&s.path).display().to_string(),
                hint: s.trigger.clone(),
                badge: None,
                action: ItemAction::Template(Arc::new(TemplateChoice { snippet: Arc::new(s.clone()), language: None })),
            })
            .collect();
        if items.is_empty() {
            return self.inform(
                window,
                cx,
                "No templates",
                "Create a local snippet with Tools > Snippets > New Snippet…",
            );
        }
        self.open_template_picker(items, "New From Template — Choose Snippet".into(), window, cx);
    }

    fn open_template_picker(
        &mut self,
        items: Vec<quick_panel::Item>,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_panel(Kind::Choices, items, window, cx);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some(title);
        panel.choice_details = true;
        panel.keys = Arc::new(panel.items.iter().map(|i| format!("{} {} {}", i.label, i.detail, i.hint)).collect());
        panel.refilter("");
    }

    /// Handle these rows before the generic picker restores focus: a peer's IME preview
    /// must remain owned by its source view even if it began while the picker was open.
    pub(crate) fn confirm_template(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let choice = self.quick_panel.as_ref().and_then(|p| match &p.items.get(*p.filtered.get(p.selected)?)?.action {
            ItemAction::Template(choice) => Some(choice.clone()),
            _ => None,
        });
        let Some(choice) = choice else { return false };
        if let Err(error) = self.apply_template_choice(&choice, window, cx) {
            self.status_message(format!("Could not create template: {error}"), cx);
        }
        true
    }

    fn apply_template_choice(
        &mut self,
        choice: &TemplateChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(editor) = self.active_editor() {
            self.selection_set_admission(&editor, cx)?;
        }
        if self.quick_panel.as_ref().is_some_and(|p| !p.input.read(cx).marked.is_empty()) {
            return Err("Finish or cancel text composition before choosing a template".into());
        }
        let current = cx.global::<SnippetStore>().0.reread(&choice.snippet.path)?;
        if current != *choice.snippet {
            return Err("The template changed; reopen New From Template".into());
        }
        let Some(language) = &choice.language else {
            let items = template_languages()
                .into_iter()
                .map(|language| {
                    quick_panel::Item::new(
                        language.name.clone(),
                        language.id.clone(),
                        ItemAction::Template(Arc::new(TemplateChoice {
                            snippet: choice.snippet.clone(),
                            language: Some(language),
                        })),
                    )
                })
                .collect();
            self.open_template_picker(
                items,
                format!("New From Template — Language for {}", editor::snippet_title(&current)),
                window,
                cx,
            );
            return Ok(());
        };
        if !template_languages().contains(language) {
            return Err("The template language changed; reopen New From Template".into());
        }
        let vars = editor::SnippetVariables::for_template(&self.roots, cx);
        let editor = self.new_editor(EditorState::new(), cx);
        editor.update(cx, |e, cx| {
            e.highlighter = Highlighter::by_name(&language.id);
            self.apply_editor_preferences(e, cx);
            e.edit(cx, |state| state.expand_snippet(&current, "", &|name, _| vars.resolve(name)));
            e.update_doc(cx, |doc| doc.forget_saved_point());
        });
        self.cancel_quick_panel(cx);
        self.drop_blank_untitled(cx);
        self.attach(editor, false, window, cx);
        Ok(())
    }

    /// Insert Snippet…: the snippets for the scope at the caret; the one chosen is expanded at
    /// the carets, in place of the selections.
    pub(crate) fn insert_snippet(&mut self, _: &InsertSnippet, window: &mut Window, cx: &mut Context<Self>) {
        if !self.need_tab("insert a snippet into", window, cx) {
            return;
        }
        let Some(editor) = self.active_editor() else {
            return;
        };
        let scope = editor.read(cx).snippet_scope(cx);
        editor::refresh_snippets(window, cx);
        let store = &cx.global::<SnippetStore>().0;
        let paths: Vec<PathBuf> = (store.for_scope(|selector| highlight::matches(scope, selector)).into_iter())
            .map(|i| store.snippets()[i].path.clone())
            .collect();
        if paths.is_empty() {
            let detail = "Create one with Tools > Snippets > New Snippet…";
            return self.inform(window, cx, "No snippets here", detail);
        }
        self.choose_snippet(paths, String::new(), window, cx);
    }

    /// The quick panel of these snippets (their files); the one chosen is expanded in place of
    /// `trigger` before the carets.
    pub(crate) fn choose_snippet(
        &mut self,
        paths: Vec<PathBuf>,
        trigger: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = &cx.global::<SnippetStore>().0;
        let items = (store.snippets().iter())
            .filter(|s| paths.contains(&s.path))
            .map(|s| quick_panel::Item {
                label: editor::snippet_title(s),
                detail: String::new(),
                hint: s.trigger.clone(),
                badge: None,
                action: ItemAction::Dispatch(Box::new(ExpandSnippet {
                    path: s.path.clone(),
                    trigger: trigger.clone(),
                })),
            })
            .collect();
        self.open_quick_panel(Kind::Commands, items, window, cx);
    }

    /// New Snippet…: asks for a name, then opens that snippet, or the template under that name
    /// (written by its first save, as New Pipeline… does).
    pub(crate) fn new_snippet(&mut self, _: &NewSnippet, window: &mut Window, cx: &mut Context<Self>) {
        let done: OnDone = Box::new(|v, _, ws, window, cx| {
            let name = crate::searching::pipeline::file_name(v.text("text"), snippet::EXT, "snippet")?;
            let file = snippet::dir().join(format!("{name}.{}", snippet::EXT));
            ws.open_or_template(file, snippet::template(), window, cx);
            Ok(())
        });
        let field = Field::text("text", "Snippet name", "");
        self.show_window_form("", vec![field], done, window, cx);
    }
}

#[cfg(test)]
mod template_tests {
    use super::super::ui_tests::{text, type_in, window};
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};
    use std::rc::Rc;

    fn definition(name: &str, content: &str) -> PathBuf {
        let dir = snippet::dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.sublime-snippet"));
        std::fs::write(&path, format!("<snippet><content><![CDATA[{content}]]></content><scope>source.rust</scope><description>{name}</description></snippet>")).unwrap();
        path
    }

    fn launch(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
        ws.update_in(cx, |ws, w, cx| {
            let result = ws.execute_command(
                &crate::keybindings::Command {
                    command: "workspace::NewFromTemplate".into(),
                    args: serde_json::json!({}),
                },
                w,
                cx,
            );
            assert!(result.is_ok(), "New From Template must be a public command: {result:?}");
        });
        cx.run_until_parked();
    }

    fn choose(cx: &mut VisualTestContext, label: &str) {
        cx.simulate_input(label);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
    }

    #[gpui::test]
    fn u02_template_command_creates_one_dirty_untitled_with_language_and_owned_fields(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        cx.update(|window, _| window.activate_window());
        definition("Rust class", "${1|one,two|}:$1$0");
        type_in(&ws, cx, "source");
        let original = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        launch(&ws, cx);
        choose(cx, "Rust class");
        ws.read_with(cx, |ws, _| assert_eq!(ws.views.tabs().count(), 1, "language confirmation is explicit"));
        choose(cx, "Rust");
        assert_eq!(text(&ws, cx), "one:one");
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 2);
            assert_eq!(original.read(cx).text(cx), "source");
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            assert!(e.doc(cx).path.is_none() && e.doc(cx).dirty);
            assert_eq!(e.highlighter.syntax_name(), "Rust");
            assert_eq!(e.snippet_choice_list(), Some((vec!["one".into(), "two".into()], 0)));
            assert!(e.read_state(cx).in_snippet());
        });
        cx.simulate_keystrokes("down enter");
        assert_eq!(text(&ws, cx), "two:two");
        cx.simulate_input("界");
        assert_eq!(text(&ws, cx), "界:界");
        cx.simulate_keystrokes("tab");
        ws.read_with(cx, |ws, cx| assert!(!ws.active_editor().unwrap().read(cx).read_state(cx).in_snippet()));
    }

    #[gpui::test]
    fn u02_template_cancel_preserves_source_and_empty_expansion_is_dirty_without_fake_edits(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        definition("Empty", "$0");
        type_in(&ws, cx, "source");
        let original = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        let before = original.update(cx, |e, cx| {
            e.sel(cx).set_selection(1, 4);
            (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).dirty, e.state.selections.clone())
        });
        for language_step in [false, true] {
            launch(&ws, cx);
            if language_step {
                choose(cx, "Empty");
            }
            cx.simulate_keystrokes("escape");
            ws.read_with(cx, |ws, cx| {
                assert_eq!(ws.views.tabs().count(), 1);
                assert_eq!(ws.active_editor().as_ref(), Some(&original));
                let e = original.read(cx);
                assert_eq!(
                    (
                        e.text(cx),
                        e.doc(cx).version,
                        e.doc(cx).save_point(),
                        e.doc(cx).dirty,
                        e.state.selections.clone()
                    ),
                    before
                );
            });
        }
        launch(&ws, cx);
        choose(cx, "Empty");
        choose(cx, "Plain Text");
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 2);
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            assert_eq!(e.text(cx), "");
            assert_eq!(e.doc(cx).version, 0, "empty expansion needs no fabricated text edit");
            assert!(e.doc(cx).unsaved());
        });
        cx.simulate_keystrokes("cmd-z");
        ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).doc(cx).unsaved()));
    }

    #[gpui::test]
    fn u02_template_revalidates_changed_removed_and_invalid_definitions_without_partial_tabs(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        for stage in [0, 1] {
            for change in ["changed", "removed", "invalid"] {
                let path = definition("Stale", "${1:old}$0");
                cx.update(|_, cx| cx.set_global(SnippetStore(snippet::Snippets::new(snippet::dir()))));
                launch(&ws, cx);
                if stage == 1 {
                    choose(cx, "Stale");
                }
                match change {
                    "removed" => std::fs::remove_file(path).unwrap(),
                    "invalid" => {
                        definition("Stale", "${1:broken");
                    }
                    _ => {
                        definition("Stale", "${1:new}$0");
                    }
                }
                choose(cx, if stage == 0 { "Stale" } else { "Rust" });
                ws.read_with(cx, |ws, _| {
                    assert_eq!(ws.views.tabs().count(), 1, "{stage}/{change}");
                    assert!(ws.quick_panel.is_some(), "refusal retains the picker");
                    assert!(ws.status.as_ref().is_some_and(|m| m.0.contains("template")), "visible stale error");
                });
                cx.simulate_keystrokes("escape");
            }
        }
        definition("Stale", "${1:broken");
        cx.update(|_, cx| cx.set_global(SnippetStore(snippet::Snippets::new(snippet::dir()))));
        launch(&ws, cx);
        assert!(cx.pending_prompt().is_some(), "invalid definitions are diagnosed");
        ws.read_with(cx, |ws, _| assert_eq!(ws.views.tabs().count(), 1));
    }

    #[gpui::test]
    fn u02_template_captures_clock_clipboard_and_untitled_context_only_at_creation(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        definition("Variables", "$CURRENT_YEAR-$CURRENT_MONTH-$CURRENT_DATE $CURRENT_HOUR:$CURRENT_MINUTE:$CURRENT_SECOND $CLIPBOARD $WORKSPACE_NAME $WORKSPACE_FOLDER [${RELATIVE_FILEPATH:none}] [${TM_FILENAME:untitled}] [$SELECTION] $TM_LINE_NUMBER:$TM_LINE_INDEX ${1:hello}:$1:${1/(.*)/\\U$1/}$0");
        let root = super::super::ui_tests::temp("orlo-u02-project");
        let calls = Rc::new(std::cell::Cell::new(0));
        cx.update(|_, cx| {
            editor::fixed_clock(cx, calls.clone());
            cx.write_to_clipboard(ClipboardItem::new_string("old".into()));
        });
        ws.update(cx, |ws, _| {
            ws.roots = vec![crate::config::project::Root {
                path: root.clone(),
                name: Some("Project".into()),
                ..Default::default()
            }];
        });
        type_in(&ws, cx, "source");
        launch(&ws, cx);
        choose(cx, "Variables");
        assert_eq!(calls.get(), 0, "opening either picker captures no variables");
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("fresh".into())));
        choose(cx, "Rust");
        assert_eq!(calls.get(), 1);
        assert_eq!(
            text(&ws, cx),
            format!("2024-02-29 19:07:25 fresh Project {} [none] [untitled] [] 1:0 hello:hello:HELLO", root.display())
        );
        cx.simulate_input("world");
        assert!(text(&ws, cx).ends_with("world:world:WORLD"));
    }

    #[gpui::test]
    fn u02_template_shared_and_local_ime_refuse_launch_and_commit_without_changing_composition(
        cx: &mut TestAppContext,
    ) {
        use gpui::EntityInputHandler;
        let (ws, cx) = window(cx);
        definition("Composed", "body");
        type_in(&ws, cx, "source");
        super::super::ui_tests::new_view(&ws, cx);
        let views: Vec<_> = ws.read_with(cx, |ws, _| ws.views.tabs().cloned().collect());
        for composing in &views {
            composing.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(Some(0..3), "sou", Some(0..1), w, cx));
            let before = composing.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
            launch(&ws, cx);
            ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_none()));
            assert_eq!(composing.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)), before);
            composing.update_in(cx, |e, w, cx| e.unmark_text(w, cx));
            launch(&ws, cx);
            choose(cx, "Composed");
            composing.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(Some(0..3), "sou", Some(0..1), w, cx));
            let before = composing.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
            choose(cx, "Rust");
            ws.read_with(cx, |ws, _| {
                assert!(ws.quick_panel.is_some());
                assert_eq!(ws.views.tabs().count(), 2);
            });
            assert_eq!(composing.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)), before);
            cx.simulate_keystrokes("escape");
            assert_eq!(composing.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)), before);
            composing.update_in(cx, |e, w, cx| e.unmark_text(w, cx));
        }
    }

    #[gpui::test]
    fn u02_template_uses_normal_save_as_and_copy_and_replaces_only_pristine_blank(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        definition("Draft", "界 text$0");
        let dir = super::super::ui_tests::temp("orlo-u02-save").canonicalize().unwrap();
        launch(&ws, cx);
        choose(cx, "Draft");
        choose(cx, "Rust");
        ws.read_with(cx, |ws, _| assert_eq!(ws.views.tabs().count(), 1));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "creation writes no file");
        cx.dispatch_action(SaveCopyAs);
        let copy = dir.join("copy.rs");
        cx.simulate_new_path_selection(|_| Some(copy.clone()));
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(copy).unwrap(), "界 text");
        ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).doc(cx).path.is_none()));
        cx.dispatch_action(Save);
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).doc(cx).dirty));
        cx.dispatch_action(Save);
        let saved = dir.join("saved.rs");
        cx.simulate_new_path_selection(|_| Some(saved.clone()));
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "界 text");
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).doc(cx).path.as_deref(), Some(saved.as_path()));
            assert!(!e.read(cx).doc(cx).dirty);
        });
    }

    #[gpui::test]
    fn u02_template_runtime_language_id_fallback_preferences_and_stale_catalog(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        definition("Runtime", "\t${1:x}$0");
        let folder = session::support_dir().join("Languages/fixture");
        std::fs::create_dir_all(&folder).unwrap();
        let manifest = folder.join("language.json");
        std::fs::write(
            &manifest,
            r#"{"schema":1,"id":"fixture","name":"Fixture Runtime","scope":"source.fixture","files":{}}"#,
        )
        .unwrap();
        ws.update_in(cx, |ws, w, cx| {
            ws.apply_settings(r#"{"overrides":[{"language":"Fixture Runtime","settings":{"tab_size":3}}]}"#, w, cx);
            ws.reload_customizations(&ReloadCustomizations, w, cx);
        });
        launch(&ws, cx);
        choose(cx, "Runtime");
        choose(cx, "Fixture Runtime");
        assert_eq!(text(&ws, cx), "   x");
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            assert_eq!(e.highlighter.syntax_name(), "Plain Text");
            assert_eq!(e.language_choice(), Some("fixture"));
            assert_eq!(e.doc(cx).tab_size, 3);
        });
        ws.update_in(cx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).language_choice(), Some("fixture")));
        launch(&ws, cx);
        choose(cx, "Runtime");
        std::fs::remove_file(manifest).unwrap();
        ws.update_in(cx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
        choose(cx, "Fixture Runtime");
        ws.read_with(cx, |ws, _| {
            assert_eq!(ws.views.tabs().count(), 1);
            assert!(ws.quick_panel.is_some());
            assert!(ws.status.as_ref().is_some_and(|m| m.0.contains("language")));
        });
    }

    #[gpui::test]
    fn u02_template_native_menu_searches_definition_metadata_and_language_ids(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let path = definition("unique-file", "template");
        std::fs::write(&path, "<snippet><content>template</content><tabTrigger>unique-trigger</tabTrigger><description>Shared title</description></snippet>").unwrap();
        let folder = session::support_dir().join("Languages/zeta-id");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("language.json"),
            r#"{"schema":1,"id":"zeta-id","name":"Data Notation","scope":"source.zeta","files":{}}"#,
        )
        .unwrap();
        ws.update_in(cx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
        let action = ws.read_with(cx, |ws, cx| {
            ws.menus(cx)
                .into_iter()
                .find(|menu| menu.name == "File")
                .unwrap()
                .items
                .into_iter()
                .find_map(|item| match item {
                    gpui::MenuItem::Action { name, action, .. } if name == "New From Template…" => Some(action),
                    _ => None,
                })
                .expect("File > New From Template")
        });
        cx.update(|w, cx| w.dispatch_action(action, cx));
        cx.run_until_parked();
        for query in ["unique-file", "unique-trigger"] {
            ws.update_in(cx, |ws, w, cx| {
                let p = ws.quick_panel.as_ref().unwrap();
                p.input.update(cx, |e, cx| e.set_text(query, cx));
                ws.quick_panel_input_changed(w, cx);
                let p = ws.quick_panel.as_ref().unwrap();
                assert_eq!(p.filtered.len(), 1, "searchable definition metadata: {query}");
                assert!(p.choice_details, "disambiguate matching descriptions by local file");
            });
        }
        cx.simulate_keystrokes("enter");
        choose(cx, "zeta-id");
        assert_eq!(text(&ws, cx), "template");
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).language_choice(), Some("zeta-id")));
    }
}
