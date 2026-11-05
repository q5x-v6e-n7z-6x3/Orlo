//! The regex playground. Its sample is a scratch tab of its own (typed, or a copy of a
//! document, which is never changed); the panel under it has the pattern, the replacement and
//! the toggles. As either changes, the sample is searched on another thread after a pause,
//! with the shared engine and its limits (`crate::searching::pattern`); a result for an older
//! pattern or text is dropped. The capture inspector and the replacement preview show the
//! current match.

use crate::chrome::find_panel::{dispatching, search_field};
use crate::searching::pattern::{Deadline, Matcher, Replacer, SearchError};
use crate::workspace::*;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

actions!(
    playground,
    [
        OpenPlayground,
        OpenPlaygroundWithDocument,
        PlaygroundNext,
        PlaygroundPrev,
        PlaygroundFocusNext,
        PlaygroundToFind,
        PlaygroundToReplace,
        PlaygroundToFindInFiles,
        PlaygroundPreview,
    ]
);

/// Bound after the editor's bindings so they win inside the panel's inputs.
pub fn key_bindings() -> Vec<KeyBinding> {
    let ctx = Some("PlaygroundPanel > Editor");
    vec![
        KeyBinding::new("enter", PlaygroundNext, ctx),
        KeyBinding::new("shift-enter", PlaygroundPrev, ctx),
        KeyBinding::new("tab", PlaygroundFocusNext, ctx),
        KeyBinding::new("escape", ClosePanel, ctx),
        KeyBinding::new("cmd-alt-r", ToggleFindRegex, ctx),
        KeyBinding::new("cmd-alt-c", ToggleFindCase, ctx),
        KeyBinding::new("cmd-alt-w", ToggleFindWord, ctx),
        KeyBinding::new("cmd-alt-a", TogglePreserveCase, ctx),
    ]
}

/// How long typing must pause before the sample is searched again.
const DEBOUNCE: Duration = Duration::from_millis(150);

/// The last search of the sample: the text and query it was for, and what it found.
struct Found {
    version: u64,
    query: String,
    opts: FindOpts,
    text: Arc<String>,
    result: Result<Vec<Range<usize>>, SearchError>,
}

pub struct PlaygroundPanel {
    pub pattern: Entity<Editor>,
    pub replace: Entity<Editor>,
    pub opts: FindOpts,
    pub preserve_case: bool,
    /// The sample tab.
    pub sample: WeakEntity<Editor>,
    /// The current match, an index into the matches.
    pub current: usize,
    found: Option<Found>,
    /// The search under way (its cancel flag), and the pause before the next one.
    running: Option<Arc<AtomicBool>>,
    debounce: Option<Task<()>>,
    /// Whether the next search selects its current match: not after typing in the sample,
    /// where the next key would overwrite it.
    select: bool,
    _subscriptions: [Subscription; 3],
}

use gpui::WeakEntity;

impl PlaygroundPanel {
    fn new(sample: &Entity<Editor>, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let field = |cx: &mut Context<Workspace>| {
            cx.new(|cx| {
                let mut e = Editor::new(EditorState::new(), true, cx);
                e.multiline = true;
                e
            })
        };
        let (pattern, replace) = (field(cx), field(cx));
        let _subscriptions = [
            Editor::on_text_change(&pattern, cx, |ws, cx| ws.playground_changed(cx)),
            // the replacement changes what is shown, not what matches
            Editor::on_text_change(&replace, cx, |_, cx| cx.notify()),
            Editor::on_text_change(sample, cx, |ws, cx| ws.playground_sample_changed(cx)),
        ];
        pattern.read(cx).focus(window);
        Self {
            pattern,
            replace,
            opts: FindOpts { regex: true, ..FindOpts::default() },
            preserve_case: false,
            sample: sample.downgrade(),
            current: 0,
            found: None,
            running: None,
            debounce: None,
            select: true,
            _subscriptions,
        }
    }

