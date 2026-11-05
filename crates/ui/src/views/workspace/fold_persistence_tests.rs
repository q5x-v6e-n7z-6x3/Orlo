//! Exercise fold persistence through the real workspace/session and reopen entry points.
use super::super::ui_tests::{settle, settle_until, temp, window};
use super::*;
use gpui::{TestAppContext, VisualTestContext};
use std::ops::Range;

fn one(range: Range<usize>) -> Vec<Range<usize>> {
    std::iter::once(range).collect()
}

fn ranges(editor: &Entity<Editor>, cx: &App) -> Vec<Range<usize>> {
    editor.read(cx).folds().chars()
}

fn finish_restore(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    settle(cx);
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
}

#[gpui::test]
fn persisted_folds_roundtrip_independent_shared_views_and_exact_bytes(cx: &mut TestAppContext) {
    let path = temp("orlo-fold-persistence-roundtrip").canonicalize().unwrap().join("f.rs");
    let bytes = "\u{feff}é🦀 fn {\r\ninner();\r\n} suffix\r\n".as_bytes().to_vec();
    std::fs::write(&path, &bytes).unwrap();
    let (ws, cx) = window(cx);
    let saved = ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path.clone(), window, cx);
        let first = ws.active_editor().unwrap();
        first.update(cx, |e, cx| e.fold_chars_now([6..18, 8..13, 19..22], cx));
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        ws.active_editor().unwrap().update(cx, |e, cx| e.fold_chars_now(one(1..3), cx));
        let saved = ws.capture(window, false, &mut Texts::new(), cx);
        let json = serde_json::to_string(&saved).unwrap();
        serde_json::from_str::<WindowState>(&json).unwrap()
    });
    ws.update_in(cx, |ws, window, cx| ws.restore(saved, window, cx));
    finish_restore(&ws, cx);
    ws.read_with(cx, |ws, cx| {
        let views: Vec<_> = ws.views.tabs().collect();
        assert_eq!(views.len(), 2);
        assert!(views[0].read(cx).same_document(views[1].read(cx)));
        assert_eq!(ranges(views[0], cx), [6..18, 8..13, 19..22]);
        assert_eq!(ranges(views[1], cx), one(1..3));
        assert!(!views[0].read(cx).doc(cx).unsaved());
    });
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[gpui::test]
fn persisted_folds_reopen_each_closed_view_and_ordinary_open_uses_latest(cx: &mut TestAppContext) {
    let path = temp("orlo-fold-persistence-reopen").canonicalize().unwrap().join("f.txt");
    std::fs::write(&path, "abcdefghijklmnop\n").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path.clone(), window, cx);
        let first = ws.active_editor().unwrap();
        first.update(cx, |e, cx| e.fold_chars_now(one(1..4), cx));
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        let second = ws.active_editor().unwrap();
        second.update(cx, |e, cx| e.fold_chars_now(one(8..12), cx));
        ws.close_editor(&first, window, cx);
        ws.close_editor(&second, window, cx);
        assert_eq!(ws.closed.len(), 2);
        assert!(ws.closed.iter().all(|v| v.unsaved.is_none()));
        // Ordinary open uses latest but does not consume the per-view stack.
        ws.open_path(path.clone(), window, cx);
        let sibling = ws.active_editor().unwrap();
        assert_eq!(ranges(&sibling, cx), one(8..12));
        sibling.update(cx, |e, cx| e.fold_chars_now(one(13..15), cx));
        ws.open_path(path.clone(), window, cx);
        assert_eq!(ranges(&sibling, cx), one(13..15), "ordinary focus leaves independent active folds alone");
        ws.reopen_closed_file(&ReopenClosedFile, window, cx);
        assert_eq!(ranges(&ws.active_editor().unwrap(), cx), one(8..12));
        ws.reopen_closed_file(&ReopenClosedFile, window, cx);
        assert_eq!(ranges(&ws.active_editor().unwrap(), cx), one(1..4));
        assert_eq!(ws.views.tabs().count(), 3);
        assert_eq!(ranges(&sibling, cx), one(13..15), "reopen does not change the existing sibling");
        ws.open_path(path.clone(), window, cx);
        assert_eq!(ranges(&sibling, cx), one(13..15), "ordinary focus does not restore into an existing view");
        assert!(ws.closed.is_empty());
        let editor = ws.active_editor().unwrap();
        editor.update(cx, |e, cx| e.fold_chars_now([], cx));
        ws.close_editor(&editor, window, cx);
        ws.reopen_closed_file(&ReopenClosedFile, window, cx);
        assert!(ranges(&ws.active_editor().unwrap(), cx).is_empty());
    });
}

