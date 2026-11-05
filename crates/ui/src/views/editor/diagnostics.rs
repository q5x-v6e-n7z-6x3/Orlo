//! Prepared inline winners from the existing admitted language-server diagnostics.
use super::*;
use crate::languages::language::Diagnostic;
use std::collections::BTreeMap;

pub(super) struct InlineDiagnostics {
    rope: ropey::Rope,
    version: u64,
    epoch: Option<(Arc<std::sync::atomic::AtomicU64>, u64)>,
    pub winners: Vec<InlineDiagnostic>,
}

pub(super) struct InlineDiagnostic {
    pub line: usize,
    #[cfg(test)]
    pub index: usize,
    pub severity: u64,
    pub text: SharedString,
}

impl InlineDiagnostics {
    pub fn current(&self, e: &Editor, cx: &App) -> bool {
        self.version == e.doc(cx).version
            && self.rope.is_instance(&e.doc(cx).rope)
            && self.epoch.as_ref().is_none_or(|(epoch, generation)| epoch.load(Ordering::Relaxed) == *generation)
    }
}

impl Editor {
    /// Rebuild only at diagnostic/source publication, never during paint or input.
    pub(crate) fn inline_diagnostics_current(&self, cx: &App) -> bool {
        self.inline_diagnostics.as_ref().is_some_and(|d| d.current(self, cx))
    }

