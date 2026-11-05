use super::*;

impl Workspace {
    pub(crate) fn toggle_viewer_mode(&mut self, _: &ToggleViewerMode, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let document = editor.read(cx).document.clone();
        if self.open_editors(cx).iter().any(|view| {
            let view = view.read(cx);
            view.document == document && !view.marked.is_empty()
        }) {
            self.status_message("Finish or cancel composition in every shared view before changing viewer mode", cx);
            return;
        }
        let enabled = !document.read(cx).model.viewer_mode();
        document.update(cx, |d, cx| d.set_viewer_mode(enabled, cx));
        self.status_message(if enabled { "Viewer mode on — editing protected" } else { "Viewer mode off" }, cx);
    }

    pub(crate) fn follow_log(&mut self, _: &FollowLog, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let document = editor.read(cx).document.clone();
        if self.open_editors(cx).iter().any(|e| e.read(cx).document == document && !e.read(cx).marked.is_empty()) {
            self.status_message("Finish or cancel composition in every shared view before following", cx);
            return;
        }
        match document.update(cx, |d, cx| d.start_follow(cx)) {
            Ok(()) => {
                editor.update(cx, |e, cx| e.follow_log_end(cx));
                self.status_message("Following log — incoming text changes clear Undo history", cx);
            }
            Err(error) => self.status_message(error, cx),
        }
    }
    pub(crate) fn stop_following_log(&mut self, _: &StopFollowingLog, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor() {
            let document = editor.read(cx).document.clone();
            document.update(cx, |d, cx| d.stop_follow(cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{settle_until, temp, window};
    use super::*;
    use crate::editing::text::Selection;
    use gpui::{EntityInputHandler, TestAppContext};

    #[gpui::test]
    fn follow_explicit_command_locks_native_input_and_shared_mutations(cx: &mut TestAppContext) {
        let path = temp("orlo-follow-input").canonicalize().unwrap().join("log");
        std::fs::write(&path, "source😀\n").unwrap();
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        assert!(!first.read_with(vcx, |e, cx| e.doc(cx).is_following()));
        vcx.dispatch_action(FollowLog);
        assert!(first.read_with(vcx, |e, cx| e.doc(cx).is_following()));
        ws.update_in(vcx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
        let peer = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        let before = first.read_with(vcx, |e, cx| (e.text(cx), e.doc(cx).version));
        for editor in [&first, &peer] {
            editor.update_in(vcx, |e, w, cx| {
                e.state.virtual_space = true.into();
                e.state.overtype = true;
                let end = e.doc(cx).len();
                e.sel(cx).set_selections(vec![Selection::from_points((end, 8), (end, 8))]);
                let sels = e.state.selections.clone();
                e.replace_text_in_range(None, "bad", w, cx);
                e.paste_entry(
                    &crate::editing::clipboard_history::ClipEntry { text: "bad paste".into(), whole_lines: false },
                    cx,
                );
                e.replace_and_mark_text_in_range(None, "中", None, w, cx);
                e.edit(cx, |s| {
                    s.insert("bad");
                    s.undo();
                    s.redo();
                });
                assert_eq!(e.state.selections, sels);
                assert!(e.marked.is_empty());
                assert_eq!((e.text(cx), e.doc(cx).version), before);
                assert!(!e.doc(cx).dirty);
                e.sel(cx).select_all();
                assert_eq!(e.read_state(cx).copy_text().0, before.0);
            });
        }
        std::fs::write(&path, "source😀\nappended中").unwrap();
        settle_until(vcx, |cx| {
            cx.executor().advance_clock(std::time::Duration::from_millis(500));
            first.read_with(cx, |e, cx| e.text(cx).contains("appended"))
        });
        assert!(peer.read_with(vcx, |e, cx| e.text(cx).contains("appended")));
        vcx.dispatch_action(StopFollowingLog);
        assert!(!first.read_with(vcx, |e, cx| e.doc(cx).is_following()));
    }
    fn opened<'a>(
        cx: &'a mut TestAppContext,
        name: &str,
    ) -> (PathBuf, Entity<Workspace>, &'a mut gpui::VisualTestContext, Entity<Editor>) {
        let path = temp(name).canonicalize().unwrap().join("log");
        std::fs::write(&path, "old😀\n").unwrap();
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        let editor = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        (path, ws, vcx, editor)
    }
    fn following(e: &Entity<Editor>, cx: &gpui::VisualTestContext) -> bool {
        e.read_with(cx, |e, cx| e.doc(cx).is_following())
    }
    fn wait_text(e: &Entity<Editor>, cx: &mut gpui::VisualTestContext, text: &str) {
        settle_until(cx, |cx| {
            cx.executor().advance_clock(std::time::Duration::from_millis(500));
            e.read_with(cx, |e, cx| e.text(cx) == text)
        });
    }

    #[gpui::test]
    fn follow_pending_delivery_cannot_cross_stop_rename_or_close(cx: &mut TestAppContext) {
        use crate::documents::follow::gate;
        let (path, ws, vcx, editor) = opened(cx, "orlo-follow-cancel");
        for reason in ["stop", "rename", "close"] {
            let paused = gate::pause(&path);
            vcx.dispatch_action(FollowLog);
            settle_until(vcx, |_| paused.arrived() > 0);
            let before = editor.read_with(vcx, |e, cx| e.text(cx));
            std::fs::write(&path, format!("changed by {reason}")).unwrap();
            match reason {
                "stop" => vcx.dispatch_action(StopFollowingLog),
                "rename" => {
                    editor.update(vcx, |e, cx| e.update_doc(cx, |d| d.path = Some(path.with_extension("renamed"))))
                }
                _ => ws.update_in(vcx, |ws, w, cx| ws.request_close(editor.clone(), After::Close, w, cx)),
            }
            assert!(!following(&editor, vcx));
            paused.release();
            vcx.run_until_parked();
            assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), before);
            if reason == "rename" {
                editor.update(vcx, |e, cx| e.update_doc(cx, |d| d.path = Some(path.clone())));
            }
        }
        // The retained entity is alive; a reopened tab still gets a fresh, non-following document.
        ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        let reopened = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        assert!(!following(&reopened, vcx));
        assert_ne!(editor.read_with(vcx, |e, cx| e.doc(cx).id()), reopened.read_with(vcx, |e, cx| e.doc(cx).id()));
    }

    #[gpui::test]
    fn follow_pending_rotation_is_rechecked_before_publication(cx: &mut TestAppContext) {
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-rotation");
        let paused = crate::documents::follow::gate::pause(&path);
        std::fs::write(&path, "read but obsolete").unwrap();
        vcx.dispatch_action(FollowLog);
        settle_until(vcx, |_| paused.arrived() > 0);
        std::fs::rename(&path, path.with_extension("old")).unwrap();
        std::fs::write(&path, "rotation 中").unwrap();
        paused.release();
        vcx.run_until_parked();
        assert_ne!(editor.read_with(vcx, |e, cx| e.text(cx)), "read but obsolete");
        wait_text(&editor, vcx, "rotation 中");
        std::fs::remove_file(&path).unwrap();
        settle_until(vcx, |cx| {
            cx.executor().advance_clock(std::time::Duration::from_millis(500));
            editor.read_with(cx, |e, cx| e.document.read(cx).following.as_ref().unwrap().status.contains("reappear"))
        });
        assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), "rotation 中");
        std::fs::write(&path, "short").unwrap();
        wait_text(&editor, vcx, "short");
        assert!(!editor.read_with(vcx, |e, cx| e.doc(cx).dirty));
    }