#[gpui::test]
fn persisted_folds_closed_history_survives_disk_session_and_rejects_changed_files(cx: &mut TestAppContext) {
    let dir = temp("orlo-fold-persistence-closed-session").canonicalize().unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "abcdefghijklmnop\n").unwrap();
    let (ws, cx) = window(cx);
    let saved = ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path.clone(), window, cx);
        let editor = ws.active_editor().unwrap();
        editor.update(cx, |e, cx| e.fold_chars_now(one(2..8), cx));
        ws.close_editor(&editor, window, cx);
        let saved = Session { windows: vec![ws.capture(window, true, &mut Texts::new(), cx)], ..Default::default() };
        let file = dir.join("Session.json");
        session::save(&file, &saved, &[], &mut HashSet::new()).unwrap();
        session::load(&file).windows.remove(0)
    });
    ws.update_in(cx, |ws, window, cx| {
        ws.restore(saved, window, cx);
        ws.reopen_closed_file(&ReopenClosedFile, window, cx);
    });
    finish_restore(&ws, cx);
    ws.update_in(cx, |ws, window, cx| {
        let editor = ws.active_editor().unwrap();
        assert_eq!(ranges(&editor, cx), one(2..8));
        ws.close_editor(&editor, window, cx);
        std::fs::write(&path, "abcdefghijklmnoq\n").unwrap();
        ws.reopen_closed_file(&ReopenClosedFile, window, cx);
    });
    finish_restore(&ws, cx);
    ws.read_with(cx, |ws, cx| assert!(ranges(&ws.active_editor().unwrap(), cx).is_empty()));
}

#[gpui::test]
fn persisted_folds_late_restore_checks_actual_shared_text_and_generation(cx: &mut TestAppContext) {
    let dir = temp("orlo-fold-persistence-stale").canonicalize().unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "abcdefghijklmnop\n").unwrap();
    let (ws, cx) = window(cx);
    let (lease, shared) = ws.update_in(cx, |ws, window, cx| {
        let mut model = EditorState::load(&path).unwrap();
        let mut folds = orlo_editor::fold::Folds::default();
        folds.fold(&model.doc, 2..8);
        let saved = ViewState {
            path: path.clone(),
            folds: session::folds::FoldState::capture(&model.doc, &folds),
            ..Default::default()
        };
        let lease = pipelines::lease(&path, cx);
        ws.restore_later(vec![(0, 0, true, saved)], window, cx);
        // A different window's view can have been edited while the disk read waited.
        model.insert("X");
        let editor = ws.new_editor(model, cx);
        let doc = editor.read(cx).document.clone();
        // Register it as shown elsewhere without adding a tab in this restoring workspace.
        Documents::shown(&doc, cx);
        (lease, editor)
    });
    cx.run_until_parked();
    drop(lease);
    finish_restore(&ws, cx);
    ws.read_with(cx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        assert!(editor.read(cx).same_document(shared.read(cx)));
        assert!(ranges(&editor, cx).is_empty());
        assert!(editor.read(cx).text(cx).starts_with('X'));
    });
    ws.update_in(cx, |ws, window, cx| {
        let editor = ws.active_editor().unwrap();
        // Read-only restoration is still view-only and preserves the existing undo stack.
        editor.update(cx, |e, cx| {
            e.change(cx, |s| s.doc.read_only = true);
            let saved =
                ViewState { folds: session::folds::FoldState::capture(e.doc(cx), e.folds()), ..Default::default() };
            assert!(saved.folds.is_none());
            e.change(cx, |s| s.doc.read_only = false);
            e.change(cx, |s| s.undo());
            assert_eq!(e.text(cx), "abcdefghijklmnop\n");
            e.change(cx, |s| s.redo());
            assert!(e.text(cx).starts_with('X'));
        });
        let lease = pipelines::lease(&path, cx);
        ws.restore_later(vec![(0, 0, true, ViewState { path: path.clone(), ..Default::default() })], window, cx);
        ws.clear_window(window, cx);
        drop(lease);
    });
    finish_restore(&ws, cx);
    ws.read_with(cx, |ws, cx| assert!(ws.views.tabs().all(|e| e.read(cx).doc(cx).path.as_ref() != Some(&path))));
}

