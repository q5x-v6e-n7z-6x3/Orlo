//! Snippets in a view: Tab expands the trigger before the carets or goes to the next field,
//! the field keys, a snippet chosen in the quick panel, and the completion list's snippet rows.

use super::*;
use crate::editing::snippet::{self, Snippet, Snippets};
use crate::languages::complete::{Candidate, Insert, Kind};
use crate::languages::highlight::ScopeId;
use gpui::{AnyElement, PromptLevel, ScrollWheelEvent, Size};
use std::collections::HashSet;
use std::path::Path;

/// The Snippets folder's snippets, for every window: looked at again when they are looked up,
/// at most once a second.
pub struct SnippetStore(pub Snippets);

impl gpui::Global for SnippetStore {}

impl Default for SnippetStore {
    fn default() -> Self {
        Self(Snippets::new(snippet::dir()))
    }
}

/// The store looked at again; when that loaded it and left files out, which and why, for the
/// load's one alert.
fn reload_problems(cx: &mut App) -> Option<String> {
    let store = &mut cx.default_global::<SnippetStore>().0;
    if !store.refresh() || store.errors().is_empty() {
        return None;
    }
    let dir = snippet::dir();
    let lines: Vec<String> = (store.errors().iter())
        .map(|(path, why)| format!("{}: {why}", path.strip_prefix(&dir).unwrap_or(path).display()))
        .collect();
    Some(lines.join("\n"))
}

fn alert_problems(window: &mut Window, cx: &mut App, detail: &str) {
    let title = "Some snippets were not loaded";
    drop(window.prompt(PromptLevel::Warning, title, Some(detail), &["OK"], cx));
}

/// The store, looked at again before it is read; a load that left files out says so, once.
pub fn refresh_snippets(window: &mut Window, cx: &mut App) {
    if let Some(detail) = reload_problems(cx) {
        alert_problems(window, cx, &detail);
    }
}

/// What a snippet is called in a list: its description, else its trigger, else its file's name.
pub fn snippet_title(s: &Snippet) -> String {
    match (s.description.as_str(), s.trigger.as_str()) {
        ("", "") => (s.path.file_stem()).map_or_else(String::new, |n| n.to_string_lossy().into()),
        ("", trigger) => trigger.to_string(),
        (description, _) => description.to_string(),
    }
}

/// Values captured only when an expansion is committed, shared by all its carets.
pub(crate) struct SnippetVariables {
    time: Option<orlo_utils::date_time::Snapshot>,
    clipboard: String,
    workspace_name: String,
    workspace_folder: String,
    relative_filepath: String,
}

impl SnippetVariables {
    pub(crate) fn for_template(roots: &[crate::config::project::Root], cx: &App) -> Self {
        let time = capture_time(cx).ok();
        let clipboard = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        Self::new(time, clipboard, roots, None)
    }