    /// The matches of the last search, if it is for the pattern and sample as they are.
    fn matches(&self, cx: &App) -> Option<&Result<Vec<Range<usize>>, SearchError>> {
        let f = self.found.as_ref()?;
        let sample = self.sample.upgrade()?;
        let fresh = f.version == sample.read(cx).doc(cx).version
            && f.query == self.pattern.read(cx).text(cx)
            && f.opts == self.opts;
        fresh.then_some(&f.result)
    }
}

impl Workspace {
    pub fn playground_panel(&self) -> Option<&PlaygroundPanel> {
        match &self.panel {
            Some(Panel::Playground(p)) => Some(p),
            _ => None,
        }
    }

    pub fn playground_panel_mut(&mut self) -> Option<&mut PlaygroundPanel> {
        match &mut self.panel {
            Some(Panel::Playground(p)) => Some(p),
            _ => None,
        }
    }

    /// Find > Regex Playground: a sample tab of its own (the one already open, or a new one),
    /// the panel under it.
    pub(crate) fn open_playground(&mut self, _: &OpenPlayground, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.views.tabs().find(|t| t.read(cx).playground).cloned();
        let sample = match existing {
            Some(e) => {
                if let Some((g, i)) = self.views.find(&e) {
                    self.views.select(g, i);
                }
                e
            }
            None => self.playground_tab("", window, cx),
        };
        self.show_playground(&sample, window, cx);
    }

    /// Find > Regex Playground with a Copy of the Document: the whole active document copied
    /// into a new sample; the document is not touched.
    pub(crate) fn open_playground_with_document(
        &mut self,
        _: &OpenPlaygroundWithDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(e) = self.active_editor() else {
            return self.open_playground(&OpenPlayground, window, cx);
        };
        let text = e.read(cx).doc(cx).text();
        let sample = self.playground_tab(&text, window, cx);
        self.show_playground(&sample, window, cx);
    }

    fn playground_tab(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<Editor> {
        let source = self.active_editor().and_then(|view| {
            let editor = view.read(cx);
            match editor.playground {
                true => editor.playground_source.clone(),
                false => Some(view.downgrade()),
            }
        });
        let mut state = EditorState::from_source(text);
        state.doc.scratch = true;
        let view = self.new_editor(state, cx);
        view.update(cx, |e, _| {
            e.label = Some("Regex Playground".into());
            e.playground = true;
            e.playground_source = source;
        });
        self.attach(view.clone(), false, window, cx);
        view
    }

    fn show_playground(&mut self, sample: &Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_panel(cx);
        let reuse = self.playground_panel().is_some_and(|p| p.sample == sample.downgrade());
        if reuse {
            if let Some(p) = self.playground_panel() {
                p.pattern.read(cx).focus(window);
            }
        } else {
            let mut p = PlaygroundPanel::new(sample, window, cx);
            p.opts = FindOpts { regex: true, ..self.last_find_opts };
            let q = self.last_query.clone();
            p.pattern.update(cx, |e, cx| e.set_text(&q, cx));
            self.show_panel(Panel::Playground(p), cx);
        }
        self.playground_changed(cx);
    }

    /// The pattern, replacement, toggles or sample changed: search again after a pause, the
    /// search under way (for what was) cancelled.
    pub(crate) fn playground_changed(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.playground_panel_mut() else {
            return;
        };
        if let Some(c) = p.running.take() {
            c.store(true, Ordering::Relaxed);
        }
        p.select = true;
        p.debounce = Some(cx.spawn(async move |ws, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            ws.update(cx, |ws, cx| ws.playground_search(cx)).ok();
        }));
        cx.notify();
    }

    fn playground_sample_changed(&mut self, cx: &mut Context<Self>) {
        self.playground_changed(cx);
        if let Some(p) = self.playground_panel_mut() {
            p.select = false;
        }
    }