#[gpui::test]
fn persisted_folds_hot_exit_and_ordered_writes_keep_the_newest_view_state(cx: &mut TestAppContext) {
    let dir = temp("orlo-fold-persistence-hot").canonicalize().unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "before\nline two\n").unwrap();
    let (ws, cx) = window(cx);
    let (saved, texts) = ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path.clone(), window, cx);
        let editor = ws.active_editor().unwrap();
        editor.update(cx, |e, cx| {
            e.change(cx, |s| s.insert("é🦀"));
            e.fold_chars_now(one(2..8), cx);
        });
        let mut texts = Texts::new();
        let saved = ws.capture(window, true, &mut texts, cx);
        let old = Session { windows: vec![saved], ..Default::default() };
        let file = dir.join("Session.json");
        let mut writer = Writer::default();
        write_session(&mut writer, &file, 1, &(old, texts), false).unwrap();
        editor.update(cx, |e, cx| e.fold_chars_now([3..5, 9..12], cx));
        let mut texts = Texts::new();
        let saved = Session { windows: vec![ws.capture(window, true, &mut texts, cx)], ..Default::default() };
        write_session(&mut writer, &file, 3, &(saved, texts.clone()), false).unwrap();
        write_session(&mut writer, &file, 2, &(Session::default(), vec![]), false).unwrap();
        (session::load(&file).windows.remove(0), texts)
    });
    assert_eq!(texts.len(), 1);
    ws.update_in(cx, |ws, window, cx| ws.restore(saved, window, cx));
    finish_restore(&ws, cx);
    ws.read_with(cx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        assert_eq!(ranges(&editor, cx), [3..5, 9..12]);
        assert_eq!(editor.read(cx).text(cx), "é🦀before\nline two\n");
        assert!(editor.read(cx).doc(cx).unsaved());
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "before\nline two\n");
}

#[gpui::test]
fn persisted_folds_commands_schedule_autosave_and_closed_metadata_is_bounded(cx: &mut TestAppContext) {
    let path = temp("orlo-fold-persistence-autosave").canonicalize().unwrap().join("f.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let (ws, cx) = window(cx);
    let editor = ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path.clone(), window, cx);
        ws.active_editor().unwrap()
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let auto = cx.default_global::<Autosave>();
        auto.since = None;
        auto.timer = None;
        editor.update(cx, |e, cx| e.fold_lines(0, 2, cx));
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let auto = cx.default_global::<Autosave>();
        assert!(auto.since.is_some() && auto.timer.is_some(), "fold-only commands schedule persistence");
    });
    ws.update_in(cx, |ws, window, cx| {
        for _ in 0..session::folds::MAX_CLOSED_VIEWS + 2 {
            let editor = ws.active_editor().unwrap();
            ws.close_editor(&editor, window, cx);
            ws.open_path(path.clone(), window, cx);
        }
        assert_eq!(ws.closed.len(), session::folds::MAX_CLOSED_VIEWS + 2);
        assert!(ws.closed[..2].iter().all(|v| v.folds.is_none()));
        let saved = ws.capture(window, true, &mut Texts::new(), cx);
        assert_eq!(saved.closed_views.len(), session::folds::MAX_CLOSED_VIEWS);
        assert!(saved.closed_views.iter().all(|v| v.unsaved.is_none() && v.folds.is_some()));
        let editor = ws.active_editor().unwrap();
        editor.update(cx, |e, cx| e.fold_chars_now([], cx));
        ws.close_editor(&editor, window, cx);
        ws.open_path(path.clone(), window, cx);
        assert!(
            ranges(&ws.active_editor().unwrap(), cx).is_empty(),
            "latest expanded state suppresses older folded history"
        );
    });
}

