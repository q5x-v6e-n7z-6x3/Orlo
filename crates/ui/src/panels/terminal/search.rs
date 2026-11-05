//! Search uses an independent snapshot so a busy PTY never invalidates every search response.

use super::*;
use crate::chrome::ui as kit;
use crate::editing::text::EditorState;
use crate::editor::Editor;
use gpui::{actions, ClickEvent, Subscription};
use orlo_terminal::search::{SearchOptions, SearchResults};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

actions!(
    terminal,
    [FindOutput, NextMatch, PreviousMatch, CloseSearch, ToggleSearchCase, ToggleSearchRegex, ResumeSearch]
);

fn compact_control(id: &'static str, label: &'static str, tip: &'static str) -> gpui::Stateful<gpui::Div> {
    kit::ghost_button(id, label)
        .w(px(22.))
        .px_0()
        .justify_center()
        .debug_selector(move || id.into())
        .tooltip(kit::tooltip(tip.into()))
}

pub(super) struct Search {
    query: Entity<Editor>,
    pub results: Option<SearchResults>,
    cancel: Arc<AtomicBool>,
    _changed: Subscription,
    _task: Option<Task<()>>,
    pub dirty: bool,
    pending: bool,
    pinned: bool,
    outdated: bool,
    options: SearchOptions,
    error: Option<String>,
    started: Instant,
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Terminal {
    pub(super) fn retention_changed(&mut self, cx: &mut Context<Self>) {
        self.retained_output = None;
        self.command_review = None;
        self.dragging = false;
        if let Some(search) = &mut self.search {
            search.cancel.store(true, Ordering::Relaxed);
            search._task = None;
            search.results = None;
            search.pending = false;
            search.pinned = false;
            search.outdated = false;
        }
        self.refresh_search(true, cx);
        cx.notify();
    }

    pub fn open_search(&mut self, _: &FindOutput, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_review.take().is_some() || self.retained_output.is_some() {
            // A new Find starts from live output, never a review's cell coordinates.
            self.backend.set_selection(None);
        }
        self.dragging = false;
        self.reporting_button = None;
        self.last_mouse = None;
        if self.search.is_none() {
            let query = cx.new(|cx| Editor::new(EditorState::new(), true, cx));
            let changed = Editor::on_text_change(&query, cx, |terminal, cx| terminal.refresh_search(true, cx));
            self.retained_output = None;
            let mut results = SearchResults::find(self.backend.screen().clone(), "", &AtomicBool::new(false)).unwrap();
            results.set_selection(self.backend.selection());
            self.search = Some(Search {
                query,
                pinned: results.selection().is_some(),
                outdated: false,
                results: Some(results),
                cancel: Arc::new(AtomicBool::new(false)),
                _changed: changed,
                _task: None,
                dirty: false,
                pending: false,
                options: SearchOptions::default(),
                error: None,
                started: Instant::now(),
            });
        }
        let query = &self.search.as_ref().unwrap().query;
        query.update(cx, |e, cx| e.sel(cx).select_all());
        query.read(cx).focus(window);
        cx.notify();
    }

    pub fn close_search(&mut self, _: &CloseSearch, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut search) = self.search.take() {
            self.retained_output = search.results.take();
        }
        self.focus(window);
        cx.notify();
    }

    pub(super) fn searching_input(&self, window: &Window, cx: &App) -> bool {
        self.search.as_ref().is_some_and(|s| s.query.read(cx).is_focused(window))
    }

    pub(super) fn toggle_search_case(&mut self, _: &ToggleSearchCase, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &mut self.search {
            search.options.case_sensitive = !search.options.case_sensitive;
        }
        self.refresh_search(true, cx);
    }
    pub(super) fn toggle_search_regex(&mut self, _: &ToggleSearchRegex, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &mut self.search {
            search.options.regex = !search.options.regex;
        }
        self.refresh_search(true, cx);
    }

    pub(super) fn resume_search(&mut self, _: &ResumeSearch, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = &mut self.search else { return };
        search.pinned = false;
        search.outdated = false;
        search.results = SearchResults::find(self.backend.screen().clone(), "", &AtomicBool::new(false));
        self.backend.set_selection(None);
        search.query.read(cx).focus(window);
        self.refresh_search(true, cx);
    }

    pub(super) fn pin_search(&mut self) {
        if let Some(search) = &mut self.search {
            search.pinned |= search.results.is_some();
        }
    }

