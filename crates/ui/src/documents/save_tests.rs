//! Saving off the UI thread, with the write held (`save::gate`) while the test acts: typing,
//! undo, a new encoding or line ending, another save, a close, a disk poll or a quit meanwhile.

use super::save::gate;
use crate::editing::encoding::Encoding;
use crate::workspace::ui_tests::{settle, temp, text, type_in, window};
use crate::workspace::*;
use gpui::{TestAppContext, VisualTestContext};

fn file(name: &str, text: &str) -> PathBuf {
    let dir = temp(name).canonicalize().unwrap();
    let p = dir.join("f.txt");
    std::fs::write(&p, text).unwrap();
    p
}

fn open(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, p: &Path) -> Entity<Editor> {
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_path(p.to_path_buf(), window, cx);
        ws.active_editor().unwrap()
    })
}

/// A window with `name`'s file, holding `text`, open in it and `typed` typed at its start.
fn opened<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
    text: &str,
    typed: &str,
) -> (PathBuf, Entity<Workspace>, &'a mut VisualTestContext, Entity<Editor>) {
    let p = file(name, text);
    let (ws, vcx) = window(cx);
    let e = open(&ws, vcx, &p);
    type_in(&ws, vcx, typed);
    (p, ws, vcx, e)
}

fn save(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
}