#[gpui::test]
fn persisted_folds_skip_capture_before_shared_edit_offsets_catch_up(cx: &mut TestAppContext) {
    let path = temp("orlo-fold-persistence-shared-capture").canonicalize().unwrap().join("f.txt");
    std::fs::write(&path, "abcdefghijklmnop\n").unwrap();
    let (ws, cx) = window(cx);
    let sibling = ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path, window, cx);
        let first = ws.active_editor().unwrap();
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        let sibling = ws.active_editor().unwrap();
        sibling.update(cx, |e, cx| e.fold_chars_now(one(8..12), cx));
        first.update(cx, |e, cx| e.change(cx, |s| s.insert("🦀")));
        assert!(sibling.read(cx).capture_folds(cx).is_none(), "old offsets have no current fingerprint");
        sibling
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let e = sibling.read(cx);
        assert_eq!(e.capture_folds(cx).unwrap().restore(e.doc(cx)).chars(), one(9..13));
    });
}

#[gpui::test]
fn persisted_folds_reopen_finishes_when_an_ordinary_open_is_already_reading(cx: &mut TestAppContext) {
    let path = temp("orlo-fold-persistence-concurrent-open").canonicalize().unwrap().join("f.txt");
    std::fs::write(&path, "text\n".repeat((LOAD_IN_BACKGROUND / 5 + 1) as usize)).unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        let model = EditorState::load(&path).unwrap();
        let mut folds = orlo_editor::fold::Folds::default();
        folds.fold(&model.doc, 1..3);
        ws.closed.push(ViewState {
            path: path.clone(),
            folds: session::folds::FoldState::capture(&model.doc, &folds),
            ..Default::default()
        });
        ws.open_path(path.clone(), window, cx);
        assert!(ws.loading.contains(&path));
        ws.reopen_closed_file(&ReopenClosedFile, window, cx);
        assert_eq!(ws.restoring.len(), 1);
    });
    finish_restore(&ws, cx);
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
    ws.read_with(cx, |ws, cx| {
        assert!(ws.views.tabs().any(|e| ranges(e, cx) == one(1..3)), "the explicit reopened view keeps its folds");
    });
}

fn reopen_preserves_current_editorconfig(cx: &mut TestAppContext, deferred: bool) {
    let dir = temp(&format!("orlo-fold-reopen-current-policy-{deferred}")).canonicalize().unwrap();
    let path = dir.join("f.txt");
    let config = dir.join(".editorconfig");
    let original = "é🦀 text\nsecond line\n";
    std::fs::write(&path, original).unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.apply_settings(r#"{"word_wrap":true,"wrap_width":72}"#, window, cx);
    });
    for folded in [false, true] {
        std::fs::write(&config, "root=true\n[*]\nindent_style=space\nindent_size=3\ntab_width=6\n").unwrap();
        let lease = ws.update_in(cx, |ws, window, cx| {
            ws.open_path(path.clone(), window, cx);
            let editor = ws.active_editor().unwrap();
            if folded {
                editor.update(cx, |e, cx| e.fold_chars_now(one(1..3), cx));
            }
            ws.close_editor(&editor, window, cx);
            if folded {
                // Even extra preferences in a hand-edited closed record are outside its
                // fold-only contract; they must not control decoding or preparation.
                let closed = ws.closed.last_mut().unwrap();
                closed.encoding = Some(crate::editing::encoding::Encoding::Utf16Le);
                closed.indent_size = Some(11);
                closed.tab_size = Some(16);
                closed.indent_override = Some([true; 2]);
                closed.wrap = Some(false);
                closed.wrap_width = Some(10);
            }
            // Reopen must use current preparation, not restore stale/default document policy.
            std::fs::write(&config, "root=true\n[*]\nindent_style=space\nindent_size=2\ntab_width=8\n").unwrap();
            let lease = deferred.then(|| pipelines::lease(&path, cx));
            ws.reopen_closed_file(&ReopenClosedFile, window, cx);
            assert_eq!(ws.restoring.len(), usize::from(deferred));
            lease
        });
        cx.run_until_parked();
        drop(lease);
        finish_restore(&ws, cx);
        ws.update_in(cx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            let doc = editor.read(cx).doc(cx);
            assert_eq!((doc.tab_size, doc.indent_size, doc.indent_width(), doc.translate_tabs), (8, Some(2), 2, true));
            assert_eq!(doc.indent_override, [false; 2]);
            assert_eq!(doc.encoding, crate::editing::encoding::Encoding::Utf8);
            let e = editor.read(cx);
            assert_eq!((e.wrap_override, e.wrap_width_override, e.view.wrap_width), (None, None, 72));
            assert_eq!(e.text(cx), original);
            assert_eq!(ranges(&editor, cx), if folded { one(1..3) } else { Vec::new() });
            ws.close_editor(&editor, window, cx);
        });
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[gpui::test]
fn persisted_folds_reopen_preserves_current_editorconfig_synchronously(cx: &mut TestAppContext) {
    reopen_preserves_current_editorconfig(cx, false);
}