    pub(super) fn refresh_search(&mut self, reset: bool, cx: &mut Context<Self>) {
        let Some(search) = &mut self.search else { return };
        if !reset && (search.pending || search.pinned && search.outdated) {
            search.dirty = true;
            return;
        }
        if search.results.as_ref().is_some_and(|r| !r.screen().same_layout(self.backend.screen())) {
            search.results = None;
            search.pinned = false;
            search.outdated = false;
        }
        if search.results.is_none() {
            search.results = SearchResults::find(self.backend.screen().clone(), "", &AtomicBool::new(false));
        }
        search.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        search.cancel = cancel.clone();
        search.started = Instant::now();
        search.dirty = false;
        search.pending = true;
        search.error = None;
        if reset {
            if let Some(results) = &mut search.results {
                results.clear_matches();
            }
            self.backend.set_selection(None);
        }
        let query = search.query.read(cx).text(cx);
        let options = search.options;
        #[cfg(test)]
        let gate_path = self.cwd.clone();
        search._task = Some(cx.spawn(async move |terminal, cx| {
            cx.background_executor().timer(Duration::from_millis(60)).await;
            let Ok(Some(snapshot)) = terminal.update(cx, |terminal, _| {
                (!cancel.load(Ordering::Relaxed)).then(|| {
                    if reset {
                        terminal
                            .search
                            .as_ref()
                            .and_then(|s| s.results.as_ref())
                            .map(|r| r.screen().clone())
                            .unwrap_or_else(|| terminal.backend.screen().clone())
                    } else {
                        terminal.backend.screen().clone()
                    }
                })
            }) else {
                return;
            };
            let source = snapshot.identity();
            #[cfg(test)]
            crate::testing::gate::wait("terminal-search-scan", &gate_path).await;
            let stopped = cancel.clone();
            let result = cx
                .background_executor()
                .spawn(async move { SearchResults::find_with_options(snapshot, &query, options, &stopped) })
                .await;
            terminal
                .update(cx, |terminal, cx| {
                    let screen = terminal.backend.screen();
                    let valid = source.matches(screen);
                    let Some(search) = &mut terminal.search else { return };
                    if !Arc::ptr_eq(&search.cancel, &cancel) || cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    search.pending = false;
                    search.started = Instant::now();
                    if !valid {
                        search.results = None;
                        search.pinned = false;
                        search.outdated = false;
                        search.dirty = true;
                    } else {
                        match result {
                            Ok(Some(mut result)) => {
                                let reviewed = search.pinned;
                                let same_view = search
                                    .results
                                    .as_ref()
                                    .is_some_and(|previous| reviewed && result.preserve_view_from(previous));
                                if !reviewed || same_view {
                                    search.results = Some(result);
                                }
                                search.outdated =
                                    search.results.as_ref().is_some_and(|r| !r.screen().same_contents(screen));
                                if !reviewed && search.outdated {
                                    search.dirty = true;
                                }
                                terminal.backend.set_selection(None);
                            }
                            Ok(None) => {}
                            Err(error) => {
                                search.outdated =
                                    search.results.as_ref().is_some_and(|r| !r.screen().same_contents(screen));
                                search.error = Some(error);
                            }
                        }
                    }
                    cx.notify();
                })
                .ok();
            // The PTY poll stops at exit; dirty final output still needs one newer snapshot.
            cx.background_executor().timer(Duration::from_millis(250)).await;
            terminal
                .update(cx, |terminal, cx| {
                    if terminal.search.as_ref().is_some_and(|s| {
                        Arc::ptr_eq(&s.cancel, &cancel)
                            && !cancel.load(Ordering::Relaxed)
                            && s.dirty
                            && !s.pending
                            && !(s.pinned && s.outdated)
                    }) {
                        terminal.refresh_search(false, cx);
                    }
                })
                .ok();
        }));
        cx.notify();
    }

    pub(super) fn tick_search(&mut self, changed: bool, cx: &mut Context<Self>) {
        let live = self.backend.screen();
        if self.retained_output.as_ref().is_some_and(|r| !r.screen().same_layout(live)) {
            self.retained_output = None;
        }
        let incompatible =
            self.search.as_ref().and_then(|s| s.results.as_ref()).is_some_and(|r| !r.screen().same_layout(live));
        if incompatible {
            self.refresh_search(true, cx);
        }
        if let Some(search) = &mut self.search {
            search.dirty |= changed;
            if search.dirty
                && !search.pending
                && !(search.pinned && search.outdated)
                && search.started.elapsed() >= Duration::from_millis(250)
            {
                self.refresh_search(false, cx);
            }
        }
    }