    pub(crate) fn install_diagnostics(
        &mut self,
        diagnostics: Vec<Diagnostic>,
        epoch: Option<(Arc<std::sync::atomic::AtomicU64>, u64)>,
        cx: &App,
    ) {
        if self.diagnostics == diagnostics && self.inline_diagnostics.as_ref().is_some_and(|d| d.current(self, cx)) {
            return;
        }
        let doc = self.doc(cx);
        let mut lines: BTreeMap<usize, usize> = BTreeMap::new();
        for (index, diagnostic) in diagnostics.iter().enumerate() {
            if !(1..=4).contains(&diagnostic.severity)
                || diagnostic.range.end > doc.len()
                || diagnostic.range.start > diagnostic.range.end
            {
                continue;
            }
            let line = doc.rope.char_to_line(diagnostic.range.start);
            let winner = lines.entry(line).or_insert(index);
            let old = &diagnostics[*winner];
            if (diagnostic.severity, diagnostic.range.start) < (old.severity, old.range.start) {
                *winner = index;
            }
        }
        let winners = lines
            .into_iter()
            .map(|(line, index)| {
                let diagnostic = &diagnostics[index];
                // Bound shaping independently of the full detail retained by the picker.
                let mut chars = diagnostic.message.chars().map(|c| if c.is_whitespace() { ' ' } else { c });
                let mut text: String = chars.by_ref().take(512).collect();
                if chars.next().is_some() {
                    text.push('…');
                }
                InlineDiagnostic {
                    line,
                    #[cfg(test)]
                    index,
                    severity: diagnostic.severity,
                    text: text.into(),
                }
            })
            .collect();
        self.inline_diagnostics =
            Some(Rc::new(InlineDiagnostics { rope: doc.rope.clone(), version: doc.version, epoch, winners }));
        self.diagnostics = diagnostics;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    impl Editor {
        pub(crate) fn painted_inline_diagnostics(&self) -> Vec<String> {
            self.layout.as_ref().unwrap().drawn.inline_diagnostics.iter().map(|d| d.2.clone()).collect()
        }
    }

    fn redraw(
        e: &Entity<Editor>,
        cx: &mut VisualTestContext,
        change: impl FnOnce(&mut Editor, &mut Context<Editor>) + 'static,
    ) {
        e.update(cx, |e, cx| {
            change(e, cx);
            cx.notify();
        });
        cx.run_until_parked();
    }

    fn diagnostic(start: usize, severity: u64, message: &str) -> Diagnostic {
        Diagnostic { range: start..start + 1, severity, message: message.into(), raw: serde_json::Value::Null }
    }

    fn install(e: &mut Editor, cx: &mut Context<Editor>, diagnostics: Vec<Diagnostic>) {
        let path = PathBuf::from("/tmp/orlo-inline.rs");
        e.update_doc(cx, |d| d.path = Some(path.clone()));
        e.language_version = Some((path, e.doc(cx).version));
        e.install_diagnostics(diagnostics, None, cx);
    }

    fn messages(e: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<(usize, usize, String)> {
        e.read_with(cx, |e, _| {
            e.layout
                .as_ref()
                .unwrap()
                .drawn
                .inline_diagnostics
                .iter()
                .map(|(line, index, text, _)| (*line, *index, text.clone()))
                .collect()
        })
    }

    #[gpui::test]
    fn inline_diagnostics_paint_priority_source_order_and_threshold(cx: &mut TestAppContext) {
        let (e, cx) = test_editor("one\ntwo\nthree", cx, |e, cx| {
            e.view.inline_diagnostics = true;
            install(
                e,
                cx,
                vec![
                    diagnostic(0, 2, "warning"),
                    diagnostic(2, 1, "later error"),
                    diagnostic(1, 1, "first error"),
                    diagnostic(1, 1, "tie loses"),
                    diagnostic(4, 2, "second warning"),
                    diagnostic(8, 3, "information"),
                ],
            );
        });
        assert_eq!(messages(&e, cx), [(0, 2, "first error".into()), (1, 4, "second warning".into())]);
        redraw(&e, cx, |e, _| e.view.inline_diagnostics_severity = crate::config::settings::DiagnosticSeverity::Error);
        assert_eq!(messages(&e, cx), [(0, 2, "first error".into())]);
        redraw(&e, cx, |e, _| e.view.inline_diagnostics_severity = crate::config::settings::DiagnosticSeverity::Hint);
        assert_eq!(messages(&e, cx).len(), 3);
        redraw(&e, cx, |e, _| e.view.inline_diagnostics = false);
        assert!(messages(&e, cx).is_empty());
    }
    #[gpui::test]
    fn inline_diagnostics_elide_without_changing_source_coordinates_copy_save_or_other_marks(cx: &mut TestAppContext) {
        use crate::editing::text::FindOpts;
        use gpui::EntityInputHandler;
        let original = "😀 café café\nnext";
        let full = "long diagnostic é😀\n".repeat(200);
        let expected_full = full.clone();
        let (e, cx) = test_editor(original, cx, move |e, cx| {
            e.view.selection_occurrences = true;
            e.sel(cx).set_selection(2, 6);
            e.find_ranges = std::iter::once(7..11).collect();
            e.read_state(cx).remember_scoped_found("café", &FindOpts::default(), None, vec![2..6, 7..11]);
            for ranges in &mut e.state.highlights {
                *ranges = std::iter::once(7..11).collect();
            }
            install(e, cx, vec![diagnostic(0, 1, &full)]);
        });
        assert!(messages(&e, cx).is_empty(), "default is off");
        let geometry = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, cx| {
                let l = e.layout.as_ref().unwrap();
                (
                    e.display_rows(cx),
                    e.state.selections.clone(),
                    e.doc(cx).version,
                    l.rows
                        .iter()
                        .map(|r| r.as_ref().map(|(col, s)| (*col, s.text.to_string(), s.width)))
                        .collect::<Vec<_>>(),
                    l.drawn.carets.clone(),
                    l.drawn.selected_bounds.clone(),
                    e.state.highlights.clone(),
                    l.drawn.occurrences.clone(),
                )
            })
        };
        let native = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.update_in(cx, |e, w, cx| {
                let bounds = e.bounds_for_range(3..7, gpui::Bounds::default(), w, cx);
                let hit = bounds.and_then(|b| e.character_index_for_point(b.origin, w, cx));
                (
                    bounds,
                    hit,
                    e.selected_text_range(false, w, cx).map(|s| s.range),
                    e.text_for_range(0..100, &mut None, w, cx),
                )
            })
        };
        let before = geometry(&e, cx);
        let before_native = native(&e, cx);
        redraw(&e, cx, |e, _| e.view.inline_diagnostics = true);
        assert_eq!(geometry(&e, cx), before);
        assert_eq!(native(&e, cx), before_native);
        e.read_with(cx, |e, cx| {
            let l = e.layout.as_ref().unwrap();
            let (_, _, shown, b) = &l.drawn.inline_diagnostics[0];
            assert!(shown.ends_with('…') && !shown.contains('\n') && shown.chars().count() < 512);
            assert!(b.left() > l.text_left && b.right() <= l.text_area.right());
            assert_eq!(e.diagnostics[0].message, expected_full);
            assert_eq!(e.inline_diagnostics.as_ref().unwrap().winners[0].text.chars().count(), 513);
            assert_eq!(e.text(cx), original);
            assert_eq!(e.find_ranges.as_slice(), std::slice::from_ref(&(7..11)));
            assert_eq!(e.read_state(cx).found("café", &FindOpts::default()), Some(vec![2..6, 7..11]));
        });
        run(&e, cx, &Copy);
        assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some("café"));
        let path = crate::views::workspace::ui_tests::temp("orlo-inline-save").join("source.rs");
        e.update(cx, |e, cx| e.update_doc(cx, |d| d.save_to(path.clone()).unwrap()));
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }

    #[gpui::test]
    fn inline_diagnostics_wrap_fold_and_filler_rows_keep_geometry(cx: &mut TestAppContext) {
        let source = "abc def ghi jkl mno pqr stu\nhidden\nlast";
        let (e, cx) = test_editor(source, cx, |e, cx| {
            e.wrap_override = Some(true);
            e.wrap_width_override = Some(12);
            e.view.minimap = false;
            let (hidden, last) = (e.doc(cx).line_start(1), e.doc(cx).line_start(2));
            e.fold_chars_now(std::iter::once(hidden..last), cx);
            e.fillers = Fillers::new(e.doc(cx), [(2, 2)]);
            install(
                e,
                cx,
                vec![diagnostic(0, 1, "wrapped"), diagnostic(hidden, 1, "hidden"), diagnostic(last, 1, "last")],
            );
        });
        let before = e.read_with(cx, |e, cx| {
            (
                e.display_rows(cx),
                e.folds.clone(),
                e.fillers.clone(),
                e.layout.as_ref().unwrap().drawn.carets.clone(),
                e.state.selections.clone(),
            )
        });
        redraw(&e, cx, |e, _| e.view.inline_diagnostics = true);
        assert_eq!(messages(&e, cx), [(0, 0, "wrapped".into()), (2, 2, "last".into())]);
        e.read_with(cx, |e, cx| {
            let l = e.layout.as_ref().unwrap();
            assert_eq!(
                (
                    e.display_rows(cx),
                    e.folds.clone(),
                    e.fillers.clone(),
                    l.drawn.carets.clone(),
                    e.state.selections.clone()
                ),
                before
            );
            let wrapped_y = l.drawn.inline_diagnostics[0].3.top();
            assert!(wrapped_y > l.bounds.top(), "only the final wrapped row gets the message");
            assert_eq!(e.text(cx), source);
        });
    }

    #[gpui::test]
    fn inline_diagnostics_offscreen_left_end_hides_and_returns_with_unchanged_geometry(cx: &mut TestAppContext) {
        let source = format!("one\n{}", "x".repeat(2_000));
        let (e, cx) = test_editor(&source, cx, |e, cx| {
            e.view.inline_diagnostics = true;
            e.view.minimap = false;
            e.wrap_override = Some(false);
            install(e, cx, vec![diagnostic(0, 1, "short diagnostic")]);
        });
        assert_eq!(messages(&e, cx), [(0, 0, "short diagnostic".into())]);
        let cache = e.read_with(cx, |e, _| e.inline_diagnostics.clone().unwrap());
        let before = e.read_with(cx, |e, cx| {
            (
                e.text(cx).to_owned(),
                e.doc(cx).version,
                e.display_rows(cx),
                e.state.selections.clone(),
                e.layout.as_ref().unwrap().drawn.carets.clone(),
            )
        });
        redraw(&e, cx, |e, _| {
            let sw = e.layout.as_ref().unwrap().space_w;
            e.scroll_left = f32::from(sw) * 40.;
            e.follow_caret = false;
        });
        assert!(messages(&e, cx).is_empty(), "an offscreen-left physical end cannot pin its message to the viewport");
        let far_carets = e.read_with(cx, |e, _| e.layout.as_ref().unwrap().drawn.carets.clone());
        redraw(&e, cx, |e, _| e.view.inline_diagnostics = false);
        e.read_with(cx, |e, _| assert_eq!(e.layout.as_ref().unwrap().drawn.carets, far_carets));
        redraw(&e, cx, |e, _| {
            e.view.inline_diagnostics = true;
            e.scroll_left = 0.;
            e.follow_caret = false;
        });
        assert_eq!(messages(&e, cx), [(0, 0, "short diagnostic".into())]);
        e.read_with(cx, |e, cx| {
            assert!(Rc::ptr_eq(&cache, e.inline_diagnostics.as_ref().unwrap()));
            assert_eq!(
                (
                    e.text(cx).to_owned(),
                    e.doc(cx).version,
                    e.display_rows(cx),
                    e.state.selections.clone(),
                    e.layout.as_ref().unwrap().drawn.carets.clone()
                ),
                before
            );
        });
    }

    #[gpui::test]
    fn inline_diagnostics_long_source_scroll_uses_visible_tail_and_reuses_winners(cx: &mut TestAppContext) {
        let source = "x".repeat(100_000);
        let (e, cx) = test_editor(&source, cx, |e, cx| {
            e.view.inline_diagnostics = true;
            e.view.minimap = false;
            install(e, cx, vec![diagnostic(0, 1, "trailing diagnostic é😀".repeat(40).as_str())]);
        });
        assert!(messages(&e, cx).is_empty(), "offscreen source tail has no trailing width");
        let cache = e.read_with(cx, |e, _| e.inline_diagnostics.clone().unwrap());
        redraw(&e, cx, |e, _| {
            let sw = e.layout.as_ref().unwrap().space_w;
            e.scroll_left = f32::from(sw) * 99_980.;
            e.follow_caret = false;
        });
        assert_eq!(messages(&e, cx).len(), 1);
        e.read_with(cx, |e, cx| {
            let l = e.layout.as_ref().unwrap();
            let (_, _, text, b) = &l.drawn.inline_diagnostics[0];
            assert!(text.ends_with('…'));
            assert!(b.left() >= l.text_area.left() && b.right() <= l.text_area.right());
            assert!(Rc::ptr_eq(&cache, e.inline_diagnostics.as_ref().unwrap()));
            assert_eq!((e.display_rows(cx), e.doc(cx).version), (1, 0));
        });
        redraw(&e, cx, |e, _| {
            e.scroll_left = 0.;
            e.follow_caret = false;
        });
        assert!(messages(&e, cx).is_empty());
    }

    #[gpui::test]
    fn inline_diagnostics_shared_edits_and_replacement_expire_until_new_publication(cx: &mut TestAppContext) {
        let (e, cx) = test_editor("one\ntwo", cx, |e, cx| {
            e.view.inline_diagnostics = true;
            install(e, cx, vec![diagnostic(0, 1, "old")]);
        });
        let peer = e.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
        peer.update(cx, |peer, _| peer.view.inline_diagnostics = false);
        e.read_with(cx, |e, cx| {
            let peer = peer.read(cx);
            assert!(e.view.inline_diagnostics && !peer.view.inline_diagnostics);
            assert!(Rc::ptr_eq(e.inline_diagnostics.as_ref().unwrap(), peer.inline_diagnostics.as_ref().unwrap()));
            assert_eq!(e.display_rows(cx), peer.display_rows(cx));
            assert_eq!(e.state.selections, peer.state.selections);
        });
        peer.read_with(cx, |peer, cx| {
            assert!(peer.language_current(cx));
            assert!(peer.inline_diagnostics.as_ref().unwrap().current(peer, cx));
        });
        peer.update(cx, |e, cx| e.edit(cx, |s| s.insert("changed")));
        cx.run_until_parked();
        assert!(messages(&e, cx).is_empty());
        redraw(&e, cx, |e, cx| install(e, cx, vec![diagnostic(0, 1, "fresh")]));
        assert_eq!(messages(&e, cx), [(0, 0, "fresh".into())]);
        let cache = e.read_with(cx, |e, _| e.inline_diagnostics.clone().unwrap());
        redraw(&e, cx, |e, cx| {
            let reports = e.diagnostics.clone();
            e.install_diagnostics(reports, None, cx);
        });
        e.read_with(cx, |e, _| assert!(Rc::ptr_eq(&cache, e.inline_diagnostics.as_ref().unwrap())));
        redraw(&e, cx, |e, cx| e.set_text("replacement", cx));
        assert!(messages(&e, cx).is_empty());
        redraw(&e, cx, |e, cx| install(e, cx, vec![diagnostic(0, 1, "current")]));
        assert_eq!(messages(&e, cx).len(), 1);
        redraw(&e, cx, |e, _| e.language_version = None);
        assert!(messages(&e, cx).is_empty(), "removed route hides admitted winners");
    }

    #[gpui::test]
    fn inline_diagnostics_native_ime_and_protected_views_preserve_input_geometry(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (e, cx) = test_editor("😀 café", cx, |e, cx| {
            e.update_doc(cx, |d| d.read_only = true);
            e.document.update(cx, |d, cx| d.set_viewer_mode(true, cx));
            e.input_enabled = false;
            e.view.inline_diagnostics = true;
            install(e, cx, vec![diagnostic(0, 1, "protected")]);
        });
        assert_eq!(messages(&e, cx).len(), 1, "protected views can read diagnostics");
        e.update(cx, |e, cx| {
            e.update_doc(cx, |d| d.read_only = false);
            e.document.update(cx, |d, cx| d.set_viewer_mode(false, cx));
            e.input_enabled = true;
        });
        e.update_in(cx, |e, w, cx| {
            e.replace_and_mark_text_in_range(Some(3..7), "漢字", Some(1..1), w, cx);
            install(e, cx, vec![diagnostic(0, 1, "composing")]);
        });
        cx.run_until_parked();
        let native = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.update_in(cx, |e, w, cx| {
                (
                    e.marked_text_range(w, cx),
                    e.selected_text_range(false, w, cx).map(|s| s.range),
                    e.bounds_for_range(3..5, gpui::Bounds::default(), w, cx),
                    e.text(cx),
                )
            })
        };
        let before = native(&e, cx);
        assert!(before.0.is_some() && before.2.is_some());
        assert!(messages(&e, cx).is_empty());
        redraw(&e, cx, |e, _| e.view.inline_diagnostics = false);
        assert_eq!(native(&e, cx), before);
    }

    #[gpui::test]
    fn inline_diagnostics_reload_and_invalid_ranges_do_not_reuse_stale_winners(cx: &mut TestAppContext) {
        let (e, cx) = test_editor("old text", cx, |e, cx| {
            e.view.inline_diagnostics = true;
            install(e, cx, vec![diagnostic(0, 1, "old"), diagnostic(40, 1, "invalid")]);
        });
        assert_eq!(messages(&e, cx), [(0, 0, "old".into())]);
        let path = crate::views::workspace::ui_tests::temp("orlo-inline-reload").join("source.rs");
        std::fs::write(&path, "new text").unwrap();
        redraw(&e, cx, move |e, cx| {
            let loaded = Loaded::read(&path).unwrap();
            let changes = loaded.diff(&e.doc(cx).rope);
            e.reload(loaded, &changes, cx);
        });
        assert!(messages(&e, cx).is_empty());
        assert_eq!(e.read_with(cx, |e, cx| e.text(cx)), "new text");
        redraw(&e, cx, |e, cx| install(e, cx, vec![diagnostic(99, 1, "invalid"), diagnostic(0, 0, "unknown")]));
        assert!(messages(&e, cx).is_empty());
    }

    #[gpui::test]
    fn inline_diagnostics_existing_server_epoch_hides_old_and_rebinds_identical_reports(cx: &mut TestAppContext) {
        let (e, cx) = test_editor("one", cx, |e, cx| {
            e.view.inline_diagnostics = true;
            install(e, cx, vec![diagnostic(0, 1, "current")]);
        });
        let epoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let bound = epoch.clone();
        redraw(&e, cx, move |e, cx| {
            e.inline_diagnostics = None;
            e.install_diagnostics(e.diagnostics.clone(), Some((bound, 0)), cx);
        });
        let old = e.read_with(cx, |e, _| e.inline_diagnostics.clone().unwrap());
        assert_eq!(messages(&e, cx).len(), 1);
        epoch.fetch_add(1, Ordering::Relaxed);
        redraw(&e, cx, |_, _| {});
        assert!(messages(&e, cx).is_empty(), "restart/settings invalidate before replacement transport work");
        redraw(&e, cx, move |e, cx| e.install_diagnostics(e.diagnostics.clone(), Some((epoch, 1)), cx));
        assert_eq!(messages(&e, cx).len(), 1);
        e.read_with(cx, |e, _| assert!(!Rc::ptr_eq(&old, e.inline_diagnostics.as_ref().unwrap())));
    }

    #[gpui::test]
    fn inline_diagnostics_overlay_keeps_trailing_click_and_virtual_drag_semantics(cx: &mut TestAppContext) {
        use gpui::{point, Modifiers, MouseButton};
        let (e, cx) = focused_editor("one|", None, cx);
        redraw(&e, cx, |e, cx| {
            e.set_virtual_space(true, cx);
            install(e, cx, vec![diagnostic(0, 1, "trailing message")]);
        });
        let at = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            point(l.text_left + l.space_w * 8., l.bounds.top() + px(e.line_height() / 2.))
        });
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
        let expected = e.read_with(cx, |e, _| e.state.selections.clone());
        assert_eq!(expected, [Selection::from_points((3, 5), (3, 5))]);
        redraw(&e, cx, |e, cx| {
            e.sel(cx).set_cursor(3);
            e.view.inline_diagnostics = true;
        });
        assert_eq!(messages(&e, cx).len(), 1);
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
        e.read_with(cx, |e, cx| {
            assert_eq!(e.state.selections, expected);
            assert_eq!(e.text(cx), "one");
            let l = e.layout.as_ref().unwrap();
            assert!(l.drawn.inline_diagnostics[0].3.left() > l.drawn.carets[0].right());
        });
        let to = point(at.x + px(24.), at.y);
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        let enabled = e.read_with(cx, |e, _| e.state.selections.clone());
        redraw(&e, cx, |e, _| e.view.inline_diagnostics = false);
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        e.read_with(cx, |e, cx| {
            assert_eq!(e.state.selections, enabled);
            assert_eq!(e.text(cx), "one");
        });
    }
}