#[gpui::test]
fn persisted_folds_reopen_preserves_current_editorconfig_after_a_write_lease(cx: &mut TestAppContext) {
    reopen_preserves_current_editorconfig(cx, true);
}

#[gpui::test]
fn persisted_folds_reopen_preserves_shared_document_overrides(cx: &mut TestAppContext) {
    let path = temp("orlo-fold-reopen-shared-policy").canonicalize().unwrap().join("f.txt");
    std::fs::write(&path, "é🦀 text\nsecond line\n").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path, window, cx);
        let first = ws.active_editor().unwrap();
        first.update(cx, |e, cx| e.fold_chars_now(one(1..3), cx));
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        let sibling = ws.active_editor().unwrap();
        sibling.update(cx, |e, cx| {
            e.fold_chars_now(one(9..12), cx);
            e.wrap_override = Some(false);
            e.update_doc(cx, |doc| {
                doc.tab_size = 7;
                doc.indent_size = Some(3);
                doc.translate_tabs = false;
                doc.indent_override = [true; 2];
                doc.line_ending = "\r\n";
                doc.line_ending_override = true;
                doc.encoding = crate::editing::encoding::Encoding::Utf16Le;
                doc.bom = true;
                doc.read_only = true;
                doc.set_following(true);
            });
        });
        // Follow mode advances the document revision even without a text edit.
        first.update(cx, |e, cx| e.catch_up(cx));
        assert!(first.read(cx).capture_folds(cx).is_some());
        ws.close_editor(&first, window, cx);
        ws.reopen_closed_file(&ReopenClosedFile, window, cx);
        let reopened = ws.active_editor().unwrap();
        assert!(reopened.read(cx).same_document(sibling.read(cx)));
        let doc = reopened.read(cx).doc(cx);
        assert_eq!(
            (doc.tab_size, doc.indent_size, doc.translate_tabs, doc.indent_override),
            (7, Some(3), false, [true; 2])
        );
        assert_eq!((doc.line_ending, doc.line_ending_override, doc.bom), ("\r\n", true, true));
        assert_eq!(doc.encoding, crate::editing::encoding::Encoding::Utf16Le);
        assert!(doc.read_only && doc.is_following());
        assert_eq!(sibling.read(cx).wrap_override, Some(false));
        assert_eq!(ranges(&sibling, cx), one(9..12));
        assert_eq!(ranges(&reopened, cx), one(1..3));
    });
}

#[gpui::test]
fn persisted_folds_project_close_and_reopen_restore_dirty_shared_views(cx: &mut TestAppContext) {
    let dir = temp("orlo-fold-project-recovered").canonicalize().unwrap();
    let path = dir.join("f.rs");
    let project = dir.join("folds.sublime-project");
    let workspace = dir.join("folds.orlo-workspace");
    let original = "é🦀 fn {\r\ninner();\r\n} suffix\r\n";
    std::fs::write(&path, original).unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path.clone(), window, cx);
        let first = ws.active_editor().unwrap();
        first.update(cx, |e, cx| {
            e.change(cx, |s| s.insert("dirty "));
            e.fold_chars_now([12..24, 14..19, 25..28], cx);
        });
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        ws.active_editor().unwrap().update(cx, |e, cx| e.fold_chars_now(one(1..4), cx));
        ws.write_project_as(project.clone(), window, cx);
        ws.close_project(&CloseProject, window, cx);
        assert!(ws.project.is_none());
        // Disk text can change while a closed project retains its independent dirty recovery.
        std::fs::write(&path, "different disk contents\r\n").unwrap();
        ws.try_open_project_path(&OpenProjectPath { path: workspace.clone(), here: true }, window, cx).unwrap();
    });
    finish_restore(&ws, cx);
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.project.as_ref().unwrap().path, project);
        let tabs: Vec<_> = ws.views.tabs().collect();
        assert_eq!(tabs.len(), 2);
        assert!(tabs[0].read(cx).same_document(tabs[1].read(cx)));
        assert_eq!(tabs[0].read(cx).text(cx), "dirty é🦀 fn {\ninner();\n} suffix\n");
        assert_eq!(tabs[0].read(cx).doc(cx).line_ending, "\r\n");
        assert!(tabs[0].read(cx).doc(cx).unsaved());
        assert_eq!(ranges(tabs[0], cx), [12..24, 14..19, 25..28]);
        assert_eq!(ranges(tabs[1], cx), one(1..4));
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "different disk contents\r\n");
}