/// Save As, with `to` chosen in the panel.
fn save_as(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, to: &Path) {
    ws.update_in(vcx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
    let chosen = to.to_path_buf();
    vcx.simulate_new_path_selection(move |_| Some(chosen.clone()));
}

fn close(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, e: &Entity<Editor>) {
    ws.update_in(vcx, |ws, window, cx| ws.request_close(e.clone(), After::Close, window, cx));
}

fn is_open(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, e: &Entity<Editor>) -> bool {
    ws.read_with(vcx, |ws, _| ws.views.find(e).is_some())
}

fn poll(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    ws.update_in(vcx, |ws, window, cx| ws.poll_disk(window, cx));
    settle(vcx);
}

fn undo(e: &Entity<Editor>, vcx: &mut VisualTestContext) {
    e.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
}

fn status(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Option<String> {
    ws.read_with(vcx, |ws, _| ws.status.as_ref().map(|s| s.0.clone()))
}

fn prompt(vcx: &mut VisualTestContext) -> Option<String> {
    vcx.pending_prompt().map(|p| p.0)
}

fn chmod(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn dirty(e: &Entity<Editor>, vcx: &mut VisualTestContext) -> bool {
    e.read_with(vcx, |e, cx| e.doc(cx).dirty)
}

fn on_disk(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

/// A save to `p` whose write is held at its start.
fn held_save(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, p: &Path) -> gate::Pause {
    let pause = gate::pause(p);
    save(ws, vcx);
    vcx.run_until_parked();
    assert_eq!(pause.arrived(), 1, "the write is under way, held");
    pause
}

/// The held write goes on, and lands.
fn land(pause: &gate::Pause, vcx: &mut VisualTestContext) {
    pause.release();
    vcx.run_until_parked();
}

#[gpui::test]
fn typing_during_a_save_leaves_the_document_unsaved_and_the_file_with_the_captured_text(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-typing", "a\n", "b");
    let pause = held_save(&ws, vcx, &p);
    // typing goes on while the write is under way
    type_in(&ws, vcx, "c");
    assert_eq!(on_disk(&p), "a\n", "nothing written yet");
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n", "the text as it was when saved");
    assert_eq!(text(&ws, vcx), "bca\n");
    assert!(dirty(&e, vcx), "the typing since is unsaved");
    // undoing it comes back to what the file holds: clean
    undo(&e, vcx);
    assert_eq!(text(&ws, vcx), "ba\n");
    assert!(!dirty(&e, vcx));
}

#[gpui::test]
fn undoing_past_the_captured_text_and_editing_differently_during_a_save_leaves_no_saved_point(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-branch", "a\n", "b");
    let pause = held_save(&ws, vcx, &p);
    undo(&e, vcx);
    type_in(&ws, vcx, "z");
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n");
    assert!(dirty(&e, vcx));
    // the text written was on the branch the edit replaced: no undo or redo gets back to it
    for _ in 0..3 {
        undo(&e, vcx);
        assert!(dirty(&e, vcx));
    }
    for _ in 0..3 {
        e.update(vcx, |e, cx| e.edit(cx, |s| s.redo()));
        assert!(dirty(&e, vcx));
    }
    assert!(e.read_with(vcx, |e, cx| e.doc(cx).saved_text()).is_none());
}

#[gpui::test]
fn a_new_encoding_line_ending_or_byte_order_mark_during_a_save_stays_unsaved(cx: &mut TestAppContext) {
    for change in ["encoding", "line ending", "bom"] {
        let name = format!("orlo-save-format-{}", change.replace(' ', "-"));
        let (p, ws, vcx, e) = opened(cx, &name, "a\n", "b");
        let pause = held_save(&ws, vcx, &p);
        e.update(vcx, |e, cx| {
            e.update_doc(cx, |d| match change {
                "encoding" => d.set_encoding(Encoding::Utf16Le, false),
                "line ending" => d.set_line_ending("\r\n"),
                _ => d.set_encoding(Encoding::Utf8, true),
            })
        });
        land(&pause, vcx);
        assert_eq!(on_disk(&p), "ba\n", "{change}: written as it was captured");
        assert_eq!(text(&ws, vcx), "ba\n");
        assert!(dirty(&e, vcx), "{change}: the file is not written that way yet");
    }
}

#[gpui::test]
fn saving_again_during_a_save_writes_the_newer_text_after_the_first(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-again", "a\n", "1");
    let first = held_save(&ws, vcx, &p);
    type_in(&ws, vcx, "2");
    save(&ws, vcx);
    vcx.run_until_parked();
    assert_eq!(first.arrived(), 1, "merged: one write at a time");
    // the first lands, then the second is taken and written, held in turn
    first.release();
    let second = gate::pause(&p);
    vcx.run_until_parked();
    assert_eq!(on_disk(&p), "1a\n", "the first, alone");
    assert_eq!(second.arrived(), 1, "the merged save, from a fresh capture");
    assert!(dirty(&e, vcx));
    type_in(&ws, vcx, "3");
    second.release();
    vcx.run_until_parked();
    assert_eq!(on_disk(&p), "12a\n", "the newer text, last");
    assert!(dirty(&e, vcx), "the 3 came after the second capture");
}

#[gpui::test]
fn closing_with_a_save_asks_again_when_the_document_changed_during_it(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-close", "a\n", "b");
    let pause = gate::pause(&p);
    close(&ws, vcx, &e);
    vcx.simulate_prompt_answer("Save");
    vcx.run_until_parked();
    assert_eq!(pause.arrived(), 1);
    type_in(&ws, vcx, "c");
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n");
    assert!(vcx.has_pending_prompt(), "asked about the edit made during the save");
    assert!(is_open(&ws, vcx, &e), "still open");
    // Don't Save now closes it, with the edit lost as asked
    vcx.simulate_prompt_answer("Don't Save");
    vcx.run_until_parked();
    assert!(!is_open(&ws, vcx, &e));
}

#[gpui::test]
fn a_close_after_a_clean_save_closes_once_it_has_landed(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-close-clean", "a\n", "b");
    let pause = gate::pause(&p);
    close(&ws, vcx, &e);
    vcx.simulate_prompt_answer("Save");
    vcx.run_until_parked();
    assert!(is_open(&ws, vcx, &e), "not before it lands");
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n");
    assert!(!is_open(&ws, vcx, &e));
}

#[gpui::test]
fn save_as_during_a_save_writes_both_files_in_turn(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-as-pending", "a\n", "b");
    let to = p.with_file_name("copy.txt");
    let pause = held_save(&ws, vcx, &p);
    type_in(&ws, vcx, "c");
    save_as(&ws, vcx, &to);
    vcx.run_until_parked();
    assert!(!to.exists(), "waits for the save under way");
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n");
    assert_eq!(on_disk(&to), "bca\n");
    e.read_with(vcx, |e, cx| {
        assert_eq!(e.doc(cx).path.as_deref(), Some(to.as_path()));
        assert!(!e.doc(cx).dirty);
    });
}

#[gpui::test]
fn a_failed_background_save_says_why_and_takes_the_stamp_it_left(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-fails-held", "old\n", "new ");
    let dir = p.parent().unwrap();
    let pause = held_save(&ws, vcx, &p);
    chmod(&p, 0o444);
    chmod(dir, 0o555);
    land(&pause, vcx);
    chmod(dir, 0o755);
    assert_eq!(prompt(vcx).as_deref(), Some("Could not save file"));
    vcx.simulate_prompt_answer("Ok");
    assert!(dirty(&e, vcx));
    assert_eq!(on_disk(&p), "old\n");
    // nothing on disk changed that Orlo did not know of: the poll asks nothing
    poll(&ws, vcx);
    assert!(!vcx.has_pending_prompt());
    chmod(&p, 0o644);
}

#[gpui::test]
fn a_background_save_writes_a_links_target_hard_links_in_place_and_in_a_locked_folder(cx: &mut TestAppContext) {
    use std::os::unix::fs::MetadataExt;
    let dir = temp("orlo-save-links").canonicalize().unwrap();
    let target = dir.join("target.txt");
    std::fs::write(&target, "t\n").unwrap();
    let link = dir.join("link.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let hard = dir.join("hard.txt");
    std::fs::write(&hard, "h\n").unwrap();
    let other_name = dir.join("other-name.txt");
    std::fs::hard_link(&hard, &other_name).unwrap();
    let locked_dir = dir.join("locked");
    std::fs::create_dir(&locked_dir).unwrap();
    let locked = locked_dir.join("f.txt");
    std::fs::write(&locked, "l\n").unwrap();
    chmod(&locked_dir, 0o555);
    let (ws, vcx) = window(cx);
    for (path, written) in [(&link, &target), (&hard, &hard), (&locked, &locked)] {
        open(&ws, vcx, path);
        type_in(&ws, vcx, "x");
        let pause = held_save(&ws, vcx, written);
        land(&pause, vcx);
        assert!(on_disk(written).starts_with('x'), "{path:?}");
    }
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(on_disk(&other_name), on_disk(&hard), "the other name sees it: in place");
    let ino = |p: &Path| std::fs::metadata(p).unwrap().ino();
    assert_eq!(ino(&hard), ino(&other_name));
    chmod(&locked_dir, 0o755);
}

#[gpui::test]
fn a_disk_poll_during_a_save_asks_nothing_and_reloads_nothing(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-poll", "a\n", "b");
    let pause = held_save(&ws, vcx, &p);
    // a poll started while the write is under way leaves the file alone
    ws.update_in(vcx, |ws, window, cx| ws.poll_disk(window, cx));
    type_in(&ws, vcx, "c");
    pause.release();
    settle(vcx);
    poll(&ws, vcx);
    assert!(!vcx.has_pending_prompt(), "its own write is not an outside change");
    assert_eq!(on_disk(&p), "ba\n");
    assert_eq!(text(&ws, vcx), "bca\n", "nothing reloaded");
    assert!(dirty(&e, vcx));
}

#[gpui::test]
fn a_disk_poll_landing_during_a_write_is_left_for_the_next_one(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-poll-lands", "a\n", "b");
    // the poll's stat is under way when a write takes the file and changes it
    let lease = ws.update_in(vcx, |ws, window, cx| {
        ws.poll_disk(window, cx);
        pipelines::lease(&p, cx)
    });
    std::fs::write(&p, "written\n").unwrap();
    settle(vcx);
    assert!(!vcx.has_pending_prompt(), "not asked about while the file is being written");
    assert_eq!(text(&ws, vcx), "ba\n");
    assert!(dirty(&e, vcx));
    drop(lease);
}

#[gpui::test]
fn quitting_during_a_save_waits_for_it(cx: &mut TestAppContext) {
    let (p, ws, vcx, _) = opened(cx, "orlo-save-quit", "a\n", "b");
    let pause = held_save(&ws, vcx, &p);
    let session = session::path();
    let _ = std::fs::remove_file(&session);
    vcx.cx.update(quit);
    vcx.run_until_parked();
    assert!(!session.exists(), "the session waits for the save");
    assert_eq!(status(&ws, vcx).as_deref(), Some("Saving…"));
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n");
    let v = &session::load(&session).windows[0].groups[0].views[0];
    assert!(v.unsaved.is_none(), "written after the save landed: clean");
}

#[gpui::test]
fn save_as_then_save_during_a_save_writes_every_file_asked_for(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-queue", "a\n", "b");
    let q = p.with_file_name("other.txt");
    let pause = held_save(&ws, vcx, &p);
    save_as(&ws, vcx, &q);
    vcx.run_until_parked();
    // a Save after the Save As goes where the Save As leaves the document
    type_in(&ws, vcx, "c");
    save(&ws, vcx);
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n", "the first save, alone");
    assert_eq!(on_disk(&q), "bca\n", "the Save As, then the Save to the same file");
    e.read_with(vcx, |e, cx| {
        assert_eq!(e.doc(cx).path.as_deref(), Some(q.as_path()));
        assert!(!e.doc(cx).dirty);
    });
}

#[gpui::test]
fn save_all_writes_a_document_shown_twice_once(cx: &mut TestAppContext) {
    let p = file("orlo-save-all-views", "a\n");
    let (ws, vcx) = window(cx);
    open(&ws, vcx, &p);
    ws.update_in(vcx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    type_in(&ws, vcx, "b");
    let first = gate::pause(&p);
    ws.update_in(vcx, |ws, window, cx| ws.save_all(&SaveAll, window, cx));
    vcx.run_until_parked();
    assert_eq!(first.arrived(), 1);
    first.release();
    let second = gate::pause(&p);
    vcx.run_until_parked();
    assert_eq!(on_disk(&p), "ba\n");
    assert_eq!(second.arrived(), 0, "one write for the two views");
}

#[gpui::test]
fn an_editorconfig_line_ending_taken_at_open_keeps_the_file_clean_through_undo(cx: &mut TestAppContext) {
    let p = file("orlo-save-editorconfig-open", "a\nb\n");
    let dir = p.parent().unwrap().to_path_buf();
    std::fs::write(dir.join(".editorconfig"), "root=true\n[*.txt]\nend_of_line=crlf\n").unwrap();
    let (ws, vcx) = window(cx);
    let e = open(&ws, vcx, &p);
    e.read_with(vcx, |e, cx| {
        assert_eq!(e.doc(cx).line_ending, "\r\n");
        assert!(!e.doc(cx).dirty);
    });
    type_in(&ws, vcx, "x");
    undo(&e, vcx);
    assert!(!dirty(&e, vcx), "back at the text opened: closing does not ask");
}

#[gpui::test]
fn closing_the_last_window_waits_for_a_save_whose_tab_was_closed(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-close-window", "a\n", "b");
    let pause = held_save(&ws, vcx, &p);
    // its tab closed without saving again: the save under way goes on
    close(&ws, vcx, &e);
    vcx.simulate_prompt_answer("Don't Save");
    vcx.run_until_parked();
    assert!(!is_open(&ws, vcx, &e));
    // closing the last window quits Orlo: not with the write cut off
    assert!(!vcx.simulate_close(), "the window waits for the save");
    assert_eq!(status(&ws, vcx).as_deref(), Some("Saving…"));
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n");
    assert!(vcx.cx.update(|cx| cx.windows().is_empty()), "then it closes");
}

#[gpui::test]
fn a_failed_write_in_place_leaves_no_saved_point_to_undo_back_to(cx: &mut TestAppContext) {
    let p = file("orlo-save-fails-in-place", "h\n");
    std::fs::hard_link(&p, p.with_file_name("other-name.txt")).unwrap();
    let (ws, vcx) = window(cx);
    let e = open(&ws, vcx, &p);
    type_in(&ws, vcx, "x");
    let pause = held_save(&ws, vcx, &p);
    // a hard-linked file is written in place: the failure comes after the file was cut
    crate::editing::text::FAULT.set(Some(("write", std::io::ErrorKind::Other)));
    land(&pause, vcx);
    crate::editing::text::FAULT.set(None);
    assert_eq!(prompt(vcx).as_deref(), Some("Could not save file"));
    vcx.simulate_prompt_answer("Ok");
    assert_ne!(on_disk(&p), "h\n", "the original is gone");
    // back at the text first opened: not what the file holds now
    undo(&e, vcx);
    assert_eq!(text(&ws, vcx), "h\n");
    assert!(dirty(&e, vcx), "closing must still ask");
}

#[gpui::test]
fn two_names_of_one_file_are_not_written_at_once(cx: &mut TestAppContext) {
    let a = file("orlo-save-hard-links", "x\n");
    let b = a.with_file_name("b.txt");
    std::fs::hard_link(&a, &b).unwrap();
    let (ws, vcx) = window(cx);
    open(&ws, vcx, &a);
    type_in(&ws, vcx, "longer text from a ");
    open(&ws, vcx, &b);
    type_in(&ws, vcx, "b ");
    let (held_a, held_b) = (gate::pause(&a), gate::pause(&b));
    ws.update_in(vcx, |ws, window, cx| ws.save_all(&SaveAll, window, cx));
    vcx.run_until_parked();
    assert_eq!(held_a.arrived() + held_b.arrived(), 1, "one write to the file at a time, whatever its name");
    // the other waits for the file, then writes it whole
    held_a.release();
    held_b.release();
    settle(vcx);
    assert!(!vcx.cx.update(|cx| Documents::saving(cx)), "both landed");
    let written = on_disk(&a);
    assert!(["longer text from a x\n", "b x\n"].contains(&written.as_str()), "{written:?}");
}

#[gpui::test]
fn a_signal_during_a_save_waits_for_it(cx: &mut TestAppContext) {
    let (p, ws, vcx, _) = opened(cx, "orlo-save-signal", "a\n", "b");
    let pause = held_save(&ws, vcx, &p);
    let session = session::path();
    let _ = std::fs::remove_file(&session);
    // what `quit_on_signals`'s thread hands the UI thread when a signal arrives
    let arrived = crate::app::handoff::Inbox::default();
    vcx.update(|_, cx| quit_on(arrived.clone(), cx));
    arrived.push(Vec::new());
    vcx.run_until_parked();
    assert!(!session.exists(), "the session waits for the save");
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "ba\n");
    let v = &session::load(&session).windows[0].groups[0].views[0];
    assert!(v.unsaved.is_none(), "written after the save landed: clean");
}

#[gpui::test]
fn closing_a_window_twice_while_it_waits_for_a_save_asks_once(cx: &mut TestAppContext) {
    let (p, ws, vcx, _) = opened(cx, "orlo-save-close-twice", "a\n", "b");
    let pause = held_save(&ws, vcx, &p);
    // edited during the save: once it lands, closing asks about the edit
    type_in(&ws, vcx, "c");
    assert!(!vcx.simulate_close());
    assert!(!vcx.simulate_close(), "clicked again while it waits");
    land(&pause, vcx);
    assert_eq!(prompt(vcx).as_deref(), Some("Save all changes before closing?"));
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();
    assert!(!vcx.has_pending_prompt(), "one sheet, not two");
    assert_eq!(vcx.cx.update(|cx| cx.windows().len()), 1);
}

#[gpui::test]
fn the_settings_a_save_applies_are_the_text_written_not_typing_since(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let p = settings::path();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "\"tab_size\":3}\n").unwrap();
    let e = open(&ws, vcx, &p);
    let p = e.read_with(vcx, |e, cx| e.doc(cx).path.clone().unwrap());
    type_in(&ws, vcx, "{");
    let pause = held_save(&ws, vcx, &p);
    // the buffer stops being a settings file the moment after it is taken
    type_in(&ws, vcx, "x");
    land(&pause, vcx);
    assert_eq!(on_disk(&p), "{\"tab_size\":3}\n");
    assert!(!vcx.has_pending_prompt(), "what was written parses");
    assert_eq!(vcx.cx.update(|cx| cx.global::<Settings>().tab_size), 3);
}

#[gpui::test]
fn a_save_that_waited_its_turn_is_cleaned_up_when_it_is_taken(cx: &mut TestAppContext) {
    let p = file("orlo-save-queued-cleanup", "a\n");
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.apply_settings(r#"{"trim_trailing_white_space_on_save":true}"#, window, cx));
    let e = open(&ws, vcx, &p);
    type_in(&ws, vcx, "b");
    let first = held_save(&ws, vcx, &p);
    save(&ws, vcx);
    // typed after the second save was asked for, before its turn
    type_in(&ws, vcx, "foo   \n");
    first.release();
    let second = gate::pause(&p);
    vcx.run_until_parked();
    assert_eq!(second.arrived(), 1);
    second.release();
    vcx.run_until_parked();
    assert_eq!(on_disk(&p), "bfoo\na\n", "trimmed as it was taken");
    assert_eq!(text(&ws, vcx), "bfoo\na\n");
    assert!(!dirty(&e, vcx));
}

#[gpui::test]
fn a_save_after_a_save_as_that_fails_writes_the_documents_own_file(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-after-failed-save-as", "a\n", "b");
    let locked = p.with_file_name("locked");
    std::fs::create_dir(&locked).unwrap();
    let q = locked.join("copy.txt");
    let pause = gate::pause(&q);
    save_as(&ws, vcx, &q);
    vcx.run_until_parked();
    assert_eq!(pause.arrived(), 1, "the Save As is under way");
    chmod(&locked, 0o555);
    type_in(&ws, vcx, "c");
    save(&ws, vcx);
    land(&pause, vcx);
    chmod(&locked, 0o755);
    assert_eq!(prompt(vcx).as_deref(), Some("Could not save file"));
    vcx.simulate_prompt_answer("Ok");
    assert!(!vcx.has_pending_prompt(), "the Save went where the document is");
    assert!(!q.exists());
    assert_eq!(on_disk(&p), text(&ws, vcx));
    e.read_with(vcx, |e, cx| {
        assert_eq!(e.doc(cx).path.as_deref(), Some(p.as_path()));
        assert!(!e.doc(cx).dirty);
    });
}

#[gpui::test(iterations = 20)]
fn saves_of_two_documents_to_one_file_write_in_the_order_they_were_taken(cx: &mut TestAppContext) {
    let p = file("orlo-save-turns", "a\n");
    let other = p.with_file_name("other.txt");
    std::fs::write(&other, "o\n").unwrap();
    let (ws, vcx) = window(cx);
    let a = open(&ws, vcx, &p);
    type_in(&ws, vcx, "older ");
    let b = open(&ws, vcx, &other);
    type_in(&ws, vcx, "newer ");
    let pause = gate::pause(&p);
    // a save of the file, then a Save As of another document over it
    vcx.update(|_, cx| {
        for e in [&a, &b] {
            let d = e.read(cx).document.clone();
            OpenDocument::save(&d, Some(p.clone()), None, |_| None, |_, _| {}, cx);
        }
    });
    vcx.run_until_parked();
    assert_eq!(pause.arrived(), 1, "one write at a time");
    pause.release();
    settle(vcx);
    assert_eq!(on_disk(&p), "newer o\n", "the one taken last lands last");
}

#[gpui::test]
fn reverting_during_a_save_reads_the_file_once_the_save_has_landed(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-save-revert", "a\n", "b");
    let pause = held_save(&ws, vcx, &p);
    ws.update_in(vcx, |ws, window, cx| ws.revert(&Revert, window, cx));
    vcx.run_until_parked();
    pause.release();
    settle(vcx);
    assert_eq!(on_disk(&p), "ba\n");
    assert_eq!(text(&ws, vcx), "ba\n", "what the file holds");
    assert!(!dirty(&e, vcx));
}

#[gpui::test]
fn a_disk_poll_leaves_the_settings_file_alone_while_it_is_written(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let p = settings::path();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    // the poll's stat is under way when a write takes the file, half written when it lands
    let lease = ws.update_in(vcx, |ws, window, cx| {
        ws.poll_disk(window, cx);
        pipelines::lease(&p, cx)
    });
    std::fs::write(&p, "{\"tab_size\":").unwrap();
    settle(vcx);
    assert!(!vcx.has_pending_prompt(), "not read while it is written");
    // and one started while it is written
    poll(&ws, vcx);
    assert!(!vcx.has_pending_prompt());
    // written: the next poll applies it
    std::fs::write(&p, "{\"tab_size\":5}").unwrap();
    drop(lease);
    poll(&ws, vcx);
    assert!(!vcx.has_pending_prompt());
    assert_eq!(vcx.cx.update(|cx| cx.global::<Settings>().tab_size), 5);
}

#[gpui::test]
fn a_disk_poll_during_a_save_of_the_settings_file_applies_nothing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let p = settings::path();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "\"tab_size\":3}\n").unwrap();
    let e = open(&ws, vcx, &p);
    let p = e.read_with(vcx, |e, cx| e.doc(cx).path.clone().unwrap());
    type_in(&ws, vcx, "{");
    let pause = held_save(&ws, vcx, &p);
    // the write under way, in place and half done, when a poll looks
    std::fs::write(&p, "{\"tab").unwrap();
    poll(&ws, vcx);
    assert!(!vcx.has_pending_prompt(), "its own write is not an outside edit");
    pause.release();
    settle(vcx);
    assert!(!vcx.has_pending_prompt());
    assert_eq!(vcx.cx.update(|cx| cx.global::<Settings>().tab_size), 3);
}

#[gpui::test]
fn file_autosave_defaults_off_then_writes_existing_unicode_file(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "orlo-file-autosave-basic", "old\n", "界😀");
    vcx.run_until_parked();
    vcx.executor().advance_clock(std::time::Duration::from_secs(2));
    vcx.run_until_parked();
    assert_eq!(on_disk(&p), "old\n");
    ws.update_in(vcx, |ws, w, cx| ws.apply_settings(r#"{"file_autosave":true}"#, w, cx));
    vcx.run_until_parked();
    vcx.executor().advance_clock(std::time::Duration::from_millis(1999));
    vcx.run_until_parked();
    assert_eq!(on_disk(&p), "old\n", "the default waits a full two seconds");
    assert!(dirty(&e, vcx));
    vcx.executor().advance_clock(std::time::Duration::from_millis(1));
    vcx.run_until_parked();
    assert_eq!(on_disk(&p), "界😀old\n");
    assert!(!dirty(&e, vcx));
}

fn auto_enable(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, w, cx| ws.apply_settings(r#"{"file_autosave":true,"lsp_enabled":false}"#, w, cx));
    cx.run_until_parked();
}
fn auto_tick(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
}

#[gpui::test]
fn file_autosave_debounces_and_disable_cancels_pending_work(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-debounce", "a", "b");
    auto_enable(&ws, cx);
    cx.executor().advance_clock(std::time::Duration::from_millis(1200));
    type_in(&ws, cx, "界");
    cx.run_until_parked();
    cx.executor().advance_clock(std::time::Duration::from_millis(1200));
    cx.run_until_parked();
    assert_eq!(on_disk(&p), "a");
    ws.update_in(cx, |ws, w, cx| ws.apply_settings(r#"{"file_autosave":false}"#, w, cx));
    auto_tick(cx);
    assert_eq!(on_disk(&p), "a");
    assert!(dirty(&e, cx));
    auto_enable(&ws, cx);
    auto_tick(cx);
    assert_eq!(on_disk(&p), text(&ws, cx));
}

/// Run the scheduler at exact boundaries while retaining actual protected writes.
fn auto_advance(cx: &mut VisualTestContext, millis: u64) {
    cx.run_until_parked();
    cx.executor().advance_clock(std::time::Duration::from_millis(millis));
    cx.run_until_parked();
}

#[gpui::test]
fn file_autosave_rapid_edits_coalesce_at_the_last_default_deadline(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-two-second-coalesce", "base", "a");
    auto_enable(&ws, cx);
    let held = gate::pause(&p);
    auto_advance(cx, 900);
    type_in(&ws, cx, "b");
    auto_advance(cx, 900);
    type_in(&ws, cx, "界");
    auto_advance(cx, 1999);
    assert_eq!(held.arrived(), 0);
    assert_eq!(on_disk(&p), "base");
    auto_advance(cx, 1);
    assert_eq!(held.arrived(), 1, "only the final edit's timer captures a write");
    held.release();
    cx.run_until_parked();
    assert_eq!(on_disk(&p), "ab界base");
    assert!(!dirty(&e, cx));
    let later = gate::pause(&p);
    auto_advance(cx, 4000);
    assert_eq!(later.arrived(), 0, "no stale timer captures a second write");
    assert_eq!(on_disk(&p), "ab界base");
    assert!(!e.read_with(cx, |e, cx| e.document.read(cx).saving()));
}

#[gpui::test]
fn file_autosave_delay_override_reschedules_pending_policy_and_project_layer(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-delay-policy", "base", "a");
    auto_enable(&ws, cx);
    auto_advance(cx, 1000);
    assert_eq!(on_disk(&p), "base");
    // A longer delay restarts from this settings change, rather than the first edit.
    ws.update_in(cx, |ws, w, cx| {
        ws.apply_settings(r#"{"file_autosave":true,"file_autosave_delay_ms":3000,"lsp_enabled":false}"#, w, cx)
    });
    auto_advance(cx, 2999);
    assert_eq!(on_disk(&p), "base", "the old default timer must not publish");
    auto_advance(cx, 1);
    assert_eq!(on_disk(&p), "abase");
    type_in(&ws, cx, "b");
    auto_advance(cx, 500);
    ws.update(cx, |ws, cx| {
        let (project, warnings) = crate::config::project::Project::parse(
            &p.with_extension("sublime-project"),
            r#"{"settings":{"file_autosave_delay_ms":250,"overrides":[{"path":"*.txt","settings":{"file_autosave_delay_ms":125}}]}}"#,
        ).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        ws.project = Some(project);
        ws.apply_view(cx);
    });
    auto_advance(cx, 100);
    ws.update(cx, |ws, cx| ws.apply_view(cx));
    auto_advance(cx, 24);
    assert_eq!(on_disk(&p), "abase", "unchanged policy refresh must not restart the timer");
    auto_advance(cx, 1);
    assert_eq!(on_disk(&p), "abbase");
    assert!(!dirty(&e, cx));
}

#[gpui::test]
fn file_autosave_owner_transfer_restarts_the_surviving_views_delay(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let p = file("orlo-auto-delay-owner", "base");
    let first = open(&a, &mut acx, &p);
    let second = open(&b, &mut bcx, &p);
    a.update_in(&mut acx, |ws, w, cx| {
        ws.set_syntax(&SetSyntax { name: "Rust".into() }, w, cx);
        ws.apply_settings(r#"{"lsp_enabled":false,"file_autosave":true,"file_autosave_delay_ms":3000,"overrides":[{"language":"Rust","settings":{"file_autosave_delay_ms":500}}]}"#, w, cx);
    });
    type_in(&b, &mut bcx, "a");
    auto_advance(&mut acx, 499);
    assert_eq!(on_disk(&p), "base");
    a.update_in(&mut acx, |ws, w, cx| ws.close_editor(&first, w, cx));
    auto_advance(&mut bcx, 2999);
    assert_eq!(on_disk(&p), "base", "the former owner's short timer must not write");
    auto_advance(&mut bcx, 1);
    assert_eq!(on_disk(&p), "abase");
    assert!(!dirty(&second, &mut bcx));
}

#[gpui::test]
fn file_autosave_skips_untitled_missing_read_only_and_follow(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-skips", "old", "draft");
    auto_enable(&ws, cx);
    e.update(cx, |e, cx| e.update_doc(cx, |d| d.scratch = true));
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old");
    e.update(cx, |e, cx| {
        e.update_doc(cx, |d| {
            d.scratch = false;
            d.read_only = true;
        })
    });
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old");
    e.update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = false));
    std::fs::remove_file(&p).unwrap();
    auto_tick(cx);
    assert!(!p.exists());
    ws.update_in(cx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
    type_in(&ws, cx, "unsaved😀");
    auto_tick(cx);
    assert!(!cx.has_pending_prompt());
    assert!(ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).path.is_none()));
    std::fs::write(&p, "restored").unwrap();
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(p.clone(), w, cx);
        ws.revert(&Revert, w, cx);
    });
    settle(cx);
    ws.update_in(cx, |ws, w, cx| ws.follow_log(&FollowLog, w, cx));
    assert!(e.read_with(cx, |e, cx| e.doc(cx).is_following()));
    auto_tick(cx);
    assert_eq!(on_disk(&p), "restored");
}

#[gpui::test]
fn file_autosave_protects_external_change_even_after_poll_acknowledges_stamp(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-external", "old", "mine");
    auto_enable(&ws, cx);
    std::fs::write(&p, "external😀").unwrap();
    // Ordinary polling acknowledges before asking. That must not become write authority.
    e.update(cx, |e, cx| e.update_doc(cx, |d| d.disk = crate::files::watch::stamp(&p)));
    auto_tick(cx);
    assert_eq!(on_disk(&p), "external😀");
    assert!(status(&ws, cx).unwrap().contains("changed on disk"));
    assert!(!cx.has_pending_prompt());
    auto_tick(cx);
    assert_eq!(on_disk(&p), "external😀");
    assert!(dirty(&e, cx));
}

#[gpui::test]
fn file_autosave_edits_during_write_debounce_another_serial_save(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-edits", "old", "one");
    auto_enable(&ws, cx);
    let held = gate::pause(&p);
    auto_tick(cx);
    assert_eq!(held.arrived(), 1);
    type_in(&ws, cx, "two");
    auto_tick(cx);
    assert_eq!(held.arrived(), 1);
    held.release();
    cx.run_until_parked();
    assert_eq!(on_disk(&p), "oneold");
    assert!(dirty(&e, cx));
    auto_tick(cx);
    assert_eq!(on_disk(&p), "onetwoold");
    assert!(!dirty(&e, cx));
}

#[gpui::test]
fn file_autosave_codec_only_fix_retries_failed_conversion(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-codec", "old", "界");
    e.update(cx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Iso8859_1, false)));
    auto_enable(&ws, cx);
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old");
    assert!(status(&ws, cx).unwrap().contains("Automatic save paused"));
    assert!(dirty(&e, cx));
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old", "failed prepared text remains paused");
    assert!(dirty(&e, cx));
    assert!(status(&ws, cx).unwrap().contains("Automatic save paused"));
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    e.update(cx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Utf8, false)));
    assert_eq!(e.read_with(cx, |e, cx| e.doc(cx).version), version);
    auto_tick(cx);
    assert_eq!(on_disk(&p), "界old");
}

