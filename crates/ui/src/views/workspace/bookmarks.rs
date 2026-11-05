//! The all-window bookmark picker derives rows from live view-owned marks.

use super::*;
use crate::overlays::quick_panel::BookmarkTarget;
use std::time::Duration;

fn selected(panel: &QuickPanel) -> Option<BookmarkTarget> {
    match panel.items.get(*panel.filtered.get(panel.selected)?)?.action {
        ItemAction::Bookmark(target) => Some(target),
        _ => None,
    }
}

impl Workspace {
    pub(crate) fn goto_bookmark(&mut self, _: &GotoBookmark, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.bookmark_items(cx);
        self.open_quick_panel(Kind::Bookmarks, items, window, cx);
        let panel = self.quick_panel.as_ref().unwrap().input.entity_id();
        // Other windows can edit marks or close tabs without notifying this workspace.
        self.quick_panel.as_mut().unwrap().rows_task = Some(cx.spawn_in(window, async move |ws, cx| loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            if !ws
                .update(cx, |ws, cx| {
                    if !ws
                        .quick_panel
                        .as_ref()
                        .is_some_and(|p| p.input.entity_id() == panel && p.kind == Kind::Bookmarks)
                    {
                        return false;
                    }
                    ws.refresh_bookmark_panel(cx);
                    true
                })
                .unwrap_or(false)
            {
                break;
            }
        }));
    }

    fn bookmark_items(&self, cx: &mut Context<Self>) -> Vec<quick_panel::Item> {
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        for view in self.open_editors(cx) {
            view.update(cx, |e, cx| e.catch_up(cx));
            let e = view.read(cx);
            let doc = e.doc(cx);
            let name = doc
                .path
                .as_deref()
                .and_then(Path::file_name)
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".into());
            for line in e.read_state(cx).bookmarked_lines() {
                if !seen.insert((doc.id(), line)) {
                    continue;
                }
                let target = BookmarkTarget { view: view.entity_id(), document: doc.id(), version: doc.version, line };
                let mut chars = doc.rope.line(line).chars().take_while(|c| !matches!(c, '\r' | '\n'));
                let mut preview: String = chars.by_ref().take(160).collect();
                if chars.next().is_some() {
                    preview.push('…');
                }
                items.push(quick_panel::Item::new(
                    format!("{name}:{}", line + 1),
                    preview,
                    ItemAction::Bookmark(target),
                ));
            }
        }
        items
    }

    pub(crate) fn refresh_bookmark_panel(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.quick_panel.as_ref().filter(|p| p.kind == Kind::Bookmarks) else { return };
        let query = p.input.read(cx).text(cx);
        let old = selected(p);
        let query_changed = p.query != query;
        let items = self.bookmark_items(cx);
        let p = self.quick_panel.as_mut().unwrap();
        let changed = items.len() != p.items.len()
            || items.iter().zip(&p.items).any(|(a, b)| {
                a.label != b.label
                    || a.detail != b.detail
                    || match (&a.action, &b.action) {
                        (ItemAction::Bookmark(a), ItemAction::Bookmark(b)) => a != b,
                        _ => true,
                    }
            });
        if !changed && !query_changed {
            return;
        }
        p.set_items(Kind::Bookmarks, items);
        p.refilter(&query);
        if !query_changed {
            p.selected = old
                .and_then(|old| {
                    p.filtered.iter().position(|&i| match p.items[i].action {
                        ItemAction::Bookmark(t) => t.document == old.document && t.line == old.line,
                        _ => false,
                    })
                })
                .unwrap_or(0);
        }
        cx.notify();
    }

    pub(crate) fn confirm_bookmark_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.quick_panel.as_ref() else { return };
        let query_changed = p.query != p.input.read(cx).text(cx);
        let old = selected(p);
        self.refresh_bookmark_panel(cx);
        let fresh = self.quick_panel.as_ref().and_then(selected);
        let target = match (old, fresh) {
            (Some(old), Some(fresh))
                if !query_changed
                    && old.document == fresh.document
                    && old.version == fresh.version
                    && old.line == fresh.line =>
            {
                fresh
            }
            _ => return self.status_message("Bookmarks changed; choose a current bookmark", cx),
        };
        let Some(view) = self.open_editors(cx).into_iter().find(|v| v.entity_id() == target.view) else { return };
        if let Err(error) = self.selection_set_admission(&view, cx) {
            return self.status_message(error, cx);
        }
        self.quick_panel = None;
        // Keep this window usable if deferred remote delivery is rejected.
        self.focus_editor(window, cx);
        self.activate_bookmark(target, window, cx);
        cx.notify();
    }

    fn activate_bookmark(&mut self, target: BookmarkTarget, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let local = self.views.tabs().find(|v| v.entity_id() == target.view).cloned();
        if let Some(view) = local {
            if let Err(error) = self.selection_set_admission(&view, cx) {
                self.status_message(error, cx);
                return false;
            }
            if view.update(cx, |e, cx| e.goto_bookmark(target.document, target.version, target.line, cx)) {
                self.activate_view(target.view, window, cx);
                return true;
            }
            self.status_message("Bookmark is no longer current", cx);
            return false;
        }
        let owner = cx
            .windows()
            .into_iter()
            .filter(|w| *w != self.window)
            .filter_map(|w| w.downcast::<Workspace>())
            .find(|w| w.read(cx).is_ok_and(|ws| ws.views.tabs().any(|v| v.entity_id() == target.view)));
        if let Some(owner) = owner {
            cx.defer(move |cx| {
                owner
                    .update(cx, |ws, window, cx| {
                        // Revalidate inside the owning window; a close between events must not reopen it.
                        if ws.views.tabs().any(|v| v.entity_id() == target.view)
                            && ws.activate_bookmark(target, window, cx)
                        {
                            window.activate_window();
                        }
                    })
                    .ok();
            });
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{new_view, two_windows, type_in, window};
    use gpui::{EntityInputHandler, TestAppContext};

    fn query(ws: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) {
        ws.quick_panel.as_ref().unwrap().input.update(cx, |e, cx| e.set_text(text, cx));
        ws.quick_panel_input_changed(window, cx);
    }

    #[gpui::test]
    fn n20_stale_source_confirmation_refreshes_mapped_lines_without_jumping(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "top\n雪 target\nend");
        ws.update_in(cx, |ws, w, cx| {
            let view = ws.active_editor().unwrap();
            view.update(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
            ws.goto_bookmark(&GotoBookmark, w, cx);
            view.update(cx, |e, cx| e.edit(cx, |s| s.insert("new\n")));
            let before = view.read(cx).state.cursor();
            ws.confirm_quick_panel(w, cx);
            assert!(ws.quick_panel.is_some());
            assert_eq!(view.read(cx).state.cursor(), before, "old version must never jump");
            assert!(ws.quick_panel.as_ref().unwrap().items[0].label.ends_with(":3"));
            ws.confirm_quick_panel(w, cx);
            assert!(ws.quick_panel.is_none());
            assert_eq!(view.read(cx).state.cursor(), 8);
            view.update(cx, |e, cx| e.goto_line(1, cx));
            ws.goto_bookmark(&GotoBookmark, w, cx);
            view.update(cx, |e, cx| {
                e.edit(cx, |s| s.delete_bookmarked_lines(false));
            });
            ws.confirm_quick_panel(w, cx);
            assert_eq!(ws.quick_panel.as_ref().unwrap().items.len(), 0);
            assert_eq!(view.read(cx).state.cursor(), 8, "deleted row must not jump to its neighbour");
        });
    }

    #[gpui::test]
    fn n20_closing_a_bookmarked_owner_falls_back_then_removes_all_dead_rows(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "top\ntarget");
        ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
        });
        new_view(&ws, cx);
        let dead = ws.update_in(cx, |ws, w, cx| {
            ws.goto_bookmark(&GotoBookmark, w, cx);
            let first = ws.views.tabs().next().unwrap().clone();
            let dead = first.downgrade();
            ws.remove_view(&first, cx);
            ws.confirm_quick_panel(w, cx);
            assert!(ws.quick_panel.is_none(), "a surviving bookmarked view owns the same row");
            let view = ws.active_editor().unwrap();
            assert_eq!(view.read(cx).state.cursor(), 4);
            ws.goto_bookmark(&GotoBookmark, w, cx);
            ws.remove_view(&view, cx);
            ws.confirm_quick_panel(w, cx);
            assert_eq!(ws.quick_panel.as_ref().unwrap().items.len(), 0);
            assert_eq!(ws.views.tabs().count(), 0, "no disk reopen or replacement tab");
            dead
        });
        cx.run_until_parked();
        assert!(dead.upgrade().is_none(), "picker rows must not retain closed editors");
    }

    #[gpui::test]
    fn n20_query_changes_and_bookmark_removal_cannot_confirm_old_rows(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "top\nold target\nnew target");
        ws.update_in(cx, |ws, w, cx| {
            let view = ws.active_editor().unwrap();
            view.update(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(3, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
            ws.goto_bookmark(&GotoBookmark, w, cx);
            query(ws, "old", w, cx);
            ws.quick_panel.as_ref().unwrap().input.update(cx, |e, cx| e.set_text("new", cx));
            ws.confirm_quick_panel(w, cx);
            assert!(ws.quick_panel.is_some(), "pending query event must invalidate Enter");
            assert_eq!(view.read(cx).state.cursor(), 0);
            ws.confirm_quick_panel(w, cx);
            assert_eq!(view.read(cx).state.cursor(), 15);
            ws.goto_bookmark(&GotoBookmark, w, cx);
            query(ws, "new", w, cx);
            view.update(cx, |e, cx| {
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
            ws.confirm_quick_panel(w, cx);
            assert!(ws.quick_panel.is_some());
            assert_eq!(view.read(cx).state.cursor(), 0);
            assert!(ws.quick_panel.as_ref().unwrap().filtered.is_empty());
        });
    }

    #[gpui::test]
    fn n20_other_window_untitled_unicode_target_uses_its_existing_owner(cx: &mut TestAppContext) {
        let (ws1, ws2, mut c1, mut c2) = two_windows(cx);
        type_in(&ws1, &mut c1, "local");
        type_in(&ws2, &mut c2, "remote\n雪😀 café");
        let target = ws2.update(&mut c2, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
            e
        });
        ws1.update_in(&mut c1, |ws, w, cx| {
            ws.goto_bookmark(&GotoBookmark, w, cx);
            query(ws, "雪😀", w, cx);
            assert_eq!(ws.quick_panel.as_ref().unwrap().filtered.len(), 1);
            assert_eq!(target.read(cx).state.cursor(), 0, "highlighting does not preview/move source");
            ws.confirm_quick_panel(w, cx);
            assert_eq!(ws.views.tabs().count(), 1, "no cloned view in originating window");
        });
        c1.run_until_parked();
        ws2.read_with(&c2, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap(), target);
            assert_eq!(target.read(cx).state.cursor(), 7);
            assert_eq!(ws.views.tabs().count(), 1);
        });
    }

    #[gpui::test]
    fn n20_deferred_other_window_confirmation_revalidates_closed_owner(cx: &mut TestAppContext) {
        let (ws1, ws2, mut c1, mut c2) = two_windows(cx);
        type_in(&ws2, &mut c2, "top\ntarget");
        let target = ws2.update(&mut c2, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
            e
        });
        ws1.update_in(&mut c1, |ws, w, cx| {
            ws.goto_bookmark(&GotoBookmark, w, cx);
            ws.confirm_quick_panel(w, cx);
            ws2.update(cx, |other, cx| other.remove_view(&target, cx));
        });
        c1.run_until_parked();
        assert_eq!(target.read_with(&c2, |e, _| e.state.cursor()), 0);
        assert_eq!(ws2.read_with(&c2, |ws, _| ws.views.tabs().count()), 0);
    }

    #[gpui::test]
    fn n20_navigation_reveals_physical_source_through_filters_and_preserves_viewer_policies(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "top\n  hidden 雪😀\ntail");
        for mode in ["ordinary", "readonly", "viewer", "follow"] {
            ws.update_in(cx, |ws, w, cx| {
                let view = ws.active_editor().unwrap();
                let before = view.update(cx, |e, cx| {
                    e.input_enabled = mode != "viewer";
                    e.update_doc(cx, |d| {
                        d.read_only = mode == "readonly";
                        d.set_following(mode == "follow");
                    });
                    e.goto_line(2, cx);
                    e.sel(cx).clear_bookmarks();
                    e.sel(cx).toggle_bookmark();
                    e.select_range(0..3, cx);
                    e.perform_action(&editor::NarrowToSelection, w, cx);
                    (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved())
                });
                ws.goto_bookmark(&GotoBookmark, w, cx);
                ws.confirm_quick_panel(w, cx);
                view.read_with(cx, |e, cx| {
                    assert_eq!(e.state.cursor(), 4, "physical source destination: {mode}");
                    assert!(e.folds().is_empty());
                    assert_eq!((e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved()), before);
                });
            });
        }
    }

    #[gpui::test]
    fn n20_shared_and_local_native_composition_refuse_navigation_without_changing_marks(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "top\n  target\ntail");
        ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
        });
        new_view(&ws, cx);
        let views: Vec<_> = ws.read_with(cx, |ws, _| ws.views.tabs().cloned().collect());
        for composing in &views {
            composing.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(Some(0..3), "top", Some(0..1), w, cx));
            ws.update_in(cx, |ws, w, cx| {
                ws.goto_bookmark(&GotoBookmark, w, cx);
                let before: Vec<_> = views
                    .iter()
                    .map(|v| {
                        let e = v.read(cx);
                        (e.state.cursor(), e.marked.clone(), e.state.bookmarks.clone(), e.text(cx))
                    })
                    .collect();
                ws.confirm_quick_panel(w, cx);
                assert!(ws.quick_panel.is_some());
                let after: Vec<_> = views
                    .iter()
                    .map(|v| {
                        let e = v.read(cx);
                        (e.state.cursor(), e.marked.clone(), e.state.bookmarks.clone(), e.text(cx))
                    })
                    .collect();
                assert_eq!(after, before, "composition admission must precede Editor borrow");
                ws.quick_panel_dismiss(&quick_panel::Dismiss, w, cx);
            });
            composing.update_in(cx, |e, w, cx| e.unmark_text(w, cx));
        }
    }

    #[gpui::test]
    fn n20_bounded_preview_and_native_menu_palette_custom_binding_are_reachable(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, &("雪😀".repeat(500) + "\nend"));
        ws.update(cx, |ws, cx| {
            ws.with_active(cx, |e, cx| {
                e.goto_line(1, cx);
                e.sel(cx).toggle_bookmark();
            });
        });
        let items = quick_panel::command_items();
        assert!(items.iter().any(|i| matches!(&i.action, ItemAction::Dispatch(a) if a.as_any().is::<GotoBookmark>())));
        std::fs::write(
            crate::config::customization::keymap_path(),
            r#"[{"keys":["cmd-alt-b"],"command":"workspace::GotoBookmark"}]"#,
        )
        .unwrap();
        ws.update_in(cx, |ws, w, cx| {
            ws.reload_customizations(&ReloadCustomizations, w, cx);
            ws.focus_editor(w, cx);
        });
        cx.simulate_keystrokes("cmd-alt-b");
        ws.update_in(cx, |ws, w, cx| {
            let p = ws.quick_panel.as_ref().expect("custom native binding opens picker");
            assert_eq!(p.items[0].detail.chars().count(), 161);
            assert!(p.items[0].detail.ends_with('…'));
            assert!(p.input.read(cx).is_focused(w));
            ws.confirm_quick_panel(w, cx);
            assert!(ws.quick_panel.is_none());
            let goto = ws.menus(cx).into_iter().find(|m| m.name == "Goto").unwrap();
            let bookmarks = goto
                .items
                .into_iter()
                .find_map(|i| match i {
                    MenuItem::Submenu(m) if m.name == "Bookmarks" => Some(m),
                    _ => None,
                })
                .unwrap();
            let action = bookmarks
                .items
                .into_iter()
                .find_map(|i| match i {
                    MenuItem::Action { action, .. } if action.as_any().is::<GotoBookmark>() => Some(action),
                    _ => None,
                })
                .expect("native Goto menu exposes bookmark picker");
            w.dispatch_action(action, cx);
        });
        assert!(ws.read_with(cx, |ws, _| ws.quick_panel.is_some()));
    }

    #[gpui::test]
    fn n20_live_timer_refreshes_bookmark_and_preview_changes_without_a_query_event(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "top\nold target");
        ws.update_in(cx, |ws, w, cx| {
            ws.with_active(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
            });
            ws.goto_bookmark(&GotoBookmark, w, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("雪 ")));
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(101));
        cx.run_until_parked();
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.quick_panel.as_ref().unwrap().items[0].detail, "雪 old target");
            ws.with_active(cx, |e, cx| e.sel(cx).clear_bookmarks());
        });
        cx.executor().advance_clock(Duration::from_millis(101));
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().unwrap().items.is_empty()));
    }

    #[gpui::test]
    fn n20_shared_document_across_windows_keeps_unique_view_owned_marks(cx: &mut TestAppContext) {
        let (ws1, ws2, mut c1, mut c2) = two_windows(cx);
        let file = crate::workspace::ui_tests::temp("n20-shared-windows").canonicalize().unwrap().join("shared.txt");
        std::fs::write(&file, "top\ncommon\nonly second").unwrap();
        ws1.update_in(&mut c1, |ws, w, cx| {
            ws.open_path(file.clone(), w, cx);
            ws.with_active(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
            });
        });
        ws2.update_in(&mut c2, |ws, w, cx| {
            ws.open_path(file, w, cx);
            ws.with_active(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(3, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
        });
        ws1.update_in(&mut c1, |ws, w, cx| {
            ws.goto_bookmark(&GotoBookmark, w, cx);
            assert_eq!(ws.quick_panel.as_ref().unwrap().items.len(), 2);
            query(ws, "only second", w, cx);
            assert_eq!(ws.quick_panel.as_ref().unwrap().filtered.len(), 1);
            ws.confirm_quick_panel(w, cx);
        });
        c1.run_until_parked();
        ws2.read_with(&c2, |ws, cx| {
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).state.cursor(), 11);
            assert_eq!(e.read(cx).read_state(cx).bookmarked_lines(), [1, 2]);
        });
        ws1.read_with(&c1, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).read_state(cx).bookmarked_lines(), [1])
        });
    }

    #[gpui::test]
    fn n20_source_redo_and_ordinary_folds_survive_bookmark_navigation(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "head\n  body\ntail");
        ws.update_in(cx, |ws, w, cx| {
            let view = ws.active_editor().unwrap();
            let before = view.update(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                let end = e.doc(cx).len();
                e.edit(cx, |s| s.replace_ranges(vec![(end..end, "!".into())]));
                e.perform_action(&editor::Undo, w, cx);
                e.goto_line(1, cx);
                e.perform_action(&editor::Fold, w, cx);
                assert!(!e.folds().is_empty());
                (e.text(cx), e.doc(cx).version, e.doc(cx).save_point())
            });
            ws.goto_bookmark(&GotoBookmark, w, cx);
            ws.confirm_quick_panel(w, cx);
            view.update(cx, |e, cx| {
                assert_eq!(e.state.cursor(), 5);
                assert!(e.folds().is_empty(), "only the destination's fold opens");
                assert_eq!((e.text(cx), e.doc(cx).version, e.doc(cx).save_point()), before);
                e.perform_action(&editor::UndoSelection, w, cx);
                assert_eq!(e.state.cursor(), 0);
                e.perform_action(&editor::Redo, w, cx);
                assert_eq!(e.text(cx), "head\n  body\ntail!");
            });
        });
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::workspace::ui_tests::{two_windows, type_in};
    use gpui::TestAppContext;

    #[gpui::test]
    fn n20_remote_bookmark_closed_before_delivery_restores_origin_focus_and_typing(cx: &mut TestAppContext) {
        let (origin, remote, mut c1, mut c2) = two_windows(cx);
        type_in(&origin, &mut c1, "origin");
        type_in(&remote, &mut c2, "top\ntarget");
        let source = origin.read_with(&c1, |ws, _| ws.active_editor().unwrap());
        let target = remote.update(&mut c2, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
            e
        });
        origin.update_in(&mut c1, |ws, w, cx| {
            w.activate_window();
            ws.goto_bookmark(&GotoBookmark, w, cx);
            ws.confirm_quick_panel(w, cx);
            remote.update(cx, |other, cx| other.remove_view(&target, cx));
        });
        c1.run_until_parked();
        c1.update(|w, cx| {
            assert!(source.read(cx).is_focused(w), "failed remote delivery must retain origin editor focus")
        });
        c1.simulate_input("雪");
        assert_eq!(source.read_with(&c1, |e, cx| e.text(cx)), "origin雪");
        assert_eq!(target.read_with(&c2, |e, _| e.state.cursor()), 0);
        assert_eq!(remote.read_with(&c2, |ws, _| ws.views.tabs().count()), 0);
    }

    #[gpui::test]
    fn n20_return_from_successful_remote_bookmark_preserves_origin_focus_and_typing(cx: &mut TestAppContext) {
        let (origin, remote, mut c1, mut c2) = two_windows(cx);
        type_in(&origin, &mut c1, "origin");
        type_in(&remote, &mut c2, "top\ntarget");
        let source = origin.read_with(&c1, |ws, _| ws.active_editor().unwrap());
        let target = remote.update(&mut c2, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.goto_line(2, cx);
                e.sel(cx).toggle_bookmark();
                e.goto_line(1, cx);
            });
            e
        });
        origin.update_in(&mut c1, |ws, w, cx| {
            w.activate_window();
            ws.goto_bookmark(&GotoBookmark, w, cx);
            ws.confirm_quick_panel(w, cx);
        });
        c1.run_until_parked();
        assert_eq!(target.read_with(&c2, |e, _| e.state.cursor()), 4);
        c1.update(|w, _| w.activate_window());
        c1.run_until_parked();
        c1.update(|w, cx| assert!(source.read(cx).is_focused(w), "returning to origin must focus its editor"));
        c1.simulate_input("雪");
        assert_eq!(source.read_with(&c1, |e, cx| e.text(cx)), "origin雪");
        assert_eq!(target.read_with(&c2, |e, cx| e.text(cx)), "top\ntarget");
        assert_eq!(target.read_with(&c2, |e, _| e.state.cursor()), 4);
    }
}