    fn new(
        time: Option<orlo_utils::date_time::Snapshot>,
        clipboard: String,
        roots: &[crate::config::project::Root],
        path: Option<&Path>,
    ) -> Self {
        let root = match path {
            Some(path) => {
                roots.iter().filter(|r| path.starts_with(&r.path)).max_by_key(|r| r.path.components().count())
            }
            None => roots.first(),
        };
        Self {
            time,
            clipboard,
            workspace_name: root.map(|r| r.label()).unwrap_or_default(),
            workspace_folder: root.map(|r| r.path.to_string_lossy().into_owned()).unwrap_or_default(),
            relative_filepath: path
                .and_then(|p| root.and_then(|r| p.strip_prefix(&r.path).ok()).or_else(|| p.file_name().map(Path::new)))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    pub(crate) fn resolve(&self, name: &str) -> Option<String> {
        if let Some(value) = self.time.and_then(|t| t.local.variable(name)) {
            return Some(value);
        }
        Some(match name {
            "CLIPBOARD" => self.clipboard.clone(),
            "WORKSPACE_NAME" => self.workspace_name.clone(),
            "WORKSPACE_FOLDER" => self.workspace_folder.clone(),
            "RELATIVE_FILEPATH" => self.relative_filepath.clone(),
            "TM_FULLNAME" => snippet::full_name(),
            _ => return None,
        })
    }
}

#[cfg(test)]
pub(crate) struct TestClock(pub Rc<dyn Fn() -> Result<orlo_utils::date_time::Snapshot, String>>);
#[cfg(test)]
impl gpui::Global for TestClock {}

pub(crate) fn capture_time(cx: &App) -> Result<orlo_utils::date_time::Snapshot, String> {
    #[cfg(test)]
    if let Some(clock) = cx.try_global::<TestClock>() {
        return (clock.0)();
    }
    let _ = cx;
    orlo_utils::date_time::Snapshot::now()
}

/// The values belong to the snippet session, never to a completion provider or its task.
pub(super) struct SnippetChoice {
    pub(super) values: Arc<[String]>,
    pub(super) selected: usize,
    pub(super) first: usize,
    wheel: f32,
    source: (u64, u64, Option<PathBuf>, String),
    selections: Vec<Selection>,
    primary: usize,
    selection_generation: u64,
    viewport: Size<Pixels>,
}

impl Editor {
    #[cfg(test)]
    pub(crate) fn snippet_choice_list(&self) -> Option<(Vec<String>, usize)> {
        self.snippet_choice.as_ref().map(|c| (c.values.to_vec(), c.selected))
    }

    pub(crate) fn close_snippet_choices(&mut self, cx: &mut Context<Self>) {
        self.snippet_choice = None;
        self.sel(cx).close_snippet_choices();
        cx.notify();
    }

    pub(super) fn refresh_snippet_choices(&mut self, window: &Window, cx: &mut Context<Self>) {
        let values = self.read_state(cx).snippet_choices().cloned();
        let Some(values) = values else {
            self.snippet_choice = None;
            return;
        };
        let doc = self.doc(cx);
        let source = (doc.id(), doc.version, doc.path.clone(), self.highlighter.syntax_name().to_string());
        let generation = self.read_state(cx).selection_generation();
        let allowed = self.input_enabled
            && !doc.read_only
            && !doc.edits_blocked()
            && self.marked.is_empty()
            && self.focus_handle.is_focused(window)
            && window.is_window_active()
            && !self.state.selections.iter().any(Selection::has_virtual);
        let stale = self.snippet_choice.as_ref().is_some_and(|list| {
            Arc::ptr_eq(&list.values, &values)
                && (list.source != source
                    || list.selections != self.state.selections
                    || list.primary != self.state.primary()
                    || list.selection_generation != generation
                    || list.viewport != window.viewport_size())
        });
        if !allowed || stale {
            self.close_snippet_choices(cx);
            return;
        }
        if self.snippet_choice.as_ref().is_some_and(|list| Arc::ptr_eq(&list.values, &values)) {
            return;
        }
        self.close_completion(cx);
        let selection = self.state.selections[self.state.primary()].range();
        let selected = values
            .iter()
            .position(|v| doc_of(&self.document, cx).rope.slice(selection.clone()) == v.as_str())
            .unwrap_or(0);
        self.snippet_choice = Some(SnippetChoice {
            values,
            selected,
            first: selected.saturating_sub(popup::ROWS - 1),
            wheel: 0.,
            source,
            selections: self.state.selections.clone(),
            primary: self.state.primary(),
            selection_generation: generation,
            viewport: window.viewport_size(),
        });
    }

    pub(super) fn commit_snippet_choice(
        &mut self,
        row: Option<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.refresh_snippet_choices(window, cx);
        let Some(choice) = self.snippet_choice.take() else { return false };
        let index = row.unwrap_or(choice.selected);
        let mut done = false;
        self.edit_text(cx, |s| done = s.choose_snippet(index));
        done
    }

    fn scroll_snippet_choices(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = &mut self.snippet_choice else { return };
        choice.wheel -= f32::from(event.delta.pixel_delta(px(popup::ROW_H)).y);
        let rows = (choice.wheel / popup::ROW_H).trunc();
        choice.wheel -= rows * popup::ROW_H;
        choice.first =
            choice.first.saturating_add_signed(rows as isize).min(choice.values.len().saturating_sub(popup::ROWS));
        cx.notify();
    }

    pub(super) fn render_snippet_choices(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let choice = self.snippet_choice.as_ref()?;
        let end = (choice.first + popup::ROWS).min(choice.values.len());
        let rows = (choice.first..end).map(|i| {
            let values = choice.values.clone();
            let selected = i == choice.selected;
            let label =
                if choice.values[i].is_empty() { "(empty)".into() } else { choice.values[i].replace('\n', "↵") };
            div()
                .id(("snippet-choice", i))
                .h(px(popup::ROW_H))
                .px(px(6.))
                .rounded(px(crate::chrome::ui::RADIUS))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(ui(if selected { theme::QP_LABEL_SELECTED } else { theme::QP_LABEL }))
                .when(selected, |d| d.bg(ui(theme::QP_SELECTED_BG)))
                .when(!selected, |d| d.hover(|s| s.bg(ui(theme::ELEMENT_HOVER).opacity(0.6))))
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |e, _, window, cx| {
                        e.refresh_snippet_choices(window, cx);
                        if e.snippet_choice.as_ref().is_some_and(|c| Arc::ptr_eq(&c.values, &values)) {
                            e.commit_snippet_choice(Some(i), window, cx);
                        }
                    }),
                )
        });
        let body = div()
            .id("snippet-choices")
            .min_w(px(160.))
            .max_w(px(560.))
            .flex()
            .flex_col()
            .on_scroll_wheel(cx.listener(Self::scroll_snippet_choices))
            .children(rows);
        let at = self.state.selections[self.state.primary()].head;
        self.popup(at, px(popup::ROW_H * (end - choice.first) as f32 + 10.), px(11.), window, body, cx)
    }

    /// Tab: in a session the next field, else the snippet whose trigger is before every
    /// caret (the quick panel asks between several). False leaves Tab to indent: a one-line
    /// field, a composition (the input method's key), or no trigger.
    pub(crate) fn tab_snippet(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.single_line || !self.marked.is_empty() {
            return false;
        }
        let mut moved = false;
        if self.state.in_snippet() {
            self.edit(cx, |s| moved = s.snippet_next(true));
        }
        moved || self.expand_trigger(window, cx)
    }

    fn expand_trigger(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.state.selections.iter().any(|x| !x.is_empty()) {
            return false;
        }
        refresh_snippets(window, cx);
        let Some((trigger, mut found)) = self.triggered(&cx.global::<SnippetStore>().0, cx) else {
            return false;
        };
        if found.len() == 1 {
            return self.expand(&found.remove(0), &trigger, cx);
        }
        let paths = found.into_iter().map(|s| s.path).collect();
        cx.emit(EditorEvent::ChooseSnippet(paths, trigger));
        true
    }