#[gpui::test]
fn file_autosave_last_view_close_and_quit_cancel_retained_documents(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-close", "old", "mine");
    auto_enable(&ws, cx);
    close(&ws, cx, &e);
    cx.simulate_prompt_answer("Don't Save");
    auto_tick(cx);
    assert!(!is_open(&ws, cx, &e));
    assert_eq!(on_disk(&p), "old");
    let e = open(&ws, cx, &p);
    type_in(&ws, cx, "quit");
    cx.run_until_parked();
    cx.cx.update(crate::workspace::sessions::app_quitting);
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old");
    assert!(dirty(&e, cx));
}

#[gpui::test]
fn file_autosave_ime_in_another_window_preserves_preview_until_commit(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let p = file("orlo-auto-ime", "old😀");
    let first = open(&a, &mut acx, &p);
    let second = open(&b, &mut bcx, &p);
    auto_enable(&a, &mut acx);
    second.update_in(&mut bcx, |e, w, cx| {
        e.state.overtype = true;
        e.replace_and_mark_text_in_range(None, "界", None, w, cx);
    });
    let before = second.read_with(&bcx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
    auto_tick(&mut acx);
    auto_tick(&mut acx);
    assert_eq!(on_disk(&p), "old😀");
    assert_eq!(before, second.read_with(&bcx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)));
    second.update_in(&mut bcx, |e, w, cx| {
        let marked = e.marked_text_range(w, cx);
        e.replace_text_in_range(marked, "中", w, cx);
    });
    auto_tick(&mut acx);
    assert_eq!(on_disk(&p), "中ld😀");
    assert_eq!(first.read_with(&acx, |e, cx| e.text(cx)), "中ld😀");
}

