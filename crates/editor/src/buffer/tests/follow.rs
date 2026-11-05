use super::*;

#[test]
fn follow_lock_protects_shared_text_history_metadata_and_saves() {
    let mut v = views("a😀\n b \n");
    v.a().insert("prior");
    v.doc.mark_saved();
    let before = (v.doc.text(), v.doc.save_point(), v.doc.history.undo.len());
    assert!(!v.doc.is_following());
    v.doc.set_following(true);
    let version = v.doc.version;
    {
        let mut e = v.a();
        e.select_all();
        e.insert("bad");
        e.type_text("(");
        e.toggle_overtype();
        e.type_literal("中");
        e.backspace();
        e.delete();
        e.newline();
        e.tab();
        e.paste("bad", false);
        e.indent();
        e.unindent();
        e.toggle_comment("//");
        e.delete_line();
        e.move_line_down();
        e.join_lines();
        e.permute_lines(Permute::Reverse);
        e.clean_whitespace(true);
        e.convert_indentation(false);
        e.replace_ranges(vec![(0..1, "bad".into())]);
        e.compose(std::slice::from_ref(&(0..1)), "bad", true);
        e.replace_all("a", "bad", &FindOpts::default());
        e.undo();
        e.redo();
        e.set_encoding(orlo_project::encoding::Encoding::Utf16Be, false);
        e.set_line_ending("\r\n");
        e.take_changes();
    }
    v.b().insert("peer");
    assert_eq!((v.doc.text(), v.doc.save_point(), v.doc.history.undo.len()), before);
    assert_eq!(v.doc.version, version);
    assert!(!v.doc.dirty);
    assert!(v.doc.capture_save(std::env::temp_dir().join("orlo-follow-never-written"), None).is_err());
    v.b().select_all();
    assert_eq!(v.b().copy_text().0, before.0);
    v.doc.set_following(false);
    v.b().insert("ordinary");
    assert!(v.doc.dirty);
}

#[test]
fn follow_does_not_change_ordinary_read_only_working_copy_editing() {
    let mut s = EditorState::from_text("source");
    s.doc.read_only = true;
    s.insert("working copy ");
    assert_eq!(s.text(), "working copy source");
    assert!(s.doc.dirty);
    s.doc.set_following(true);
    s.doc.set_following(false);
    assert!(s.doc.read_only);
    s.undo();
    assert_eq!(s.text(), "source");
}

#[test]
fn follow_snapshots_are_clean_shared_and_history_stays_bounded() {
    let mut v = views("start\n");
    v.a().insert("saved ");
    v.doc.mark_saved();
    v.doc.set_following(true);
    let history = v.doc.history.undo.len();
    let same = loaded(&v.doc.text());
    v.a().follow_from(same, &[]);
    assert_eq!(v.doc.history.undo.len(), history);
    v.b().set_cursor(2);
    for n in 1..=20 {
        let next = loaded(&format!("saved start\n{}", "中😀\n".repeat(n)));
        let changes = next.diff(&v.doc.rope);
        v.a().follow_from(next, &changes);
        assert_eq!(v.b().text(), format!("saved start\n{}", "中😀\n".repeat(n)));
        assert_eq!(v.b().cursor(), 2);
        assert!(!v.doc.dirty);
        assert!(v.doc.history.undo.is_empty() && v.doc.history.redo.is_empty());
    }
    v.doc.set_following(false);
    let text = v.doc.text();
    v.b().undo();
    assert_eq!(v.doc.text(), text);
    v.b().insert("new edit");
    v.b().undo();
    assert_eq!(v.doc.text(), text);
}

#[test]
fn follow_refuses_server_snippet_without_creating_a_session_or_selection() {
    let mut s = EditorState::from_text("a");
    s.doc.set_following(true);
    let selections = s.view.selections.clone();
    assert!(s.complete_snippet_ranges(0..1, "${1:large replacement}$0", Vec::new()).is_err());
    assert!(!s.in_snippet());
    assert_eq!(s.view.selections, selections);
}

#[test]
fn follow_rejects_streamed_outlines_along_with_their_text() {
    let mut s = EditorState::from_text("log");
    s.doc.set_following(true);
    let version = s.doc.version;
    s.append_outlined("unexpected result", std::iter::once(0..17));
    assert!(s.doc.outlines.is_empty());
    assert_eq!(s.text(), "log");
    assert_eq!(s.doc.version, version);
}