    /// The longest trigger `trigger_before` finds at every caret, with the snippets whose scope
    /// matches where it starts at each caret.
    fn triggered(&self, store: &Snippets, cx: &App) -> Option<(String, Vec<Snippet>)> {
        let s = self.read_state(cx);
        let caret = s.cursor();
        let mut triggers: Vec<&str> =
            (store.snippets().iter()).map(|x| x.trigger.as_str()).filter(|t| s.trigger_before(caret, t)).collect();
        triggers.sort_by(|a, b| (b.chars().count().cmp(&a.chars().count())).then(a.cmp(b)));
        triggers.dedup();
        let sels = &s.view.selections;
        for t in triggers {
            if !sels.iter().all(|x| s.trigger_before(x.head, t)) {
                continue;
            }
            // a line just typed on is not coloured yet: its top scope then
            let n = t.chars().count();
            let ids: HashSet<ScopeId> =
                (sels.iter()).map(|x| self.highlighter.scope_id_at(&s.doc.rope, x.head - n)).collect();
            // the snippets whose scope matches at every caret
            let found = (ids.into_iter())
                .map(|id| store.with_trigger(t, |selector| highlight::matches(id, selector)))
                .reduce(|found, here| found.into_iter().filter(|i| here.contains(i)).collect())
                .unwrap_or_default();
            if !found.is_empty() {
                let snippets = found.iter().map(|&i| store.snippets()[i].clone()).collect();
                return Some((t.to_string(), snippets));
            }
        }
        None
    }

    pub(crate) fn insert_date_time(&mut self, action: &InsertDateTime, _: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.try_insert_date_time(action, cx) {
            self.status(error, cx);
        }
    }

    pub(crate) fn try_insert_date_time(
        &mut self,
        action: &InsertDateTime,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        orlo_utils::date_time::validate_format(&action.format)?;
        if !self.input_enabled || self.single_line || self.doc(cx).read_only || self.doc(cx).edits_blocked() {
            return Err("Document is read only".into());
        }
        if !self.marked.is_empty() {
            return Err("Finish or cancel text composition before inserting date/time".into());
        }
        let time = capture_time(cx)?;
        self.edit(cx, |s| s.insert_date_time(&action.format, action.utc, &time)).map(|_| ())
    }

    pub(crate) fn snippet_variables(&self, cx: &App) -> SnippetVariables {
        let time = capture_time(cx).ok();
        let clipboard = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        let index = self.completion_index.as_ref().and_then(|i| i.upgrade());
        let roots = index.as_ref().map(|i| i.read(cx).roots()).unwrap_or_default();
        SnippetVariables::new(time, clipboard, roots, self.doc(cx).path.as_deref())
    }

    /// `s` expanded at the carets with `trigger` before them (at every selection for an empty
    /// one), as one undo step; false when none had it.
    fn expand(&mut self, s: &Snippet, trigger: &str, cx: &mut Context<Self>) -> bool {
        let vars = self.snippet_variables(cx);
        let mut done = false;
        self.edit_text(cx, |st| done = st.expand_snippet(s, trigger, &|name, _| vars.resolve(name)));
        done
    }

    /// Tab in a session (`Editor && snippet`): the next field. A session gone since the frame
    /// that bound the key leaves Tab to do what it does without one.
    pub(crate) fn next_field(&mut self, _: &NextField, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_snippet_choice(None, window, cx);
        self.step_field(true, cx);
    }

    pub(crate) fn prev_field(&mut self, _: &PrevField, _: &mut Window, cx: &mut Context<Self>) {
        self.close_snippet_choices(cx);
        self.step_field(false, cx);
    }

    fn step_field(&mut self, forward: bool, cx: &mut Context<Self>) {
        let mut moved = false;
        if self.marked.is_empty() && self.state.in_snippet() {
            self.edit(cx, |s| moved = s.snippet_next(forward));
        }
        if !moved {
            cx.propagate();
        }
    }

    /// Escape in a session: it ends, the selections staying.
    pub(crate) fn clear_fields(&mut self, _: &ClearFields, _: &mut Window, cx: &mut Context<Self>) {
        if !self.marked.is_empty() || !self.state.in_snippet() {
            return cx.propagate();
        }
        self.sel(cx).end_snippet();
        cx.notify();
    }

    /// A snippet chosen in the quick panel (Insert Snippet's, or one of several with the trigger),
    /// or named by a key binding or a macro.
    pub(crate) fn expand_snippet(&mut self, a: &ExpandSnippet, window: &mut Window, cx: &mut Context<Self>) {
        if self.single_line {
            return;
        }
        // a key or a macro may come before anything read the folder; a list may be older than
        // the folder's last load
        refresh_snippets(window, cx);
        let store = &cx.global::<SnippetStore>().0;
        let Some(s) = store.snippets().iter().find(|s| s.path == a.path).cloned() else {
            return self.status("That snippet is no longer in the Snippets folder", cx);
        };
        self.expand(&s, &a.trigger, cx);
    }

    /// Where Insert Snippet looks snippets up: the scope at the primary selection's start.
    pub fn snippet_scope(&self, cx: &App) -> ScopeId {
        let s = self.read_state(cx);
        (self.highlighter).scope_id_at(&s.doc.rope, s.view.selections[s.primary()].start())
    }