#[gpui::test]
fn three_coalesced_async_save_preparations_capture_latest_and_settle_every_waiter(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    let (p, ws, cx, e) = opened(cx, "orlo-auto-three-preparations", "base", "initial");
    let held = held_save(&ws, cx, &p);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let idle = Rc::new(std::cell::Cell::new(false));
    let mut releases = Vec::new();
    for n in 0..3 {
        let prep_path = p.join(format!("prep-{n}"));
        releases.push(gate::pause(&prep_path));
        let (prep_calls, done_calls, view) = (calls.clone(), calls.clone(), e.clone());
        let document = e.read_with(cx, |e, _| e.document.clone());
        cx.update(|_, cx| {
            OpenDocument::save(
                &document,
                None,
                None,
                move |cx| {
                    prep_calls.borrow_mut().push(format!("prepare{n}"));
                    Some(cx.spawn(async move |cx| {
                        gate::wait(&prep_path).await;
                        cx.update(|cx| view.update(cx, |e, cx| e.edit(cx, |s| s.insert(&format!("{n}"))))).unwrap();
                    }))
                },
                move |outcome, _| {
                    assert!(outcome.is_ok());
                    done_calls.borrow_mut().push(format!("done{n}"));
                },
                cx,
            );
        });
    }
    let d = e.read_with(cx, |e, _| e.document.clone());
    let done = idle.clone();
    cx.update(|_, cx| OpenDocument::when_saved(&d, move |_| done.set(true), cx));
    held.release();
    cx.run_until_parked();
    assert_eq!(&*calls.borrow(), &["prepare0"]);
    assert!(!idle.get());
    for (n, release) in releases.into_iter().enumerate() {
        release.release();
        cx.run_until_parked();
        if n < 2 {
            assert_eq!(on_disk(&p), "initialbase");
            assert!(!idle.get());
            assert!(d.read_with(cx, |d, _| d.saving()));
        }
    }
    assert_eq!(on_disk(&p), "initial012base");
    assert_eq!(&*calls.borrow(), &["prepare0", "prepare1", "prepare2", "done0", "done1", "done2"]);
    assert!(idle.get());
    assert!(!d.read_with(cx, |d, _| d.saving()));
}

#[gpui::test]
fn file_autosave_waits_for_close_decision_and_resumes_after_cancel(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-close-prompt", "old", "mine");
    auto_enable(&ws, cx);
    close(&ws, cx, &e);
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old", "a discard decision must not race a timer");
    cx.simulate_prompt_answer("Cancel");
    auto_tick(cx);
    assert_eq!(on_disk(&p), "mineold");
}

#[gpui::test]
fn file_autosave_oldest_visible_source_controls_scoped_policy_and_transfers(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let p = file("orlo-auto-source-policy", "old");
    let first = open(&a, &mut acx, &p);
    let second = open(&b, &mut bcx, &p);
    a.update_in(&mut acx, |ws, w, cx| {
        ws.set_syntax(&SetSyntax { name: "Rust".into() }, w, cx);
        ws.apply_settings(
            r#"{"lsp_enabled":false,"overrides":[{"language":"Rust","settings":{"file_autosave":true}}]}"#,
            w,
            cx,
        );
    });
    type_in(&b, &mut bcx, "one");
    let pause = gate::pause(&p);
    auto_tick(&mut acx);
    auto_tick(&mut bcx);
    assert_eq!(pause.arrived(), 1, "one shared scheduler/write despite two windows");
    pause.release();
    acx.run_until_parked();
    assert_eq!(on_disk(&p), "oneold");
    type_in(&b, &mut bcx, "two");
    a.update_in(&mut acx, |ws, w, cx| ws.close_editor(&first, w, cx));
    auto_tick(&mut bcx);
    assert_eq!(on_disk(&p), "oneold", "remaining Plain Text view owns disabled policy");
    assert!(dirty(&second, &mut bcx));
    auto_enable(&b, &mut bcx);
    auto_tick(&mut bcx);
    assert_eq!(on_disk(&p), "onetwoold");
}

#[gpui::test]
fn file_autosave_ordinary_unmark_without_revision_change_resumes(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (p, ws, cx, e) = opened(cx, "orlo-auto-unmark", "old", "");
    auto_enable(&ws, cx);
    e.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(None, "界", None, w, cx));
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old");
    let version = e.read_with(cx, |e, cx| e.doc(cx).version);
    e.update_in(cx, |e, w, cx| e.unmark_text(w, cx));
    assert_eq!(version, e.read_with(cx, |e, cx| e.doc(cx).version));
    auto_tick(cx);
    assert_eq!(on_disk(&p), "界old");
}