    #[gpui::test]
    fn follow_partial_stop_and_malformed_pause_keep_raw_source_protected(cx: &mut TestAppContext) {
        let (path, ws, vcx, editor) = opened(cx, "orlo-follow-partial-stop");
        vcx.dispatch_action(FollowLog);
        let partial = b"prefix\xf0\x9f";
        std::fs::write(&path, partial).unwrap();
        wait_text(&editor, vcx, "prefix");
        assert!(editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
        assert_eq!(editor.read_with(vcx, |e, cx| e.doc(cx).disk), None);
        vcx.dispatch_action(StopFollowingLog);
        editor.update(vcx, |e, cx| {
            e.edit(cx, |s| s.insert("edit"));
            assert!(e.update_doc(cx, |d| d.capture_save(path.clone(), None)).is_err());
            e.edit(cx, |s| s.undo());
        });
        assert_eq!(std::fs::read(&path).unwrap(), partial);
        // Normal watching resolves the incomplete file only after an actual complete read.
        std::fs::write(&path, "complete😀").unwrap();
        ws.update_in(vcx, |ws, w, cx| ws.poll_disk(w, cx));
        wait_text(&editor, vcx, "complete😀");
        assert!(!editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
        vcx.dispatch_action(FollowLog);
        std::fs::write(&path, b"malformed\xff").unwrap();
        settle_until(vcx, |cx| {
            cx.executor().advance_clock(std::time::Duration::from_millis(500));
            editor.read_with(cx, |e, cx| e.document.read(cx).following.as_ref().unwrap().paused)
        });
        assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), "complete😀");
        vcx.dispatch_action(StopFollowingLog);
        assert!(editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
        assert_eq!(std::fs::read(&path).unwrap(), b"malformed\xff");
    }

    #[gpui::test]
    fn follow_refuses_dirty_pending_save_shared_ime_and_skips_save_preparation(cx: &mut TestAppContext) {
        let (path, ws, vcx, editor) = opened(cx, "orlo-follow-entry");
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.insert("dirty")));
        vcx.dispatch_action(FollowLog);
        assert!(!following(&editor, vcx));
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
        let save = crate::documents::save::gate::pause(&path);
        vcx.dispatch_action(Save);
        vcx.dispatch_action(FollowLog);
        assert!(!following(&editor, vcx));
        save.release();
        settle_until(vcx, |cx| !editor.read_with(cx, |e, cx| e.document.read(cx).saving()));
        ws.update_in(vcx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
        editor.update_in(vcx, |e, w, cx| e.replace_and_mark_text_in_range(None, "中", None, w, cx));
        vcx.dispatch_action(FollowLog);
        assert!(!following(&editor, vcx));
        assert!(editor.read_with(vcx, |e, _| !e.marked.is_empty()));
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
        vcx.dispatch_action(FollowLog);
        assert!(following(&editor, vcx));
        let document = editor.read_with(vcx, |e, _| e.document.clone());
        let before = std::fs::read(&path).unwrap();
        vcx.update(|_, cx| {
            let result = OpenDocument::save(
                &document,
                Some(path.with_extension("copy")),
                None,
                |_| panic!("follow must refuse before formatting or cleanup"),
                |_, _| {},
                cx,
            );
            assert!(matches!(result, Some(Err(_))));
        });
        vcx.dispatch_action(SaveAs);
        assert!(!path.with_extension("copy").exists());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[gpui::test]
    fn follow_shared_windows_keep_one_owner_until_the_last_tab_closes(cx: &mut TestAppContext) {
        use super::super::ui_tests::two_windows;
        let path = temp("orlo-follow-two-windows").canonicalize().unwrap().join("log");
        std::fs::write(&path, "source").unwrap();
        let (a, b, mut acx, mut bcx) = two_windows(cx);
        a.update_in(&mut acx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        b.update_in(&mut bcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        let first = a.read_with(&acx, |ws, _| ws.active_editor().unwrap());
        let peer = b.read_with(&bcx, |ws, _| ws.active_editor().unwrap());
        first.update_in(&mut acx, |e, w, cx| e.replace_and_mark_text_in_range(None, "中", None, w, cx));
        bcx.dispatch_action(FollowLog);
        assert!(!following(&peer, &bcx));
        first.update(&mut acx, |e, cx| e.edit(cx, |s| s.undo()));
        acx.dispatch_action(FollowLog);
        bcx.dispatch_action(FollowLog);
        assert_eq!(first.read_with(&acx, |e, _| e.document.clone()), peer.read_with(&bcx, |e, _| e.document.clone()));
        a.update_in(&mut acx, |ws, w, cx| ws.request_close(first.clone(), After::Close, w, cx));
        assert!(following(&peer, &bcx));
        std::fs::write(&path, "shared append😀").unwrap();
        wait_text(&peer, &mut bcx, "shared append😀");
        b.update_in(&mut bcx, |ws, w, cx| ws.request_close(peer.clone(), After::Close, w, cx));
        assert!(!following(&peer, &bcx));
        assert!(!following(&first, &acx));
    }

    #[gpui::test]
    fn follow_preserves_inspection_and_only_scrolls_tail_carets(cx: &mut TestAppContext) {
        let (path, ws, vcx, editor) = opened(cx, "orlo-follow-caret");
        let source = "line 中😀\n".repeat(300);
        std::fs::write(&path, &source).unwrap();
        vcx.dispatch_action(FollowLog);
        wait_text(&editor, vcx, &source);
        assert_eq!(editor.read_with(vcx, |e, _| e.state.cursor()), source.chars().count());
        ws.update_in(vcx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
        let peer = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        peer.update(vcx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 4)]));
        let next = format!("{source}tail");
        std::fs::write(&path, &next).unwrap();
        wait_text(&editor, vcx, &next);
        assert_eq!(editor.read_with(vcx, |e, _| e.state.cursor()), next.chars().count());
        assert_eq!(peer.read_with(vcx, |e, _| e.state.selections.clone()), vec![Selection::new(0, 4)]);
        editor.update_in(vcx, |e, w, cx| e.scroll_lines(&crate::views::editor::ScrollLines { up: true }, w, cx));
        let before = editor.read_with(vcx, |e, cx| (e.state.cursor(), e.scroll_position(cx)));
        let more = format!("{next} more\n");
        std::fs::write(&path, &more).unwrap();
        wait_text(&editor, vcx, &more);
        assert_eq!(editor.read_with(vcx, |e, cx| (e.state.cursor(), e.scroll_position(cx))), before);
        // Selecting and copying remain available, including the Print options snapshot.
        vcx.dispatch_action(Print);
        assert!(ws.read_with(vcx, |ws, _| matches!(ws.panel, Some(Panel::Form(_)))));
        vcx.simulate_keystrokes("escape");
        std::fs::write(&path, "x").unwrap();
        wait_text(&peer, vcx, "x");
        assert!(peer.read_with(vcx, |e, cx| e.state.selections.iter().all(|s| s.end() <= e.doc(cx).len())));
    }

    #[gpui::test]
    fn follow_respects_write_leases_and_preserves_source_read_only_on_stop(cx: &mut TestAppContext) {
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-leases");
        let lease = vcx.update(|_, cx| pipelines::lease(&path, cx));
        vcx.dispatch_action(FollowLog);
        assert!(!following(&editor, vcx));
        drop(lease);
        editor.update(vcx, |e, cx| e.update_doc(cx, |d| d.read_only = true));
        let paused = crate::documents::follow::gate::pause(&path);
        vcx.dispatch_action(FollowLog);
        settle_until(vcx, |_| paused.arrived() > 0);
        vcx.dispatch_action(StopFollowingLog);
        assert!(editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
        paused.release();
        // A write acquired during the read prevents its result from landing.
        std::fs::write(&path, "leased candidate").unwrap();
        let paused = crate::documents::follow::gate::pause(&path);
        vcx.dispatch_action(FollowLog);
        settle_until(vcx, |_| paused.arrived() > 0);
        let lease = vcx.update(|_, cx| pipelines::lease(&path, cx));
        paused.release();
        vcx.run_until_parked();
        assert_ne!(editor.read_with(vcx, |e, cx| e.text(cx)), "leased candidate");
        std::fs::write(&path, "final writer text").unwrap();
        drop(lease);
        wait_text(&editor, vcx, "final writer text");
    }

    #[gpui::test]
    fn follow_uses_disk_codec_and_external_bom_then_preserves_permissions(cx: &mut TestAppContext) {
        use crate::editing::encoding::{encode, Encoding};
        use std::os::unix::fs::PermissionsExt;
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-codec-permission");
        editor.update(vcx, |e, cx| {
            e.update_doc(cx, |d| {
                d.encoding = Encoding::Utf16Be;
            })
        });
        vcx.dispatch_action(FollowLog);
        std::fs::write(&path, "correct UTF8 中😀").unwrap();
        wait_text(&editor, vcx, "correct UTF8 中😀");
        assert_eq!(editor.read_with(vcx, |e, cx| e.doc(cx).disk_encoding), Encoding::Utf8);
        let bytes = encode("BOM switched 中😀", Encoding::Utf32Le, true).unwrap();
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        wait_text(&editor, vcx, "BOM switched 中😀");
        assert_eq!(editor.read_with(vcx, |e, cx| e.doc(cx).disk_encoding), Encoding::Utf32Le);
        assert!(editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
        vcx.dispatch_action(StopFollowingLog);
        assert!(editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    #[gpui::test]
    fn follow_incomplete_bom_stop_keeps_original_bytes_safe(cx: &mut TestAppContext) {
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-partial-bom");
        let paused = crate::documents::follow::gate::pause(&path);
        std::fs::write(&path, b"\xff").unwrap();
        vcx.dispatch_action(FollowLog);
        settle_until(vcx, |_| paused.arrived() > 0);
        paused.release();
        vcx.run_until_parked();
        vcx.dispatch_action(StopFollowingLog);
        editor.update(vcx, |e, cx| assert!(e.update_doc(cx, |d| d.capture_save(path.clone(), None)).is_err()));
        assert_eq!(std::fs::read(path).unwrap(), b"\xff");
    }
    #[gpui::test]
    fn follow_window_close_cancels_even_with_retained_workspace_and_editor(cx: &mut TestAppContext) {
        use super::super::ui_tests::two_windows;
        let path = temp("orlo-follow-window-close").canonicalize().unwrap().join("log");
        std::fs::write(&path, "source").unwrap();
        let (a, _b, mut acx, bcx) = two_windows(cx);
        a.update_in(&mut acx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        let editor = a.read_with(&acx, |ws, _| ws.active_editor().unwrap());
        acx.dispatch_action(FollowLog);
        assert!(following(&editor, &acx));
        a.update_in(&mut acx, |ws, w, cx| ws.confirm_close(w, cx));
        assert!(!following(&editor, &bcx));
        assert!(a.read_with(&bcx, |ws, _| ws.views.tabs().next().is_none()));
    }

    #[gpui::test]
    fn follow_project_clear_cancels_pending_read_with_retained_editor(cx: &mut TestAppContext) {
        let (path, ws, vcx, editor) = opened(cx, "orlo-follow-project-clear");
        let paused = crate::documents::follow::gate::pause(&path);
        vcx.dispatch_action(FollowLog);
        settle_until(vcx, |_| paused.arrived() > 0);
        ws.update_in(vcx, |ws, w, cx| ws.clear_window(w, cx));
        assert!(!following(&editor, vcx));
        let before = editor.read_with(vcx, |e, cx| e.text(cx));
        std::fs::write(path, "discarded result").unwrap();
        paused.release();
        vcx.run_until_parked();
        assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), before);
    }

    #[gpui::test]
    fn follow_cut_action_preserves_clipboard_caret_marks_and_tail(cx: &mut TestAppContext) {
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-cut");
        std::fs::write(&path, "first\nlast").unwrap();
        vcx.dispatch_action(FollowLog);
        wait_text(&editor, vcx, "first\nlast");
        vcx.write_to_clipboard(gpui::ClipboardItem::new_string("keep clipboard".into()));
        vcx.update(|_, cx| {
            cx.default_global::<crate::editing::clipboard_history::ClipboardHistory>().record("keep history", false)
        });
        let before = editor.read_with(vcx, |e, cx| {
            (e.text(cx), e.doc(cx).version, e.state.selections.clone(), e.marked.clone(), e.doc(cx).save_point())
        });
        vcx.simulate_keystrokes("cmd-x");
        assert_eq!(
            editor.read_with(vcx, |e, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.state.selections.clone(),
                e.marked.clone(),
                e.doc(cx).save_point()
            )),
            before
        );
        assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "keep clipboard");
        vcx.update(|_, cx| {
            let history = cx.default_global::<crate::editing::clipboard_history::ClipboardHistory>();
            assert_eq!(history.entries().len(), 1);
            assert_eq!(history.entries()[0].text, "keep history");
        });
        std::fs::write(&path, "first\nlast appended中").unwrap();
        wait_text(&editor, vcx, "first\nlast appended中");
        assert_eq!(editor.read_with(vcx, |e, _| e.state.cursor()), "first\nlast appended中".chars().count());
        vcx.dispatch_action(StopFollowingLog);
        editor.update(vcx, |e, cx| e.update_doc(cx, |d| d.read_only = true));
        vcx.simulate_keystrokes("cmd-x");
        assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), "first\n");
        assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "last appended中\n");
        vcx.simulate_keystrokes("cmd-z");
        assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), "first\nlast appended中");
    }

    #[gpui::test]
    fn follow_failure_from_old_bytes_does_not_pause_rotated_valid_file(cx: &mut TestAppContext) {
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-failed-decode-rotation");
        std::fs::write(&path, b"obsolete malformed\xff").unwrap();
        let paused = crate::documents::follow::gate::pause(&path);
        crate::files::watch::follow::BEFORE_DECODE.set(Some(|p| {
            let next = p.with_extension("replacement");
            std::fs::write(&next, "valid replacement中").unwrap();
            std::fs::rename(next, p).unwrap();
        }));
        vcx.dispatch_action(FollowLog);
        settle_until(vcx, |_| paused.arrived() > 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "valid replacement中");
        paused.release();
        vcx.run_until_parked();
        assert!(!editor.read_with(vcx, |e, cx| e.document.read(cx).following.as_ref().unwrap().paused));
        wait_text(&editor, vcx, "valid replacement中");
    }

    #[gpui::test]
    fn follow_failed_bytes_under_hard_link_write_lease_retry_without_pausing(cx: &mut TestAppContext) {
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-failed-alias-lease");
        let alias = path.with_extension("alias");
        std::fs::hard_link(&path, &alias).unwrap();
        std::fs::write(&alias, b"transient malformed\xff").unwrap();
        let paused = crate::documents::follow::gate::pause(&path);
        vcx.dispatch_action(FollowLog);
        settle_until(vcx, |_| paused.arrived() > 0);
        let lease = vcx.update(|_, cx| {
            let lease = pipelines::lease_file(&alias, pipelines::file_id(&alias), cx);
            assert!(!pipelines::writing(&path, cx));
            lease
        });
        paused.release();
        vcx.run_until_parked();
        assert!(!editor.read_with(vcx, |e, cx| e.document.read(cx).following.as_ref().unwrap().paused));
        assert!(!editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
        std::fs::write(&alias, "writer completed中").unwrap();
        drop(lease);
        wait_text(&editor, vcx, "writer completed中");
    }

    #[gpui::test]
    fn follow_accepted_failures_forget_complete_disk_until_recovery(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;
        for failure in ["malformed", "size", "read"] {
            let (path, _ws, vcx, editor) = opened(cx, &format!("orlo-follow-error-facts-{failure}"));
            let original = std::fs::read(&path).unwrap();
            match failure {
                "malformed" => std::fs::write(&path, b"bad\xff").unwrap(),
                "size" => std::fs::File::create(&path).unwrap().set_len(orlo_project::io::MAX_SIZE + 1).unwrap(),
                _ => std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap(),
            }
            vcx.dispatch_action(FollowLog);
            settle_until(vcx, |cx| {
                editor.read_with(cx, |e, cx| e.document.read(cx).following.as_ref().unwrap().paused)
            });
            assert!(editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
            assert_eq!(editor.read_with(vcx, |e, cx| e.doc(cx).disk), None, "{failure}");
            vcx.dispatch_action(StopFollowingLog);
            assert!(editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
            assert_eq!(editor.read_with(vcx, |e, cx| e.doc(cx).disk), None);
            vcx.dispatch_action(FollowLog);
            assert!(!following(&editor, vcx), "{failure}: explicit recovery required");
            editor.update(vcx, |e, cx| {
                e.edit(cx, |s| s.insert("edit"));
                assert!(e.update_doc(cx, |d| d.capture_save(path.clone(), None)).is_err());
            });
            match failure {
                "malformed" => assert_eq!(std::fs::read(&path).unwrap(), b"bad\xff"),
                "size" => assert_eq!(std::fs::metadata(&path).unwrap().len(), orlo_project::io::MAX_SIZE + 1),
                _ => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                    assert_eq!(std::fs::read(&path).unwrap(), original);
                }
            }
        }
    }

    #[gpui::test]
    fn follow_restart_before_old_delivery_release_keeps_new_owner(cx: &mut TestAppContext) {
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-restart-before-release");
        std::fs::write(&path, b"obsolete failed read\xff").unwrap();
        let old = crate::documents::follow::gate::pause(&path);
        vcx.dispatch_action(FollowLog);
        settle_until(vcx, |_| old.arrived() > 0);
        vcx.dispatch_action(StopFollowingLog);
        std::fs::write(&path, "new owner中").unwrap();
        let new = crate::documents::follow::gate::pause(&path);
        vcx.dispatch_action(FollowLog);
        assert!(following(&editor, vcx));
        settle_until(vcx, |_| new.arrived() > 0);
        old.release();
        vcx.run_until_parked();
        assert!(following(&editor, vcx));
        assert!(!editor.read_with(vcx, |e, cx| e.document.read(cx).following.as_ref().unwrap().paused));
        assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), "old😀\n");
        new.release();
        wait_text(&editor, vcx, "new owner中");
        assert!(!editor.read_with(vcx, |e, cx| e.document.read(cx).following.as_ref().unwrap().paused));
        std::fs::write(&path, "new owner still polls😀").unwrap();
        wait_text(&editor, vcx, "new owner still polls😀");
    }

    #[gpui::test]
    fn follow_append_only_split_codecs_publish_complete_characters_without_writing_source(cx: &mut TestAppContext) {
        use crate::editing::encoding::{encode, Encoding};
        use std::io::Write;
        // Literal tails make the expectations independent of the encoder used for the header.
        let cases: &[(Encoding, &[u8], &[u8], &str)] = &[
            (Encoding::Utf8, b"\xf0\x9f", b"\x98\x80", "😀"),
            (Encoding::Utf16Le, b"\x3d\xd8", b"\x00\xde", "😀"),
            (Encoding::Utf32Be, b"\x00\x01\xf6", b"\x00", "😀"),
            (Encoding::ShiftJis, b"\x93", b"\xfa", "日"),
            (Encoding::Iso2022Jp, b"\x1b\x24\x42\x46", b"\x7c\x1b\x28\x42", "日"),
            (Encoding::Gb18030, b"\x94\x39", b"\xfc\x36", "😀"),
        ];
        for &(encoding, partial, rest, character) in cases {
            let path = temp(&format!("orlo-follow-append-{encoding:?}")).canonicalize().unwrap().join("log");
            let mut expected_bytes = encode("head\n", encoding, !encoding.bom_bytes().is_empty()).unwrap();
            std::fs::write(&path, &expected_bytes).unwrap();
            let (ws, vcx) = window(cx);
            ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
            let editor = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
            // Exercise the ordinary explicit disk-codec choice for BOM-less legacy files.
            ws.update_in(vcx, |ws, w, cx| {
                let e = editor.read(cx);
                let d = e.doc(cx);
                let action = encoding::ChangeFileEncoding {
                    encoding,
                    bom: false,
                    reopen: true,
                    editor: editor.entity_id(),
                    buffer: d.id(),
                    revision: d.version,
                    point: d.save_point(),
                    path: d.path.clone(),
                    disk: d.disk,
                    disk_encoding: d.disk_encoding,
                    read_only: d.read_only,
                };
                ws.change_file_encoding(&action, w, cx);
            });
            settle_until(vcx, |cx| editor.read_with(cx, |e, cx| e.doc(cx).disk_encoding == encoding));
            let mut writer = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            vcx.dispatch_action(FollowLog);
            assert!(following(&editor, vcx));
            writer.write_all(partial).unwrap();
            expected_bytes.extend_from_slice(partial);
            settle_until(vcx, |cx| {
                cx.executor().advance_clock(std::time::Duration::from_millis(500));
                editor.read_with(cx, |e, cx| {
                    e.document.read(cx).following.as_ref().unwrap().status.contains("incomplete character")
                })
            });
            assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), "head\n");
            assert_eq!(editor.read_with(vcx, |e, cx| e.doc(cx).disk), None);
            assert_eq!(std::fs::read(&path).unwrap(), expected_bytes);
            writer.write_all(rest).unwrap();
            expected_bytes.extend_from_slice(rest);
            wait_text(&editor, vcx, &format!("head\n{character}"));
            assert!(editor.read_with(vcx, |e, cx| !e.doc(cx).dirty && e.doc(cx).disk.is_some()));
            vcx.dispatch_action(StopFollowingLog);
            assert_eq!(std::fs::read(&path).unwrap(), expected_bytes);
            assert!(!editor.read_with(vcx, |e, cx| e.doc(cx).read_only));
        }
    }

    #[gpui::test]
    fn follow_rendered_shared_views_pause_upward_and_resume_at_bottom(cx: &mut TestAppContext) {
        use super::super::ui_tests::two_windows;
        use std::io::Write;
        let path = temp("orlo-follow-rendered-scroll").canonicalize().unwrap().join("log");
        let mut text = "line 中😀\n".repeat(300);
        std::fs::write(&path, &text).unwrap();
        let (a, b, mut acx, mut bcx) = two_windows(cx);
        a.update_in(&mut acx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        b.update_in(&mut bcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        let first = a.read_with(&acx, |ws, _| ws.active_editor().unwrap());
        let peer = b.read_with(&bcx, |ws, _| ws.active_editor().unwrap());
        acx.update(|w, cx| w.draw(cx).clear());
        first.update_in(&mut acx, |e, w, cx| {
            for _ in 0..400 {
                e.scroll_lines(&crate::views::editor::ScrollLines { up: false }, w, cx);
            }
            assert_eq!(e.state.cursor(), 0, "ordinary scrolling must not move the caret");
        });
        acx.dispatch_action(FollowLog);
        bcx.dispatch_action(FollowLog);
        acx.update(|w, cx| w.draw(cx).clear());
        bcx.update(|w, cx| w.draw(cx).clear());
        first.update_in(&mut acx, |e, w, cx| {
            assert!(e.max_scroll(cx) > 0., "the actual painted editor must be scrollable");
            for _ in 0..8 {
                e.scroll_lines(&crate::views::editor::ScrollLines { up: true }, w, cx);
            }
        });
        acx.update(|w, cx| w.draw(cx).clear());
        let held = first.read_with(&acx, |e, cx| (e.state.cursor(), e.scroll_position(cx)));
        let peer_before = peer.read_with(&bcx, |e, cx| e.scroll_position(cx));
        let mut writer = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        let appended = "incoming 😀\n".repeat(12);
        writer.write_all(appended.as_bytes()).unwrap();
        text.push_str(&appended);
        wait_text(&first, &mut acx, &text);
        bcx.run_until_parked();
        acx.update(|w, cx| w.draw(cx).clear());
        bcx.update(|w, cx| w.draw(cx).clear());
        assert_eq!(first.read_with(&acx, |e, cx| (e.state.cursor(), e.scroll_position(cx))), held);
        assert_eq!(peer.read_with(&bcx, |e, _| e.state.cursor()), text.chars().count());
        assert!(peer.read_with(&bcx, |e, cx| e.scroll_position(cx).0) > peer_before.0);
        first.update_in(&mut acx, |e, w, cx| {
            for _ in 0..40 {
                e.scroll_lines(&crate::views::editor::ScrollLines { up: false }, w, cx);
            }
            let bottom = e.scroll_position(cx);
            e.scroll_lines(&crate::views::editor::ScrollLines { up: false }, w, cx);
            assert_eq!(e.scroll_position(cx), bottom, "scrolling down has reached the painted bottom");
        });
        let resumed = first.read_with(&acx, |e, cx| e.scroll_position(cx));
        writer.write_all(b"resumed\n").unwrap();
        text.push_str("resumed\n");
        wait_text(&first, &mut acx, &text);
        acx.update(|w, cx| w.draw(cx).clear());
        assert_eq!(first.read_with(&acx, |e, _| e.state.cursor()), text.chars().count());
        assert!(first.read_with(&acx, |e, cx| e.scroll_position(cx).0) > resumed.0);
        peer.update_in(&mut bcx, |e, w, cx| {
            e.state.virtual_space = true.into();
            for selections in [
                vec![Selection::new(0, 4)],
                vec![Selection::caret(3)],
                vec![Selection::caret(0), Selection::caret(4)],
                vec![Selection::from_points((e.doc(cx).len(), 2), (e.doc(cx).len(), 2))],
            ] {
                e.sel(cx).set_selections(selections.clone());
                assert_eq!(e.state.selections, selections, "the review fixture must survive ordinary normalization");
                e.scroll_lines(&crate::views::editor::ScrollLines { up: false }, w, cx);
                assert_eq!(e.state.selections, selections, "bottom scrolling must preserve inspection selections");
            }
            e.sel(cx).set_selections(vec![Selection::new(0, 4)]);
        });
        writer.write_all(b"inspection\n").unwrap();
        text.push_str("inspection\n");
        wait_text(&first, &mut acx, &text);
        bcx.run_until_parked();
        assert_eq!(peer.read_with(&bcx, |e, _| e.state.selections.clone()), vec![Selection::new(0, 4)]);
        assert_eq!(first.read_with(&acx, |e, _| e.state.cursor()), text.chars().count());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[gpui::test]
    fn follow_paused_tail_caret_expires_on_no_frame_review_away_and_back(cx: &mut TestAppContext) {
        use std::io::Write;
        let (path, _ws, vcx, editor) = opened(cx, "orlo-follow-no-frame-review");
        let mut text = "line 中😀\n".repeat(300);
        std::fs::write(&path, &text).unwrap();
        vcx.dispatch_action(FollowLog);
        wait_text(&editor, vcx, &text);
        vcx.update(|w, cx| w.draw(cx).clear());
        editor.update_in(vcx, |e, w, cx| {
            e.scroll_lines(&crate::views::editor::ScrollLines { up: true }, w, cx);
        });
        let held = editor.read_with(vcx, |e, _| e.state.cursor());
        let mut writer = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writer.write_all(b"arrived while paused\n").unwrap();
        text.push_str("arrived while paused\n");
        wait_text(&editor, vcx, &text);
        editor.update_in(vcx, |e, w, cx| {
            // The same coordinates do not restore an intention invalidated by explicit review.
            e.sel(cx).set_cursor(0);
            e.sel(cx).set_cursor(held);
            for _ in 0..40 {
                e.scroll_lines(&crate::views::editor::ScrollLines { up: false }, w, cx);
            }
            assert_eq!(e.state.cursor(), held);
        });
        let before = editor.read_with(vcx, |e, cx| (e.state.selections.clone(), e.scroll_position(cx)));
        writer.write_all(b"more output\n").unwrap();
        text.push_str("more output\n");
        wait_text(&editor, vcx, &text);
        vcx.update(|w, cx| w.draw(cx).clear());
        assert_eq!(editor.read_with(vcx, |e, cx| (e.state.selections.clone(), e.scroll_position(cx))), before);
    }

    #[gpui::test]
    fn follow_dirty_refusal_preserves_undo_recovery_and_source_until_ordinary_save(cx: &mut TestAppContext) {
        let (path, ws, vcx, editor) = opened(cx, "orlo-follow-dirty-recovery");
        let original = std::fs::read(&path).unwrap();
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.insert("unsaved 中")));
        let before = editor.read_with(vcx, |e, cx| {
            (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.state.selections.clone())
        });
        vcx.dispatch_action(FollowLog);
        assert!(!following(&editor, vcx));
        assert_eq!(
            editor.read_with(vcx, |e, cx| {
                (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.state.selections.clone())
            }),
            before
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        ws.update_in(vcx, |ws, w, cx| {
            let mut texts = Texts::new();
            let captured = ws.capture(w, true, &mut texts, cx);
            assert!(captured.groups.iter().flat_map(|g| &g.views).any(|v| v.unsaved.is_some()));
            assert!(texts.iter().any(|(_, rope)| *rope == before.0));
        });
        editor.update(vcx, |e, cx| {
            e.edit(cx, |s| s.undo());
            assert_eq!(e.text(cx), std::str::from_utf8(&original).unwrap());
            assert!(!e.doc(cx).dirty);
            e.edit(cx, |s| s.redo());
            assert_eq!(e.text(cx), before.0);
        });
        vcx.dispatch_action(Save);
        settle_until(vcx, |cx| editor.read_with(cx, |e, cx| !e.doc(cx).dirty && !e.document.read(cx).saving()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before.0);
        vcx.dispatch_action(FollowLog);
        assert!(following(&editor, vcx));
        vcx.dispatch_action(StopFollowingLog);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before.0);
    }
}