#[test]
fn follow_cut_preserves_caret_and_history_before_whole_line_expansion() {
    let mut s = EditorState::from_text("first\nlast");
    s.insert("saved ");
    s.doc.mark_saved();
    let end = s.len();
    s.set_cursor(end);
    s.doc.set_following(true);
    let before = (s.text(), s.doc.version, s.view.selections.clone(), s.doc.history.undo.len());
    s.cut();
    assert_eq!((s.text(), s.doc.version, s.view.selections.clone(), s.doc.history.undo.len()), before);
    s.doc.set_following(false);
    s.doc.read_only = true;
    assert_eq!(s.cut(), ("last\n".into(), true));
    assert_eq!(s.text(), "saved first\n");
    s.undo();
    assert_eq!(s.text(), "saved first\nlast");
}

#[test]
fn n23_viewer_preserves_dirty_shared_history_save_identity_and_reading() {
    let mut v = views("a😀\n b \n");
    assert!(!v.doc.viewer_mode());
    v.a().insert("dirty ");
    v.a().set_typing(false);
    v.a().insert("second ");
    v.a().undo();
    v.a().take_changes();
    let before = (v.doc.text(), v.doc.save_point(), v.doc.dirty, v.doc.history.undo.len(), v.doc.history.redo.len());
    let change = v.doc.set_viewer_mode(true).unwrap();
    assert!(change.changes.is_empty());
    let version = v.doc.version;
    assert!(v.doc.set_viewer_mode(true).is_none());
    for peer in [false, true] {
        let mut e = if peer { v.b() } else { v.a() };
        e.select_all();
        let sels = e.view.selections.clone();
        e.insert("bad");
        e.type_text("(");
        e.type_literal("中");
        e.backspace();
        e.delete();
        e.newline();
        e.tab();
        e.paste("bad", false);
        e.indent();
        e.unindent();
        e.toggle_comment("//");
        e.delete_line();
        e.move_line_down();
        e.join_lines();
        e.permute_lines(Permute::Reverse);
        e.clean_whitespace(true);
        e.convert_indentation(false);
        assert!(!e.replace_ranges(vec![(0..1, "bad".into())]));
        assert!(e.compose(std::slice::from_ref(&(0..1)), "bad", true).is_empty());
        assert_eq!(e.replace_all("a", "bad", &FindOpts::default()), 0);
        assert!(e.transform_document(|_| Ok("bad".into())).is_err());
        assert!(e.complete_snippet_ranges(0..1, "${1:bad}$0", Vec::new()).is_err());
        e.undo();
        e.redo();
        e.set_encoding(orlo_project::encoding::Encoding::Utf16Be, false);
        e.set_line_ending("\r\n");
        e.append_outlined("bad", std::iter::once(0..3));
        e.reset_text("bad");
        let next = loaded("bad");
        let changes = next.diff(&e.doc.rope);
        e.reload_from(next, &changes);
        assert_eq!(e.view.selections, sels);
        assert_eq!(e.copy_text().0, before.0);
        assert!(e.take_changes().is_empty());
    }
    assert_eq!(
        (v.doc.text(), v.doc.save_point(), v.doc.dirty, v.doc.history.undo.len(), v.doc.history.redo.len()),
        before
    );
    assert_eq!(v.doc.version, version);
    assert!(v.doc.unsaved());
    let request = v.doc.capture_save(std::env::temp_dir().join("orlo-viewer-not-written"), None).unwrap();
    assert_eq!(request.rope.to_string(), before.0);
    v.doc.set_viewer_mode(false);
    assert_eq!(v.doc.version, version + 1);
    v.b().redo();
    assert_ne!(v.doc.text(), before.0);
    v.b().undo();
    assert_eq!(v.doc.text(), before.0);
    v.b().set_cursor(0);
    v.b().insert("restored ");
    v.b().undo();
    assert_eq!(v.doc.text(), before.0);
    assert!(!Document::from_text(&before.0).viewer_mode());
}

#[test]
fn n23_viewer_off_preserves_follow_disk_protection_and_breaks_typing_groups() {
    let mut s = EditorState::from_text("source");
    s.insert("first");
    s.doc.read_only = true;
    s.doc.path = Some(std::env::current_exe().unwrap());
    s.doc.set_viewer_mode(true);
    s.doc.set_following(true);
    let next = loaded("incoming");
    let changes = next.diff(&s.doc.rope);
    s.follow_from(next, &changes);
    assert_eq!(s.text(), "firstsource");
    s.doc.set_viewer_mode(false);
    assert!(s.doc.read_only && s.doc.is_following() && s.doc.edits_blocked());
    s.insert("bad");
    assert_eq!(s.text(), "firstsource");
    s.doc.set_following(false);
    assert!(s.doc.check_save(s.doc.path.as_ref().unwrap()).is_err());
    s.insert("second");
    s.undo();
    assert_eq!(s.text(), "firstsource");
    s.undo();
    assert_eq!(s.text(), "source");
    assert!(s.doc.read_only);
}