#[gpui::test]
fn file_autosave_window_discard_ends_visible_ownership_with_retained_handles(cx: &mut TestAppContext) {
    let (a, _b, mut acx, _bcx) = crate::workspace::ui_tests::two_windows(cx);
    let p = file("orlo-auto-window-close", "old");
    let retained = open(&a, &mut acx, &p);
    type_in(&a, &mut acx, "draft");
    auto_enable(&a, &mut acx);
    a.update_in(&mut acx, |ws, w, cx| ws.confirm_close(w, cx));
    auto_tick(&mut acx);
    assert_eq!(on_disk(&p), "old");
    acx.simulate_prompt_answer("Discard All");
    acx.run_until_parked();
    cx.executor().advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(on_disk(&p), "old");
    assert_eq!(retained.read_with(cx, |e, cx| e.document.read(cx).tabs), 0);
    assert!(a.read_with(cx, |ws, _| ws.views.tabs().next().is_none()));
}

#[gpui::test]
fn file_autosave_recovery_and_history_remain_independent(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-recovery-history", "old", "draft");
    let history = p.parent().unwrap().join("history");
    crate::files::file_history::with_root(history.clone(), || {
        cx.cx.update(|cx| crate::workspace::sessions::save_for_quit(cx).unwrap());
        let recovered = session::load(&session::path());
        assert_eq!(recovered.windows[0].groups[0].views[0].unsaved.as_ref().unwrap().text, "draftold");
        assert_eq!(on_disk(&p), "old");
        assert!(crate::files::file_history::list_in(&history, &p).unwrap().is_empty());
        auto_enable(&ws, cx);
        auto_tick(cx);
        assert_eq!(on_disk(&p), "draftold");
        assert!(crate::files::file_history::list_in(&history, &p).unwrap().is_empty());
        cx.update(|_, cx| cx.global_mut::<Settings>().file_history = true);
        cx.run_until_parked();
        type_in(&ws, cx, "next");
        auto_tick(cx);
        let versions = crate::files::file_history::list_in(&history, &p).unwrap();
        assert_eq!(versions.len(), 1);
        let record = crate::files::file_history::read_in(&history, &p, &versions[0].id).unwrap();
        assert_eq!(record.bytes, b"draftold");
        assert!(!dirty(&e, cx));
    });
}

#[gpui::test]
fn file_autosave_waits_for_shared_save_as_choice_and_cancel_resumes(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let p = file("orlo-auto-save-as-choice", "old");
    open(&a, &mut acx, &p);
    open(&b, &mut bcx, &p);
    auto_enable(&a, &mut acx);
    type_in(&b, &mut bcx, "draft");
    b.update_in(&mut bcx, |ws, w, cx| ws.save_as(&SaveAs, w, cx));
    auto_tick(&mut acx);
    assert_eq!(on_disk(&p), "old");
    bcx.simulate_new_path_selection(|_| None);
    bcx.run_until_parked();
    auto_tick(&mut acx);
    assert_eq!(on_disk(&p), "draftold");
}

#[gpui::test]
fn file_autosave_recovered_dirty_source_requires_explicit_save_before_automatic_writes(cx: &mut TestAppContext) {
    let p = file("orlo-auto-restored", "external");
    let (ws, cx) = window(cx);
    let e = ws.update_in(cx, |ws, w, cx| {
        let unsaved = session::Unsaved { text: "recovered".into(), ..Default::default() };
        let e = ws.new_editor(unsaved.state(&p).unwrap(), cx);
        ws.attach(e.clone(), false, w, cx);
        e
    });
    auto_enable(&ws, cx);
    auto_tick(cx);
    assert_eq!(on_disk(&p), "external");
    save(&ws, cx);
    assert_eq!(on_disk(&p), "recovered");
    type_in(&ws, cx, "new");
    auto_tick(cx);
    assert_eq!(on_disk(&p), "newrecovered");
    assert!(!dirty(&e, cx));
}

#[gpui::test]
fn file_autosave_checks_permissions_and_raced_write_before_history(cx: &mut TestAppContext) {
    let (p, ws, cx, e) = opened(cx, "orlo-auto-write-race", "old", "mine");
    chmod(&p, 0o444);
    // Complete reread establishes this file's current stamp, but permissions still protect it.
    let loaded = Loaded::read(&p).unwrap();
    e.update(cx, |e, cx| e.reload(loaded, &[], cx));
    type_in(&ws, cx, "new");
    auto_enable(&ws, cx);
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old");
    assert!(status(&ws, cx).unwrap().contains("read-only"));
    assert!(dirty(&e, cx));
    chmod(&p, 0o644);
    save(&ws, cx);
    type_in(&ws, cx, "after");
    cx.update(|_, cx| cx.global_mut::<Settings>().file_history = true);
    cx.run_until_parked();
    let history = p.parent().unwrap().join("history");
    let pause = gate::pause(&p);
    auto_tick(cx);
    assert_eq!(pause.arrived(), 1);
    std::fs::write(&p, "changed while held").unwrap();
    crate::files::file_history::with_root(history.clone(), || {
        pause.release();
        cx.run_until_parked();
    });
    assert_eq!(on_disk(&p), "changed while held");
    assert!(crate::files::file_history::list_in(&history, &p).unwrap().is_empty());
    assert!(dirty(&e, cx));
}

#[gpui::test]
fn file_autosave_preserves_shared_snippet_fields_and_mirrors_until_exit(cx: &mut TestAppContext) {
    let (p, ws, cx, first) = opened(cx, "orlo-auto-snippet", "old", "");
    auto_enable(&ws, cx);
    ws.update_in(cx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
    let snippet = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    assert_ne!(first, snippet);
    snippet.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.complete_snippet_ranges(0..3, "${1:中} $1 ${2:end}$0  ", Vec::new()).unwrap();
        })
    });
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old");
    snippet.update(cx, |e, cx| {
        e.edit(cx, |s| {
            assert!(s.in_snippet());
            s.insert("界");
            assert_eq!(s.text(), "界 界 end  ");
            assert!(s.snippet_next(true));
            assert_eq!(s.doc.rope.slice(s.view.selections[0].range()).to_string(), "end");
        })
    });
    auto_tick(cx);
    assert_eq!(on_disk(&p), "old");
    // Navigation to $0 changes no text version; the deferred timer must still resume.
    snippet.update(cx, |e, cx| {
        e.edit(cx, |s| {
            assert!(s.snippet_next(true));
            assert!(!s.in_snippet());
        })
    });
    auto_tick(cx);
    assert_eq!(on_disk(&p), "界 界 end  ");
}

#[gpui::test]
fn file_autosave_failure_follows_surviving_owner_after_source_window_closes(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let p = file("orlo-auto-error-owner", "old");
    open(&a, &mut acx, &p);
    let survivor = open(&b, &mut bcx, &p);
    auto_enable(&a, &mut acx);
    type_in(&a, &mut acx, "draft");
    let pause = gate::pause(&p);
    auto_tick(&mut acx);
    assert_eq!(pause.arrived(), 1);
    // The source window is gone, but a peer remains and must receive the outcome.
    a.update_in(&mut acx, |ws, w, cx| {
        let views: Vec<_> = ws.views.tabs().cloned().collect();
        for e in views {
            ws.remove_view(&e, cx);
        }
        w.remove_window();
    });
    std::fs::write(&p, "external").unwrap();
    pause.release();
    bcx.run_until_parked();
    assert_eq!(on_disk(&p), "external");
    assert!(status(&b, &mut bcx).is_some_and(|s| s.contains("Automatic save paused")));
    assert!(dirty(&survivor, &mut bcx));
}

#[gpui::test]
fn file_autosave_not_on_caret_cleanup_uses_owning_view_and_stays_undoable(cx: &mut TestAppContext) {
    let (p, ws, cx, first) = opened(cx, "orlo-auto-caret-cleanup", "owner  \nother  \n", "");
    first.update(cx, |e, cx| e.sel(cx).set_cursor(6));
    ws.update_in(cx, |ws, w, cx| {
        ws.apply_settings(
            r#"{"file_autosave":true,"lsp_enabled":false,"trim_trailing_white_space_on_save":"not_on_caret"}"#,
            w,
            cx,
        );
        ws.new_view_into_file(&NewViewIntoFile, w, cx);
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(8);
                s.insert("new ");
            })
        });
    });
    auto_tick(cx);
    assert_eq!(on_disk(&p), "owner  \nnew other\n");
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    assert_eq!(text(&ws, cx), "owner  \nnew other  \n");
    ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.redo())));
    assert_eq!(text(&ws, cx), "owner  \nnew other\n");
}

#[gpui::test]
fn save_copy_action_is_registered(cx: &mut TestAppContext) {
    let (_ws, vcx) = window(cx);
    vcx.update(|_, cx| {
        assert!(cx.build_action("workspace::SaveCopyAs", None).is_ok(), "Save a Copy As must be reachable");
    });
}

#[gpui::test]
fn save_copy_named_and_untitled_preserve_source_and_typing_group(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "n22-copy-named", "disk\n", "café 🦀");
    let to = p.with_file_name("copy.txt");
    let before = e.read_with(vcx, |e, cx| (e.doc(cx).path.clone(), e.doc(cx).disk, e.doc(cx).save_point()));
    vcx.dispatch_action(SaveCopyAs);
    vcx.simulate_new_path_selection(|_| Some(to.clone()));
    vcx.run_until_parked();
    assert_eq!(on_disk(&to), "café 🦀disk\n");
    assert_eq!(e.read_with(vcx, |e, cx| (e.doc(cx).path.clone(), e.doc(cx).disk, e.doc(cx).save_point())), before);
    assert!(dirty(&e, vcx));
    type_in(&ws, vcx, "X");
    undo(&e, vcx);
    assert_eq!(text(&ws, vcx), "disk\n", "copy must not split typing group");
    assert!(!dirty(&e, vcx));
    ws.update_in(vcx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
    type_in(&ws, vcx, "未保存 🦀");
    let to = p.with_file_name("untitled.txt");
    vcx.dispatch_action(SaveCopyAs);
    vcx.simulate_new_path_selection(|_| Some(to.clone()));
    vcx.run_until_parked();
    assert_eq!(on_disk(&to), "未保存 🦀");
    ws.read_with(vcx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        assert!(e.read(cx).doc(cx).path.is_none());
        assert!(e.read(cx).doc(cx).dirty);
    });
}