    pub(super) fn next_match(&mut self, _: &NextMatch, _: &mut Window, cx: &mut Context<Self>) {
        self.step_match(true, cx);
    }
    pub(super) fn previous_match(&mut self, _: &PreviousMatch, _: &mut Window, cx: &mut Context<Self>) {
        self.step_match(false, cx);
    }
    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(results) = self.search.as_mut().and_then(|s| s.results.as_mut()) {
            results.step(forward);
            if !results.is_empty() {
                self.search.as_mut().unwrap().pinned = true;
            }
            self.backend.set_selection(None);
            cx.notify();
        }
    }

    pub(super) fn display_screen(&self) -> &orlo_terminal::Screen {
        if let Some(review) = &self.command_review {
            return &review.screen;
        }
        self.output_snapshot().map_or(self.backend.screen(), |r| r.screen())
    }

    pub(super) fn output_snapshot(&self) -> Option<&SearchResults> {
        self.search.as_ref().and_then(|s| s.results.as_ref()).or(self.retained_output.as_ref())
    }
    pub(super) fn output_snapshot_mut(&mut self) -> Option<&mut SearchResults> {
        self.search.as_mut().and_then(|s| s.results.as_mut()).or(self.retained_output.as_mut())
    }
    pub(super) fn display_selection(&self) -> Option<orlo_terminal::Selection> {
        self.output_snapshot().map_or_else(|| self.backend.selection(), |r| r.selection())
    }
    pub(super) fn set_display_selection(&mut self, selection: Option<orlo_terminal::Selection>) {
        if selection.is_some() {
            self.pin_search();
        }
        if let Some(snapshot) = self.output_snapshot_mut() {
            snapshot.set_selection(selection);
        } else {
            self.backend.set_selection(selection);
        }
    }
    pub(super) fn extend_display_selection(&mut self, position: orlo_terminal::CellPosition) -> bool {
        if let Some(snapshot) = self.output_snapshot_mut() {
            snapshot.extend_selection(position)
        } else {
            self.backend.extend_selection(position)
        }
    }

    pub(super) fn render_search(&self, message: Option<String>, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let search = self.search.as_ref()?;
        let count = search.error.clone().or(message).unwrap_or_else(|| {
            search
                .results
                .as_ref()
                .map_or_else(|| "Searching…".into(), |r| format!("{}/{}", r.current().map_or(0, |i| i + 1), r.len()))
        });
        let count = if search.pinned { format!("Paused · {count}") } else { count };
        let tip = format!("{count} · Update from live output: click here or ⌘⌥U");
        Some(
            kit::panel_header()
                .gap(px(2.))
                .pl(px(2.))
                .pr(px(2.))
                .key_context("TerminalSearch")
                .font_family(theme::UI_FONT)
                .child(
                    kit::field()
                        .debug_selector(|| "term-search-query".into())
                        .flex_1()
                        .min_w(px(56.))
                        .h(px(24.))
                        .px_1()
                        .py_1()
                        .child(search.query.clone()),
                )
                .child(
                    compact_control("term-search-case", "Aa", "Match case  ⌘⌥C")
                        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                        .when(search.options.case_sensitive, |d| {
                            d.bg(crate::chrome::theme::ui(theme::BUTTON_SELECTED_BG))
                        })
                        .on_click(
                            cx.listener(|t, _: &ClickEvent, w, cx| t.toggle_search_case(&ToggleSearchCase, w, cx)),
                        ),
                )
                .child(
                    compact_control("term-search-regex", ".*", "Regular expression  ⌘⌥R")
                        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                        .when(search.options.regex, |d| d.bg(crate::chrome::theme::ui(theme::BUTTON_SELECTED_BG)))
                        .on_click(
                            cx.listener(|t, _: &ClickEvent, w, cx| t.toggle_search_regex(&ToggleSearchRegex, w, cx)),
                        ),
                )
                .child(
                    div()
                        .id("term-search-status")
                        .debug_selector(|| "term-search-status".into())
                        .min_w(px(24.))
                        .max_w(px(160.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                        .on_click(cx.listener(|t, _: &ClickEvent, w, cx| t.resume_search(&ResumeSearch, w, cx)))
                        .tooltip(kit::tooltip(tip.into()))
                        .child(count),
                )
                .child(
                    compact_control("term-search-prev", "↑", "Previous match  ⇧Enter")
                        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                        .on_click(cx.listener(|t, _: &ClickEvent, _, cx| t.step_match(false, cx))),
                )
                .child(
                    compact_control("term-search-next", "↓", "Next match  Enter")
                        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                        .on_click(cx.listener(|t, _: &ClickEvent, _, cx| t.step_match(true, cx))),
                )
                .child(
                    compact_control("term-search-close", "×", "Close Find  Esc")
                        .on_click(cx.listener(|t, _: &ClickEvent, w, cx| t.close_search(&CloseSearch, w, cx))),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::settle_until;

    fn command(script: &str) -> orlo_terminal::CommandBuilder {
        let mut command = orlo_terminal::CommandBuilder::new("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[gpui::test]
    fn narrow_find_keeps_query_controls_and_status_inside_the_output_width(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (terminal, cx) = cx.add_window_view(|window, cx| {
            let mut t = Terminal::new(std::env::temp_dir(), cx);
            t._poll = None;
            t.backend = TerminalSession::from_command(
                command("stty -echo; printf 'READY\\n'; IFS= read -r v; exit 7"),
                Default::default(),
            );
            t.focus(window);
            t
        });
        // A 220px host leaves 206px after the terminal panel's existing output insets.
        cx.simulate_resize(gpui::size(px(206.), px(320.)));
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        cx.simulate_keystrokes("cmd-f");
        let check = |cx: &mut gpui::VisualTestContext, phase: &str| {
            cx.run_until_parked();
            let output = terminal.read_with(cx, |t, _| t.layout.unwrap().bounds);
            let case = cx.debug_bounds("term-search-case").expect("Case must be visible");
            assert!(case.left() - output.left() >= px(60.), "{phase}: query space collapsed: {case:?}");
            let query = cx.debug_bounds("term-search-query").expect("Find field must be visible");
            assert!(query.size.width >= px(56.), "{phase}: query field is not usable: {query:?}");
            for id in [
                "term-search-case",
                "term-search-regex",
                "term-search-prev",
                "term-search-next",
                "term-search-close",
                "term-search-status",
            ] {
                let control = cx.debug_bounds(id).unwrap_or_else(|| panic!("{phase}: missing {id}"));
                assert!(
                    control.left() >= output.left() && control.right() <= output.right(),
                    "{phase}: {id} outside {output:?}: {control:?}"
                );
                assert!(control.size.height <= px(28.), "{phase}: status/control changed the fixed row");
            }
        };
        check(cx, "live");
        for id in ["term-search-case", "term-search-regex", "term-search-prev", "term-search-next"] {
            let bounds = cx.debug_bounds(id).unwrap();
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            terminal.update_in(cx, |t, window, cx| assert!(t.searching_input(window, cx), "{id} moved Find focus"));
        }
        terminal.update(cx, |t, _| {
            t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"go\n".to_vec())).unwrap()
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.exited() == Some(7)
            })
        });
        check(cx, "shell exit status");
        cx.simulate_input("[");
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().error.is_some())
        });
        check(cx, "invalid regex");
        terminal.update(cx, |t, _| t.set_display_selection(Some(((0, 0), (0, 1)))));
        check(cx, "Paused affordance");
        let update = cx.debug_bounds("term-search-status").unwrap();
        cx.simulate_click(update.center(), gpui::Modifiers::none());
        terminal.update_in(cx, |t, w, cx| {
            assert!(!t.search.as_ref().unwrap().pinned);
            assert!(t.display_selection().is_none());
            assert!(t.searching_input(w, cx));
            assert!(t.display_screen().same_contents(t.backend.screen()));
        });
        check(cx, "resumed status");
        let close = cx.debug_bounds("term-search-close").unwrap();
        cx.simulate_click(close.center(), gpui::Modifiers::none());
        terminal.read_with(cx, |t, _| assert!(t.search.is_none()));
    }

    #[gpui::test]
    fn actual_query_option_debounce_and_invalid_error_keep_the_selected_display(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (terminal,cx)=cx.add_window_view(|w,cx| {
            let mut t=Terminal::new(std::env::temp_dir(),cx); t._poll=None;
            t.backend=TerminalSession::from_command(command("stty -echo; i=0; while [ $i -lt 120 ]; do printf 'selected\\n'; i=$((i+1)); done; printf 'READY\\n'; IFS= read -r v"),orlo_terminal::TerminalConfiguration {scrollback:200,..Default::default()});
            t.focus(w); t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        for phase in ["debounce", "held-option", "invalid"] {
            let before = terminal.update_in(cx, |t, w, cx| {
                t.return_to_live(cx);
                t.backend.scroll_lines(10);
                t.backend.set_selection(Some(((0, 0), (0, 8))));
                let before = (t.backend.screen().contents(), t.backend.screen().scrollback());
                t.open_search(&FindOutput, w, cx);
                before
            });
            let path = terminal.read_with(cx, |t, _| t.cwd.clone());
            let hold = (phase == "held-option").then(|| crate::testing::gate::pause("terminal-search-scan", &path));
            if phase == "invalid" {
                cx.simulate_keystrokes("cmd-alt-r");
                cx.simulate_input("[");
            } else {
                cx.simulate_input("s");
            }
            if let Some(hold) = &hold {
                settle_until(cx, |cx| {
                    cx.executor().advance_clock(Duration::from_millis(100));
                    hold.arrived() > 0
                });
                cx.simulate_keystrokes("cmd-alt-c");
            }
            if phase == "invalid" {
                settle_until(cx, |cx| {
                    cx.executor().advance_clock(Duration::from_millis(100));
                    terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().error.is_some())
                });
            }
            cx.simulate_keystrokes("escape");
            terminal.update_in(cx, |t, w, cx| {
                assert!(t.search.is_none());
                assert_eq!(t.display_screen().contents(), before.0, "{phase}");
                assert_eq!(t.display_screen().scrollback(), before.1, "{phase}");
                assert_eq!(t.display_selection(), Some(((0, 0), (0, 8))), "{phase}");
                t.copy(&crate::editor::Copy, w, cx);
                assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("selected"), "{phase}");
            });
            drop(hold);
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            terminal.read_with(cx, |t, _| assert!(t.search.is_none()));
        }
    }

    #[gpui::test]
    fn completed_append_preserves_reviewed_selection_view_and_later_navigation(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (terminal,cx)=cx.add_window_view(|w,cx| {
            let mut t=Terminal::new(std::env::temp_dir(),cx);t._poll=None;
            t.backend=TerminalSession::from_command(command("stty -echo; i=0; while [ $i -lt 120 ]; do printf 'hit selected\\n'; i=$((i+1)); done; printf 'READY\\n'; IFS= read -r v; printf 'UNRELATED-ONE\\n'; IFS= read -r v; printf 'UNRELATED-TWO\\n'; exit 0"),orlo_terminal::TerminalConfiguration{scrollback:400,..Default::default()});
            t.focus(w);t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        for (select, marker) in [(true, "UNRELATED-ONE"), (false, "UNRELATED-TWO")] {
            terminal.update_in(cx, |t, w, cx| {
                t.return_to_live(cx);
                t.open_search(&FindOutput, w, cx);
            });
            cx.simulate_input("hit");
            settle_until(cx, |cx| {
                cx.executor().advance_clock(Duration::from_millis(100));
                terminal.read_with(cx, |t, _| {
                    let s = t.search.as_ref().unwrap();
                    !s.pending && s.results.as_ref().is_some_and(|r| r.len() == 120)
                })
            });
            cx.simulate_keystrokes("cmd-g cmd-g cmd-g");
            if select {
                terminal.update(cx, |t, _| t.output_snapshot_mut().unwrap().scroll_lines(-1));
                cx.run_until_parked();
                let (a, b) = terminal.read_with(cx, |t, _| {
                    let l = t.layout.unwrap();
                    (
                        l.bounds.origin + point(l.cell_width * 4., l.line_height / 2.),
                        l.bounds.origin + point(l.cell_width * 12., l.line_height / 2.),
                    )
                });
                cx.simulate_mouse_down(a, MouseButton::Left, gpui::Modifiers::none());
                cx.simulate_mouse_move(b, Some(MouseButton::Left), gpui::Modifiers::none());
                cx.simulate_mouse_up(b, MouseButton::Left, gpui::Modifiers::none());
            }
            let (before, old) = terminal.read_with(cx, |t, _| {
                let s = t.search.as_ref().unwrap();
                let r = s.results.as_ref().unwrap();
                ((r.screen().contents(), r.screen().scrollback(), r.selection(), r.current()), s.cancel.clone())
            });
            assert_eq!(before.3, Some(3));
            terminal.update(cx, |t, _| {
                t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"go\n".to_vec())).unwrap()
            });
            settle_until(cx, |cx| {
                terminal.update(cx, |t, cx| {
                    t.search.as_mut().unwrap().started = Instant::now() - Duration::from_secs(1);
                    t.drain(cx);
                    t.backend.screen().contents().contains(marker)
                })
            });
            settle_until(cx, |cx| {
                cx.executor().advance_clock(Duration::from_millis(100));
                terminal.read_with(cx, |t, _| {
                    let s = t.search.as_ref().unwrap();
                    !Arc::ptr_eq(&old, &s.cancel) && !s.pending
                })
            });
            terminal.update_in(cx, |t, w, cx| {
                let r = t.search.as_ref().unwrap().results.as_ref().unwrap();
                assert_eq!(r.current(), before.3, "append must not reset later match");
                assert_eq!(r.screen().scrollback(), before.1, "append must not move reviewed viewport");
                assert_eq!(r.screen().contents(), before.0, "review keeps the same independent cells");
                assert_eq!(r.selection(), before.2, "append must not clear reviewed selection");
                if select {
                    t.close_search(&CloseSearch, w, cx);
                    t.copy(&crate::editor::Copy, w, cx);
                    assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("selected"));
                }
            });
        }
        // Explicit update resumes current live/final output with the same query, without old selection coordinates.
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.exited() == Some(0)
            })
        });
        cx.simulate_keystrokes("cmd-f cmd-alt-u");
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| {
                let s = t.search.as_ref().unwrap();
                !s.pending && s.results.as_ref().is_some_and(|r| r.screen().same_contents(t.backend.screen()))
            })
        });
        terminal.read_with(cx, |t, _| {
            let r = t.search.as_ref().unwrap().results.as_ref().unwrap();
            assert!(r.selection().is_none());
            assert_eq!(r.current(), Some(0));
            assert!(t.backend.screen().contents().contains("UNRELATED-TWO"));
        });
    }

    #[gpui::test]
    fn paused_click_resumes_fresh_output_and_pending_work_cannot_resurrect_cleared_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (terminal, cx) = cx.add_window_view(|w, cx| {
            let mut t = Terminal::new(std::env::temp_dir(), cx);
            t._poll = None;
            t.backend = TerminalSession::from_command(
                command(
                    "stty -echo; printf 'hit selected hit\\nREADY\\n'; IFS= read -r v; printf 'NEW-OUTPUT\\n'; exit 0",
                ),
                Default::default(),
            );
            t.focus(w);
            t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        terminal.update_in(cx, |t, w, cx| t.open_search(&FindOutput, w, cx));
        cx.simulate_input("hit");
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| {
                let s = t.search.as_ref().unwrap();
                !s.pending && s.results.as_ref().is_some_and(|r| r.len() == 2)
            })
        });
        terminal.update(cx, |t, _| t.set_display_selection(Some(((0, 4), (0, 12)))));
        let path = terminal.read_with(cx, |t, _| t.cwd.clone());
        let hold = crate::testing::gate::pause("terminal-search-scan", &path);
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("selected");
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            hold.arrived() > 0
        });
        let obsolete = terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().cancel.clone());
        terminal.update(cx, |t, _| t.set_display_selection(None));
        cx.simulate_keystrokes("cmd-alt-c");
        assert!(obsolete.load(Ordering::Relaxed));
        hold.release();
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| {
                let s = t.search.as_ref().unwrap();
                !s.pending && s.results.as_ref().is_some_and(|r| r.selected_text().as_deref() == Some("selected"))
            })
        });
        terminal.read_with(cx, |t, _| {
            assert!(t.display_selection().is_none(), "superseding work cannot restore a cleared selection")
        });
        terminal.update(cx, |t, _| {
            t.set_display_selection(Some(((0, 4), (0, 12))));
            t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"go\n".to_vec())).unwrap();
        });
        let old = terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().cancel.clone());
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.search.as_mut().unwrap().started = Instant::now() - Duration::from_secs(1);
                t.drain(cx);
                t.backend.exited() == Some(0)
            })
        });
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| {
                let s = t.search.as_ref().unwrap();
                !Arc::ptr_eq(&old, &s.cancel) && !s.pending && s.outdated
            })
        });
        terminal.update(cx, |t, cx| {
            let current = t.search.as_ref().unwrap().cancel.clone();
            for _ in 0..3 {
                t.search.as_mut().unwrap().started = Instant::now() - Duration::from_secs(1);
                t.tick_search(true, cx);
            }
            assert!(
                Arc::ptr_eq(&current, &t.search.as_ref().unwrap().cancel),
                "Paused dirty output must not spin repeated scans"
            );
            assert!(!t.display_screen().contents().contains("NEW-OUTPUT"));
        });
        cx.run_until_parked();
        let update = cx.debug_bounds("term-search-status").unwrap();
        cx.simulate_click(update.center(), gpui::Modifiers::none());
        terminal.update_in(cx, |t, w, cx| {
            assert!(t.searching_input(w, cx));
            assert!(t.display_selection().is_none());
            assert!(!t.search.as_ref().unwrap().pinned);
        });
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| {
                let s = t.search.as_ref().unwrap();
                !s.pending && s.results.as_ref().is_some_and(|r| r.screen().same_contents(t.backend.screen()))
            })
        });
        terminal.read_with(cx, |t, _| {
            assert!(t.display_screen().contents().contains("NEW-OUTPUT"));
            assert!(t.display_selection().is_none());
        });
    }

    #[gpui::test]
    fn output_refresh_does_not_cancel_a_pending_snapshot_scan(cx: &mut gpui::TestAppContext) {
        let path = crate::workspace::ui_tests::temp("terminal-search-pending");
        let (terminal, cx) = cx.add_window_view(|window, cx| {
            let mut t = Terminal::new(path.clone(), cx);
            t._poll = None;
            t.backend = TerminalSession::from_command(command(
                "stty -echo; printf 'oldhit\\n'; IFS= read -r v; i=0; while [ $i -lt 200 ]; do printf 'newline\\n'; i=$((i+1)); done; printf 'FINISHED\\n'; exit 0"
            ), orlo_terminal::TerminalConfiguration { scrollback: 2, ..Default::default() });
            t.focus(window);
            t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("oldhit")
            })
        });
        terminal.update_in(cx, |t, window, cx| t.open_search(&FindOutput, window, cx));
        let path = terminal.read_with(cx, |t, _| t.cwd.clone());
        let hold = crate::testing::gate::pause("terminal-search-scan", &path);
        terminal.update(cx, |t, cx| {
            t.search.as_ref().unwrap().query.clone().update(cx, |e, cx| e.set_text("oldhit", cx));
        });
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            hold.arrived() > 0
        });
        let cancel = terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().cancel.clone());
        terminal.update(cx, |t, _| {
            t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"go\n".to_vec())).unwrap()
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.search.as_mut().unwrap().started = Instant::now() - Duration::from_secs(1);
                t.drain(cx);
                assert!(!cancel.load(Ordering::Relaxed), "output alone must allow the pending scan to finish");
                t.backend.screen().contents().contains("FINISHED") && t.backend.exited() == Some(0)
            })
        });
        terminal.update(cx, |t, cx| {
            for _ in 0..3 {
                t.tick_search(true, cx);
            }
            assert!(Arc::ptr_eq(&cancel, &t.search.as_ref().unwrap().cancel));
            let mut live = t.backend.screen().clone();
            live.set_scrollback(usize::MAX);
            assert!(!live.contents().contains("oldhit"), "old match really was evicted");
        });
        hold.release();
        settle_until(cx, |cx| {
            terminal.read_with(cx, |t, _| {
                let search = t.search.as_ref().unwrap();
                !search.pending && search.results.is_some()
            })
        });
        terminal.read_with(cx, |t, _| {
            assert_eq!(t.search.as_ref().unwrap().results.as_ref().unwrap().selected_text().as_deref(), Some("oldhit"));
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        terminal.read_with(cx, |t, _| {
            let s = t.search.as_ref().unwrap();
            assert!(
                s.pending || s.results.as_ref().is_some_and(|r| r.is_empty()),
                "final dirty output must refresh even after the PTY poll stops"
            );
        });
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| {
                let s = t.search.as_ref().unwrap();
                !s.pending && s.results.as_ref().is_some_and(|r| r.is_empty())
            })
        });
    }

    #[gpui::test]
    fn close_during_held_refresh_keeps_pre_find_selection_while_output_runs(cx: &mut gpui::TestAppContext) {
        let path = crate::workspace::ui_tests::temp("terminal-search-close-held");
        let (terminal, cx) = cx.add_window_view(|window, cx| {
            let mut t = Terminal::new(path.clone(), cx); t._poll = None;
            t.backend = TerminalSession::from_command(command(
                "stty -echo; i=0; while [ $i -lt 120 ]; do printf 'selected\\n'; i=$((i+1)); done; printf 'READY\\n'; IFS= read -r v; printf 'MORE\\n'; IFS= read -r v"
            ), orlo_terminal::TerminalConfiguration { scrollback: 200, ..Default::default() });
            t.focus(window); t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        let before = terminal.update_in(cx, |t, w, cx| {
            t.backend.scroll_lines(10);
            t.backend.set_selection(Some(((0, 0), (0, 8))));
            let before = (t.backend.screen().contents(), t.backend.screen().scrollback());
            t.open_search(&FindOutput, w, cx);
            before
        });
        let path = terminal.read_with(cx, |t, _| t.cwd.clone());
        let hold = crate::testing::gate::pause("terminal-search-scan", &path);
        cx.simulate_input("s");
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            hold.arrived() > 0
        });
        terminal.update(cx, |t, _| {
            t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"go\n".to_vec())).unwrap()
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                let mut live = t.backend.screen().clone();
                live.set_scrollback(0);
                live.contents().contains("MORE")
            })
        });
        terminal.update_in(cx, |t, w, cx| {
            t.close_search(&CloseSearch, w, cx);
            assert_eq!(t.display_screen().contents(), before.0);
            assert_eq!(t.display_screen().scrollback(), before.1);
            assert_eq!(t.display_selection(), Some(((0, 0), (0, 8))));
            t.copy(&crate::editor::Copy, w, cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("selected"));
        });
        hold.release();
        cx.run_until_parked();
        terminal.read_with(cx, |t, _| assert!(t.search.is_none()));
        assert!(cx.debug_bounds("term-search-live").is_some());
        let live = cx.debug_bounds("term-search-live").unwrap();
        cx.simulate_click(live.center(), gpui::Modifiers::none());
        terminal.read_with(cx, |t, _| assert!(t.retained_output.is_none()));
    }

    #[gpui::test]
    fn reachable_options_copy_navigation_and_mouse_do_not_send_search_bytes(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (terminal, cx) = cx.add_window_view(|w, cx| {
            let mut t = Terminal::new(std::env::temp_dir(), cx); t._poll = None;
            t.backend = TerminalSession::from_command(command(
                "stty -echo; printf '\\033[?1000h\\033[?1006hAbC abc a.b\\nREADY\\n'; IFS= read -r v; printf 'INPUT:'; printf '%s' \"$v\" | od -An -tx1 | tr -d ' \\n'; printf '\\nINPUT-DONE\\n'; IFS= read -r v"
            ), Default::default()); t.focus(w); t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("abc");
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().results.as_ref().is_some_and(|r| r.len() == 1))
        });
        cx.run_until_parked();
        let case = cx.debug_bounds("term-search-case").unwrap();
        cx.simulate_click(case.center(), gpui::Modifiers::none());
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().results.as_ref().is_some_and(|r| r.len() == 2))
        });
        terminal.update_in(cx, |t, w, cx| {
            assert!(t.searching_input(w, cx), "clicking Case keeps typing in Find");
            t.focus(w);
        });
        cx.simulate_keystrokes("cmd-c");
        assert_eq!(cx.read(|cx| cx.read_from_clipboard().unwrap().text()).as_deref(), Some("AbC"));
        cx.simulate_keystrokes("cmd-g");
        terminal
            .read_with(cx, |t, _| assert_eq!(t.search.as_ref().unwrap().results.as_ref().unwrap().current(), Some(1)));
        cx.simulate_keystrokes("cmd-f");
        cx.run_until_parked();
        let regex = cx.debug_bounds("term-search-regex").unwrap();
        cx.simulate_click(regex.center(), gpui::Modifiers::none());
        terminal.update_in(cx, |t, w, cx| {
            assert!(t.search.as_ref().unwrap().options.regex);
            assert!(t.searching_input(w, cx), "clicking Regex keeps typing in Find");
        });
        cx.simulate_keystrokes("cmd-alt-r cmd-alt-r");
        terminal.update(cx, |t, cx| t.search.as_ref().unwrap().query.clone().update(cx, |e, cx| e.set_text("a.c", cx)));
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().results.as_ref().is_some_and(|r| r.len() == 2))
        });
        cx.simulate_keystrokes("shift-enter");
        terminal.read_with(cx, |t, _| {
            let r = t.search.as_ref().unwrap().results.as_ref().unwrap();
            assert_eq!(r.current(), Some(1));
            assert!(r.highlights(0, 4));
            assert_eq!(t.display_screen().cell(0, 4).unwrap().contents(), "a");
        });
        terminal.update(cx, |t, cx| t.search.as_ref().unwrap().query.clone().update(cx, |e, cx| e.set_text("[", cx)));
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().error.is_some())
        });
        cx.simulate_keystrokes("cmd-alt-r cmd-alt-c cmd-alt-u escape");
        // Retained output uses local drag even when the live application requested mouse reports.
        terminal.update_in(cx, |t, w, cx| {
            t.open_search(&FindOutput, w, cx);
            t.close_search(&CloseSearch, w, cx);
        });
        cx.run_until_parked();
        let (a, b) = terminal.read_with(cx, |t, _| {
            let l = t.layout.unwrap();
            (
                l.bounds.origin + point(l.cell_width, l.line_height / 2.),
                l.bounds.origin + point(l.cell_width * 3., l.line_height / 2.),
            )
        });
        cx.simulate_mouse_down(a, MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_move(b, Some(MouseButton::Left), gpui::Modifiers::none());
        cx.simulate_mouse_up(b, MouseButton::Left, gpui::Modifiers::none());
        terminal.read_with(cx, |t, _| assert!(t.display_selection().is_some()));
        cx.simulate_input("SENTINEL");
        cx.simulate_keystrokes("enter");
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("INPUT-DONE")
            })
        });
        terminal.read_with(cx, |t, _| {
            assert!(
                t.backend.screen().contents().lines().any(|line| line == "INPUT:53454e54494e454c"),
                "{}",
                t.backend.screen().contents()
            );
            assert!(t.search.is_none());
            assert!(t.retained_output.is_none());
        });
    }

    #[gpui::test]
    fn held_query_and_option_changes_reject_obsolete_results(cx: &mut gpui::TestAppContext) {
        let path = crate::workspace::ui_tests::temp("terminal-search-stale-query");
        let (terminal, cx) = cx.add_window_view(|w, cx| {
            let mut t = Terminal::new(path.clone(), cx);
            t._poll = None;
            t.backend =
                TerminalSession::from_command(command("printf 'AbC abc\\n'; IFS= read -r v"), Default::default());
            t.focus(w);
            t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("AbC abc")
            })
        });
        terminal.update_in(cx, |t, w, cx| t.open_search(&FindOutput, w, cx));
        let path = terminal.read_with(cx, |t, _| t.cwd.clone());
        let hold = crate::testing::gate::pause("terminal-search-scan", &path);
        terminal
            .update(cx, |t, cx| t.search.as_ref().unwrap().query.clone().update(cx, |e, cx| e.set_text("absent", cx)));
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            hold.arrived() > 0
        });
        let old = terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().cancel.clone());
        terminal.update_in(cx, |t, w, cx| {
            t.search.as_ref().unwrap().query.clone().update(cx, |e, cx| e.set_text("abc", cx));
            t.toggle_search_case(&ToggleSearchCase, w, cx);
            t.toggle_search_regex(&ToggleSearchRegex, w, cx);
        });
        assert!(old.load(Ordering::Relaxed));
        hold.release();
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().results.as_ref().is_some_and(|r| r.len() == 2))
        });
        terminal.read_with(cx, |t, _| {
            assert_eq!(t.search.as_ref().unwrap().results.as_ref().unwrap().selected_text().as_deref(), Some("AbC"))
        });
    }

    #[gpui::test]
    fn held_jobs_and_closed_views_reject_session_geometry_and_screen_epoch_changes(cx: &mut gpui::TestAppContext) {
        let path = crate::workspace::ui_tests::temp("terminal-search-stale-screen");
        let (terminal, cx) = cx.add_window_view(|w, cx| {
            let mut t = Terminal::new(path.clone(), cx);
            t._poll = None;
            t.focus(w);
            t
        });
        for change in ["session", "resize", "alternate", "roundtrip", "reset"] {
            let control = match change {
                "alternate" => "\\033[?1049h",
                "roundtrip" => "\\033[?1049hOTHER\\033[?1049l",
                "reset" => "\\033c",
                _ => "",
            };
            terminal.update(cx, |t, _| {
                t.search = None;
                t.retained_output = None;
                t.backend = TerminalSession::from_command(
                    command(&format!(
                        "stty -echo; printf 'oldhit\\n'; IFS= read -r v; printf '{control}CHANGED\\n'; IFS= read -r v"
                    )),
                    Default::default(),
                );
            });
            settle_until(cx, |cx| {
                terminal.update(cx, |t, cx| {
                    t.drain(cx);
                    t.backend.screen().contents().contains("oldhit")
                })
            });
            terminal.update_in(cx, |t, w, cx| t.open_search(&FindOutput, w, cx));
            let path = terminal.read_with(cx, |t, _| t.cwd.clone());
            let hold = crate::testing::gate::pause("terminal-search-scan", &path);
            terminal.update(cx, |t, cx| {
                t.search.as_ref().unwrap().query.clone().update(cx, |e, cx| e.set_text("oldhit", cx))
            });
            settle_until(cx, |cx| {
                cx.executor().advance_clock(Duration::from_millis(100));
                hold.arrived() > 0
            });
            terminal.update(cx, |t, _| match change {
                "session" => {
                    let (rows, cols) = t.backend.screen().size();
                    t.backend = TerminalSession::from_command(
                        command("printf 'CHANGED\\n'; IFS= read -r v"),
                        orlo_terminal::TerminalConfiguration { rows, cols, ..Default::default() },
                    )
                }
                "resize" => {
                    let (r, c) = t.backend.screen().size();
                    t.backend.resize(r + 1, c);
                    t.backend.resize(r, c);
                }
                _ => t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"go\n".to_vec())).unwrap(),
            });
            if change != "resize" {
                settle_until(cx, |cx| {
                    terminal.update(cx, |t, cx| {
                        t.drain(cx);
                        t.backend.screen().contents().contains("CHANGED")
                    })
                });
            }
            hold.release();
            settle_until(cx, |cx| {
                cx.executor().advance_clock(Duration::from_millis(100));
                terminal.read_with(cx, |t, _| !t.search.as_ref().unwrap().pending)
            });
            terminal.read_with(cx, |t, _| {
                if let Some(result) = &t.search.as_ref().unwrap().results {
                    assert!(
                        result.screen().same_layout(t.backend.screen()),
                        "{change}: only a current-epoch replacement may publish"
                    );
                }
            });
            // The same identity rule discards an already closed view, without restoring its cells to live output.
            terminal.update_in(cx, |t, w, cx| {
                t.search = None;
                t.open_search(&FindOutput, w, cx);
                t.close_search(&CloseSearch, w, cx);
                assert!(t.retained_output.is_some());
                let (r, c) = t.backend.screen().size();
                t.backend.resize(r + 1, c);
                t.backend.resize(r, c);
                t.tick_search(false, cx);
                assert!(t.retained_output.is_none());
            });
        }
    }

    #[gpui::test]
    fn find_retained_command_records_and_exit_share_stable_output_geometry(cx: &mut gpui::TestAppContext) {
        let (terminal,cx)=cx.add_window_view(|w,cx| {
            let mut t=Terminal::new(std::env::temp_dir(),cx); t._poll=None;
            t.backend=TerminalSession::from_command(command(
                "stty -echo; printf 'READY\\n'; IFS= read -r v; printf '\\033]133;A\\007$ \\033]133;B\\007echo hello\\r\\n\\033]133;C\\007hello\\r\\n\\033]133;D;0\\007'; IFS= read -r v; exit 7"
            ),Default::default()); t.focus(w); t
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("READY")
            })
        });
        cx.run_until_parked();
        let (bounds, size) = terminal.read_with(cx, |t, _| (t.layout.unwrap().bounds, t.backend.screen().size()));
        terminal.update_in(cx, |t, w, cx| {
            assert!(t.backend.commands().records().is_empty());
            t.backend.set_selection(Some(((0, 0), (0, 5))));
            t.open_search(&FindOutput, w, cx);
            t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"go\n".to_vec())).unwrap();
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                !t.backend.commands().records().is_empty()
            })
        });
        cx.run_until_parked();
        terminal.update_in(cx, |t, w, cx| {
            assert_eq!(t.layout.unwrap().bounds, bounds);
            assert_eq!(t.backend.screen().size(), size);
            t.close_search(&CloseSearch, w, cx);
            assert_eq!(t.display_selection(), Some(((0, 0), (0, 5))));
            t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"finish\n".to_vec())).unwrap();
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.exited() == Some(7)
            })
        });
        cx.run_until_parked();
        terminal.update_in(cx, |t, w, cx| {
            assert_eq!(t.layout.unwrap().bounds, bounds);
            assert_eq!(t.backend.screen().size(), size);
            assert_eq!(t.display_selection(), Some(((0, 0), (0, 5))));
            t.copy(&crate::editor::Copy, w, cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().as_deref(), Some("READY"));
            t.review_command(false, cx);
        });
        cx.run_until_parked();
        terminal.update_in(cx, |t, w, cx| {
            assert!(t.command_review.is_some());
            assert_eq!(t.layout.unwrap().bounds, bounds);
            t.backend.set_selection(Some(((0, 0), (0, 5))));
            t.open_search(&FindOutput, w, cx);
            assert!(t.display_selection().is_none(), "command-review cells must not become a live-output selection");
            t.close_search(&CloseSearch, w, cx);
            t.backend.set_selection(Some(((0, 0), (0, 5))));
            t.set_display_selection(Some(((0, 1), (0, 3))));
            t.open_search(&FindOutput, w, cx);
            assert!(t.display_selection().is_none(), "retained-view cells must not become a live-output selection");
            t.return_to_live(cx);
        });
        cx.run_until_parked();
        terminal.read_with(cx, |t, _| assert_eq!(t.layout.unwrap().bounds, bounds));
    }

    #[gpui::test]
    fn close_find_preserves_displayed_scroll_and_selection(cx: &mut gpui::TestAppContext) {
        let (terminal, cx) = cx.add_window_view(|window, cx| {
            let mut t = Terminal::new(std::env::temp_dir(), cx);
            t._poll = None;
            t.focus(window);
            t
        });
        terminal.update_in(cx, |t, window, cx| {
            t.open_search(&FindOutput, window, cx);
            let mut e =
                orlo_terminal::Emulator::with_size(t.backend.screen().size().0, t.backend.screen().size().1, 100);
            for _ in 0..100 {
                e.process(b"hit\r\n");
            }
            t.search.as_mut().unwrap().results =
                SearchResults::find(e.screen().clone(), "hit", &AtomicBool::new(false));
            t.set_display_selection(Some(((0, 0), (0, 3))));
            let before = t.display_screen().contents();
            let offset = t.display_screen().scrollback();
            assert!(offset > 0);
            t.close_search(&CloseSearch, window, cx);
            assert_eq!(t.display_screen().scrollback(), offset);
            assert_eq!(t.display_screen().contents(), before);
            assert_eq!(t.display_selection(), Some(((0, 0), (0, 3))));
        });
    }

    #[gpui::test]
    fn search_keys_stale_queries_and_live_pty_output(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (terminal, cx) = cx.add_window_view(|window, cx| {
            let mut terminal = Terminal::new(std::env::temp_dir(), cx);
            terminal._poll = None;
            let mut command = orlo_terminal::CommandBuilder::new("/bin/sh");
            command.args([
                "-c",
                "printf 'hit hit\\n'; IFS= read -r value; printf 'new:%s\\n' \"$value\"; IFS= read -r value",
            ]);
            terminal.backend = TerminalSession::from_command(command, orlo_terminal::TerminalConfiguration::default());
            terminal.focus(window);
            terminal
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("hit hit")
            })
        });
        cx.simulate_keystrokes("cmd-f");
        terminal.update(cx, |t, cx| {
            t.search.as_ref().unwrap().query.clone().update(cx, |e, cx| e.set_text("hit", cx));
        });
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| t.search.as_ref().unwrap().results.as_ref().is_some_and(|r| r.len() == 2))
        });
        cx.simulate_keystrokes("enter");
        terminal
            .read_with(cx, |t, _| assert_eq!(t.search.as_ref().unwrap().results.as_ref().unwrap().current(), Some(1)));
        // Writing directly to the PTY models output arriving independently of the search UI.
        terminal.update(cx, |t, _| {
            t.backend.pty().unwrap().send(orlo_terminal::PtyCommand::Input(b"hit\r".to_vec())).unwrap()
        });
        settle_until(cx, |cx| {
            terminal.update(cx, |t, cx| {
                t.drain(cx);
                t.backend.screen().contents().contains("new:hit")
            })
        });
        terminal.update(cx, |t, cx| {
            let query = t.search.as_ref().unwrap().query.clone();
            query.update(cx, |e, cx| e.set_text("absent", cx));
            t.refresh_search(true, cx);
            query.update(cx, |e, cx| e.set_text("new:hit", cx));
        });
        cx.simulate_keystrokes("cmd-alt-u");
        settle_until(cx, |cx| {
            cx.executor().advance_clock(Duration::from_millis(100));
            terminal.read_with(cx, |t, _| {
                t.search
                    .as_ref()
                    .unwrap()
                    .results
                    .as_ref()
                    .is_some_and(|r| r.selected_text().as_deref() == Some("new:hit"))
            })
        });
        cx.simulate_keystrokes("escape");
        terminal.read_with(cx, |t, _| {
            assert!(t.search.is_none());
            assert_eq!(t.backend.screen().scrollback(), 0);
            assert!(t.marked.is_empty());
        });
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        terminal.read_with(cx, |t, _| assert!(t.search.is_none(), "late query must not reopen search"));
    }
}