#[gpui::test]
fn persisted_folds_bad_optional_records_keep_recovered_shared_text_and_valid_sibling(cx: &mut TestAppContext) {
    let dir = temp("orlo-fold-corrupt-recovery").canonicalize().unwrap();
    let (ws, cx) = window(cx);
    for deferred in [false, true] {
        for kind in ["missing", "malformed", "unknown-version", "crossing"] {
            let path = dir.join(format!("{kind}-{deferred}.txt"));
            let file = dir.join(format!("{kind}-{deferred}.json"));
            std::fs::write(&path, "abcdefghijklmnop\n").unwrap();
            ws.update_in(cx, |ws, window, cx| {
                ws.clear_window(window, cx);
                ws.open_path(path.clone(), window, cx);
                let first = ws.active_editor().unwrap();
                first.update(cx, |e, cx| {
                    e.change(cx, |s| s.insert("é🦀"));
                    e.fold_chars_now([2..8, 3..5], cx);
                });
                ws.new_view_into_file(&NewViewIntoFile, window, cx);
                ws.active_editor().unwrap().update(cx, |e, cx| e.fold_chars_now(one(9..12), cx));
                let mut texts = Texts::new();
                let state = Session { windows: vec![ws.capture(window, true, &mut texts, cx)], ..Default::default() };
                assert_eq!(texts.len(), 1);
                session::save(&file, &state, &texts, &mut HashSet::new()).unwrap();
            });
            let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            let first = &mut json["windows"][0]["groups"][0]["views"][0];
            match kind {
                "missing" => {
                    first.as_object_mut().unwrap().remove("folds");
                }
                "malformed" => first["folds"] = serde_json::json!(9),
                "unknown-version" => first["folds"]["version"] = serde_json::json!(2),
                "crossing" => first["folds"]["ranges"] = serde_json::json!([[2, 8], [4, 10]]),
                _ => unreachable!(),
            }
            std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
            let saved = session::load(&file).windows.remove(0);
            assert_eq!(saved.groups[0].views.len(), 2, "{kind}");
            assert_eq!(saved.groups[0].views[0].unsaved.as_ref().unwrap().text, "é🦀abcdefghijklmnop\n");
            let lease = ws.update_in(cx, |ws, window, cx| {
                std::fs::write(&path, "new disk contents\n").unwrap();
                let lease = deferred.then(|| pipelines::lease(&path, cx));
                ws.restore(saved, window, cx);
                assert_eq!(ws.restoring.len(), if deferred { 2 } else { 0 });
                if deferred {
                    let pending = ws.capture(window, true, &mut Texts::new(), cx);
                    assert_eq!(pending.groups[0].views.len(), 2);
                    assert!(pending.groups[0].views.iter().all(|v| v.unsaved.is_some()));
                    assert_eq!(pending.groups[0].views[0].buffer, pending.groups[0].views[1].buffer);
                    assert!(pending.groups[0].views[1].folds.is_some());
                }
                lease
            });
            cx.run_until_parked();
            drop(lease);
            finish_restore(&ws, cx);
            ws.read_with(cx, |ws, cx| {
                let tabs: Vec<_> = ws.views.tabs().collect();
                assert_eq!(tabs.len(), 2, "{kind}, deferred={deferred}");
                assert!(tabs[0].read(cx).same_document(tabs[1].read(cx)));
                assert_eq!(tabs[0].read(cx).text(cx), "é🦀abcdefghijklmnop\n");
                assert!(tabs[0].read(cx).doc(cx).unsaved());
                assert!(ranges(tabs[0], cx).is_empty(), "{kind} opens expanded");
                assert_eq!(ranges(tabs[1], cx), one(9..12), "valid sibling metadata survives {kind}");
                assert_eq!(tabs[0].read(cx).state.cursor(), 2);
                assert_eq!(tabs[1].read(cx).state.cursor(), 2);
            });
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "new disk contents\n");
        }
    }
}