fn copy_as(vcx: &mut VisualTestContext, to: &Path) {
    vcx.dispatch_action(SaveCopyAs);
    let chosen = to.to_path_buf();
    vcx.simulate_new_path_selection(move |_| Some(chosen.clone()));
    vcx.run_until_parked();
}

#[gpui::test]
fn save_copy_menu_and_palette_dispatch_the_registered_action(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let action = ws.read_with(vcx, |ws, cx| {
        let file = ws.menus(cx).into_iter().find(|menu| menu.name == "File").unwrap();
        file.items
            .into_iter()
            .find_map(|item| match item {
                gpui::MenuItem::Action { name, action, .. } if name == "Save a Copy As…" => Some(action),
                _ => None,
            })
            .unwrap()
    });
    assert!(action.as_any().is::<SaveCopyAs>());
    let item =
        crate::chrome::quick_panel::command_items().into_iter().find(|i| i.label == "File: Save a Copy As…").unwrap();
    match item.action {
        crate::chrome::quick_panel::ItemAction::Dispatch(action) => {
            assert!(action.as_any().is::<SaveCopyAs>());
            vcx.update(|w, cx| w.dispatch_action(action, cx));
        }
        _ => panic!("copy palette item must dispatch"),
    }
    assert!(ws.read_with(vcx, |ws, _| ws.saves.busy()));
    vcx.simulate_new_path_selection(|_| None);
    vcx.run_until_parked();
}

#[gpui::test]
fn save_copy_paused_shared_source_edits_format_and_path_remain_independent(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "n22-copy-paused", "disk\n", "α🦀 ");
    ws.update_in(vcx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
    let to = p.with_file_name("copy.txt");
    let pause = gate::pause(&to);
    copy_as(vcx, &to);
    assert_eq!(pause.arrived(), 1);
    let doc = e.read_with(vcx, |e, _| e.document.clone());
    assert!(doc.read_with(vcx, |d, _| d.saving()));
    let original_disk = e.read_with(vcx, |e, cx| e.doc(cx).disk);
    type_in(&ws, vcx, "later");
    e.update(vcx, |e, cx| {
        e.update_doc(cx, |d| {
            d.set_encoding(Encoding::Utf16Le, true);
            d.set_line_ending("\r\n");
            d.path = Some(p.with_file_name("moved.txt"));
        })
    });
    let before = e.read_with(vcx, |e, cx| (e.doc(cx).path.clone(), e.doc(cx).save_point(), e.doc(cx).version));
    land(&pause, vcx);
    assert_eq!(on_disk(&to), "α🦀 disk\n");
    assert_eq!(on_disk(&p), "disk\n");
    assert_eq!(e.read_with(vcx, |e, cx| (e.doc(cx).path.clone(), e.doc(cx).save_point(), e.doc(cx).version)), before);
    assert_eq!(e.read_with(vcx, |e, cx| e.doc(cx).disk), original_disk);
    assert!(dirty(&e, vcx));
    assert_eq!(ws.read_with(vcx, |ws, _| ws.views.tabs().count()), 2);
    undo(&e, vcx);
    assert_eq!(text(&ws, vcx), "α🦀 disk\n", "the shared view owns its own typing group");
    undo(&e, vcx);
    assert_eq!(text(&ws, vcx), "disk\n");
}

#[gpui::test]
fn save_copy_refuses_aliases_and_live_owners_in_other_windows(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let p = file("n22-copy-owner", "source");
    let source = open(&a, &mut acx, &p);
    let hard = p.with_file_name("hard");
    let sym = p.with_file_name("sym");
    std::fs::hard_link(&p, &hard).unwrap();
    std::os::unix::fs::symlink(&p, &sym).unwrap();
    for path in [&p, &hard, &sym] {
        copy_as(&mut acx, path);
        assert_eq!(prompt(&mut acx).as_deref(), Some("Could not save copy"));
        acx.simulate_prompt_answer("Ok");
    }
    let target = p.with_file_name("target");
    std::fs::write(&target, "other").unwrap();
    let other = open(&b, &mut bcx, &target);
    let alias = p.with_file_name("target-alias");
    std::fs::hard_link(&target, &alias).unwrap();
    copy_as(&mut acx, &alias);
    assert_eq!(prompt(&mut acx).as_deref(), Some("Could not save copy"));
    assert_eq!(on_disk(&target), "other");
    assert_eq!(source.read_with(&acx, |e, cx| e.doc(cx).path.clone()), Some(p));
    assert!(!other.read_with(&bcx, |e, cx| e.doc(cx).dirty));
}

#[gpui::test]
fn save_copy_rechecks_owner_and_overwrite_after_waiting(cx: &mut TestAppContext) {
    let (p, _ws, vcx, e) = opened(cx, "n22-copy-stale", "disk", "dirty");
    let doc = e.read_with(vcx, |e, _| e.document.clone());
    for change in ["owner", "appeared", "replaced", "changed", "source-path"] {
        let to = p.with_file_name(change);
        if change == "replaced" || change == "changed" {
            std::fs::write(&to, "before").unwrap();
        }
        let pause = gate::pause(&to);
        copy_as(vcx, &to);
        assert_eq!(pause.arrived(), 1);
        let owner = if change == "owner" {
            Some(vcx.update(|_, cx| {
                let mut model = crate::editing::text::Document::new();
                model.path = Some(to.clone());
                Documents::open(model, cx)
            }))
        } else {
            None
        };
        if change == "replaced" {
            std::fs::remove_file(&to).unwrap();
        }
        if matches!(change, "appeared" | "replaced" | "changed") {
            std::fs::write(&to, "external").unwrap();
        }
        if change == "source-path" {
            e.update(vcx, |e, cx| e.update_doc(cx, |d| d.path = Some(to.clone())));
        }
        let before = e.read_with(vcx, |e, cx| (e.doc(cx).path.clone(), e.doc(cx).disk, e.doc(cx).save_point()));
        land(&pause, vcx);
        assert_eq!(prompt(vcx).as_deref(), Some("Could not save copy"), "{change}");
        vcx.simulate_prompt_answer("Ok");
        assert_eq!(e.read_with(vcx, |e, cx| (e.doc(cx).path.clone(), e.doc(cx).disk, e.doc(cx).save_point())), before);
        assert!(dirty(&e, vcx));
        if matches!(change, "appeared" | "replaced" | "changed") {
            assert_eq!(on_disk(&to), "external");
        } else {
            assert!(!to.exists());
        }
        assert!(!doc.read_with(vcx, |d, _| d.saving()));
        drop(owner);
    }
}

#[gpui::test]
fn save_copy_waits_for_destination_lease_captures_before_wait_and_survives_source_close(cx: &mut TestAppContext) {
    use std::{cell::Cell, rc::Rc};
    let (p, ws, vcx, e) = opened(cx, "n22-copy-close", "disk", "captured");
    let to = p.with_file_name("copy");
    let lease = vcx.update(|_, cx| pipelines::lease(&to, cx));
    let doc = e.read_with(vcx, |e, _| e.document.clone());
    copy_as(vcx, &to);
    assert!(!to.exists());
    type_in(&ws, vcx, "later");
    let idle = Rc::new(Cell::new(false));
    let done = idle.clone();
    vcx.update(|_, cx| Documents::when_all_saved(move |_| done.set(true), cx));
    ws.update_in(vcx, |ws, w, cx| ws.close_editor(&e, w, cx));
    assert!(!is_open(&ws, vcx, &e));
    assert!(!idle.get());
    assert!(doc.read_with(vcx, |d, _| d.saving()));
    drop(lease);
    settle(vcx);
    crate::workspace::ui_tests::settle_until(vcx, |_| to.exists());
    vcx.run_until_parked();
    assert_eq!(on_disk(&to), "captureddisk");
    assert!(idle.get());
    assert_eq!(on_disk(&p), "disk");
    assert!(dirty(&e, vcx));
}

#[gpui::test]
fn save_copy_cancel_lossy_error_and_readonly_native_input_preserve_source(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (p, ws, vcx, e) = opened(cx, "n22-copy-cancel", "disk", "🦀");
    vcx.dispatch_action(SaveCopyAs);
    vcx.simulate_new_path_selection(|_| None);
    vcx.run_until_parked();
    assert!(status(&ws, vcx).unwrap().contains("canceled"));
    let to = p.with_file_name("copy");
    std::fs::write(&to, "keep").unwrap();
    e.update(vcx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Windows1252, false)));
    copy_as(vcx, &to);
    assert_eq!(prompt(vcx).as_deref(), Some("Could not save copy"));
    vcx.simulate_prompt_answer("Ok");
    assert_eq!(on_disk(&to), "keep");
    e.update_in(vcx, |e, w, cx| {
        e.update_doc(cx, |d| {
            d.set_encoding(Encoding::Utf8, false);
            d.read_only = true;
        });
        e.state.overtype = true;
        e.replace_and_mark_text_in_range(None, "仮", None, w, cx);
    });
    let before = e.read_with(vcx, |e, cx| {
        (e.text(cx), e.marked.clone(), e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).disk)
    });
    copy_as(vcx, &to);
    assert_eq!(on_disk(&to), before.0, "explicit Save policy captures the current native-input rope");
    assert_eq!(
        e.read_with(vcx, |e, cx| (
            e.text(cx),
            e.marked.clone(),
            e.doc(cx).version,
            e.doc(cx).save_point(),
            e.doc(cx).disk
        )),
        before
    );
    assert!(e.read_with(vcx, |e, cx| e.doc(cx).read_only));
    // An actual safe-writer failure is reported without source disk/saved metadata changes.
    orlo_project::io::FAULT.set(Some(("write", std::io::ErrorKind::StorageFull)));
    copy_as(vcx, &to);
    orlo_project::io::FAULT.set(None);
    assert_eq!(prompt(vcx).as_deref(), Some("Could not save copy"));
    assert_eq!(
        e.read_with(vcx, |e, cx| (
            e.text(cx),
            e.marked.clone(),
            e.doc(cx).version,
            e.doc(cx).save_point(),
            e.doc(cx).disk
        )),
        before
    );
}

