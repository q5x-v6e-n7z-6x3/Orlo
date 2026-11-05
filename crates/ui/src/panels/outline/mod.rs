//! The active buffer's outline, and a file's related files (found from names and its text, no
//! language server needed).

use crate::languages::index::{EntryKey, IndexedSymbol};
use crate::workspace::*;
use gpui::{uniform_list, ScrollStrategy};
use ropey::Rope;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
struct Key {
    editor: EntityId,
    version: u64,
    syntax: String,
    tab_size: usize,
    path: Option<PathBuf>,
}

impl Key {
    fn new(editor: &Entity<Editor>, cx: &App) -> Self {
        let e = editor.read(cx);
        Self {
            editor: editor.entity_id(),
            version: e.doc(cx).version,
            syntax: e.highlighter.syntax_name().into(),
            tab_size: e.doc(cx).tab_size,
            path: e.doc(cx).path.clone(),
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub visible: bool,
    wanted: Option<Key>,
    ready: Option<Key>,
    symbols: Arc<Vec<IndexedSymbol>>,
    depths: Vec<usize>,
    selected: Option<usize>,
    parsing: bool,
    build: Option<Task<()>>,
    delay: Option<Task<()>>,
    scroll: gpui::UniformListScrollHandle,
    file_request: u64,
    file_task: Option<Task<()>>,
}

impl State {
    /// Show `symbols` at `depths`; the rows they replace are freed off the UI thread.
    fn replace_rows(&mut self, symbols: Arc<Vec<IndexedSymbol>>, depths: Vec<usize>, cx: &App) {
        let previous = (std::mem::replace(&mut self.symbols, symbols), std::mem::replace(&mut self.depths, depths));
        cx.background_spawn(async move { drop(previous) }).detach();
    }
}

fn depths(text: &Rope, symbols: &[IndexedSymbol], tab_size: usize) -> Vec<usize> {
    let mut stack = Vec::new();
    symbols
        .iter()
        .map(|s| {
            let indent = text.get_line(s.line).and_then(|l| crate::editing::fold::indent(l, tab_size)).unwrap_or(0);
            while stack.last().is_some_and(|i| *i >= indent) {
                stack.pop();
            }
            let depth = stack.len();
            stack.push(indent);
            depth
        })
        .collect()
}

fn selected(symbols: &[IndexedSymbol], line: usize, col: usize) -> Option<usize> {
    symbols.partition_point(|s| (s.line, s.col) <= (line, col)).checked_sub(1)
}

impl Workspace {
    pub(crate) fn toggle_outline(&mut self, _: &ToggleOutline, _: &mut Window, cx: &mut Context<Self>) {
        self.outline.visible = !self.outline.visible;
        if !self.outline.visible {
            self.outline.wanted = None;
            self.outline.build = None;
            self.outline.delay = None;
            self.outline.parsing = false;
        }
        self.refresh_menus(cx);
        autosave(cx);
        cx.notify();
    }

    fn outline_key(&self, cx: &App) -> Option<Key> {
        self.active_editor().map(|e| Key::new(&e, cx))
    }

    fn outline_fresh(&self, key: &Key, cx: &App) -> bool {
        let Some(editor) = self.active_editor().filter(|e| Key::new(e, cx) == *key) else {
            return false;
        };
        self.index
            .read(cx)
            .entry_for(&editor, cx)
            .is_some_and(|e| matches!(e.key, EntryKey::Buffer(_, v) if v == key.version) && e.syntax == key.syntax)
    }

    /// Called as the outline draws, it only starts work: the indentation reads and the rows are
    /// made off the UI thread.
    fn sync_outline(&mut self, cx: &mut Context<Self>) {
        if !self.outline.visible {
            return;
        }
        let Some(editor) = self.active_editor() else {
            self.outline.wanted = None;
            self.outline.ready = None;
            self.outline.replace_rows(Default::default(), Vec::new(), cx);
            self.outline.parsing = false;
            self.outline.delay = None;
            self.outline.build = None;
            return;
        };
        let key = Key::new(&editor, cx);
        if self.outline.wanted.as_ref() != Some(&key) {
            let source_changed = self.outline.wanted.as_ref().is_none_or(|k| {
                k.editor != key.editor || k.syntax != key.syntax || k.path != key.path || k.tab_size != key.tab_size
            });
            if source_changed {
                self.outline.replace_rows(Default::default(), Vec::new(), cx);
                self.outline.ready = None;
                self.outline.delay = None;
                self.outline.parsing = false;
            }
            self.outline.wanted = Some(key.clone());
            self.outline.selected = None;
            self.outline.build = None;
            if source_changed {
                self.index.update(cx, |i, cx| i.ensure_fresh(&editor, cx)).detach();
            }
        }
        if self.outline_fresh(&key, cx) {
            self.outline.delay = None;
            self.outline.parsing = false;
            let entry = self.index.read(cx).entry_for(&editor, cx).unwrap();
            if self.outline.ready.as_ref() == Some(&key) && Arc::ptr_eq(&entry.symbols, &self.outline.symbols) {
                let e = editor.read(cx);
                let caret = e.state.cursor();
                let line = e.doc(cx).rope.char_to_line(caret);
                let at = selected(&self.outline.symbols, line, caret - e.doc(cx).rope.line_to_char(line));
                if at != self.outline.selected {
                    self.outline.selected = at;
                    if let Some(at) = at {
                        self.outline.scroll.scroll_to_item(at, ScrollStrategy::Center);
                    }
                }
            } else if self.outline.build.is_none() {
                let symbols = entry.symbols.clone();
                let text = editor.read(cx).doc(cx).rope.clone();
                let tab_size = key.tab_size;
                let work = cx.background_spawn(async move {
                    let nesting = depths(&text, &symbols, tab_size);
                    (symbols, nesting)
                });
                self.outline.build = Some(cx.spawn(async move |ws, cx| {
                    let (symbols, depths) = work.await;
                    ws.update(cx, |ws, cx| {
                        if !ws.outline.visible || ws.outline.wanted.as_ref() != Some(&key) {
                            return;
                        }
                        ws.outline.build = None;
                        let current = ws.active_editor().is_some_and(|editor| {
                            (ws.index.read(cx).entry_for(&editor, cx))
                                .is_some_and(|e| Arc::ptr_eq(&e.symbols, &symbols))
                        });
                        if !ws.outline_fresh(&key, cx) || !current {
                            cx.notify();
                            return;
                        }
                        ws.outline.delay = None;
                        ws.outline.parsing = false;
                        ws.outline.ready = Some(key);
                        ws.outline.replace_rows(symbols, depths, cx);
                        ws.outline.selected = None;
                        cx.notify();
                    })
                    .ok();
                }));
            }
        } else if self.outline.delay.is_none() {
            self.outline.delay = Some(cx.spawn(async move |ws, cx| {
                cx.background_executor().timer(Duration::from_millis(300)).await;
                ws.update(cx, |ws, cx| {
                    if ws.outline.visible && ws.outline.wanted.as_ref().is_some_and(|key| !ws.outline_fresh(key, cx)) {
                        ws.outline.parsing = true;
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
    }

    fn jump_outline(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.outline.ready.as_ref() else {
            return;
        };
        if self.outline.wanted.as_ref() != Some(key) || !self.outline_fresh(key, cx) {
            return;
        }
        let Some(symbol) = self.outline.symbols.get(at) else {
            return;
        };
        let (line, col) = (symbol.line, symbol.col);
        if let Some(editor) = self.active_editor() {
            editor.update(cx, |e, cx| {
                let offset = e.doc(cx).offset(line, col);
                e.select_range(offset..offset, cx);
            });
        }
        self.focus_editor(window, cx);
    }

    pub(crate) fn render_outline(&mut self, width: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.sync_outline(cx);
        if !self.outline.visible {
            return None;
        }
        let mut panel = div()
            .id("outline")
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(ui(theme::PANEL_BG))
            .border_l_1()
            .border_color(ui(theme::BORDER))
            .child(div().id("outline-heading").px(px(12.)).py(px(8.)).child("OUTLINE").tooltip(kit::tooltip(
                "Symbols nest by indentation under the nearest earlier symbol with a smaller indent.".into(),
            )));
        if self.outline.parsing
            || (self.outline.ready == self.outline.wanted && self.outline.symbols.is_empty())
            || self.active_editor().is_none()
        {
            panel = panel.child(
                div()
                    .id("outline-empty")
                    .px(px(12.))
                    .py(px(8.))
                    .text_color(ui(theme::TEXT_MUTED))
                    .child(if self.outline.parsing { "Parsing…" } else { "No symbols" }),
            );
        } else {
            panel =
                panel.child(
                    uniform_list(
                        "outline-symbols",
                        self.outline.symbols.len(),
                        cx.processor(|ws, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|i| {
                                    let s = &ws.outline.symbols[i];
                                    let (badge, color) = highlight::symbol_badge(&s.kind);
                                    div()
                                        .id(gpui::SharedString::from(format!("outline-row-{i}")))
                                        .debug_selector(move || format!("outline-row-{i}"))
                                        .h(px(24.))
                                        .flex()
                                        .items_center()
                                        .pl(px(12. + 12. * ws.outline.depths[i] as f32))
                                        .pr(px(8.))
                                        .gap(px(6.))
                                        .overflow_hidden()
                                        .when(ws.outline.selected == Some(i), |d| d.bg(ui(theme::SIDEBAR_SELECTED_BG)))
                                        .hover(|s| s.bg(ui(theme::SIDEBAR_HOVER_BG)))
                                        .cursor(CursorStyle::PointingHand)
                                        .child(div().text_color(c(color)).child(badge.to_string()))
                                        .child(div().overflow_hidden().text_ellipsis().child(s.name.clone()))
                                        .on_click(cx.listener(move |ws, _: &ClickEvent, window, cx| {
                                            ws.jump_outline(i, window, cx)
                                        }))
                                        .into_any_element()
                                })
                                .collect()
                        }),
                    )
                    .flex_1()
                    .track_scroll(self.outline.scroll.clone()),
                );
        }
        Some(panel.into_any_element())
    }

    forward_actions! {
    pub(crate) switch_file(SwitchFile, window, cx) => discover_related(true, window, cx);
    pub(crate) related_files(RelatedFiles, window, cx) => discover_related(false, window, cx);
    }

    fn discover_related(&mut self, switch: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.outline.file_request += 1;
        self.outline.file_task = None;
        self.invalidate_navigation();
        let Some(editor) = self.active_editor() else {
            return;
        };
        let key = Key::new(&editor, cx);
        let Some(path) = key.path.clone() else {
            return self.status_message("No related file", cx);
        };
        let roots = self.roots.clone();
        let text = editor.read(cx).doc(cx).rope.clone();
        let panel = if switch {
            None
        } else {
            self.open_quick_panel(Kind::Choices, Vec::new(), window, cx);
            let p = self.quick_panel.as_mut().unwrap();
            p.title = Some("Related Files…".into());
            Some(p.input.entity_id())
        };
        let request = self.outline.file_request;
        let navigation = self.language.navigation_generation;
        let expected_roots = roots.clone();
        let work = cx.background_spawn(async move {
            if switch {
                (crate::languages::related::switch_file(&path), None)
            } else {
                let items = crate::languages::related::discover(&path, &roots, &text)
                    .into_iter()
                    .map(|r| {
                        let label = r.path.to_string_lossy().into_owned();
                        quick_panel::Item {
                            hint: r.reason.into(),
                            ..quick_panel::Item::new(label, "", ItemAction::OpenFile(r.path))
                        }
                    })
                    .collect();
                (None, Some(quick_panel::Rows::new(Kind::Choices, items)))
            }
        });
        self.outline.file_task = Some(cx.spawn_in(window, async move |ws, cx| {
            let (path, rows) = work.await;
            ws.update_in(cx, |ws, window, cx| {
                if (switch && navigation != ws.language.navigation_generation)
                    || request != ws.outline.file_request
                    || ws.outline_key(cx).as_ref() != Some(&key)
                    || ws.roots != expected_roots
                {
                    return;
                }
                ws.outline.file_task = None;
                if switch {
                    match path {
                        Some(p) => ws.open_path(p, window, cx),
                        None => ws.status_message("No related file", cx),
                    }
                } else if let Some(p) = ws.quick_panel.as_mut().filter(|p| Some(p.input.entity_id()) == panel) {
                    p.replace_rows(Kind::Choices, rows.unwrap());
                    p.loaded = true;
                    let query = p.query.clone();
                    p.refilter(&query);
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{settle_until, temp, window};
    use gpui::{TestAppContext, VisualTestContext};
    use std::sync::Arc;

    fn set_syntax(ws: &mut Workspace, name: &str, window: &mut Window, cx: &mut Context<Workspace>) {
        ws.set_syntax(&SetSyntax { name: name.into() }, window, cx);
    }

    fn active_path(ws: &Workspace, cx: &App) -> Option<PathBuf> {
        ws.active_editor().unwrap().read(cx).doc(cx).path.clone()
    }

    fn ready(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| ws.outline.ready == ws.outline.wanted && ws.outline.ready.is_some())
        });
    }

    #[gpui::test]
    fn outline_uses_unsaved_shared_symbols_nests_highlights_and_clicks(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let dir = temp("orlo-outline-ui").canonicalize().unwrap();
        std::fs::write(dir.join("x.py"), "def saved(): pass\n").unwrap();
        let editor = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("x.py"), window, cx);
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("class Outer:\n    def inner(self): pass\ndef last(): pass\n");
                })
            });
            ws.toggle_outline(&ToggleOutline, window, cx);
            e
        });
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, cx| {
            let entry = ws.index.read(cx).entry_for(&editor, cx).unwrap();
            assert!(Arc::ptr_eq(&ws.outline.symbols, &entry.symbols));
            assert_eq!(
                ws.outline.symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
                ["Outer", "inner", "last"]
            );
            assert_eq!(ws.outline.depths, vec![0, 1, 0]);
        });
        editor.update(vcx, |e, cx| {
            sel_of(&e.document, &mut e.state, cx).set_cursor(doc_of(&e.document, cx).len());
            cx.notify();
        });
        vcx.run_until_parked();
        assert_eq!(ws.read_with(vcx, |ws, _| ws.outline.selected), Some(2));
        let b = vcx.debug_bounds("outline-row-1").unwrap();
        vcx.simulate_click(b.center(), gpui::Modifiers::none());
        editor.read_with(vcx, |e, cx| {
            assert_eq!(e.doc(cx).rope.char_to_line(e.state.cursor()), 1);
        });
        vcx.update(|window, cx| assert!(editor.read(cx).is_focused(window)));
        editor.update(vcx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert("def changed(): pass\n");
            })
        });
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(300));
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| assert_eq!(ws.outline.symbols[0].name, "changed"));
    }

    #[gpui::test]
    fn outline_lists_and_nests_a_pony_files_tags_scm_definitions(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let dir = temp("orlo-outline-pony").canonicalize().unwrap();
        std::fs::write(
            dir.join("x.pony"),
            "actor Main\n  new create(env: Env) => None\n\nclass Counter\n  fun total(): U64 => 0\n",
        )
        .unwrap();
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("x.pony"), window, cx);
            ws.toggle_outline(&ToggleOutline, window, cx);
        });
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| {
            let names: Vec<_> = ws.outline.symbols.iter().map(|s| (s.name.as_str(), s.kind.as_str())).collect();
            assert_eq!(names, [("Main", "class"), ("create", "method"), ("Counter", "class"), ("total", "method")]);
            assert_eq!(ws.outline.depths, vec![0, 1, 0, 1]);
        });
    }

    #[gpui::test]
    fn outline_keeps_open_buffer_index_with_index_files_off(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            let mut settings = cx.global::<Settings>().clone();
            settings.index_files = false;
            cx.set_global(settings);
            set_syntax(ws, "Rust", window, cx);
            ws.active_editor().unwrap().update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("fn unsaved() {}\n");
                })
            });
            ws.toggle_outline(&ToggleOutline, window, cx);
        });
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            assert!(Arc::ptr_eq(&ws.outline.symbols, &ws.index.read(cx).entry_for(&e, cx).unwrap().symbols));
            assert_eq!(ws.outline.symbols[0].name, "unsaved");
        });
    }

    #[gpui::test]
    fn outline_visibility_roundtrips_only_the_per_window_flag(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let state = ws.update_in(vcx, |ws, window, cx| {
            ws.toggle_outline(&ToggleOutline, window, cx);
            ws.capture(window, false, &mut Vec::new(), cx)
        });
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"outline_visible\":true"));
        let state = serde_json::from_str(&json).unwrap();
        ws.update_in(vcx, |ws, window, cx| {
            ws.toggle_outline(&ToggleOutline, window, cx);
            ws.restore(state, window, cx);
            assert!(ws.outline.visible);
        });
        assert!(!serde_json::from_str::<WindowState>("{}").unwrap().outline_visible);
    }

    #[gpui::test]
    fn switch_file_binding_cycles_and_reports_missing(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let dir = temp("orlo-switch-ui").canonicalize().unwrap();
        for p in ["foo.c", "foo.h", "foo.json", "alone.rs"] {
            std::fs::write(dir.join(p), "").unwrap();
        }
        ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.join("foo.c"), window, cx));
        for p in ["foo.h", "foo.json", "foo.c"] {
            vcx.simulate_keystrokes("cmd-alt-up");
            settle_until(vcx, |vcx| ws.read_with(vcx, |ws, cx| active_path(ws, cx) == Some(dir.join(p))));
        }
        ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.join("alone.rs"), window, cx));
        vcx.simulate_keystrokes("cmd-alt-up");
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| ws.status.as_ref().is_some_and(|s| s.0 == "No related file"))
        });
    }

    #[gpui::test]
    fn related_panel_reads_unsaved_text_and_opens_choice(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let dir = temp("orlo-related-ui").canonicalize().unwrap();
        std::fs::write(dir.join("main.rs"), "").unwrap();
        std::fs::write(dir.join("unsaved.txt"), "target").unwrap();
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("main.rs"), window, cx);
            ws.active_editor().unwrap().update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("\"unsaved.txt\"");
                })
            });
            ws.related_files(&RelatedFiles, window, cx);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.loaded && p.items.len() == 1))
        });
        ws.read_with(vcx, |ws, _| assert_eq!(ws.quick_panel.as_ref().unwrap().items[0].hint, "referenced"));
        vcx.simulate_keystrokes("enter");
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, cx| active_path(ws, cx) == Some(dir.join("unsaved.txt"))));
    }
    fn symbol(name: &str, line: usize, col: usize) -> IndexedSymbol {
        IndexedSymbol { name: name.into(), kind: "function".into(), line, col, signature: String::new(), doc: None }
    }

    #[test]
    fn indentation_is_not_name_column_and_same_line_symbols_stay_distinct() {
        let text = Rope::from_str("class Outer:\n\tdef nested(): pass\n  def sibling(): pass\nfn a() {} fn b() {}\n");
        let symbols = vec![
            symbol("Outer", 0, 6),
            symbol("nested", 1, 5),
            symbol("sibling", 2, 6),
            symbol("a", 3, 3),
            symbol("b", 3, 13),
        ];
        assert_eq!(depths(&text, &symbols, 4), [0, 1, 1, 0, 0]);
        assert_eq!(selected(&symbols, 3, 10), Some(3));
        assert_eq!(selected(&symbols, 3, 13), Some(4));
        assert_eq!(selected(&symbols, 0, 0), None);
    }

    struct HeldWorker(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl HeldWorker {
        fn new() -> Self {
            use std::sync::atomic::{AtomicBool, Ordering};
            let release = Arc::new(AtomicBool::new(false));
            let started = Arc::new(AtomicBool::new(false));
            let (stop, running) = (release.clone(), started.clone());
            let _job = highlight::on_worker_urgent(move || {
                running.store(true, Ordering::Relaxed);
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            while !started.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Self(release)
        }
    }
    impl Drop for HeldWorker {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[gpui::test]
    fn parsing_waits_300ms_stale_rows_cannot_jump_and_tab_changes_win(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let editor = ws.update_in(vcx, |ws, window, cx| {
            set_syntax(ws, "Rust", window, cx);
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| e.edit(cx, |s| s.insert("fn old() {}\n")));
            ws.toggle_outline(&ToggleOutline, window, cx);
            e
        });
        ready(&ws, vcx);
        let hold = HeldWorker::new();
        editor.update(vcx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("// inserted\n");
            })
        });
        vcx.run_until_parked();
        ws.update_in(vcx, |ws, window, cx| {
            assert!(!ws.outline.parsing);
            let caret = editor.read(cx).state.cursor();
            ws.jump_outline(0, window, cx);
            assert_eq!(editor.read(cx).state.cursor(), caret);
        });
        vcx.executor().advance_clock(Duration::from_millis(299));
        vcx.run_until_parked();
        assert!(!ws.read_with(vcx, |ws, _| ws.outline.parsing));
        vcx.executor().advance_clock(Duration::from_millis(2));
        vcx.run_until_parked();
        assert!(ws.read_with(vcx, |ws, _| ws.outline.parsing));
        ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
        drop(hold);
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| {
            assert!(ws.outline.symbols.is_empty());
            assert!(!ws.outline.parsing);
        });
    }

    #[gpui::test]
    fn outline_virtualizes_and_reveals_the_caret_symbol(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            set_syntax(ws, "Rust", window, cx);
            ws.active_editor().unwrap().update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.insert(&(0..200).map(|i| format!("fn item_{i}() {{}}\n")).collect::<String>());
                })
            });
            ws.toggle_outline(&ToggleOutline, window, cx);
        });
        ready(&ws, vcx);
        vcx.run_until_parked();
        assert_eq!(ws.read_with(vcx, |ws, _| ws.outline.selected), Some(199));
        assert!(vcx.debug_bounds("outline-row-199").is_some());
        assert!(vcx.debug_bounds("outline-row-0").is_none());
    }

    #[gpui::test]
    fn pending_file_jobs_cannot_replace_a_new_active_file_or_panel(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let dir = temp("orlo-related-stale").canonicalize().unwrap();
        for p in ["foo.c", "foo.h", "other.rs"] {
            std::fs::write(dir.join(p), "").unwrap();
        }
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("foo.c"), window, cx);
            ws.switch_file(&SwitchFile, window, cx);
            ws.switch_file(&SwitchFile, window, cx);
            ws.open_path(dir.join("other.rs"), window, cx);
        });
        crate::workspace::ui_tests::settle(vcx);
        ws.read_with(vcx, |ws, cx| assert_eq!(active_path(ws, cx), Some(dir.join("other.rs"))));
        ws.update_in(vcx, |ws, window, cx| {
            ws.related_files(&RelatedFiles, window, cx);
            ws.command_palette(&CommandPalette, window, cx);
        });
        crate::workspace::ui_tests::settle(vcx);
        ws.read_with(vcx, |ws, _| assert!(ws.quick_panel.as_ref().unwrap().kind == Kind::Commands));
    }

    #[gpui::test]
    fn pending_related_scan_rejects_text_and_syntax_changes(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let dir = temp("orlo-related-context").canonicalize().unwrap();
        std::fs::write(dir.join("main.rs"), "\"target.txt\"").unwrap();
        std::fs::write(dir.join("target.txt"), "").unwrap();
        for syntax in [false, true] {
            ws.update_in(vcx, |ws, window, cx| {
                ws.open_path(dir.join("main.rs"), window, cx);
                ws.related_files(&RelatedFiles, window, cx);
                if syntax {
                    set_syntax(ws, "Python", window, cx);
                } else {
                    ws.active_editor().unwrap().update(cx, |e, cx| e.edit(cx, |s| s.insert("// changed")));
                }
            });
            crate::workspace::ui_tests::settle(vcx);
            ws.read_with(vcx, |ws, _| assert!(ws.quick_panel.as_ref().is_none_or(|p| p.items.is_empty())));
        }
    }

    #[gpui::test]
    fn outline_narrow_window_preserves_editor_room(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| ws.toggle_outline(&ToggleOutline, window, cx));
        vcx.simulate_resize(gpui::size(px(450.), px(600.)));
        vcx.run_until_parked();
        ws.read_with(vcx, |ws, _| assert!(f32::from(ws.groups_bounds.get().size.width) >= docks::EDITOR_MIN_W - 1.));
    }
    #[gpui::test]
    fn r1_all_related_reasons_are_rendered_and_paths_still_filter(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let dir = temp("orlo-related-rendered-reasons").canonicalize().unwrap();
        for name in ["foo.rs", "foo.h", "foo_test.rs", "inc.h", "ref.txt"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("foo.rs"), window, cx);
            ws.active_editor().unwrap().update(cx, |e, cx| {
                e.edit(cx, |s| s.insert("#include \"inc.h\"\n\"ref.txt\"\n"));
            });
            ws.related_files(&RelatedFiles, window, cx);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.loaded && p.items.len() == 4))
        });
        for (reason, selector) in [
            ("counterpart", "qp-hint-0"),
            ("test", "qp-hint-1"),
            ("included", "qp-hint-2"),
            ("referenced", "qp-hint-3"),
        ] {
            let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("{reason} hint was not rendered"));
            assert!(f32::from(bounds.size.width) > 0.);
            ws.read_with(vcx, |ws, _| {
                assert!(ws.quick_panel.as_ref().unwrap().items.iter().any(|i| i.hint == reason));
            });
        }
        ws.update(vcx, |ws, cx| {
            ws.quick_panel.as_ref().unwrap().input.update(cx, |e, cx| e.set_text("foo_test", cx));
        });
        vcx.run_until_parked();
        ws.read_with(vcx, |ws, _| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert_eq!(p.filtered.len(), 1);
            assert!(p.items[p.filtered[0]].label.ends_with("foo_test.rs"));
        });
        vcx.simulate_keystrokes("escape");
        ws.read_with(vcx, |ws, cx| {
            assert!(ws.quick_panel.is_none());
            assert_eq!(active_path(ws, cx), Some(dir.join("foo.rs")));
        });
    }

    #[gpui::test]
    fn r2_outline_preserves_the_shared_index_edit_debounce(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let editor = ws.update_in(vcx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| e.edit(cx, |s| s.insert("old ")));
            ws.toggle_outline(&ToggleOutline, window, cx);
            editor
        });
        ready(&ws, vcx);
        let before = ws.read_with(vcx, |ws, cx| ws.index.read(cx).entry_for(&editor, cx).unwrap().key.clone());
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.insert("first ")));
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(200));
        vcx.run_until_parked();
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.insert("second ")));
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(299));
        vcx.run_until_parked();
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(
                ws.index.read(cx).entry_for(&editor, cx).unwrap().key,
                before,
                "Outline must leave the old shared entry until 300 ms after the last edit"
            );
        });
        vcx.executor().advance_clock(Duration::from_millis(1));
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, cx| {
            let entry = ws.index.read(cx).entry_for(&editor, cx).unwrap();
            assert!(matches!(entry.key, EntryKey::Buffer(_, version) if version == editor.read(cx).doc(cx).version));
            assert!(Arc::ptr_eq(&ws.outline.symbols, &entry.symbols));
        });
    }

    #[gpui::test]
    fn r3_continuous_stale_interval_resets_on_fresh_and_close(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let editor = ws.update_in(vcx, |ws, window, cx| {
            set_syntax(ws, "Rust", window, cx);
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| e.edit(cx, |s| s.insert("fn old() {}\n")));
            ws.toggle_outline(&ToggleOutline, window, cx);
            editor
        });
        ready(&ws, vcx);
        let hold = HeldWorker::new();
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.insert("// first\n")));
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(200));
        vcx.run_until_parked();
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.insert("// second\n")));
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(99));
        vcx.run_until_parked();
        assert!(!ws.read_with(vcx, |ws, _| ws.outline.parsing));
        vcx.executor().advance_clock(Duration::from_millis(2));
        vcx.run_until_parked();
        assert!(ws.read_with(vcx, |ws, _| ws.outline.parsing), "the entry has been continuously stale for 301 ms");
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.insert("// third\n")));
        vcx.run_until_parked();
        assert!(ws.read_with(vcx, |ws, _| ws.outline.parsing), "another edit must retain the stale status");
        drop(hold);
        vcx.executor().advance_clock(Duration::from_millis(300));
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| assert!(!ws.outline.parsing && ws.outline.delay.is_none()));
        let hold = HeldWorker::new();
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.insert("// fourth\n")));
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(301));
        vcx.run_until_parked();
        assert!(ws.read_with(vcx, |ws, _| ws.outline.parsing));
        ws.update_in(vcx, |ws, window, cx| {
            ws.toggle_outline(&ToggleOutline, window, cx);
            assert!(!ws.outline.parsing && ws.outline.delay.is_none());
        });
        drop(hold);
    }
}