    /// The completion list's snippets: those whose scope matches `id`, labelled by their
    /// trigger, else by their description.
    pub(crate) fn snippet_rows(&self, id: ScopeId, window: &Window, cx: &mut Context<Self>) -> Vec<Candidate> {
        if let Some(detail) = reload_problems(cx) {
            cx.defer_in(window, move |_, window, cx| alert_problems(window, cx, &detail));
        }
        let store = &cx.global::<SnippetStore>().0;
        (store.for_scope(|selector| highlight::matches(id, selector)).into_iter())
            .map(|i| {
                let s = &store.snippets()[i];
                let (label, detail) = match s.trigger.is_empty() {
                    true => (snippet_title(s), String::new()),
                    false => (s.trigger.clone(), s.description.clone()),
                };
                Candidate { label, kind: Kind::Snippet, detail, insert: Insert::Snippet(s.path.clone()) }
            })
            .collect()
    }

    /// A snippet row committed: the snippet in place of what is typed of its trigger (`\beg` of
    /// `\begin`), else of the prefix, at every caret that has it.
    pub(crate) fn commit_snippet(&mut self, path: &Path, prefix: &str, cx: &mut Context<Self>) {
        let store = &cx.default_global::<SnippetStore>().0;
        let Some(s) = store.snippets().iter().find(|s| s.path == path).cloned() else {
            return;
        };
        let st = self.read_state(cx);
        let caret = st.cursor();
        let typed = (prefix.chars().count()..=s.trigger.chars().count())
            .rev()
            .map(|k| s.trigger.chars().take(k).collect::<String>())
            .find(|t| st.trigger_before(caret, t))
            .unwrap_or_else(|| prefix.to_string());
        self.expand(&s, &typed, cx);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};

    /// A folder of this test's own with these snippet files: (file name, text).
    fn folder(name: &str, files: &[(&str, String)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orlo-snip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        dir
    }

    /// A snippet file's text.
    fn xml(content: &str, trigger: &str, scope: &str, description: &str) -> String {
        format!(
            "<snippet><content><![CDATA[{content}]]></content><tabTrigger>{trigger}</tabTrigger>\
             <scope>{scope}</scope><description>{description}</description></snippet>"
        )
    }

    /// An editor of the file `path` alone in the active 600 × 300 window, with the editor's keys
    /// and the snippets of `dir`, focused, its caret at `|`.
    fn editor<'a>(
        text: &str,
        path: &str,
        dir: PathBuf,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Editor>, &'a mut VisualTestContext) {
        cx.update(|cx| cx.set_global(SnippetStore(Snippets::new(dir))));
        super::super::focused_editor(text, Some(Path::new(path)), cx)
    }

    fn text(e: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
        e.read_with(cx, |e, cx| e.text(cx))
    }

    /// The selected texts, and whether a session is on.
    fn fields(e: &Entity<Editor>, cx: &mut VisualTestContext) -> (Vec<String>, bool) {
        e.read_with(cx, |e, cx| {
            let s = e.read_state(cx);
            let sel = (s.view.selections.iter()).map(|x| s.doc.rope.slice(x.range()).to_string());
            (sel.collect(), s.in_snippet())
        })
    }

    fn list(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<Vec<String>> {
        e.read_with(cx, |e, _| e.completion_list().map(|l| l.0))
    }

    pub(crate) fn fixed_clock(cx: &mut App, calls: Rc<std::cell::Cell<usize>>) {
        cx.set_global(TestClock(Rc::new(move || {
            calls.set(calls.get() + 1);
            orlo_utils::date_time::Snapshot::capture(
                || std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_709_251_645),
                |seconds| orlo_utils::date_time::DateTime::with_offset(seconds, -18_000),
            )
        })));
    }