#[gpui::test]
fn save_copy_ignores_destination_cleanup_and_preserves_project_and_views(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let dir = temp("n22-no-cleanup").canonicalize().unwrap();
    let to = dir.join("new.txt");
    std::fs::write(dir.join(".editorconfig"), "root=true\n[*.txt]\ncharset=utf-8-bom\nend_of_line=crlf\ntrim_trailing_whitespace=true\ninsert_final_newline=true").unwrap();
    ws.update(vcx, |ws, cx| ws.with_active(cx, |e, cx| e.set_text("a  \nb  ", cx)));
    let before = ws.read_with(vcx, |ws, _| {
        (
            ws.project.clone(),
            ws.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>(),
            ws.active_editor().unwrap().entity_id(),
            ws.views.find(&ws.active_editor().unwrap()),
        )
    });
    copy_as(vcx, &to);
    assert_eq!(std::fs::read(to).unwrap(), b"a  \nb  ");
    assert_eq!(text(&ws, vcx), "a  \nb  ");
    let after = ws.read_with(vcx, |ws, _| {
        (
            ws.project.clone(),
            ws.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>(),
            ws.active_editor().unwrap().entity_id(),
            ws.views.find(&ws.active_editor().unwrap()),
        )
    });
    assert_eq!(before, after);
}

#[gpui::test]
fn save_copy_hardlink_lease_and_inflight_copy_lease_survive_last_view_removal(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "n22-copy-inode-lease", "disk", "captured");
    let to = p.with_file_name("target");
    let alias = p.with_file_name("target-alias");
    std::fs::write(&to, "old").unwrap();
    std::fs::hard_link(&to, &alias).unwrap();
    let lease = vcx.update(|_, cx| pipelines::lease_file(&alias, pipelines::file_id(&alias), cx));
    let pause = gate::pause(&to);
    copy_as(vcx, &to);
    assert_eq!(pause.arrived(), 0, "a lease of another name of the inode blocks copy");
    drop(lease);
    settle(vcx);
    assert_eq!(pause.arrived(), 1);
    ws.update_in(vcx, |ws, w, cx| ws.close_editor(&e, w, cx));
    drop(e);
    vcx.update(|_, cx| {
        assert!(Documents::saving(cx), "the worker alone retains accepted work");
        assert!(pipelines::writing(&to, cx));
        assert!(pipelines::writing_file(pipelines::file_id(&alias), cx));
    });
    land(&pause, vcx);
    assert_eq!(on_disk(&to), "captureddisk");
    assert_eq!(on_disk(&alias), "captureddisk", "safe writer preserves hardlinks");
    vcx.update(|_, cx| {
        assert!(!Documents::saving(cx));
        assert!(!pipelines::writing(&to, cx));
    });
}

#[gpui::test]
fn save_copy_follow_refuses_before_a_native_sheet(cx: &mut TestAppContext) {
    let (p, ws, vcx, e) = opened(cx, "n22-copy-follow", "disk\n", "");
    vcx.dispatch_action(FollowLog);
    assert!(e.read_with(vcx, |e, cx| e.doc(cx).is_following()));
    vcx.dispatch_action(SaveCopyAs);
    assert!(!ws.read_with(vcx, |ws, _| ws.saves.busy()));
    assert!(status(&ws, vcx).unwrap().contains("Stop following"));
    assert_eq!(on_disk(&p), "disk\n");
    vcx.dispatch_action(StopFollowingLog);
}

#[gpui::test]
fn save_copy_close_and_normal_save_waiters_cover_both_operations(cx: &mut TestAppContext) {
    use std::{cell::Cell, rc::Rc};
    let (p, ws, vcx, e) = opened(cx, "n22-copy-both-waiters", "disk", "dirty");
    let save_pause = held_save(&ws, vcx, &p);
    let to = p.with_file_name("copy");
    let copy_pause = gate::pause(&to);
    copy_as(vcx, &to);
    let doc = e.read_with(vcx, |e, _| e.document.clone());
    let idle = Rc::new(Cell::new(false));
    let done = idle.clone();
    vcx.update(|_, cx| OpenDocument::when_saved(&doc, move |_| done.set(true), cx));
    assert!(!vcx.simulate_close());
    assert!(!vcx.simulate_close());
    land(&save_pause, vcx);
    assert!(!idle.get(), "ordinary save completion must still wait for the accepted copy");
    assert_eq!(vcx.cx.update(|cx| cx.windows().len()), 1);
    land(&copy_pause, vcx);
    assert!(idle.get());
    assert_eq!(on_disk(&to), "dirtydisk");
    assert_eq!(vcx.cx.update(|cx| cx.windows().len()), 0);
}

#[gpui::test]
fn save_copy_post_preflight_hardlink_open_waits_for_publication(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let source = file("n22-fix-hardlink-source", "captured");
    let target = source.with_file_name("target");
    let alias = source.with_file_name("alias");
    std::fs::write(&target, "old").unwrap();
    std::fs::hard_link(&target, &alias).unwrap();
    open(&a, &mut acx, &source);
    let pause = crate::testing::gate::pause("copy-write", &target);
    copy_as(&mut acx, &target);
    assert_eq!(pause.arrived(), 1, "after final live-owner preflight");
    b.update_in(&mut bcx, |ws, w, cx| {
        assert!(
            pipelines::observe_read(&alias, cx).changed(),
            "a writer already active at observation invalidates admission"
        );
        ws.open_path(alias.clone(), w, cx);
    });
    bcx.run_until_parked();
    b.read_with(&bcx, |ws, cx| {
        assert!(ws.tab_for_path(&alias, cx).is_none(), "hardlink ownership must wait while copy publishes");
    });
    pause.release();
    acx.run_until_parked();
    settle(&mut bcx);
    b.read_with(&bcx, |ws, cx| assert_eq!(ws.tab_for_path(&alias, cx).unwrap().read(cx).text(cx), "captured"));
}

#[gpui::test]
fn save_copy_post_preflight_pending_read_waits_and_rereads(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let source = file("n22-fix-pending-source", "captured");
    let target = source.with_file_name("target");
    std::fs::write(&target, "old").unwrap();
    open(&a, &mut acx, &source);
    let read_pause = crate::testing::gate::pause("open-read", &target);
    b.update_in(&mut bcx, |ws, w, cx| ws.read_in_background(target.clone(), w, cx));
    crate::workspace::ui_tests::settle_until(&mut bcx, |_| read_pause.arrived() == 1);
    let pause = crate::testing::gate::pause("copy-write", &target);
    copy_as(&mut acx, &target);
    assert_eq!(pause.arrived(), 1);
    read_pause.release();
    bcx.run_until_parked();
    settle(&mut bcx);
    b.read_with(&bcx, |ws, cx| {
        assert!(ws.tab_for_path(&target, cx).is_none(), "an earlier read cannot become an owner during copy");
    });
    pause.release();
    acx.run_until_parked();
    settle(&mut bcx);
    b.read_with(&bcx, |ws, cx| assert_eq!(ws.tab_for_path(&target, cx).unwrap().read(cx).text(cx), "captured"));
}

#[gpui::test]
fn save_copy_post_preflight_recovery_restore_waits_without_losing_drafts(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    for recovered in [false, true] {
        let source = file(&format!("n22-fix-restore-{recovered}"), "captured");
        let target = source.with_file_name("target");
        let alias = source.with_file_name("alias");
        std::fs::write(&target, "old").unwrap();
        std::fs::hard_link(&target, &alias).unwrap();
        open(&a, &mut acx, &source);
        let pause = crate::testing::gate::pause("copy-write", &target);
        copy_as(&mut acx, &target);
        assert_eq!(pause.arrived(), 1);
        let draft = recovered.then(|| Unsaved {
            text: "draft α🦀".into(),
            read_only: true,
            encoding: Encoding::Utf16Le,
            bom: true,
            line_ending: "\r\n".into(),
            ..Default::default()
        });
        let first = ViewState {
            path: alias.clone(),
            buffer: usize::from(recovered),
            unsaved: draft.clone(),
            pinned: true,
            selections: vec![(2, 2)],
            ..Default::default()
        };
        let second = ViewState {
            path: alias.clone(),
            buffer: usize::from(recovered),
            selections: vec![(4, 4)],
            ..Default::default()
        };
        b.update_in(&mut bcx, |ws, w, cx| {
            ws.restore(
                WindowState {
                    groups: vec![GroupState { views: vec![first, second], active: 1 }],
                    ..Default::default()
                },
                w,
                cx,
            )
        });
        bcx.run_until_parked();
        b.update_in(&mut bcx, |ws, w, cx| {
            assert!(ws.tab_for_path(&alias, cx).is_none(), "restored ownership must wait for inode lease");
            let captured = ws.capture(w, true, &mut Texts::new(), cx);
            assert_eq!(captured.groups[0].views.len(), 2);
            assert_eq!(captured.groups[0].views[0].unsaved.as_ref().map(|u| &u.text), draft.as_ref().map(|u| &u.text));
        });
        pause.release();
        acx.run_until_parked();
        settle(&mut bcx);
        crate::workspace::ui_tests::settle_until(&mut bcx, |cx| b.read_with(cx, |ws, _| ws.restoring.is_empty()));
        b.read_with(&bcx, |ws, cx| {
            let tabs: Vec<_> = ws.views.tabs().collect();
            assert_eq!(tabs.len(), 2);
            assert!(tabs[0].read(cx).same_document(tabs[1].read(cx)));
            assert_eq!(tabs[0].read(cx).text(cx), if recovered { "draft α🦀" } else { "captured" });
            if recovered {
                let d = tabs[0].read(cx).doc(cx);
                assert!(d.dirty && d.read_only && d.bom);
                assert_eq!(d.encoding, Encoding::Utf16Le);
                assert_eq!(d.line_ending, "\r\n");
            }
            assert_eq!(tabs[0].read(cx).state.cursor(), 2);
            assert_eq!(tabs[1].read(cx).state.cursor(), 4);
        });
    }
}

#[gpui::test]
fn save_copy_post_preflight_missing_settings_template_waits(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let source = file("n22-fix-template-source", "{\"tab_size\":3}\n");
    let target = settings::path();
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let target = crate::editing::text::resolve(&target);
    assert!(!target.exists());
    open(&a, &mut acx, &source);
    let pause = crate::testing::gate::pause("copy-write", &target);
    copy_as(&mut acx, &target);
    assert_eq!(pause.arrived(), 1);
    b.update_in(&mut bcx, |ws, w, cx| ws.open_settings(&OpenSettings, w, cx));
    b.read_with(&bcx, |ws, cx| {
        assert!(ws.tab_for_path(&target, cx).is_none(), "template must honor copy's absent-path reservation")
    });
    pause.release();
    acx.run_until_parked();
    settle(&mut bcx);
    b.read_with(&bcx, |ws, cx| {
        assert_eq!(ws.tab_for_path(&target, cx).unwrap().read(cx).text(cx), "{\"tab_size\":3}\n")
    });
}