    fn playground_search(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.playground_panel() else {
            return;
        };
        let Some(sample) = p.sample.upgrade() else {
            return;
        };
        let (query, opts) = (p.pattern.read(cx).text(cx), p.opts);
        let s = sample.read(cx).read_state(cx);
        let (version, text) = (s.doc.version, Arc::new(s.text()));
        let sample = sample.downgrade();
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(p) = self.playground_panel_mut() {
            p.running = Some(cancel.clone());
        }
        let work = cx.background_spawn({
            let (text, query) = (text.clone(), query.clone());
            async move {
                if query.is_empty() {
                    return Ok(Vec::new());
                }
                let m = Matcher::new(&query, &opts).map_err(|_| SearchError::Cancelled)?;
                let d = Deadline::new(crate::searching::pattern::BACKGROUND_SECONDS);
                let found = m.find(&text, &|| cancel.load(Ordering::Relaxed) || d.passed());
                found.map(|f| f.into_iter().map(|(b, _)| b).collect()).map_err(|e| d.explain(e))
            }
        });
        cx.spawn(async move |ws, cx| {
            let result = work.await;
            ws.update(cx, |ws, cx| {
                let Some(p) = ws.playground_panel_mut() else {
                    return;
                };
                if result == Err(SearchError::Cancelled) && !query.is_empty() {
                    // an invalid pattern says so through `error`; a cancelled search is dropped
                    if Matcher::new(&query, &opts).is_ok() {
                        return;
                    }
                }
                // a result for a pattern or sample that has changed since is dropped
                let still = p.sample == sample
                    && p.pattern.read(cx).text(cx) == query
                    && p.opts == opts
                    && p.sample.upgrade().is_some_and(|s| s.read(cx).doc(cx).version == version);
                if !still {
                    return;
                }
                p.running = None;
                p.found = Some(Found { version, query, opts, text, result });
                let select = p.select;
                ws.playground_outlines(select, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The sample's matches outlined (chars), the current one selected if `select`.
    fn playground_outlines(&mut self, select: bool, cx: &mut Context<Self>) {
        let Some(p) = self.playground_panel_mut() else {
            return;
        };
        let Some(sample) = p.sample.upgrade() else {
            return;
        };
        let (Some(f), found) = (p.found.as_ref(), p.matches(cx).cloned()) else {
            return;
        };
        let chars = match &found {
            Some(Ok(bytes)) => crate::editing::text::byte_to_char_ranges(&f.text, bytes),
            _ => Vec::new(),
        };
        p.current = p.current.min(chars.len().saturating_sub(1));
        let current = chars.get(p.current).cloned();
        sample.update(cx, |e, cx| {
            e.find_ranges = chars;
            if let Some(r) = current.filter(|_| select) {
                e.select_range(r, cx);
            }
            cx.notify();
        });
        cx.notify();
    }

    fn playground_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(p) = self.playground_panel_mut() else {
            return;
        };
        let n = match p.matches(cx) {
            Some(Ok(m)) => m.len(),
            _ => 0,
        };
        if n == 0 {
            return;
        }
        p.current = match forward {
            true => (p.current + 1) % n,
            false => (p.current + n - 1) % n,
        };
        self.playground_outlines(true, cx);
    }

    forward_actions! {
    pub(crate) playground_next(PlaygroundNext, _, cx) => playground_step(true, cx);
    pub(crate) playground_prev(PlaygroundPrev, _, cx) => playground_step(false, cx);
    pub(crate) playground_to_find(PlaygroundToFind, window, cx) => transfer(Transfer::Find, window, cx);
    pub(crate) playground_to_replace(PlaygroundToReplace, window, cx) => transfer(Transfer::Replace, window, cx);
    pub(crate) playground_to_find_in_files(PlaygroundToFindInFiles, window, cx) => transfer(Transfer::FindInFiles, window, cx);
    }

    pub(crate) fn playground_focus_next(
        &mut self,
        _: &PlaygroundFocusNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(p) = self.playground_panel() {
            let target = match p.pattern.read(cx).is_focused(window) {
                true => &p.replace,
                false => &p.pattern,
            };
            target.read(cx).focus(window);
        }
    }

    /// The playground's search, as a saved search keeps it (Save Search… from the playground).
    pub(crate) fn playground_search_values(&self, cx: &App) -> Option<(String, String, FindOpts, bool)> {
        let p = self.playground_panel()?;
        Some((p.pattern.read(cx).text(cx), p.replace.read(cx).text(cx), p.opts, p.preserve_case))
    }

    /// A search into the playground (a saved one).
    pub(crate) fn load_playground(
        &mut self,
        query: &str,
        rep: &str,
        opts: FindOpts,
        preserve: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.playground_panel_mut() else {
            return;
        };
        (p.opts, p.preserve_case) = (opts, preserve);
        let fields = (p.pattern.clone(), p.replace.clone());
        fill_fields(Some(fields), query, rep, cx);
        self.playground_changed(cx);
    }

    fn transfer(&mut self, to: Transfer, window: &mut Window, cx: &mut Context<Self>) {
        let Some((query, rep, opts, preserve)) = self.playground_search_values(cx) else {
            return;
        };
        if query.is_empty() {
            return self.status_message("The playground has no pattern", cx);
        }
        // the search acts on the document behind the sample, if the sample is active
        if self.active_editor().is_some_and(|e| e.read(cx).playground) {
            let back = self
                .playground_panel()
                .and_then(|p| p.sample.upgrade())
                .and_then(|sample| sample.read(cx).playground_source.as_ref().and_then(|source| source.upgrade()))
                .and_then(|source| self.views.find(&source));
            match back {
                Some((g, i)) => self.views.select(g, i),
                None if matches!(to, Transfer::Find | Transfer::Replace) => {
                    return self.status_message(
                        "The source document is no longer open here; select a document before using Find or Replace",
                        cx,
                    );
                }
                None => {}
            }
        }
        // what goes to Find is remembered as a search would be
        self.query_history.remember(&query);
        match to {
            Transfer::Find | Transfer::Replace => {
                match to {
                    Transfer::Find => self.find(&Find, window, cx),
                    _ => self.replace(&Replace, window, cx),
                }
                let fields = self.find_panel_mut().map(|p| {
                    (p.opts, p.preserve_case) = (opts, preserve);
                    (p.query.clone(), p.replace.clone())
                });
                fill_fields(fields, &query, &rep, cx);
                self.refresh_matches(cx);
            }
            Transfer::FindInFiles => {
                self.find_in_files(&FindInFiles, window, cx);
                let fields = self.fif_panel_mut().map(|p| {
                    (p.opts, p.flags.preserve_case) = (opts, preserve);
                    (p.query.clone(), p.replace.clone())
                });
                fill_fields(fields, &query, &rep, cx);
                self.validate_fif(cx);
            }
        }
        cx.notify();
    }

    /// The whole sample with every match replaced, in a scratch tab; the sample stays as it is.
    pub(crate) fn playground_preview(&mut self, _: &PlaygroundPreview, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.playground_panel() else {
            return;
        };
        let Some((Ok(found), f)) = p.matches(cx).cloned().zip(p.found.as_ref()) else {
            return self.status_message("Nothing found to replace yet", cx);
        };
        let r = match Replacer::new(&f.query, &p.replace.read(cx).text(cx), &p.opts, p.preserve_case) {
            Ok(r) => r,
            Err(e) => return self.status_message(e, cx),
        };
        let text = &f.text;
        let mut out = String::new();
        let mut last = 0;
        for m in found {
            out.push_str(&text[last..m.start]);
            match r.replacement(text, m.clone()) {
                Some(t) => out.push_str(&t),
                None => out.push_str(&text[m.clone()]),
            }
            last = m.end;
        }
        out.push_str(&text[last..]);
        self.scratch_tab(&out, "Playground Preview".into(), window, cx);
    }

    pub(crate) fn render_playground(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(p) = self.playground_panel() else {
            return div().into_any_element();
        };
        let query = p.pattern.read(cx).text(cx);
        let rep = p.replace.read(cx).text(cx);
        let invalid = (!query.is_empty()).then(|| Matcher::new(&query, &p.opts).err()).flatten();
        let replacer = Replacer::new(&query, &rep, &p.opts, p.preserve_case);
        let (status, inspector, preview) = match (&invalid, p.matches(cx), p.found.as_ref()) {
            (Some(e), ..) => (e.clone(), Vec::new(), None),
            (None, _, _) if query.is_empty() => ("Type a pattern".into(), Vec::new(), None),
            (None, None, _) => ("Searching…".into(), Vec::new(), None),
            (None, Some(Err(e)), _) => (e.message(), Vec::new(), None),
            (None, Some(Ok(m)), Some(f)) if !m.is_empty() => {
                let cur = p.current.min(m.len() - 1);
                let text = &f.text;
                let at = m[cur].clone();
                let rows = inspect(&f.query, &f.opts, text, at.clone());
                let rep = match &replacer {
                    Ok(r) => r
                        .replacement(text, at)
                        .map(|t| format!("Replacement: “{}”", crate::searching::results::escape(&t))),
                    Err(e) => Some(format!("Replacement: {e}")),
                };
                (format!("Match {} of {}", cur + 1, m.len()), rows, rep)
            }
            (None, Some(Ok(_)), _) => ("No matches".into(), Vec::new(), None),
        };
        let toggle = |id: &'static str, i: Icon, tip: &'static str, on: bool, flip: fn(&mut PlaygroundPanel)| {
            kit::icon_toggle(id, i, tip, on)
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(move |ws, _: &ClickEvent, _, cx| {
                    if let Some(p) = ws.playground_panel_mut() {
                        flip(p);
                    }
                    ws.playground_changed(cx);
                }))
        };
        let field = |e, options| search_field(e, None, options, cx);
        let label = |t: &'static str| {
            div().w(px(60.)).flex().justify_end().pt(px(5.)).text_color(ui(theme::TEXT_MUTED)).child(t)
        };
        let row = || div().flex().flex_row().items_start().gap(px(6.));
        let o = p.opts;
        div()
            .key_context("PlaygroundPanel")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(ui(theme::PANEL_BG))
            .border_t_1()
            .border_color(ui(theme::BORDER))
            .text_color(ui(theme::UI_FG))
            .child(
                kit::panel_header()
                    .child(icon(Icon::Regex, 14.).text_color(ui(theme::ICON_MUTED)))
                    .child("Regex Playground")
                    .child(div().flex_1())
                    .child(dispatching(kit::ghost_button("pg-to-find", "Use in Find"), PlaygroundToFind))
                    .child(dispatching(kit::ghost_button("pg-to-replace", "Use in Replace"), PlaygroundToReplace))
                    .child(dispatching(kit::ghost_button("pg-to-fif", "Use in Find in Files"), PlaygroundToFindInFiles))
                    .child(dispatching(kit::ghost_button("pg-save", "Save…"), SaveSearch))
                    .child(dispatching(kit::ghost_button("pg-load", "Load…"), OpenSavedSearch))
                    .child(
                        kit::icon_button("close-playground", Icon::Close, "Close  ⎋")
                            .ml(px(4.))
                            .on_click(|_: &ClickEvent, window, cx| window.dispatch_action(Box::new(ClosePanel), cx)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .pl(px(4.))
                    .pr(px(10.))
                    .py(px(8.))
                    .child(
                        row()
                            .child(label("Pattern"))
                            .child(field(
                                &p.pattern,
                                vec![
                                    toggle("pg-case", Icon::CaseSensitive, "Case Sensitive", o.case_sensitive, |p| {
                                        p.opts.case_sensitive ^= true
                                    }),
                                    toggle("pg-word", Icon::WholeWord, "Whole Word", o.whole_word, |p| {
                                        p.opts.whole_word ^= true
                                    }),
                                    toggle("pg-regex", Icon::Regex, "Regular Expression", o.regex, |p| {
                                        p.opts.regex ^= true
                                    }),
                                ],
                            ))
                            .child(
                                div()
                                    .w(px(96.))
                                    .h(px(26.))
                                    .flex()
                                    .items_center()
                                    .gap(px(2.))
                                    .child(dispatching(
                                        kit::icon_button("pg-prev", Icon::ChevronLeft, "Previous Match"),
                                        PlaygroundPrev,
                                    ))
                                    .child(dispatching(
                                        kit::icon_button("pg-next", Icon::ChevronRight, "Next Match"),
                                        PlaygroundNext,
                                    )),
                            ),
                    )
                    .child(
                        row()
                            .child(label("Replace"))
                            .child(field(
                                &p.replace,
                                vec![toggle(
                                    "pg-preserve",
                                    Icon::PreserveCase,
                                    "Preserve Case",
                                    p.preserve_case,
                                    |p| p.preserve_case ^= true,
                                )],
                            ))
                            .child(
                                div()
                                    .w(px(96.))
                                    .child(dispatching(kit::button("pg-preview", "Preview All"), PlaygroundPreview)),
                            ),
                    )
                    .child(
                        div()
                            .pl(px(66.))
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(10.))
                            .items_center()
                            .child(div().id("pg-status").font_weight(gpui::FontWeight::SEMIBOLD).child(status))
                            .when_some(preview, |d, t| {
                                d.child(div().id("pg-replacement").text_color(ui(theme::TEXT_MUTED)).child(t))
                            }),
                    )
                    .child(
                        div()
                            .id("pg-captures")
                            .ml(px(66.))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .font_family(theme::EDITOR_FONT)
                            .text_size(px(theme::EDITOR_FONT_SIZE))
                            .text_color(ui(theme::UI_FG))
                            .children(inspector.into_iter().map(|r| div().child(r))),
                    ),
            )
            .into_any_element()
    }
}

/// `query` and `rep` into a panel's pattern and replacement fields, if it is open.
fn fill_fields(fields: Option<(Entity<Editor>, Entity<Editor>)>, query: &str, rep: &str, cx: &mut App) {
    if let Some((q, r)) = fields {
        q.update(cx, |e, cx| e.set_text(query, cx));
        r.update(cx, |e, cx| e.set_text(rep, cx));
    }
}

#[derive(Clone, Copy)]
enum Transfer {
    Find,
    Replace,
    FindInFiles,
}

/// The capture inspector's rows for the match at `m` (bytes of `text`): each group by number
/// and name, where it is (line:column, 1-based) and its text; a group that took no part in the
/// match is "unmatched", one that matched nothing "empty".
pub fn inspect(query: &str, o: &FindOpts, text: &str, m: Range<usize>) -> Vec<String> {
    let loc = |b: usize| {
        let before = &text[..b];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        format!("{line}:{col}")
    };
    let show = |r: &Range<usize>| match r.is_empty() {
        true => format!("{}  empty", loc(r.start)),
        false => format!("{}–{}  “{}”", loc(r.start), loc(r.end), crate::searching::results::escape(&text[r.clone()])),
    };
    if !o.regex {
        return vec![format!("0  {}", show(&m))];
    }
    let Ok(p) = crate::searching::pattern::Pattern::new(query, o) else {
        return Vec::new();
    };
    let Some(groups) = p.captures(text, m) else {
        return Vec::new();
    };
    (groups.iter().enumerate())
        .map(|(i, g)| {
            let name = p.names().get(i).cloned().flatten().map_or(String::new(), |n| format!(" <{n}>"));
            let what = match g {
                Some(r) => show(r),
                None => "unmatched".into(),
            };
            format!("{i}{name}  {what}")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inspector_tells_unmatched_groups_from_empty_ones() {
        let o = FindOpts { regex: true, case_sensitive: true, ..FindOpts::default() };
        let text = "x\nab";
        let rows = inspect(r"(?<first>a)(z)?(c*)b", &o, text, 2..4);
        assert_eq!(rows, ["0  2:1–2:3  “ab”", "1 <first>  2:1–2:2  “a”", "2  unmatched", "3  2:2  empty",]);
    }
}

#[cfg(test)]
#[gpui::test]
fn playground_rejects_a_result_for_another_sample(cx: &mut gpui::TestAppContext) {
    let (ws, vcx) = crate::workspace::ui_tests::window(cx);
    crate::workspace::ui_tests::type_in(&ws, vcx, "foo");
    ws.update_in(vcx, |ws, window, cx| {
        ws.last_query = "foo".into();
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        // the first sample's search starts, and a second sample replaces the panel before it
        // lands
        ws.playground_search(cx);
        ws.add_tab(EditorState::from_text("bar"), window, cx);
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
    });
    // the debounce clock stays put: only the first sample's search can have finished
    vcx.run_until_parked();
    ws.read_with(vcx, |ws, cx| {
        let sample = ws.active_editor().unwrap();
        let sample = sample.read(cx);
        assert_eq!(sample.doc(cx).text(), "bar");
        assert!(
            sample.find_ranges.is_empty(),
            "the old sample's foo match must not outline bar: {:?}",
            sample.find_ranges
        );
    });
}

#[cfg(test)]
fn settle_playground(cx: &mut gpui::VisualTestContext) {
    cx.executor().advance_clock(DEBOUNCE * 2);
    cx.run_until_parked();
}

#[cfg(test)]
#[gpui::test]
fn typing_in_the_playground_sample_leaves_the_caret(cx: &mut gpui::TestAppContext) {
    let (ws, vcx) = crate::workspace::ui_tests::window(cx);
    crate::workspace::ui_tests::type_in(&ws, vcx, "foo bar foo");
    ws.update_in(vcx, |ws, window, cx| {
        ws.last_query = "foo".into();
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
    });
    settle_playground(vcx);
    let sample = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    sample.update(vcx, |e, cx| {
        assert_eq!(e.state.selections[0].range(), 0..3, "shown on the first match");
        e.edit(cx, |s| {
            s.set_cursor(5);
            s.insert("x");
        })
    });
    settle_playground(vcx);
    sample.read_with(vcx, |e, cx| {
        assert_eq!(e.find_ranges, vec![0..3, 9..12], "the search follows the sample");
        assert_eq!(e.state.selections[0].range(), 6..6, "the caret stays where typed");
        assert_eq!(e.doc(cx).text(), "foo bxar foo");
    });
    // stepping still selects
    ws.update_in(vcx, |ws, window, cx| {
        ws.playground_next(&PlaygroundNext, window, cx);
    });
    sample.read_with(vcx, |e, _| assert_eq!(e.state.selections[0].range(), 9..12));
}

#[cfg(test)]
#[gpui::test]
fn the_playground_replacement_does_not_search_again(cx: &mut gpui::TestAppContext) {
    let (ws, vcx) = crate::workspace::ui_tests::window(cx);
    crate::workspace::ui_tests::type_in(&ws, vcx, "foo");
    ws.update_in(vcx, |ws, window, cx| {
        ws.last_query = "foo".into();
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
    });
    settle_playground(vcx);
    ws.update(vcx, |ws, cx| {
        let p = ws.playground_panel_mut().unwrap();
        p.debounce = None;
        let r = p.replace.clone();
        r.update(cx, |e, cx| e.set_text("bar", cx));
    });
    ws.read_with(vcx, |ws, _| {
        assert!(ws.playground_panel().unwrap().debounce.is_none());
    });
}

#[cfg(test)]
#[gpui::test]
fn a_replaced_playground_sample_no_longer_wakes_the_panel(cx: &mut gpui::TestAppContext) {
    let (ws, vcx) = crate::workspace::ui_tests::window(cx);
    crate::workspace::ui_tests::type_in(&ws, vcx, "foo");
    let first = ws.update_in(vcx, |ws, window, cx| {
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        let first = ws.active_editor().unwrap();
        ws.add_tab(EditorState::from_text("bar"), window, cx);
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        first
    });
    settle_playground(vcx);
    ws.update(vcx, |ws, _| ws.playground_panel_mut().unwrap().debounce = None);
    first.update(vcx, |e, cx| e.edit(cx, |s| s.insert("x")));
    ws.read_with(vcx, |ws, _| {
        assert!(ws.playground_panel().unwrap().debounce.is_none());
    });
}