    #[gpui::test]
    fn n32_local_expansion_captures_one_instant_only_on_commit(cx: &mut TestAppContext) {
        let body = "$CURRENT_YEAR-$CURRENT_MONTH-$CURRENT_DATE $CURRENT_HOUR:$CURRENT_MINUTE:$CURRENT_SECOND $CURRENT_SECONDS_UNIX $TM_LINE_NUMBER ${CLIPBOARD:empty} ${1|one,two|}:$1$0";
        let dir = folder("n32-clock", &[("clock.sublime-snippet", xml(body, "c", "", ""))]);
        let (e, cx) = editor("c|\nc", "/tmp/x.rs", dir, cx);
        let calls = Rc::new(std::cell::Cell::new(0));
        cx.update(|_, cx| fixed_clock(cx, calls.clone()));
        e.update_in(cx, |e, w, cx| {
            let id = e.snippet_scope(cx);
            assert_eq!(e.snippet_rows(id, w, cx).len(), 1);
        });
        assert_eq!(calls.get(), 0, "discovery must not capture runtime values");
        e.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::caret(1), Selection::caret(3)]));
        cx.simulate_keystrokes("tab");
        assert_eq!(calls.get(), 1);
        assert_eq!(
            text(&e, cx),
            "2024-02-29 19:07:25 1709251645 1 empty one:one\n2024-02-29 19:07:25 1709251645 2 empty one:one"
        );
        cx.simulate_keystrokes("down enter");
        assert!(text(&e, cx).contains("two:two"));
        assert_eq!(calls.get(), 1, "choices keep the captured expansion");
    }

    #[gpui::test]
    fn n32_clock_failure_preserves_plain_insertion_and_snippet_defaults(cx: &mut TestAppContext) {
        let dir = folder(
            "n32-clock-error",
            &[("clock.sublime-snippet", xml("${CURRENT_YEAR:missing}:$TM_FILENAME$0", "c", "", ""))],
        );
        let (e, cx) = editor("c|", "/tmp/x.rs", dir, cx);
        cx.update(|_, cx| cx.set_global(TestClock(Rc::new(|| Err("Clock conversion failed".into())))));
        e.update(cx, |e, cx| {
            let before = (e.text(cx), e.state.selections.clone(), e.doc(cx).version);
            let action = InsertDateTime { format: orlo_utils::date_time::DEFAULT_FORMAT.into(), utc: false };
            assert_eq!(e.try_insert_date_time(&action, cx).unwrap_err(), "Clock conversion failed");
            assert_eq!((e.text(cx), e.state.selections.clone(), e.doc(cx).version), before);
        });
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "missing:x.rs");
    }

    #[test]
    fn n32_workspace_context_uses_containing_root_and_empty_or_filename_fallbacks() {
        use crate::config::project::Root;
        let mut nested = Root::new("/work/project/sub".into());
        nested.name = Some("Named root".into());
        let roots = [Root::new("/work/project".into()), nested];
        for (path, name, folder, relative) in [
            (Some("/work/project/sub/界.rs"), "Named root", "/work/project/sub", "界.rs"),
            (Some("/work/project/src/x.rs"), "project", "/work/project", "src/x.rs"),
            (None, "project", "/work/project", ""),
            (Some("/work/project-other/x.rs"), "", "", "x.rs"),
        ] {
            let vars = SnippetVariables::new(None, String::new(), &roots, path.map(Path::new));
            for (key, expected) in [
                ("WORKSPACE_NAME", name),
                ("WORKSPACE_FOLDER", folder),
                ("RELATIVE_FILEPATH", relative),
                ("CLIPBOARD", ""),
            ] {
                assert_eq!(vars.resolve(key).as_deref(), Some(expected));
            }
        }
        let vars = SnippetVariables::new(None, String::new(), &[], None);
        for name in ["WORKSPACE_NAME", "WORKSPACE_FOLDER", "RELATIVE_FILEPATH", "CLIPBOARD"] {
            assert_eq!(vars.resolve(name).as_deref(), Some(""));
        }
        for name in ["RANDOM", "UUID", "date", "CURRENT_day"] {
            assert!(vars.resolve(name).is_none());
        }
    }

    #[gpui::test]
    fn n32_clipboard_variables_are_raw_shared_values_and_undo_normally(cx: &mut TestAppContext) {
        let dir = folder(
            "n32-clipboard",
            &[("clip.sublime-snippet", xml("${CLIPBOARD:missing}:$TM_FILENAME$0", "c", "", ""))],
        );
        let (e, cx) = editor("c|\nc", "/tmp/x.rs", dir, cx);
        cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("界\n$(date) `whoami` $1".into())));
        e.update(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::caret(1), Selection::caret(3)]));
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "界\n$(date) `whoami` $1:x.rs\n界\n$(date) `whoami` $1:x.rs");
        cx.dispatch_action(Undo);
        assert_eq!(text(&e, cx), "c\nc");
    }

    #[gpui::test]
    fn n31_local_choice_keys_confirm_then_navigate_fields(cx: &mut TestAppContext) {
        let dir = folder("choice-keys", &[("f.sublime-snippet", xml("${1|one,界|}=$1 ${2:end}$0", "f", "", ""))]);
        let (e, cx) = editor("f|", "/tmp/x.txt", dir, cx);
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "one=one end");
        cx.simulate_keystrokes("down enter");
        assert_eq!(text(&e, cx), "界=界 end");
        assert_eq!(fields(&e, cx), (vec!["界".into(), "界".into()], true));
        cx.simulate_keystrokes("tab");
        assert_eq!(fields(&e, cx), (vec!["end".into()], true));
        cx.simulate_keystrokes("shift-tab up tab");
        assert_eq!(text(&e, cx), "one=one end", "Tab confirms before moving forward");
        assert_eq!(fields(&e, cx), (vec!["end".into()], true));
        cx.simulate_keystrokes("shift-tab down tab");
        assert_eq!(text(&e, cx), "界=界 end");
        assert_eq!(fields(&e, cx), (vec!["end".into()], true));
        cx.simulate_keystrokes("shift-tab up escape");
        assert_eq!(text(&e, cx), "界=界 end", "cancel never applies the highlighted row");
        assert_eq!(fields(&e, cx), (vec!["界".into(), "界".into()], true));
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_none()));
        cx.simulate_input("é");
        assert_eq!(text(&e, cx), "é=é end");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&e, cx), "界=界 end");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&e, cx), "one=one end");
        cx.simulate_keystrokes("cmd-shift-z");
        assert_eq!(text(&e, cx), "界=界 end");
    }

    #[gpui::test]
    fn n31_multiline_choice_has_a_visible_anchor_and_keyboard_confirmation(cx: &mut TestAppContext) {
        let long = format!("{}last", "row\n".repeat(100));
        let body = format!("${{1|{long},short|}} ${{2:end}}$0");
        let dir = folder("choice-multiline-anchor", &[("f.sublime-snippet", xml(&body, "f", "", ""))]);
        let (e, cx) = editor("f|", "/tmp/x.txt", dir, cx);
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        assert_eq!(text(&e, cx), format!("{long} end"));
        e.update_in(cx, |e, w, cx| {
            assert!(e.snippet_choice_list().is_some());
            assert!(e.char_bounds(e.state.cursor(), cx).is_some(), "the selected field's head is visible");
            assert!(e.char_bounds(e.state.selections[0].start(), cx).is_none(), "exercise an offscreen field start");
            assert!(e.render_snippet_choices(w, cx).is_some(), "a list consuming keys must have a visible anchor");
        });
        cx.simulate_keystrokes("down enter");
        assert_eq!(text(&e, cx), "short end");
        assert_eq!(fields(&e, cx), (vec!["short".into()], true));
        cx.simulate_keystrokes("tab");
        assert_eq!(fields(&e, cx), (vec!["end".into()], true));
    }

    #[gpui::test]
    fn n31_choice_list_keeps_editor_focus_and_closes_on_ime_and_incompatible_context(cx: &mut TestAppContext) {
        let dir = folder("choice-context", &[("f.sublime-snippet", xml("${1|one,two|} $1 ${2:end}$0", "f", "", ""))]);
        let (e, cx) = editor("f|", "/tmp/x.txt", dir, cx);
        cx.simulate_keystrokes("tab");
        assert_eq!(e.read_with(cx, |e, _| e.snippet_choice_list()), Some((vec!["one".into(), "two".into()], 0)));
        e.update_in(cx, |e, w, _| assert!(e.is_focused(w), "the editor retains keyboard focus"));
        e.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(None, "k", None, w, cx));
        cx.run_until_parked();
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_none()));
        e.update_in(cx, |_, w, _| assert!(!w.context_stack().iter().any(|c| c.contains("snippet_choice"))));
        cx.simulate_keystrokes("tab");
        assert!(fields(&e, cx).0.iter().all(|s| s != "end"), "Tab remains the input method's key");
        e.update_in(cx, |e, w, cx| e.replace_text_in_range(None, "k", w, cx));
        cx.simulate_keystrokes("tab shift-tab");
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_some()));
        e.update(cx, |e, cx| e.edit(cx, |s| s.move_cursors(crate::editing::text::Motion::Right, false)));
        cx.run_until_parked();
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_none()));
        assert!(fields(&e, cx).1, "moving within the field keeps its session");
        cx.simulate_keystrokes("tab shift-tab");
        e.update(cx, |e, cx| {
            e.highlighter = Highlighter::by_name("Python");
            cx.notify();
        });
        cx.run_until_parked();
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_none()));
        cx.simulate_keystrokes("tab shift-tab");
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_some()));
        cx.simulate_resize(gpui::size(px(650.), px(300.)));
        cx.run_until_parked();
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_none()));
        cx.simulate_keystrokes("tab shift-tab");
        e.update_in(cx, |e, w, cx| {
            let other = cx.focus_handle();
            w.focus(&other);
            e.refresh_snippet_choices(w, cx);
            assert!(e.snippet_choice_list().is_none());
        });
    }

    #[gpui::test]
    fn n31_multicaret_choices_keep_mirrors_and_close_on_peer_and_protected_changes(cx: &mut TestAppContext) {
        let dir = folder(
            "choice-peers",
            &[("f.sublime-snippet", xml(r"${1|a,b|}:$1:${1/(.*)/<$1>/} ${2:end}$0", "f", "", ""))],
        );
        let (e, cx) = editor("f\nf|", "/tmp/x.txt", dir, cx);
        e.update(cx, |e, cx| e.edit(cx, |s| s.set_selections(vec![Selection::caret(1), Selection::caret(3)])));
        cx.simulate_keystrokes("tab down enter");
        assert_eq!(text(&e, cx), "b:b:<b> end\nb:b:<b> end");
        assert_eq!(fields(&e, cx), (vec!["b".into(); 4], true));
        // Direct model field navigation also reopens the new active field's choices.
        e.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.snippet_next(true);
                s.snippet_next(false);
            })
        });
        cx.run_until_parked();
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_some()));
        let other = e.update(cx, |e, cx| {
            let document = e.document.clone();
            let view = e.state.duplicate(e.doc(cx));
            cx.new(|cx| Editor::of_document(document, view, false, cx))
        });
        other.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("!");
            })
        });
        cx.run_until_parked();
        assert!(e.read_with(cx, |e, _| e.snippet_choice_list().is_none()));
        assert!(!fields(&e, cx).1);
        for mode in ["read-only", "viewer", "follow", "immutable", "path", "scroll", "outside"] {
            e.update_in(cx, |e, w, cx| {
                e.edit(cx, |s| {
                    let end = s.len();
                    s.set_cursor(end);
                    s.complete_snippet_ranges(end..end, "${1|a,b|} ${2:end}$0", Vec::new()).unwrap();
                });
                e.refresh_snippet_choices(w, cx);
                assert!(e.snippet_choice_list().is_some(), "{mode}");
                match mode {
                    "read-only" => e.document.update(cx, |d, _| d.model.read_only = true),
                    "viewer" => {
                        e.document.update(cx, |d, _| d.model.set_viewer_mode(true));
                    }
                    "follow" => {
                        e.document.update(cx, |d, _| d.model.set_following(true));
                    }
                    "immutable" => e.input_enabled = false,
                    "path" => e.document.update(cx, |d, _| d.model.path = Some("/tmp/renamed.txt".into())),
                    "scroll" => e.scrolled(cx),
                    "outside" => e.edit(cx, |s| {
                        s.set_cursor(0);
                        s.insert("!");
                    }),
                    _ => unreachable!(),
                }
                e.refresh_snippet_choices(w, cx);
                assert!(e.snippet_choice_list().is_none(), "{mode}");
                let before = e.text(cx);
                assert!(!e.commit_snippet_choice(Some(1), w, cx), "{mode}");
                assert_eq!(e.text(cx), before, "{mode}");
                e.document.update(cx, |d, _| {
                    d.model.read_only = false;
                    d.model.set_viewer_mode(false);
                    d.model.set_following(false);
                });
                e.input_enabled = true;
            });
        }
    }

    #[gpui::test]
    fn tab_expands_a_trigger_only_where_the_snippets_scope_matches(cx: &mut TestAppContext) {
        let dir = folder(
            "scope",
            &[
                ("fn.sublime-snippet", xml("fn ${1:name}() {}", "fn", "source.rust - comment", "")),
                ("def.sublime-snippet", xml("def ${1:f}():", "def", "source.python", "")),
                ("all.sublime-snippet", xml("[${1:x}]", "all", "", "")),
            ],
        );
        let (e, cx) = editor("// fn|\nfn\ndef\nall\n", "/tmp/x.rs", dir, cx);
        // in a comment the Rust snippet is left out: Tab indents
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "// fn   \nfn\ndef\nall\n");
        assert!(!fields(&e, cx).1, "no session");
        e.update(cx, |e, cx| e.edit(cx, |s| s.set_cursor(s.line_end(1))));
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "// fn   \nfn name() {}\ndef\nall\n");
        assert_eq!(fields(&e, cx), (vec!["name".into()], true));
        // another syntax's snippet is left out; one without a scope applies everywhere
        cx.simulate_keystrokes("escape");
        for (line, after) in [(2, "def "), (3, "[x]")] {
            e.update(cx, |e, cx| e.edit(cx, |s| s.set_cursor(s.line_end(line))));
            cx.simulate_keystrokes("tab");
            let t = text(&e, cx);
            assert_eq!(t.lines().nth(line), Some(after), "{t:?}");
        }
    }

    #[gpui::test]
    fn tab_commits_the_list_first_then_moves_between_fields(cx: &mut TestAppContext) {
        let dir = folder("keys", &[("f.sublime-snippet", xml("f(${1:x}, ${2:y})", "f", "", ""))]);
        // plain text: no keywords on the list
        let (e, cx) = editor("alpha\n|", "/tmp/x.txt", dir, cx);
        cx.simulate_input("f");
        cx.simulate_keystrokes("tab");
        assert_eq!(fields(&e, cx), (vec!["x".into()], true));
        // typing in the field opens the list: Tab commits it, and the session goes on
        cx.simulate_input("al");
        assert_eq!(list(&e, cx), Some(vec!["alpha".into()]));
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "alpha\nf(alpha, y)");
        assert_eq!(list(&e, cx), None);
        cx.simulate_keystrokes("tab");
        assert_eq!(fields(&e, cx), (vec!["y".into()], true));
        // Shift-Tab with the list open closes it, then goes to the previous field
        cx.simulate_input("al");
        assert!(list(&e, cx).is_some());
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(list(&e, cx), None);
        assert_eq!(fields(&e, cx), (vec!["alpha".into()], true));
        // Escape: the list first, then the session (the selections stay), then the editor's
        cx.simulate_input("al");
        assert!(list(&e, cx).is_some());
        cx.simulate_keystrokes("escape");
        assert_eq!((list(&e, cx), fields(&e, cx).1), (None, true));
        cx.simulate_keystrokes("escape");
        assert_eq!(fields(&e, cx), (vec![String::new()], false));
        assert_eq!(text(&e, cx), "alpha\nf(al, al)");
        // Tab on the last field goes to the end of the expansion and ends the session
        e.update(cx, |e, cx| e.set_text("alpha\n", cx));
        cx.simulate_keystrokes("cmd-down");
        cx.simulate_input("f");
        cx.simulate_keystrokes("tab tab tab");
        let caret = e.read_with(cx, |e, _| e.state.cursor());
        assert_eq!((text(&e, cx), caret, fields(&e, cx).1), ("alpha\nf(x, y)".into(), 13, false));
    }

    #[gpui::test]
    fn a_click_in_the_field_keeps_the_session_and_one_outside_ends_it(cx: &mut TestAppContext) {
        let dir = folder("click", &[("f.sublime-snippet", xml("f(${1:abc}, $2)", "f", "", ""))]);
        let (e, cx) = editor("|\n\nend", "/tmp/x.txt", dir, cx);
        cx.simulate_input("f");
        cx.simulate_keystrokes("tab");
        assert_eq!(fields(&e, cx), (vec!["abc".into()], true));
        let click = |at: usize, cx: &mut VisualTestContext| {
            let p = e.read_with(cx, |e, cx| e.char_bounds(at, cx).expect("on screen").center());
            cx.simulate_click(p, gpui::Modifiers::none());
        };
        click(3, cx);
        assert!(fields(&e, cx).1, "inside the field");
        click(9, cx);
        assert!(!fields(&e, cx).1, "on another line");
        // the keys are the editor's again
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "f(abc, )\n    \nend");
    }

    #[gpui::test]
    fn a_composition_keeps_tab_for_the_input_method(cx: &mut TestAppContext) {
        let dir = folder("ime", &[("f.sublime-snippet", xml("f(${1:x}, ${2:y})", "f", "", ""))]);
        let (e, cx) = editor("|", "/tmp/x.rs", dir, cx);
        // a trigger being composed is not expanded
        e.update_in(cx, |e, window, cx| e.replace_and_mark_text_in_range(None, "f", None, window, cx));
        cx.simulate_keystrokes("tab");
        assert!(!fields(&e, cx).1, "no session");
        assert!(!text(&e, cx).contains('('), "{:?}", text(&e, cx));
        // composing in a field: Tab is not the next field
        cx.simulate_keystrokes("cmd-a backspace");
        cx.simulate_input("f");
        cx.simulate_keystrokes("tab");
        assert_eq!(fields(&e, cx), (vec!["x".into()], true));
        e.update_in(cx, |e, window, cx| e.replace_and_mark_text_in_range(None, "k", None, window, cx));
        cx.run_until_parked();
        let context = |cx: &mut VisualTestContext| {
            e.update_in(cx, |_, window, _| window.context_stack().iter().any(|c| c.contains("snippet")))
        };
        assert!(!context(cx), "no field keys while composing");
        e.update_in(cx, |e, window, cx| e.replace_text_in_range(None, "k", window, cx));
        cx.run_until_parked();
        assert!(context(cx));
        cx.simulate_keystrokes("tab");
        assert_eq!(fields(&e, cx), (vec!["y".into()], true));
        assert_eq!(text(&e, cx), "f(k, y)");
    }

    #[gpui::test]
    fn a_snippet_is_on_the_list_by_its_trigger_and_committing_it_expands_it(cx: &mut TestAppContext) {
        let dir = folder(
            "rows",
            &[
                ("m.sublime-snippet", xml("match ${1:x} {}", "mtch", "source.rust", "match it")),
                ("h.sublime-snippet", xml("// ${1:note}", "", "", "header comment")),
                ("b.sublime-snippet", xml("\\begin{${1:env}}", "\\begin", "", "")),
                ("py.sublime-snippet", xml("pass", "mtpy", "source.python", "")),
            ],
        );
        let (e, cx) = editor("|", "/tmp/x.rs", dir, cx);
        cx.simulate_input("mt");
        let rows = e.read_with(cx, |e, _| e.completion_rows());
        let row = rows.iter().find(|r| r.label == "mtch").expect("the Rust snippet");
        assert_eq!((&row.kind, row.detail.as_str()), (&Kind::Snippet, "match it"));
        assert!(!rows.iter().any(|r| r.label == "mtpy"), "another syntax's");
        cx.simulate_keystrokes("enter");
        assert_eq!(text(&e, cx), "match x {}");
        assert_eq!(fields(&e, cx), (vec!["x".into()], true));
        cx.simulate_keystrokes("escape cmd-a backspace");
        // no trigger: its description is its label
        cx.simulate_input("hea");
        assert_eq!(list(&e, cx), Some(vec!["header comment".into()]));
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "// note");
        cx.simulate_keystrokes("escape cmd-a backspace");
        // what is typed of a trigger with punctuation goes with the word
        cx.simulate_input("\\beg");
        assert_eq!(list(&e, cx), Some(vec!["\\begin".into()]));
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "\\begin{env}");
    }

    #[gpui::test]
    fn several_snippets_for_a_trigger_go_to_the_workspace_to_choose(cx: &mut TestAppContext) {
        use std::{cell::RefCell, rc::Rc};
        let dir = folder(
            "several",
            &[
                ("a.sublime-snippet", xml("one", "x", "", "")),
                ("b.sublime-snippet", xml("two", "x", "source.rust", "")),
                ("c.sublime-snippet", xml("three", "x", "source.python", "")),
                ("d.sublime-snippet", xml("longer", "ax", "", "")),
            ],
        );
        let (e, cx) = editor("|", "/tmp/x.rs", dir.clone(), cx);
        let asked = Rc::new(RefCell::new(Vec::new()));
        let sink = asked.clone();
        let _sub = cx.update(|_, cx| {
            cx.subscribe(&e, move |_, event: &EditorEvent, _| {
                if let EditorEvent::ChooseSnippet(paths, trigger) = event {
                    sink.borrow_mut().push((paths.clone(), trigger.clone()));
                }
            })
        });
        cx.simulate_input(" x");
        cx.simulate_keystrokes("tab");
        let files = |names: &[&str]| -> Vec<PathBuf> { names.iter().map(|n| dir.join(n)).collect() };
        let want = (files(&["a.sublime-snippet", "b.sublime-snippet"]), "x".to_string());
        assert_eq!(*asked.borrow(), [want]);
        assert_eq!(text(&e, cx), " x", "nothing changed yet");
        // the choice comes back as an action; the longest trigger wins
        e.update_in(cx, |e, window, cx| {
            let path = dir.join("b.sublime-snippet");
            e.expand_snippet(&ExpandSnippet { path, trigger: "x".into() }, window, cx)
        });
        assert_eq!(text(&e, cx), " two");
        cx.simulate_input(" ax");
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), " two longer");
    }

    #[gpui::test]
    fn a_load_that_leaves_files_out_says_which_once(cx: &mut TestAppContext) {
        let dir = folder(
            "errors",
            &[
                ("good.sublime-snippet", xml("ok", "g", "", "")),
                ("bad.sublime-snippet", "<snippet><content>${1:x</content></snippet>".into()),
            ],
        );
        let (e, cx) = editor("g|", "/tmp/x.rs", dir, cx);
        cx.simulate_keystrokes("tab");
        let (message, detail) = cx.pending_prompt().expect("an alert");
        assert_eq!(message, "Some snippets were not loaded");
        assert!(detail.contains("bad.sublime-snippet") && detail.contains("`${`"), "{detail}");
        cx.simulate_prompt_answer("Ok");
        assert_eq!(text(&e, cx), "ok", "the good one still expands");
        cx.simulate_input(" g");
        cx.simulate_keystrokes("tab");
        assert!(!cx.has_pending_prompt(), "the same load says it once");
        assert_eq!(text(&e, cx), "ok ok");
    }
}