#[gpui::test]
fn save_copy_post_preflight_template_failure_keeps_normal_template(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let source = file("n22-fix-template-fail", "🦀");
    let e = open(&a, &mut acx, &source);
    e.update(&mut acx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Windows1252, false)));
    for kind in ["settings", "keybindings", "pipeline"] {
        let target = match kind {
            "settings" => settings::path(),
            "keybindings" => crate::keybindings::keymap_path(),
            _ => source.with_file_name("new.orlo-pipeline"),
        };
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let target = crate::editing::text::resolve(&target);
        assert!(!target.exists());
        let pause = crate::testing::gate::pause("copy-write", &target);
        copy_as(&mut acx, &target);
        assert_eq!(pause.arrived(), 1);
        b.update_in(&mut bcx, |ws, w, cx| match kind {
            "settings" => ws.open_settings(&OpenSettings, w, cx),
            "keybindings" => ws.open_keybindings(&OpenKeybindings, w, cx),
            _ => ws.open_or_template(target.clone(), "pipeline template", w, cx),
        });
        assert!(b.read_with(&bcx, |ws, cx| ws.tab_for_path(&target, cx).is_none()));
        pause.release();
        acx.run_until_parked();
        settle(&mut bcx);
        assert_eq!(prompt(&mut acx).as_deref(), Some("Could not save copy"));
        acx.simulate_prompt_answer("Ok");
        assert!(!target.exists());
        b.read_with(&bcx, |ws, cx| {
            let e = ws.tab_for_path(&target, cx).unwrap();
            let text = e.read(cx).text(cx);
            match kind {
                "settings" => assert_eq!(text, settings::TEMPLATE),
                "keybindings" => assert!(text.contains("Hello")),
                _ => assert_eq!(text, "pipeline template"),
            }
            assert!(!e.read(cx).doc(cx).dirty);
        });
    }
}

#[gpui::test]
fn save_copy_post_preflight_pending_reopen_rereads_after_copy(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let source = file("n22-fix-reopen-source", "captured");
    let target = source.with_file_name("target");
    std::fs::write(&target, "old\n".repeat((LOAD_IN_BACKGROUND / 4 + 1) as usize)).unwrap();
    open(&a, &mut acx, &source);
    let read_pause = crate::testing::gate::pause("restore-read", &target);
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.closed.push(ViewState { path: target.clone(), selections: vec![(2, 2)], ..Default::default() });
        ws.reopen_closed_file(&ReopenClosedFile, w, cx);
    });
    crate::workspace::ui_tests::settle_until(&mut bcx, |_| read_pause.arrived() == 1);
    let pause = crate::testing::gate::pause("copy-write", &target);
    copy_as(&mut acx, &target);
    assert_eq!(pause.arrived(), 1);
    // Copy can land while a result is pending: the stamp check must still force reread.
    pause.release();
    acx.run_until_parked();
    read_pause.release();
    settle(&mut bcx);
    b.read_with(&bcx, |ws, cx| assert_eq!(ws.tab_for_path(&target, cx).unwrap().read(cx).text(cx), "captured"));
}

#[gpui::test]
fn save_copy_post_preflight_pending_recovery_capture_keeps_distinct_buffer_ids(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let source = file("n22-fix-recovery-ids", "captured");
    let target = source.with_file_name("target");
    std::fs::write(&target, "old").unwrap();
    open(&a, &mut acx, &source);
    let pause = crate::testing::gate::pause("copy-write", &target);
    copy_as(&mut acx, &target);
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.restore(
            WindowState {
                groups: vec![GroupState {
                    views: vec![ViewState {
                        path: target.clone(),
                        buffer: 1,
                        unsaved: Some(Unsaved { text: "recover this".into(), ..Default::default() }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            w,
            cx,
        );
        ws.new_file(&NewFile, w, cx);
    });
    type_in(&b, &mut bcx, "new draft");
    b.update_in(&mut bcx, |ws, w, cx| {
        let captured = ws.capture(w, true, &mut Texts::new(), cx);
        let views = &captured.groups[0].views;
        assert_eq!(views.len(), 2);
        assert_ne!(views[0].buffer, views[1].buffer, "pending and new drafts must not restore as one buffer");
        assert!(views.iter().any(|v| v.unsaved.as_ref().is_some_and(|u| u.text == "recover this")));
    });
    pause.release();
    acx.run_until_parked();
    settle(&mut bcx);
    b.read_with(&bcx, |ws, cx| {
        let texts: Vec<_> = ws.views.tabs().map(|e| e.read(cx).text(cx)).collect();
        assert!(texts.contains(&"recover this".into()));
        assert!(texts.contains(&"new draft".into()));
    });
}

#[gpui::test]
fn save_copy_post_preflight_external_append_does_not_require_idle_log(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let target = file("n22-fix-external-log", "old\n");
    let pause = crate::testing::gate::pause("open-read", &target);
    ws.update_in(vcx, |ws, w, cx| ws.read_in_background(target.clone(), w, cx));
    crate::workspace::ui_tests::settle_until(vcx, |_| pause.arrived() == 1);
    // An arbitrary external append is handled by existing disk polling, not read retry.
    std::fs::write(&target, "old\nexternal append\n").unwrap();
    pause.release();
    crate::workspace::ui_tests::settle_until(vcx, |cx| {
        ws.read_with(cx, |ws, cx| ws.tab_for_path(&target, cx).is_some())
    });
    ws.read_with(vcx, |ws, cx| {
        let e = ws.tab_for_path(&target, cx).unwrap();
        assert_eq!(e.read(cx).text(cx), "old\n");
        assert_ne!(e.read(cx).doc(cx).disk, crate::files::watch::stamp(&target));
    });
    vcx.dispatch_action(FollowLog);
    assert!(ws.read_with(vcx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).is_following()));
    vcx.dispatch_action(StopFollowingLog);
}

#[gpui::test]
fn save_copy_post_preflight_pending_recovery_close_waits_then_prompts(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let source = file("n22-fix-recovery-close", "captured");
    let target = source.with_file_name("target");
    std::fs::write(&target, "old").unwrap();
    open(&a, &mut acx, &source);
    let pause = crate::testing::gate::pause("copy-write", &target);
    copy_as(&mut acx, &target);
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.restore(
            WindowState {
                groups: vec![GroupState {
                    views: vec![ViewState {
                        path: target,
                        buffer: 1,
                        unsaved: Some(Unsaved { text: "keep recovery".into(), ..Default::default() }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            w,
            cx,
        )
    });
    assert!(!bcx.simulate_close(), "closing cannot silently discard a deferred recovery");
    bcx.run_until_parked();
    assert_eq!(bcx.cx.update(|cx| cx.windows().len()), 2);
    pause.release();
    acx.run_until_parked();
    settle(&mut bcx);
    assert_eq!(prompt(&mut bcx).as_deref(), Some("Save all changes before closing?"));
    bcx.simulate_prompt_answer("Cancel");
    assert_eq!(text(&b, &mut bcx), "keep recovery");
}

#[gpui::test]
fn save_copy_post_preflight_pending_recovery_is_not_an_empty_project_window(cx: &mut TestAppContext) {
    let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
    let source = file("n22-fix-recovery-project", "captured");
    let target = source.with_file_name("target");
    std::fs::write(&target, "old").unwrap();
    open(&a, &mut acx, &source);
    let pause = crate::testing::gate::pause("copy-write", &target);
    copy_as(&mut acx, &target);
    let project = source.with_extension(crate::config::project::PROJECT_EXT);
    std::fs::write(&project, "{}").unwrap();
    b.update_in(&mut bcx, |ws, w, cx| {
        ws.restore(
            WindowState {
                groups: vec![GroupState {
                    views: vec![ViewState {
                        path: target,
                        buffer: 1,
                        unsaved: Some(Unsaved { text: "keep recovery".into(), ..Default::default() }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            w,
            cx,
        );
        ws.try_open_project_path(&OpenProjectPath { path: project, here: false }, w, cx).unwrap();
        assert!(ws.project.is_none(), "pending recovery makes this a nonempty window");
        assert!(ws.restoring.iter().any(|v| v.view.unsaved.is_some()));
    });
    pause.release();
    acx.run_until_parked();
    settle(&mut bcx);
    assert_eq!(text(&b, &mut bcx), "keep recovery");
}

#[gpui::test]
fn expanded_f01_save_save_as_and_copy_show_scalar_location_to_shared_views(cx: &mut TestAppContext) {
    let (path, ws, vcx, e) = opened(cx, "f01-save-detail", "disk", "é\ncafé😀");
    ws.update_in(vcx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    let shared = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    e.update(vcx, |e, cx| {
        e.update_doc(cx, |d| {
            d.set_encoding(Encoding::Windows1252, false);
            d.line_ending = "\r\n";
        })
    });
    let before = e.read_with(vcx, |e, cx| (e.text(cx), e.doc(cx).save_point(), e.doc(cx).version, e.doc(cx).disk));
    let to = path.with_file_name("copy.txt");
    std::fs::write(&to, "keep").unwrap();
    for operation in ["save", "save_as", "copy"] {
        match operation {
            "save" => save(&ws, vcx),
            "save_as" => save_as(&ws, vcx, &to),
            _ => copy_as(vcx, &to),
        }
        vcx.run_until_parked();
        let (_, detail) = vcx.pending_prompt().expect("conversion error is visible");
        assert!(detail.contains("'😀' (U+1F600) at line 2, column 5"), "{operation}: {detail}");
        vcx.simulate_prompt_answer("Ok");
        for editor in [&e, &shared] {
            assert_eq!(
                editor.read_with(vcx, |e, cx| (e.text(cx), e.doc(cx).save_point(), e.doc(cx).version, e.doc(cx).disk)),
                before
            );
            assert!(dirty(editor, vcx));
        }
        assert_eq!(on_disk(&path), "disk");
        assert_eq!(on_disk(&to), "keep");
    }
    undo(&e, vcx);
    assert_eq!(e.read_with(vcx, |e, cx| e.text(cx)), "disk");
}
