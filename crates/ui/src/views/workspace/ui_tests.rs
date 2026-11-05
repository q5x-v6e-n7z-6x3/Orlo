//! Window-level tests in gpui's test app: prompts, panels, tabs and menus.

use super::*;
use crate::editing::text::Selection;
use gpui::{TestAppContext, VisualTestContext};

/// A window as `Orlo` opens one, on default settings, with the user's files left alone.
pub(crate) fn window(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    // the session, settings and socket in a folder of this test's own
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let support = temp(&format!("orlo-support-{}-{n}", std::process::id()));
    session::TEST_SUPPORT.set(Some(support));
    cx.update(|cx| {
        cx.set_global(Settings { lsp_enabled: false, ..Settings::default() });
        cx.set_global(SettingsDisk(None));
        cx.set_global(Recent::default());
        crate::config::customization::bind_defaults(cx);
    });
    cx.add_window_view(Workspace::new)
}

/// A window with `text` saved as f.txt in the fresh folder `dir` and opened: the file's path.
fn window_on<'a>(
    cx: &'a mut TestAppContext,
    dir: &str,
    text: &str,
) -> (Entity<Workspace>, &'a mut VisualTestContext, PathBuf) {
    let p = temp(dir).canonicalize().unwrap().join("f.txt");
    std::fs::write(&p, text).unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    (ws, vcx, p)
}

/// Two windows as `window` opens the first, each with a test context of its own.
pub(crate) fn two_windows(
    cx: &mut TestAppContext,
) -> (Entity<Workspace>, Entity<Workspace>, VisualTestContext, VisualTestContext) {
    let (ws1, _) = window(cx);
    let (ws2, _) = cx.add_window_view(Workspace::new);
    let (w1, w2) = (cx.windows()[0], cx.windows()[1]);
    let vcx1 = VisualTestContext::from_window(w1, cx);
    (ws1, ws2, vcx1, VisualTestContext::from_window(w2, cx))
}

/// A Set Syntax command for `name`.
pub(crate) fn syntax(name: &str) -> SetSyntax {
    SetSyntax { name: name.into() }
}

/// Find opened on `query`, with the regex toggle on when `regex`.
pub(crate) fn find_text(
    ws: &mut Workspace,
    query: &str,
    regex: bool,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    ws.find(&Find, window, cx);
    set_query(ws, query, regex, cx);
}

/// Replace opened on `query` and `with`, with the regex toggle on when `regex`.
pub(crate) fn replace_text(
    ws: &mut Workspace,
    query: &str,
    with: &str,
    regex: bool,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    ws.replace(&Replace, window, cx);
    set_query(ws, query, regex, cx);
    let r = ws.find_panel().unwrap().replace.clone();
    r.update(cx, |e, cx| e.set_text(with, cx));
}

/// The find panel's query set to `query`, as a regex when `regex`.
pub(crate) fn set_query(ws: &mut Workspace, query: &str, regex: bool, cx: &mut Context<Workspace>) {
    let p = ws.find_panel_mut().unwrap();
    if regex {
        p.opts.regex = true;
    }
    let q = p.query.clone();
    q.update(cx, |e, cx| e.set_text(query, cx));
}

pub(crate) fn new_view(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
}

/// Run the app until the quick panel lists `n` rows.
pub(crate) fn settle_rows(ws: &Entity<Workspace>, cx: &mut VisualTestContext, n: usize) {
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.items.len() == n)));
}

/// The active view's text.
pub(crate) fn text(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    ws.update(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).text())
}

pub(crate) fn type_in(ws: &Entity<Workspace>, cx: &mut VisualTestContext, t: &str) {
    ws.update(cx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        e.update(cx, |e, cx| e.edit(cx, |s| s.insert(t)));
    });
}

#[gpui::test]
fn rewiring_a_view_cancels_the_previous_windows_completion(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "alpha\nal");
    ws.update_in(cx, |ws, window, cx| {
        let editor = ws.active_editor().unwrap();
        assert!(editor.update(cx, |editor, cx| editor.expect_server_items(window, cx)));
        assert!(editor.read(cx).completion_waiting().is_some());
        let generation = ws.index.read(cx).generation;
        let destination = cx.new(crate::languages::index::ProjectIndex::new);
        destination.update(cx, |index, _| index.generation = generation);
        ws.index = destination;
        ws.wire(&editor, window, cx);
        assert_eq!(editor.read(cx).completion_waiting(), None);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).completion_waiting(), None);
    });
}

#[gpui::test]
fn selection_search_replaces_captures_only_inside_the_original_selection(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a1 a2 a3");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_selection(3, 5));
        replace_text(ws, r"a(\d)", "b${1}b", true, window, cx);
        ws.toggle_find_selection(&ToggleFindSelection, window, cx);
        ws.refresh_matches(cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).find_ranges, vec![3..5]);
        ws.replace_all(&ReplaceAll, window, cx);
    });
    assert_eq!(text(&ws, cx), "a1 b2b a3");
    ws.update_in(cx, |ws, window, cx| {
        let q = ws.find_panel().unwrap().query.clone();
        q.update(cx, |e, cx| e.set_text(r"b(\d)b", cx));
        ws.refresh_matches(cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).find_ranges, vec![3..6]);
        ws.find_all(&FindAll, window, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).state.selections, vec![Selection::new(3, 6)]);
    });
}

#[gpui::test]
fn regex_replace_next_handles_empty_matches_without_getting_stuck(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a\nb");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(0));
        replace_text(ws, "$", "_", true, window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(1));
        ws.replace_one(&find_panel::ReplaceOne, window, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 4);
        ws.replace_one(&find_panel::ReplaceOne, window, cx);
    });
    assert_eq!(text(&ws, cx), "a_\nb_");
}

#[gpui::test]
fn invalid_regex_is_visible_and_cannot_replace_text(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "draft");
    ws.update_in(cx, |ws, window, cx| {
        ws.replace(&Replace, window, cx);
        set_query(ws, "[", true, cx);
        ws.refresh_matches(cx);
        assert!(ws.find_panel().unwrap().error.is_some());
        ws.replace_all(&ReplaceAll, window, cx);
    });
    assert_eq!(text(&ws, cx), "draft");
}

#[gpui::test]
fn find_query_arrow_keys_recall_history_and_restore_the_draft(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "first second");
    ws.update_in(cx, |ws, window, cx| {
        find_text(ws, "first", false, window, cx);
        ws.find_next(&FindNext, window, cx);
        set_query(ws, "draft", false, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("up");
    ws.read_with(cx, |ws, cx| assert_eq!(ws.find_panel().unwrap().query.read(cx).text(cx), "first"));
    cx.simulate_keystrokes("down");
    ws.read_with(cx, |ws, cx| assert_eq!(ws.find_panel().unwrap().query.read(cx).text(cx), "draft"));
}

#[gpui::test]
fn f4_opens_each_search_match_from_the_target_file_and_wraps(cx: &mut TestAppContext) {
    let dir = temp("orlo-f4-navigation").canonicalize().unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "one one\n").unwrap();
    std::fs::write(&b, "xx one").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(dir, window, cx);
        ws.find_in_files(&FindInFiles, window, cx);
        let q = ws.fif_panel().unwrap().query.clone();
        q.update(cx, |e, cx| e.set_text("one", cx));
        ws.run_find_in_files(&RunFindInFiles, window, cx);
    });
    settle(cx);
    new_view(&ws, cx);
    for (key, path, at) in [("f4", &a, 0), ("f4", &a, 4), ("f4", &b, 3), ("f4", &a, 0), ("shift-f4", &b, 3)] {
        cx.simulate_keystrokes(key);
        settle(cx);
        assert_eq!(active_path(&ws, cx).as_ref(), Some(path));
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), at));
    }
}

#[gpui::test]
fn indentation_controls_follow_the_buffer_without_rewriting_text(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "\tx");
    ws.update_in(cx, |ws, window, cx| {
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        ws.set_tab_size(&SetTabSize { size: 2 }, window, cx);
        ws.set_indent_style(&SetIndentStyle { to_spaces: false }, window, cx);
        ws.increase_font_size(&IncreaseFontSize, window, cx);
        for e in ws.views.tabs() {
            let s = e.read(cx).read_state(cx);
            assert_eq!(s.doc.tab_size, 2);
            assert!(!s.doc.translate_tabs);
            assert_eq!(s.text(), "\tx");
        }
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        let e = ws.active_editor().unwrap();
        assert_eq!(e.read(cx).doc(cx).tab_size, 2);
        assert!(!e.read(cx).doc(cx).translate_tabs);
        ws.set_tab_size(&SetTabSize { size: 0 }, window, cx);
        assert_eq!(e.read(cx).doc(cx).tab_size, 1);
    });
}

#[gpui::test]
fn selection_search_keeps_disjoint_scopes_and_resets_when_switching_buffers(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "aa aa aa");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 2), Selection::new(6, 8)]));
        ws.replace(&Replace, window, cx);
        set_query(ws, "aa", false, cx);
        ws.toggle_find_selection(&ToggleFindSelection, window, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).find_ranges, vec![0..2, 6..8]);
        ws.new_file(&NewFile, window, cx);
        ws.refresh_matches(cx);
        assert!(!ws.find_panel().unwrap().in_selection);
        assert!(ws.active_editor().unwrap().read(cx).find_ranges.is_empty());
    });
}

#[gpui::test]
fn find_in_files_recalls_queries_and_rejects_invalid_regex(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "old");
    ws.update_in(cx, |ws, window, cx| {
        find_text(ws, "old", false, window, cx);
        ws.find_next(&FindNext, window, cx);
        ws.find_in_files(&FindInFiles, window, cx);
        let q = ws.fif_panel().unwrap().query.clone();
        q.update(cx, |e, cx| e.set_text("draft", cx));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("up");
    ws.update_in(cx, |ws, window, cx| {
        let q = ws.fif_panel().unwrap().query.clone();
        assert_eq!(q.read(cx).text(cx), "old");
        q.update(cx, |e, cx| e.set_text("[", cx));
        ws.toggle_find_regex(&ToggleFindRegex, window, cx);
        ws.run_find_in_files(&RunFindInFiles, window, cx);
        assert!(ws.fif_panel().unwrap().error.is_some());
        assert!(ws.search.is_none());
    });
}

#[gpui::test]
fn regex_replace_next_does_not_skip_adjacent_consuming_matches(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "aaa");
    ws.update_in(cx, |ws, window, cx| replace_text(ws, "a", "_", true, window, cx));
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_selection(0, 1));
        ws.replace_one(&find_panel::ReplaceOne, window, cx);
    });
    assert_eq!(sels(&ws, cx), vec![Selection::new(1, 2)]);
}

#[gpui::test]
fn scoped_replace_next_advances_after_the_scope_has_grown(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a a");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).select_all());
        replace_text(ws, "a", "aa", false, window, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| {
        ws.toggle_find_selection(&ToggleFindSelection, window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_selection(0, 1));
        ws.replace_one(&find_panel::ReplaceOne, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(text(&ws, cx), "aa a");
    assert_eq!(sels(&ws, cx), vec![Selection::new(3, 4)]);
}

#[gpui::test]
fn background_search_uses_each_selections_boundary(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    find::SEARCH_BG.set(Some(2));
    type_in(&ws, cx, "aaa");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_selection(1, 3));
        ws.find(&Find, window, cx);
        ws.toggle_find_selection(&ToggleFindSelection, window, cx);
    });
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).find_ranges == std::slice::from_ref(&(1..3)))
    });
    find::SEARCH_BG.set(None);
}

#[gpui::test]
fn selection_search_starts_at_the_selection_boundary(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "aaa");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_selection(1, 3));
        ws.replace(&Replace, window, cx);
        ws.toggle_find_selection(&ToggleFindSelection, window, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).find_ranges, std::slice::from_ref(&(1..3)));
    });
}

#[gpui::test]
fn scoped_replace_all_keeps_adjacent_scopes_valid_after_growth(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "aa");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 1), Selection::new(1, 2)]));
        ws.replace(&Replace, window, cx);
        ws.find_panel_mut().unwrap().opts.regex = true;
        let r = ws.find_panel().unwrap().replace.clone();
        r.update(cx, |e, cx| e.set_text("aa", cx));
        ws.toggle_find_selection(&ToggleFindSelection, window, cx);
        ws.replace_all(&ReplaceAll, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(text(&ws, cx), "aaaa");
    ws.update_in(cx, |ws, window, cx| {
        let r = ws.find_panel().unwrap().replace.clone();
        r.update(cx, |e, cx| e.set_text("_", cx));
        ws.replace_all(&ReplaceAll, window, cx);
    });
    assert_eq!(text(&ws, cx), "____");
}

#[gpui::test]
fn replace_next_with_the_find_panel_closed_uses_the_last_replacement(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a a a");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.set_cursor(0)));
        ws.replace(&Replace, window, cx);
    });
    ws.update_in(cx, |ws, _, cx| {
        let p = ws.find_panel().unwrap();
        let (q, r) = (p.query.clone(), p.replace.clone());
        q.update(cx, |e, cx| e.set_text("a", cx));
        r.update(cx, |e, cx| e.set_text("b", cx));
        ws.find_panel_mut().unwrap().opts.whole_word = true;
    });
    ws.update_in(cx, |ws, window, cx| ws.close_panel(&ClosePanel, window, cx));
    for _ in 0..2 {
        ws.update_in(cx, |ws, window, cx| ws.replace_one(&find_panel::ReplaceOne, window, cx));
    }
    assert_eq!(text(&ws, cx), "b b a");
    // the panel opens again with the replacement and toggles it had
    ws.update_in(cx, |ws, window, cx| {
        ws.replace(&Replace, window, cx);
        let p = ws.find_panel().unwrap();
        assert_eq!(p.replace.read(cx).doc(cx).text(), "b");
        assert!(p.opts.whole_word);
    });
}

/// A fresh folder for a test's files, this test process's own.
pub(crate) fn temp(name: &str) -> PathBuf {
    let dir = (std::env::temp_dir()).join(format!("orlo-ui-tests-{}", std::process::id())).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `contents` written to `name` in `dir`: the file's path.
pub(crate) fn write_in(dir: &Path, name: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, contents).unwrap();
    p
}

pub(crate) fn tab_count(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    ws.read_with(cx, |ws, _| ws.views.tabs().count())
}

#[gpui::test]
fn a_file_saved_through_a_linked_folder_opens_in_the_same_tab(cx: &mut TestAppContext) {
    let dir = temp("orlo-save-as-link");
    std::fs::create_dir(dir.join("real")).unwrap();
    std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "hi");
    ws.update_in(vcx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
    let linked = dir.join("link/a.txt");
    vcx.simulate_new_path_selection(|_| Some(linked.clone()));
    vcx.run_until_parked();
    assert_eq!(std::fs::read_to_string(dir.join("real/a.txt")).unwrap(), "hi");
    let real = dir.join("real/a.txt").canonicalize().unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.open_path(real, window, cx));
    assert_eq!(tab_count(&ws, vcx), 1);
}

fn active_path(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<PathBuf> {
    ws.update(cx, |ws, cx| ws.active_editor()?.read(cx).doc(cx).path.clone())
}

/// The active view's caret.
fn caret(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    ws.update(cx, |ws, cx| ws.active_editor().unwrap().read(cx).state.cursor())
}

/// The line of the active view's caret.
fn caret_line(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    ws.update(cx, |ws, cx| {
        let s = ws.active_editor().unwrap().read(cx).read_state(cx);
        s.point(s.cursor()).0
    })
}

/// The window's session state, as a quit would write it.
fn capture(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> WindowState {
    ws.update_in(cx, |ws, window, cx| ws.capture(window, false, &mut Texts::new(), cx))
}

fn close_window(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, window, cx| ws.close_window(&CloseWindow, window, cx));
}

fn columns(n: usize) -> SetLayout {
    SetLayout { preset: Preset::Columns(n) }
}

/// A file big enough to be read in the background.
pub(crate) fn write_large(path: &Path) {
    std::fs::write(path, "line\n".repeat((LOAD_IN_BACKGROUND / 5 + 1) as usize)).unwrap();
}

/// The session's unsaved text for its first window's first view.
fn saved_draft() -> String {
    let saved = session::load(&session::path());
    let v = &saved.windows[0].groups[0].views[0];
    v.unsaved.as_ref().unwrap().text.clone()
}

/// `secs` seconds of the test clock, and what they let run.
fn advance(vcx: &mut VisualTestContext, secs: u64) {
    vcx.executor().advance_clock(std::time::Duration::from_secs(secs));
    vcx.run_until_parked();
}

#[gpui::test]
fn devices_and_pipes_are_refused_and_huge_files_ask_first(cx: &mut TestAppContext) {
    let dir = temp("orlo-open-odd");
    let fifo = dir.join("pipe");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let huge = dir.join("huge.log");
    std::fs::File::create(&huge).unwrap().set_len((1 << 30) + 1).unwrap();
    let (ws, vcx) = window(cx);
    for p in [fifo, PathBuf::from("/dev/zero")] {
        ws.update_in(vcx, |ws, window, cx| ws.open_path(p, window, cx));
        assert!(vcx.has_pending_prompt());
        vcx.simulate_prompt_answer("Ok");
    }
    ws.update_in(vcx, |ws, window, cx| ws.open_path(huge.clone(), window, cx));
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();
    assert_eq!(active_path(&ws, vcx), None, "only the blank untitled tab");
    assert_eq!(tab_count(&ws, vcx), 1);
}

#[gpui::test]
fn save_all_recovers_clean_deleted_files_once_per_buffer(cx: &mut TestAppContext) {
    let dir = temp("orlo-save-all-deleted-clean").canonicalize().unwrap();
    let files = [dir.join("a.txt"), dir.join("b.txt")];
    let (ws, vcx) = window(cx);
    for (p, contents) in files.iter().zip(["a", "b"]) {
        std::fs::write(p, contents).unwrap();
        ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    }
    new_view(&ws, vcx);
    for p in &files {
        std::fs::remove_file(p).unwrap();
    }
    ws.update_in(vcx, |ws, window, cx| ws.poll_disk(window, cx));
    settle_until(vcx, |cx| {
        ws.read_with(cx, |ws, cx| {
            ws.views.tabs().all(|t| {
                let s = t.read(cx).read_state(cx);
                s.doc.deleted && !s.doc.dirty
            })
        })
    });
    ws.update_in(vcx, |ws, window, cx| ws.save_all(&SaveAll, window, cx));
    assert!(ws.read_with(vcx, |ws, _| ws.saves.busy()), "Save As opens");
    for (p, contents) in files.iter().zip(["a", "b"]) {
        vcx.simulate_new_path_selection(|_| Some(p.clone()));
        vcx.run_until_parked();
        assert_eq!(std::fs::read_to_string(p).unwrap(), contents);
    }
    assert!(ws.read_with(vcx, |ws, cx| { !ws.saves.busy() && ws.views.tabs().all(|t| !t.read(cx).doc(cx).unsaved()) }));
}

#[gpui::test]
fn changing_syntax_refreshes_goto_symbols_without_editing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "fn main() {}\n");
    for (syntax, expected) in [("Rust", 1), ("Plain Text", 0), ("Rust", 1)] {
        ws.update_in(vcx, |ws, window, cx| {
            ws.cancel_quick_panel(cx);
            ws.set_syntax(&self::syntax(syntax), window, cx);
            ws.goto_symbol(&GotoSymbol, window, cx);
        });
        // the buffer's index entry, parsed as the syntax now set, fills the panel
        settle_until(vcx, |cx| {
            ws.read_with(cx, |ws, cx| {
                let e = ws.active_editor().unwrap();
                let entry = ws.index.read(cx).entry_for(&e, cx);
                let rows = ws.quick_panel.as_ref().map(|p| p.items.len());
                entry.is_some_and(|en| en.syntax == syntax) && rows == Some(expected)
            })
        });
        let labels = ws.read_with(vcx, |ws, _| {
            let items = &ws.quick_panel.as_ref().unwrap().items;
            items.iter().map(|i| i.label.clone()).collect::<Vec<_>>()
        });
        assert_eq!(labels.len(), expected, "symbols for {syntax}: {labels:?}");
        if expected == 1 {
            assert_eq!(labels, ["main"]);
        }
    }
}

#[gpui::test]
fn goto_symbol_uses_the_requesting_views_syntax_and_refreshes_after_customizations_reload(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "fn main() {}\n");
    ws.update_in(vcx, |ws, window, cx| {
        ws.set_syntax(&self::syntax("Rust"), window, cx);
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
    });
    for (syntax, expected) in [("Plain Text", 0), ("Rust", 1)] {
        ws.update_in(vcx, |ws, window, cx| {
            ws.set_syntax(&self::syntax(syntax), window, cx);
            ws.goto_symbol(&GotoSymbol, window, cx);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                let e = ws.active_editor().unwrap();
                let entry = ws.index.read(cx).entry_for(&e, cx);
                let rows = ws.quick_panel.as_ref().map(|p| p.items.len());
                entry.is_some_and(|entry| entry.syntax == syntax) && rows == Some(expected)
            })
        });
        vcx.simulate_keystrokes("escape");
    }
    ws.update_in(vcx, |ws, window, cx| {
        ws.reload_customizations(&ReloadCustomizations, window, cx);
        let e = ws.active_editor().unwrap();
        assert!(
            ws.index.read(cx).entry_for(&e, cx).is_none(),
            "the same version and syntax still need new grammar checkpoints"
        );
        ws.goto_symbol(&GotoSymbol, window, cx);
    });
    settle_rows(&ws, vcx, 1);
    ws.read_with(vcx, |ws, _| assert_eq!(ws.quick_panel.as_ref().unwrap().items[0].label, "main"));
}

#[gpui::test]
fn goto_symbol_on_a_100_mb_buffer_queues_the_parse_and_keeps_the_panel_responsive(cx: &mut TestAppContext) {
    let dir = temp("orlo-symbol-huge");
    let path = dir.join("huge.rs");
    let text = format!("{}\nfn last() {{}}\n", "x".repeat(100_000_000));
    let state = EditorState::from_text(&text);
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.opened(path.clone(), Ok(state), window, cx);
        ws.set_syntax(&syntax("Rust"), window, cx);
    });
    ws.update_in(vcx, |ws, window, cx| {
        let started = std::time::Instant::now();
        ws.goto_symbol(&GotoSymbol, window, cx);
        ws.quick_panel_input_changed(window, cx);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "navigation queues the parse without scanning the buffer"
        );
        ws.quick_panel_dismiss(&quick_panel::Dismiss, window, cx);
    });
    assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
    ws.update_in(vcx, |ws, window, cx| ws.goto_symbol(&GotoSymbol, window, cx));
    settle_rows(&ws, vcx, 1);
    ws.read_with(vcx, |ws, _| assert_eq!(ws.quick_panel.as_ref().unwrap().items[0].label, "last"));
}

#[gpui::test]
fn a_local_definition_request_does_not_jump_after_the_active_view_changes(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "fn target() {}\nfn caller() { target(); }\n");
    ws.update_in(vcx, |ws, window, cx| {
        ws.set_syntax(&syntax("Rust"), window, cx);
        ws.goto_symbol(&GotoSymbol, window, cx);
    });
    settle_rows(&ws, vcx, 2);
    let (source, cursor) = ws.update_in(vcx, |ws, window, cx| {
        ws.quick_panel_dismiss(&quick_panel::Dismiss, window, cx);
        let source = ws.active_editor().unwrap();
        let cursor = source.update(cx, |e, cx| {
            let cursor = e.doc(cx).text().rfind("target").unwrap();
            e.sel(cx).set_cursor(cursor);
            cursor
        });
        ws.local_definition(false, window, cx);
        ws.new_file(&NewFile, window, cx);
        (source, cursor)
    });
    vcx.run_until_parked();
    assert_eq!(source.read_with(vcx, |e, _| e.state.cursor()), cursor);
}

#[gpui::test]
fn goto_anything_keeps_open_files_outside_the_folder_cache(cx: &mut TestAppContext) {
    let dir = temp("orlo-goto-open-files").canonicalize().unwrap();
    let root = dir.join("project");
    std::fs::create_dir(&root).unwrap();
    let inside = root.join("inside.txt");
    let outside = dir.join("outside.txt");
    std::fs::write(&inside, "").unwrap();
    std::fs::write(&outside, "").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(root, window, cx));
    vcx.run_until_parked();
    for p in [&inside, &outside] {
        ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    }
    ws.update_in(vcx, |ws, window, cx| ws.goto_anything(&GotoAnything, window, cx));
    for _ in 0..2 {
        let paths = ws.read_with(vcx, |ws, _| {
            (ws.quick_panel.as_ref().unwrap().items.iter())
                .filter_map(|i| match &i.action {
                    ItemAction::OpenFile(p) => Some(p.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(paths.len(), 2);
        assert_eq!(paths.iter().filter(|p| **p == inside).count(), 1);
        assert_eq!(paths.iter().filter(|p| **p == outside).count(), 1);
        vcx.run_until_parked();
    }
}

#[gpui::test]
fn find_in_files_uses_unsaved_text_through_a_file_symlink(cx: &mut TestAppContext) {
    let dir = temp("orlo-fif-unsaved-file-link").canonicalize().unwrap();
    let root = dir.join("project");
    std::fs::create_dir(&root).unwrap();
    let real = write_in(&dir, "real.txt", "old");
    std::os::unix::fs::symlink(&real, root.join("link.txt")).unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_path(root, window, cx);
        ws.open_path(real.clone(), window, cx);
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert("needle");
            })
        });
        ws.find_in_files(&FindInFiles, window, cx);
        let p = ws.fif_panel().unwrap();
        let (q, w) = (p.query.clone(), p.where_.clone());
        q.update(cx, |e, cx| e.set_text("needle", cx));
        w.update(cx, |e, cx| e.set_text("<open folders>", cx));
        ws.run_find_in_files(&RunFindInFiles, window, cx);
    });
    settle(vcx);
    let results = text(&ws, vcx);
    assert!(results.contains("link.txt:"), "{results}");
    assert!(results.contains("needle"), "{results}");
    assert!(results.contains("1 match in 1 file"), "{results}");
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "old");
}

#[gpui::test]
fn a_missing_file_opens_as_an_empty_buffer_of_that_name(cx: &mut TestAppContext) {
    let dir = temp("orlo-open-missing").canonicalize().unwrap();
    let p = dir.join("new.txt");
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    assert!(!vcx.has_pending_prompt());
    assert_eq!(active_path(&ws, vcx), Some(p.clone()));
    assert_eq!(tab_count(&ws, vcx), 1, "it takes the blank untitled tab's place");
    type_in(&ws, vcx, "x");
    ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "x");
}

/// Run the app until `done`, letting the highlighting thread (a real one, which the test
/// executor does not wait for) finish its work. Twenty seconds, then the test fails saying so:
/// a slow CI machine fails clearly instead of flaking.
#[track_caller]
pub(crate) fn settle_until(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(20) {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("settle_until: still not done after 20 s");
}

/// Let background work and gpui timers (the search drain ticks every 100 ms) run.
pub(crate) fn settle(cx: &mut VisualTestContext) {
    for _ in 0..20 {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
    }
}

#[gpui::test]
fn closing_one_view_of_find_results_keeps_the_search_going_in_the_other(cx: &mut TestAppContext) {
    let dir = temp("orlo-fif-two-views");
    std::fs::write(dir.join("a.txt"), "a needle here\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.clone(), window, cx));
    ws.update_in(vcx, |ws, window, cx| ws.find_in_files(&FindInFiles, window, cx));
    ws.update_in(vcx, |ws, _, cx| {
        let q = ws.fif_panel().unwrap().query.clone();
        q.update(cx, |e, cx| e.set_text("needle", cx));
    });
    ws.update_in(vcx, |ws, window, cx| {
        ws.run_find_in_files(&RunFindInFiles, window, cx);
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
    });
    let first = ws.read_with(vcx, |ws, _| (ws.search.as_ref().unwrap().results.as_ref().unwrap()).upgrade().unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.request_close(first, After::Close, window, cx));
    settle(vcx);
    let t = text(&ws, vcx);
    assert!(t.contains("a needle here"), "{t}");
    assert!(!t.contains("cancelled"), "{t}");
}

#[gpui::test]
fn closing_one_view_of_a_dirty_buffer_asks_and_the_other_stays_dirty(cx: &mut TestAppContext) {
    // Sublime (build 4215) asks on either view; Don't Save closes just that one
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "edits");
    new_view(&ws, vcx);
    let second = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.request_close(second, After::Close, window, cx));
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer("Don't Save");
    vcx.run_until_parked();
    assert_eq!(tab_count(&ws, vcx), 1);
    assert_eq!(text(&ws, vcx), "edits");
    assert!(ws.read_with(vcx, |ws, cx| { ws.active_editor().unwrap().read(cx).doc(cx).dirty }));
}

#[gpui::test]
fn closing_the_last_window_with_discard_all_drops_the_drafts(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "draft");
    // the red button: a prompt, and the window stays until it is answered
    assert!(!vcx.simulate_close() && vcx.has_pending_prompt());
    vcx.simulate_prompt_answer("Discard All");
    vcx.run_until_parked();
    let saved = session::load(&session::path());
    assert!(saved.windows[0].groups.iter().all(|g| g.views.is_empty()), "the untitled draft is not kept");
}

#[gpui::test]
fn the_app_quitting_from_the_dock_keeps_unsaved_buffers(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    // main.rs's hook
    vcx.update(|_, cx| save_on_quit(cx));
    type_in(&ws, vcx, "draft");
    vcx.cx.update(|cx| cx.shutdown());
    let saved = session::load(&session::path());
    let v = &saved.windows[0].groups[0].views[0];
    assert_eq!(v.unsaved.as_ref().unwrap().text, "draft");
}

#[gpui::test]
fn quitting_keeps_unsaved_buffers_and_the_next_launch_brings_them_back(cx: &mut TestAppContext) {
    let dir = temp("orlo-hot-exit").canonicalize().unwrap();
    let file = write_in(&dir, "f.txt", "on disk\n");
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "draft");
    ws.update_in(vcx, |ws, window, cx| ws.open_path(file.clone(), window, cx));
    type_in(&ws, vcx, "edited ");
    new_view(&ws, vcx);
    // cmd-q: the session is written before Orlo goes
    vcx.simulate_keystrokes("cmd-q");
    vcx.run_until_parked();
    let saved = session::load(&session::path()).windows.into_iter().next();
    let st = saved.expect("the window was saved");
    let (ws2, vcx2) = window(vcx);
    ws2.update_in(vcx2, |ws, window, cx| ws.restore(st, window, cx));
    ws2.update(vcx2, |ws, cx| {
        let tabs: Vec<_> = ws.views.tabs().cloned().collect();
        assert_eq!(tabs.len(), 3);
        let s: Vec<_> = tabs.iter().map(|t| t.read(cx).read_state(cx)).collect();
        assert_eq!((s[0].text(), s[0].doc.path.clone(), s[0].doc.dirty), ("draft".into(), None, true));
        assert_eq!(s[1].text(), "edited on disk\n");
        assert_eq!(s[1].doc.path.as_ref(), Some(&file));
        assert!(s[1].doc.dirty && s[1].buffer_id() == s[2].buffer_id());
        assert!(s[0].buffer_id() != s[1].buffer_id());
    });
    // undo cannot make it clean: the edit happened before the quit
    ws2.update(vcx2, |ws, cx| {
        let e = ws.active_editor().unwrap();
        e.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert!(e.read(cx).doc(cx).dirty);
    });
}

/// A file big enough that opening it reads in the background.
fn big_file(dir: &Path) -> PathBuf {
    let p = write_in(dir, "big.txt", "x".repeat((LOAD_IN_BACKGROUND + 1) as usize));
    p.canonicalize().unwrap()
}

#[gpui::test]
fn a_file_read_in_the_background_does_not_take_the_find_panels_keys(cx: &mut TestAppContext) {
    let dir = temp("orlo-bg-focus");
    let big = big_file(&dir);
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "mine");
    ws.update_in(vcx, |ws, window, cx| ws.open_path(big.clone(), window, cx));
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    settle_until(vcx, |cx| tab_count(&ws, cx) == 2);
    assert_eq!(tab_count(&ws, vcx), 2, "the file is open");
    assert!(text(&ws, vcx) == "mine", "behind the tab the user was in");
    assert_find_field_has_the_keys(&ws, vcx);
}

#[track_caller]
fn assert_find_field_has_the_keys(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    ws.update_in(vcx, |ws, window, cx| {
        let q = ws.find_panel().unwrap().query.clone();
        assert!(q.read(cx).is_focused(window), "the find field keeps the keys");
    });
}

#[gpui::test]
fn a_background_read_of_a_file_opened_meanwhile_adds_no_view(cx: &mut TestAppContext) {
    let dir = temp("orlo-bg-twice");
    let big = big_file(&dir);
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_path(big.clone(), window, cx);
        // Find Results opens its files at once, without waiting for the read
        let state = EditorState::load(&big);
        ws.opened(big.clone(), state, window, cx);
        ws.set_layout(&columns(2), window, cx);
    });
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
    assert!(ws.read_with(vcx, |ws, _| ws.loading.is_empty()), "the read landed");
    assert_eq!(tab_count(&ws, vcx), 1);
}

/// Run `f` on the active view with its window, for input-method calls.
fn on_editor<R>(
    ws: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut Editor, &mut Window, &mut Context<Editor>) -> R,
) -> R {
    ws.update_in(cx, |ws, window, cx| ws.active_editor().unwrap().update(cx, |e, cx| f(e, window, cx)))
}

#[gpui::test]
fn dead_keys_and_input_methods_type_once_at_every_caret(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    // US International: the dead quote shows marked, a space commits it, typed once (paired)
    on_editor(&ws, vcx, |e, w, cx| {
        e.replace_and_mark_text_in_range(None, "\"", Some(1..1), w, cx);
        e.replace_text_in_range(None, "\"", w, cx);
    });
    assert_eq!(text(&ws, vcx), "\"\"");
    // ´ then e
    on_editor(&ws, vcx, |e, w, cx| {
        e.edit(cx, |s| s.select_all());
        e.replace_and_mark_text_in_range(None, "´", Some(1..1), w, cx);
        e.replace_text_in_range(None, "é", w, cx);
    });
    assert_eq!(text(&ws, vcx), "é");
    // a Japanese composition at two carets: one undo step, marked at both, committed at both
    on_editor(&ws, vcx, |e, w, cx| {
        e.edit(cx, |s| {
            s.select_all();
            s.insert("a\nb");
            s.set_cursor(1);
            s.add_cursor(3);
        });
        for step in ["k", "か", "かn", "かな"] {
            e.replace_and_mark_text_in_range(None, step, None, w, cx);
        }
        assert_eq!(e.marked.len(), 2);
        assert_eq!(e.doc(cx).text(), "aかな\nbかな");
        e.replace_text_in_range(None, "かな", w, cx);
        assert!(e.marked.is_empty());
        assert_eq!(e.doc(cx).text(), "aかな\nbかな");
        e.edit(cx, |s| s.undo());
        assert_eq!(e.doc(cx).text(), "a\nb", "the whole composition is one undo step");
    });
    // press-and-hold names the letter before the primary caret: every caret's goes
    on_editor(&ws, vcx, |e, w, cx| {
        e.edit(cx, |s| s.type_text("e"));
        let c = e.doc(cx).char_to_utf16(e.state.cursor());
        e.replace_text_in_range(Some(c - 1..c), "é", w, cx);
        assert_eq!(e.doc(cx).text(), "aé\nbé");
    });
    // UTF-16 ranges around an emoji
    on_editor(&ws, vcx, |e, w, cx| {
        e.edit(cx, |s| {
            s.select_all();
            s.insert("a😀b");
        });
        let mut actual = None;
        assert_eq!(e.text_for_range(1..3, &mut actual, w, cx).as_deref(), Some("😀"));
        e.replace_text_in_range(Some(1..3), "x", w, cx);
        assert_eq!(e.doc(cx).text(), "axb");
    });
}

#[gpui::test]
fn a_click_or_an_undo_ends_a_composition(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    on_editor(&ws, vcx, |e, w, cx| {
        e.replace_and_mark_text_in_range(None, "k", None, w, cx);
        assert_eq!(e.marked.first(), Some(&(0..1)));
        e.edit(cx, |s| s.undo());
        assert!(e.marked.is_empty());
        // the next key starts afresh instead of replacing text the undo moved
        e.replace_text_in_range(None, "x", w, cx);
        assert_eq!(e.doc(cx).text(), "x");
    });
}

#[gpui::test]
fn replace_next_replaces_only_the_match_it_is_on(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "foo bar foo foo");
    ws.update_in(vcx, |ws, window, cx| ws.replace(&Replace, window, cx));
    ws.update_in(vcx, |ws, _, cx| {
        let p = ws.find_panel().unwrap();
        let (q, r) = (p.query.clone(), p.replace.clone());
        q.update(cx, |e, cx| e.set_text("foo", cx));
        r.update(cx, |e, cx| e.set_text("X", cx));
    });
    // a caret on "bar" and the second foo selected (Sublime looks at the last only)
    ws.update_in(vcx, |ws, _, cx| {
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.set_selections(vec![Selection::caret(5), Selection::new(8, 11)])));
    });
    ws.update_in(vcx, |ws, window, cx| ws.replace_one(&find_panel::ReplaceOne, window, cx));
    assert_eq!(text(&ws, vcx), "foo bar X foo");
    let next = sels(&ws, vcx);
    assert_eq!(next, vec![Selection::new(10, 13)], "the next match, alone");
}

/// The active view's selections.
fn sels(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<Selection> {
    ws.update(cx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        e.read(cx).state.selections.clone()
    })
}

#[gpui::test]
fn escape_after_a_goto_line_preview_brings_back_every_selection_and_the_scroll(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    type_in(&ws, vcx, &text);
    let before = vec![Selection::new(7, 9), Selection::caret(20)];
    let top = ws.update_in(vcx, |ws, _, cx| {
        ws.with_active(cx, |e, cx| {
            e.restore_view(before.clone(), (70, 0.), cx);
            e.top(cx)
        })
        .unwrap()
    });
    for key in [":", "1", "5", "0"] {
        ws.update_in(vcx, |ws, window, cx| {
            if ws.quick_panel.is_none() {
                ws.goto_line(&GotoLine, window, cx);
            }
            let input = ws.quick_panel.as_ref().unwrap().input.clone();
            input.update(cx, |e, cx| e.edit(cx, |s| s.insert(key)));
        });
    }
    assert_ne!(sels(&ws, vcx), before, "the preview moved the caret");
    ws.update_in(vcx, |ws, window, cx| ws.quick_panel_dismiss(&quick_panel::Dismiss, window, cx));
    assert_eq!(sels(&ws, vcx), before);
    let after = ws.update(vcx, |ws, cx| ws.active_editor().unwrap().read(cx).top(cx));
    assert_eq!(after, top);
    // ⌘F during a preview starts from where the user was, not the previewed line
    ws.update_in(vcx, |ws, window, cx| {
        ws.goto_line(&GotoLine, window, cx);
        let input = ws.quick_panel.as_ref().unwrap().input.clone();
        input.update(cx, |e, cx| e.edit(cx, |s| s.insert(":150")));
    });
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    assert_eq!(sels(&ws, vcx), before);
}

#[gpui::test]
fn a_tab_closing_ends_goto_anything(cx: &mut TestAppContext) {
    // a click into the text ends it too (the editor's focus listener; checked by hand in the
    // app, as the test window does not report focus changes)
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "x");
    ws.update_in(vcx, |ws, window, cx| {
        ws.new_file(&NewFile, window, cx);
        ws.goto_anything(&GotoAnything, window, cx);
    });
    let first = ws.read_with(vcx, |ws, _| ws.views.tabs().next().cloned().unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.request_close(first, After::Close, window, cx));
    vcx.simulate_prompt_answer("Don't Save");
    vcx.run_until_parked();
    assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
}

#[gpui::test]
fn line_commands_put_no_line_break_into_the_find_field_but_a_paste_keeps_its_lines(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "some text");
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    vcx.simulate_keystrokes("a cmd-enter cmd-shift-enter cmd-shift-d cmd-alt-q cmd-/");
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("b\r\nc\nd".into()));
    vcx.simulate_keystrokes("cmd-v");
    let q = ws.read_with(vcx, |ws, cx| ws.find_panel().unwrap().query.read(cx).doc(cx).text());
    // the field is multiline, its line breaks `\n` as a buffer's are
    assert_eq!(q, "ab\nc\nd");
    assert_eq!(text(&ws, vcx), "some text", "nothing reached the view behind");
}

#[gpui::test]
fn colon_n_jumps_only_in_goto_anything(cx: &mut TestAppContext) {
    // Sublime 4215: not in the Command Palette, not after `@`
    let (ws, vcx) = window(cx);
    for (open, jumps) in [(0, true), (1, false)] {
        ws.update_in(vcx, |ws, window, cx| {
            ws.cancel_quick_panel(cx);
            match open {
                0 => ws.goto_anything(&GotoAnything, window, cx),
                _ => ws.command_palette(&CommandPalette, window, cx),
            }
            let input = ws.quick_panel.as_ref().unwrap().input.clone();
            input.update(cx, |e, cx| e.set_text(":5", cx));
        });
        let line = ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().unwrap().line_query());
        assert_eq!(line.is_some(), jumps, "panel {open}");
    }
}

#[gpui::test]
fn only_the_active_window_sets_the_menu_bar(cx: &mut TestAppContext) {
    // gpui's test platform keeps no menus, so this asks the windows whether they set them
    let (ws1, ws2, mut vcx1, mut vcx2) = two_windows(cx);
    vcx2.update(|window, _| window.activate_window());
    vcx2.run_until_parked();
    assert!(!ws1.update(&mut vcx1, |ws, cx| ws.set_menus_now(cx)), "inactive");
    assert!(ws2.update(&mut vcx2, |ws, cx| ws.set_menus_now(cx)), "active");
}

#[gpui::test]
fn new_window_works_with_no_window_to_take_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(Settings::default());
        cx.set_global(SettingsDisk(None));
        cx.set_global(Recent::default());
        global_actions(cx);
        assert!(cx.active_window().is_none());
        cx.dispatch_action(&NewWindow);
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
}

#[gpui::test]
fn jump_back_lands_on_the_same_text_after_an_edit_above_it(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "one\ntwo\nthree");
    on_editor(&ws, vcx, |e, _, cx| {
        e.record_jump(4); // "two"
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("zero\n");
        });
    });
    vcx.simulate_keystrokes("ctrl--");
    let at = ws.update(vcx, |ws, cx| {
        let s = ws.active_editor().unwrap().read(cx).read_state(cx);
        s.doc.rope.slice(s.cursor()..s.cursor() + 3).to_string()
    });
    assert_eq!(at, "two");
}

#[gpui::test]
fn ordinary_lossy_buffers_accept_native_input_commands_and_save_as(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let dir = temp("orlo-lossy-edit-save-as");
    let original = dir.join("original.txt");
    let saved = dir.join("saved.txt");
    std::fs::write(&original, b"a\xff").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, w, cx| ws.open_path(original.clone(), w, cx));
    on_editor(&ws, vcx, |e, _, cx| {
        assert!(e.doc(cx).read_only);
        e.select_range(0..0, cx);
    });
    vcx.simulate_keystrokes("x");
    assert_eq!(text(&ws, vcx), "xa�");
    on_editor(&ws, vcx, |e, w, cx| e.replace_and_mark_text_in_range(None, "仮", None, w, cx));
    assert_eq!(text(&ws, vcx), "x仮a�");
    on_editor(&ws, vcx, |e, w, cx| e.replace_text_in_range(None, "確", w, cx));
    assert_eq!(text(&ws, vcx), "x確a�");
    vcx.simulate_keystrokes("backspace");
    assert_eq!(text(&ws, vcx), "xa�");
    ws.update_in(vcx, |ws, w, cx| ws.save_as(&SaveAs, w, cx));
    let chosen = saved.clone();
    vcx.simulate_new_path_selection(move |_| Some(chosen.clone()));
    vcx.run_until_parked();
    assert_eq!(std::fs::read_to_string(saved).unwrap(), "xa�");
    assert_eq!(std::fs::read(original).unwrap(), b"a\xff");
    assert!(!on_editor(&ws, vcx, |e, _, cx| e.doc(cx).read_only));
}

#[gpui::test]
fn saving_a_deleted_lossy_file_asks_for_a_path(cx: &mut TestAppContext) {
    let dir = temp("orlo-save-deleted-lossy");
    let p = write_in(&dir, "latin1.txt", b"caf\xe9\n");
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    std::fs::remove_file(&p).unwrap();
    on_editor(&ws, vcx, |e, _, cx| {
        assert!(e.doc(cx).read_only);
        e.update_doc(cx, |d| d.deleted = true);
    });
    ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
    assert!(!vcx.has_pending_prompt(), "no \"Cannot save\" alert");
    assert!(vcx.did_prompt_for_new_path());
}

#[gpui::test]
fn new_file_in_a_fresh_window_shows_one_untitled_tab(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    assert_eq!(tab_count(&ws, vcx), 1);
    // with text in it, the next ⌘N adds a second
    type_in(&ws, vcx, "x");
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    assert_eq!(tab_count(&ws, vcx), 2);
}

#[gpui::test]
fn files_on_a_drive_not_mounted_stay_in_the_session(cx: &mut TestAppContext) {
    let away = PathBuf::from("/Volumes/orlo-not-mounted-26/proj");
    let gone = temp("orlo-session-gone").join("deleted.txt");
    let view = |p: &Path| ViewState { path: p.to_path_buf(), ..ViewState::default() };
    let st = WindowState {
        folder: Some(away.clone()),
        groups: vec![GroupState { views: vec![view(&away.join("a.txt")), view(&gone)], active: 0 }],
        ..WindowState::default()
    };
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.restore(st, window, cx));
    let back = capture(&ws, vcx);
    assert_eq!(back.folder, Some(away.clone()));
    let paths: Vec<_> = back.groups[0].views.iter().map(|v| &v.path).collect();
    assert_eq!(paths, [&away.join("a.txt")], "the deleted file goes, the unmounted one stays");
}

#[gpui::test]
fn a_window_saved_on_a_bigger_screen_opens_on_this_one(cx: &mut TestAppContext) {
    window(cx);
    // saved on this very display, back when it had more pixels
    let display = cx.update(|cx| cx.primary_display()?.uuid().ok().map(|u| u.to_string()));
    assert!(display.is_some());
    let st = WindowState { bounds: Some([3000., 2000., 2500., 900.]), display, ..WindowState::default() };
    let handle = cx.update(|cx| open_window(Some(st), cx));
    let b = handle.update(cx, |_, window, _| window.bounds()).unwrap();
    // the test display is 1920 x 1080
    assert!(f32::from(b.right()) <= 1920. && f32::from(b.bottom()) <= 1080., "{b:?}");
}

#[gpui::test]
fn save_as_opens_in_the_files_own_folder(cx: &mut TestAppContext) {
    // Sublime 4215: the file's folder, not the window's; untitled, the last file's
    let project = temp("orlo-save-as-project");
    let elsewhere = temp("orlo-save-as-elsewhere").canonicalize().unwrap();
    let file = write_in(&elsewhere, "f.txt", "x");
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_path(project.clone(), window, cx);
        ws.open_path(file.clone(), window, cx);
        ws.save_as(&SaveAs, window, cx);
    });
    let asks_in_elsewhere = |vcx: &mut VisualTestContext| {
        let asked = std::rc::Rc::new(std::cell::RefCell::new(None));
        let a = asked.clone();
        vcx.simulate_new_path_selection(move |dir| {
            *a.borrow_mut() = Some(dir.to_path_buf());
            None
        });
        vcx.run_until_parked();
        assert_eq!(asked.borrow().as_deref(), Some(elsewhere.as_path()));
    };
    asks_in_elsewhere(vcx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.new_file(&NewFile, window, cx);
        ws.save_as(&SaveAs, window, cx);
    });
    asks_in_elsewhere(vcx);
}

#[gpui::test]
fn page_down_moves_a_view_less_one_row(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let text: String = (0..300).map(|i| format!("{i}\n")).collect();
    type_in(&ws, vcx, &text);
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.set_cursor(0)));
    vcx.run_until_parked();
    let page = on_editor(&ws, vcx, |e, _, _| e.page_lines());
    assert_ne!(page, 40, "measured from the view, not fixed");
    vcx.simulate_keystrokes("pagedown");
    assert_eq!(caret_line(&ws, vcx), page);
}

#[gpui::test]
fn a_big_file_opens_with_the_lines_its_warm_up_coloured(cx: &mut TestAppContext) {
    let dir = temp("orlo-warm-up");
    let p = dir.join("big.rs");
    let lines = "fn f() {}\n".repeat((LOAD_IN_BACKGROUND / 10 + 1) as usize);
    std::fs::write(&p, lines).unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
    on_editor(&ws, vcx, |e, _, _| {
        assert_eq!(e.highlighter.syntax_name(), "Rust");
        assert!(e.highlighter.cached_lines() > 40, "the warm-up's lines were kept");
    });
}

#[gpui::test]
fn a_big_buffer_is_searched_off_the_ui_thread_as_the_query_is_typed(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let big = format!("{}needle\n", "x".repeat(find::SEARCH_IN_BACKGROUND + 10));
    type_in(&ws, vcx, &big);
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.set_cursor(0)));
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    ws.update_in(vcx, |ws, _, cx| set_query(ws, "needle", false, cx));
    // the frame after the keystroke does not wait for the search
    let outlines = |cx: &mut VisualTestContext| on_editor(&ws, cx, |e, _, _| e.find_ranges.len());
    settle_until(vcx, |cx| outlines(cx) == 1);
    assert_eq!(outlines(vcx), 1);
    assert_eq!(caret(&ws, vcx), big.len() - 1, "selected when it landed");
}

#[gpui::test]
fn a_hung_stat_holds_up_only_its_own_file(cx: &mut TestAppContext) {
    let dir = temp("orlo-poll-hung").canonicalize().unwrap();
    let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "b").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_path(a.clone(), window, cx);
        ws.open_path(b.clone(), window, cx);
        // a's last stat never came back (a dropped share), and the folder poll is stuck too
        ws.checking.insert(a.clone());
        ws.polling = true;
    });
    std::fs::write(&b, "b changed elsewhere").unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.poll_disk(window, cx));
    settle(vcx);
    assert_eq!(text(&ws, vcx), "b changed elsewhere");
}

#[gpui::test]
fn closing_every_tab_rebuilds_the_menu_bar_once(cx: &mut TestAppContext) {
    let dir = temp("orlo-close-all-menus");
    let (ws, vcx) = window(cx);
    vcx.update(|window, _| window.activate_window());
    for i in 0..20 {
        let p = write_in(&dir, format!("{i}.txt"), "x");
        ws.update_in(vcx, |ws, window, cx| ws.open_path(p, window, cx));
    }
    vcx.run_until_parked();
    let before = ws.read_with(vcx, |ws, _| ws.menus_built);
    ws.update_in(vcx, |ws, window, cx| ws.close_all(&CloseAll, window, cx));
    vcx.run_until_parked();
    assert_eq!(tab_count(&ws, vcx), 0);
    assert_eq!(ws.read_with(vcx, |ws, _| ws.menus_built) - before, 1);
}

#[gpui::test]
fn a_sessions_big_file_is_read_in_the_background_and_takes_its_place(cx: &mut TestAppContext) {
    let dir = temp("orlo-restore-big").canonicalize().unwrap();
    let (a, big, c) = (dir.join("a.txt"), dir.join("big.txt"), dir.join("c.txt"));
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&c, "c").unwrap();
    write_large(&big);
    let view = |p: &Path| ViewState { path: p.to_path_buf(), selections: vec![(10, 10)], ..ViewState::default() };
    let st = WindowState {
        groups: vec![GroupState { views: vec![view(&a), view(&big), view(&c)], active: 1 }],
        ..WindowState::default()
    };
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.restore(st, window, cx));
    // restored without waiting for the read, and a quit now keeps it in the session
    let kept = capture(&ws, vcx);
    assert!(kept.groups[0].views.iter().any(|v| v.path == big));
    settle_until(vcx, |cx| tab_count(&ws, cx) == 3);
    let order: Vec<_> =
        ws.read_with(vcx, |ws, cx| (ws.views.tabs()).map(|t| t.read(cx).doc(cx).path.clone().unwrap()).collect());
    assert_eq!(order, [a, big.clone(), c]);
    let at = (active_path(&ws, vcx), caret(&ws, vcx));
    assert_eq!(at, (Some(big), 10), "the active tab again, with its caret");
}

#[gpui::test]
fn a_find_results_line_in_a_big_file_lands_when_the_file_is_read(cx: &mut TestAppContext) {
    let dir = temp("orlo-result-big").canonicalize().unwrap();
    let big = dir.join("big.txt");
    write_large(&big);
    let (ws, vcx) = window(cx);
    let results = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.open_result(results, big.clone(), 1_000, None, window, cx));
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
    assert_eq!(caret_line(&ws, vcx), 1_000);
}

#[gpui::test]
fn closing_a_window_with_unsaved_tabs_does_what_its_prompt_says(cx: &mut TestAppContext) {
    let (ws, vcx, p) = window_on(cx, "orlo-close-window", "old");
    type_in(&ws, vcx, "new ");
    // Cancel: nothing happens
    close_window(&ws, vcx);
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();
    assert_eq!(vcx.windows().len(), 1);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
    // Save All: the file is written and the window goes
    close_window(&ws, vcx);
    vcx.simulate_prompt_answer("Save All");
    vcx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "new old");
    assert_eq!(vcx.windows().len(), 0);
}

#[gpui::test]
fn discard_all_closes_the_window_and_leaves_the_file(cx: &mut TestAppContext) {
    let (ws, vcx, p) = window_on(cx, "orlo-discard-all", "old");
    type_in(&ws, vcx, "new ");
    close_window(&ws, vcx);
    vcx.simulate_prompt_answer("Discard All");
    vcx.run_until_parked();
    assert_eq!(vcx.windows().len(), 0);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
}

#[gpui::test]
fn a_save_that_fails_says_why_and_keeps_the_tab_dirty(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let (ws, vcx, p) = window_on(cx, "orlo-save-fails-ui", "old");
    let dir = p.parent().unwrap();
    type_in(&ws, vcx, "new ");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
    let prompt = vcx.pending_prompt();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(prompt.map(|p| p.0), Some("Could not save file".into()));
    vcx.simulate_prompt_answer("Ok");
    assert!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).dirty));
    // and the poll finds nothing to offer
    ws.update_in(vcx, |ws, window, cx| ws.poll_disk(window, cx));
    settle(vcx);
    assert!(!vcx.has_pending_prompt());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
}

#[gpui::test]
fn a_session_comes_back_as_it_was_written(cx: &mut TestAppContext) {
    let dir = temp("orlo-session-round-trip").canonicalize().unwrap();
    let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
    std::fs::write(&a, "one\ntwo\nthree\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_path(dir.clone(), window, cx);
        ws.open_path(a.clone(), window, cx);
        ws.set_layout(&columns(2), window, cx);
        ws.open_path(b.clone(), window, cx);
    });
    on_editor(&ws, vcx, |e, _, cx| {
        e.edit(cx, |s| {
            s.set_selections(vec![Selection::new(0, 1)]);
            s.toggle_bookmark();
        })
    });
    let st = capture(&ws, vcx);
    let json = serde_json::to_string(&st).unwrap();
    let back: WindowState = serde_json::from_str(&json).unwrap();
    let (ws2, vcx2) = window(vcx);
    ws2.update_in(vcx2, |ws, window, cx| ws.restore(back, window, cx));
    let again = capture(&ws2, vcx2);
    assert_eq!(again.folder, st.folder);
    assert_eq!(again.layout, st.layout);
    assert_eq!(again.active_group, st.active_group);
    let views = |w: &WindowState| -> Vec<Vec<_>> {
        (w.groups.iter())
            .map(|g| (g.views.iter()).map(|v| (v.path.clone(), v.selections.clone(), v.bookmarks.clone())).collect())
            .collect()
    };
    assert_eq!(views(&again), views(&st));
}

#[gpui::test]
fn tests_never_touch_the_users_application_support(cx: &mut TestAppContext) {
    let real = std::env::home_dir().unwrap().join("Library/Application Support/Orlo");
    let stamp = || std::fs::metadata(real.join("Session.json")).and_then(|m| m.modified()).ok();
    let before = stamp();
    let (ws, vcx) = window(cx);
    for p in [session::path(), settings::path(), crate::app::handoff::socket_path()] {
        assert!(!p.starts_with(&real), "{}", p.display());
    }
    // closing the last window writes the session
    type_in(&ws, vcx, "x");
    ws.update_in(vcx, |ws, window, cx| ws.save_if_last(window, cx));
    assert!(session::path().exists());
    assert_eq!(stamp(), before);
}

#[gpui::test]
fn an_edit_in_one_view_moves_the_other_views_carets_bookmarks_and_jumps_once(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, &"x\n".repeat(100));
    new_view(&ws, vcx);
    // the new view is active: a jump at 60, a bookmark and the caret at 100
    on_editor(&ws, vcx, |e, _, cx| {
        e.record_jump(60);
        e.edit(cx, |s| {
            s.set_cursor(100);
            s.view.bookmarks = vec![100];
        });
    });
    let first = ws.read_with(vcx, |ws, _| ws.views.groups[0].tabs[0].clone());
    first.update(vcx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("y");
        })
    });
    let (cursor, bookmarks) = ws.read_with(vcx, |ws, cx| {
        let s = ws.active_editor().unwrap().read(cx).read_state(cx);
        (s.cursor(), s.view.bookmarks.clone())
    });
    assert_eq!((cursor, bookmarks), (101, vec![101]));
    vcx.simulate_keystrokes("ctrl--");
    assert_eq!(caret(&ws, vcx), 61);
    // a view focused runs its focus listener once (a frame reports focus changes)
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    ws.update(vcx, |ws, cx| {
        ws.views.select(0, 0);
        cx.notify();
    });
    vcx.update(|window, cx| window.draw(cx).clear());
    let runs = FOCUS_RUNS.get();
    vcx.update(|window, cx| {
        window.focus(&first.read(cx).focus_handle(cx));
        window.draw(cx).clear();
    });
    assert_eq!(FOCUS_RUNS.get() - runs, 1, "one focus listener per view");
}

#[gpui::test]
fn edits_reach_the_session_a_few_seconds_after_they_settle(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "draft");
    vcx.run_until_parked();
    assert!(!session::path().exists(), "not on every keystroke");
    advance(vcx, 3);
    // what a crash, `kill` or a closed Terminal leaves for the next launch
    assert_eq!(saved_draft(), "draft");
}

#[gpui::test]
fn quit_says_so_when_the_session_cannot_be_written(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "draft");
    // a file where the folder of unsaved texts goes
    std::fs::write(session::support_dir().join("Session"), "").unwrap();
    vcx.simulate_keystrokes("cmd-q");
    vcx.run_until_parked();
    assert!(vcx.has_pending_prompt(), "an alert, not a quit");
    vcx.simulate_prompt_answer("Cancel");
}

#[gpui::test]
fn a_permissions_change_is_not_an_outside_edit_but_a_rewrite_keeping_the_mtime_is(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let (ws, vcx, p) = window_on(cx, "orlo-ctime-only", "old\n");
    type_in(&ws, vcx, "x");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.poll_disk(window, cx));
    settle(vcx);
    assert!(!vcx.has_pending_prompt(), "chmod changes no text");
    // `cp -p`: other text of the same size, the mtime put back
    let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
    std::fs::write(&p, "new\n").unwrap();
    let f = std::fs::File::options().write(true).open(&p).unwrap();
    f.set_modified(mtime).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.poll_disk(window, cx));
    settle(vcx);
    assert!(vcx.has_pending_prompt(), "asked");
    vcx.simulate_prompt_answer("Cancel");
}

#[gpui::test]
fn a_late_big_buffer_search_leaves_the_selection_alone_while_typing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    // every buffer is a big one: searched in the background
    find::SEARCH_BG.set(Some(0));
    type_in(&ws, vcx, "xaa yaa zaa\n");
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    ws.update_in(vcx, |ws, _, cx| {
        let p = ws.find_panel().unwrap();
        let (q, r) = (p.query.clone(), p.replace.clone());
        // "aa" overlaps itself: an edit's matches need a whole search again
        q.update(cx, |e, cx| e.set_text("aa", cx));
        r.update(cx, |e, cx| e.set_text("b", cx));
    });
    settle(vcx);
    let sel = |cx: &mut VisualTestContext| on_editor(&ws, cx, |e, _, _| e.state.selections[0]);
    assert_eq!(sel(vcx), Selection::new(1, 3), "the typed query's match, once found");
    // typing in the text with the panel open: the search after it selects nothing
    on_editor(&ws, vcx, |e, _, cx| {
        e.edit(cx, |s| {
            s.set_cursor(12);
            s.insert("aa");
        })
    });
    settle(vcx);
    assert_eq!(sel(vcx), Selection::caret(14));
    let outlines = on_editor(&ws, vcx, |e, _, _| e.find_ranges.len());
    assert_eq!(outlines, 4, "the outlines caught up");
    // Replace Next goes on to the next match and stays there
    on_editor(&ws, vcx, |e, _, cx| e.select_range(5..7, cx));
    ws.update_in(vcx, |ws, window, cx| ws.replace_one(&find_panel::ReplaceOne, window, cx));
    settle(vcx);
    assert_eq!(text(&ws, vcx), "xaa yb zaa\naa");
    assert_eq!(sel(vcx), Selection::new(8, 10));
    find::SEARCH_BG.set(None);
}

#[gpui::test]
fn a_big_file_landing_in_a_fresh_window_leaves_the_keys_in_the_find_panel(cx: &mut TestAppContext) {
    let dir = temp("orlo-bg-focus-fresh");
    let big = big_file(&dir);
    let (ws, vcx) = window(cx);
    // the blank untitled tab is active, and gives way when the file lands
    ws.update_in(vcx, |ws, window, cx| ws.open_path(big.clone(), window, cx));
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
    assert_eq!(tab_count(&ws, vcx), 1, "the file took the blank tab's place");
    assert_find_field_has_the_keys(&ws, vcx);
}

#[gpui::test]
fn a_big_file_restored_from_the_session_leaves_the_keys_in_the_find_panel(cx: &mut TestAppContext) {
    let dir = temp("orlo-restore-focus");
    let big = big_file(&dir);
    let (ws, vcx) = window(cx);
    let st = WindowState {
        groups: vec![GroupState { views: vec![ViewState { path: big.clone(), ..ViewState::default() }], active: 0 }],
        ..WindowState::default()
    };
    ws.update_in(vcx, |ws, window, cx| {
        ws.restore(st, window, cx);
        ws.find(&Find, window, cx);
    });
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
    assert_eq!(tab_count(&ws, vcx), 1);
    assert_find_field_has_the_keys(&ws, vcx);
}

#[gpui::test]
fn a_file_opened_in_an_inactive_window_reaches_the_active_windows_menu_bar(cx: &mut TestAppContext) {
    let dir = temp("orlo-recent-inactive");
    let f = write_in(&dir, "a.txt", "a");
    let (ws1, ws2, mut vcx1, mut vcx2) = two_windows(cx);
    vcx2.update(|window, _| window.activate_window());
    vcx2.run_until_parked();
    let built = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| ws.read_with(vcx, |ws, _| ws.menus_built);
    let (before1, before2) = (built(&ws1, &mut vcx1), built(&ws2, &mut vcx2));
    ws1.update_in(&mut vcx1, |ws, window, cx| ws.open_path(f.clone(), window, cx));
    vcx1.run_until_parked();
    assert_eq!(built(&ws1, &mut vcx1), before1, "the inactive window sets nothing");
    assert_eq!(built(&ws2, &mut vcx2) - before2, 1, "the active one shows the new file");
}

#[gpui::test]
fn a_font_error_and_bad_settings_keys_are_both_shown(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.apply_settings(r#"{"font_face": "No Such Font 27", "tab_size": "wide"}"#, window, cx)
    });
    let (_, detail) = vcx.pending_prompt().expect("an alert");
    assert!(detail.contains("No Such Font 27"), "{detail}");
    assert!(detail.contains("tab_size"), "{detail}");
    vcx.simulate_prompt_answer("Ok");
}

#[gpui::test]
fn a_highlighting_job_that_panics_is_not_asked_for_again_at_once(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, &"let x = 1;\n".repeat(100));
    on_editor(&ws, vcx, |e, _, cx| {
        e.highlighter.panics = true;
        e.highlight_in_background(50, 60, cx);
    });
    let state = |cx: &mut VisualTestContext| on_editor(&ws, cx, |e, _, _| e.highlighting());
    // a frame's own job may be under way: each frame asks again, until one has panicked
    settle_until(vcx, |cx| {
        on_editor(&ws, cx, |e, _, cx| e.highlight_in_background(50, 60, cx));
        state(cx).1
    });
    // the next frame would ask again: it waits
    let (again, waiting) = on_editor(&ws, vcx, |e, _, cx| {
        e.highlight_in_background(50, 60, cx);
        e.highlighting()
    });
    assert!(waiting && !again);
}

#[gpui::test]
fn goto_anything_lists_files_from_a_walk_off_the_ui_thread_and_keeps_up(cx: &mut TestAppContext) {
    let dir = temp("orlo-goto-walk").canonicalize().unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "").unwrap();
    std::fs::write(dir.join("sub/b.txt"), "").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.clone(), window, cx));
    vcx.run_until_parked();
    let items = |ws: &Workspace| ws.quick_panel.as_ref().map_or(0, |p| p.items.len());
    // cmd-p: no walk while the keystroke is handled, the list from the walk at once
    let (walks, listed) = ws.update_in(vcx, |ws, window, cx| {
        let before = quick_panel::WALKS.get();
        ws.goto_anything(&GotoAnything, window, cx);
        (quick_panel::WALKS.get() - before, items(ws))
    });
    assert_eq!((walks, listed), (0, 2));
    // a file added since: the walk behind the panel brings it in
    std::fs::write(dir.join("c.txt"), "").unwrap();
    ws.update_in(vcx, |ws, window, cx| {
        ws.cancel_quick_panel(cx);
        ws.goto_anything(&GotoAnything, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(ws.read_with(vcx, |ws, _| items(ws)), 3);
}

/// Two clicks at `at`, the second a double click's.
fn double_click(vcx: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>) {
    for click_count in [1, 2] {
        vcx.simulate_event(gpui::MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: gpui::Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        vcx.simulate_event(gpui::MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: gpui::Modifiers::default(),
            click_count,
        });
    }
}

#[gpui::test]
fn a_double_click_on_a_tabs_close_button_closes_one_tab(cx: &mut TestAppContext) {
    // the neighbour that slides under the pointer does not take the second click
    let dir = temp("orlo-tab-close-double");
    let (ws, vcx) = window(cx);
    for n in ["a", "b", "c"] {
        let p = write_in(&dir, n, n);
        ws.update_in(vcx, |ws, window, cx| ws.open_path(p, window, cx));
    }
    vcx.run_until_parked();
    let close = vcx.debug_bounds("tab-close-0").expect("the first tab's close button");
    double_click(vcx, close.center());
    assert_eq!(tab_count(&ws, vcx), 2);
}

#[gpui::test]
fn a_bookmark_dot_stays_on_a_long_line_scrolled_to_its_end(cx: &mut TestAppContext) {
    // a line over 4,096 chars, scrolled right, keeps its dot
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, &"x".repeat(9_000));
    on_editor(&ws, vcx, |e, _, cx| {
        e.wrap_override = Some(false);
        e.edit(cx, |s| {
            s.set_cursor(9_000);
            s.toggle_bookmark();
        });
    });
    vcx.run_until_parked();
    let before = editor::BOOKMARK_DOTS.get();
    on_editor(&ws, vcx, |_, _, cx| cx.notify());
    vcx.run_until_parked();
    assert!(editor::BOOKMARK_DOTS.get() > before, "the dot is drawn");
}

#[gpui::test]
fn find_as_you_type_starts_after_the_last_region_and_counts_one_match_selected(cx: &mut TestAppContext) {
    // as Sublime 4215 does it
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a x a x a");
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.set_selections(vec![Selection::new(0, 1), Selection::new(4, 5)])));
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    ws.update_in(vcx, |ws, _, cx| set_query(ws, "a", false, cx));
    let (sels, status) = ws.update(vcx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        (e.state.selections.clone(), ws.position_text(e, cx))
    });
    assert_eq!(sels, [Selection::new(8, 9)], "after the last region, not the first");
    assert_eq!(status, "3 of 3 matches");
    // two selections, both matches: the total alone
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.set_selections(vec![Selection::new(0, 1), Selection::new(4, 5)])));
    let status = ws.update(vcx, |ws, cx| ws.position_text(ws.active_editor().unwrap().read(cx), cx));
    assert_eq!(status, "3 matches");
}

#[gpui::test]
fn clicking_a_find_toggle_leaves_the_keys_in_the_find_field(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "Abc abc");
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    vcx.run_until_parked();
    let toggle = vcx.debug_bounds("case").expect("the case toggle");
    vcx.simulate_click(toggle.center(), gpui::Modifiers::default());
    ws.update_in(vcx, |ws, window, cx| {
        let p = ws.find_panel().unwrap();
        assert!(p.opts.case_sensitive, "the click toggled");
        assert!(p.query.read(cx).is_focused(window), "the field kept the keys");
    });
}

#[gpui::test]
fn a_tab_opening_ends_goto_anything(cx: &mut TestAppContext) {
    // a tab closing ends Goto Anything; so does a tab opening
    let dir = temp("orlo-goto-tab-opening");
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "x");
    ws.update_in(vcx, |ws, window, cx| {
        ws.goto_anything(&GotoAnything, window, cx);
        ws.new_file(&NewFile, window, cx);
    });
    assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()), "cmd-n");
    // a file opened another way (Finder, `Orlo a.txt`)
    ws.update_in(vcx, |ws, window, cx| {
        ws.goto_anything(&GotoAnything, window, cx);
        ws.open_path(dir.join("a.txt"), window, cx);
    });
    assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()), "a file");
}

#[gpui::test]
fn an_input_methods_commit_with_a_range_lands_at_every_caret(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "ae\nae");
    // press-and-hold on "e" at two carets: the IME replaces the letter before the primary
    on_editor(&ws, vcx, |e, window, cx| {
        e.edit(cx, |s| s.set_selections(vec![Selection::caret(2), Selection::caret(5)]));
        e.replace_text_in_range(Some(4..5), "é", window, cx);
    });
    assert_eq!(text(&ws, vcx), "aé\naé");
    // a composition at both carets, then its commit: the committed text once at each
    on_editor(&ws, vcx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, "k", None, window, cx);
        e.replace_and_mark_text_in_range(None, "か", None, window, cx);
        e.replace_text_in_range(None, "か", window, cx);
    });
    assert_eq!(text(&ws, vcx), "aéか\naéか");
    let sels = on_editor(&ws, vcx, |e, _, _| e.state.selections.clone());
    assert_eq!(sels, [Selection::caret(3), Selection::caret(7)]);
}

/// Every text file under the test's support folder (Session/ or a kept copy), by contents.
fn kept_texts() -> Vec<String> {
    fn walk(d: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "txt") {
                out.push(std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }
    let mut out = Vec::new();
    walk(&session::support_dir(), &mut out);
    out
}

/// The test's support folder with `json` as its session and `texts` in Session/, then a launch
/// as main.rs makes it and one autosave cycle.
fn launch_after(cx: &mut TestAppContext, json: &str, texts: &[(&str, &str)]) {
    let dir = no_window(cx);
    std::fs::create_dir_all(dir.join("Session")).unwrap();
    std::fs::write(session::path(), json).unwrap();
    for (n, t) in texts {
        std::fs::write(dir.join("Session").join(n), t).unwrap();
    }
    launch(cx);
    cx.executor().advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
}

const UNSAVED: &str = r#"{"windows": [{"groups": [{"views": [
    {"path": "", "unsaved": {"file": "a.txt"}, "buffer": 1},
    {"path": "", "unsaved": {"file": "b.txt"}, "buffer": 2}]}]}]}"#;

#[gpui::test]
fn a_corrupt_session_keeps_its_drafts(cx: &mut TestAppContext) {
    launch_after(cx, "{oops", &[("a.txt", "draft a"), ("b.txt", "draft b")]);
    let kept = kept_texts();
    assert!(kept.contains(&"draft a".into()) && kept.contains(&"draft b".into()), "{kept:?}");
}

#[gpui::test]
fn a_session_with_a_missing_text_keeps_the_other_drafts(cx: &mut TestAppContext) {
    // a.txt is gone; b.txt is restored; c.txt is named by nothing
    launch_after(cx, UNSAVED, &[("b.txt", "draft b"), ("c.txt", "draft c")]);
    let kept = kept_texts();
    assert!(kept.contains(&"draft b".into()) && kept.contains(&"draft c".into()), "{kept:?}");
}

#[gpui::test]
fn a_view_beyond_the_layout_keeps_its_draft(cx: &mut TestAppContext) {
    let json = r#"{"windows": [{"groups": [
        {"views": [{"path": "", "unsaved": {"file": "a.txt"}, "buffer": 1}]},
        {"views": [{"path": "", "unsaved": {"file": "b.txt"}, "buffer": 2}]}]}]}"#;
    launch_after(cx, json, &[("a.txt", "draft a"), ("b.txt", "draft b")]);
    let kept = kept_texts();
    assert!(kept.contains(&"draft b".into()), "{kept:?}");
}

#[gpui::test]
fn a_failed_backup_of_unplaced_views_blocks_session_autosave(cx: &mut TestAppContext) {
    let dir = no_window(cx);
    let path = session::path();
    let json = r#"{"windows":[{"groups":[
        {"views":[{"unsaved":{"file":"a.txt"},"buffer":1}]},
        {"views":[{"unsaved":{"file":"b.txt"},"buffer":2}]}]}]}"#;
    std::fs::create_dir_all(dir.join("Session")).unwrap();
    std::fs::write(&path, json).unwrap();
    std::fs::write(dir.join("Session/a.txt"), "draft a").unwrap();
    std::fs::write(dir.join("Session/b.txt"), "unplaced draft b").unwrap();
    crate::editing::text::FAULT.set(Some(("write", std::io::ErrorKind::Other)));
    launch(cx);
    crate::editing::text::FAULT.set(None);
    cx.update(|cx| {
        assert!(cx.global::<SessionBlocked>().0.is_some());
        with_writer(&path, |writer| {
            assert!(writer.blocked.is_some());
            assert!(writer.owned.is_empty());
        });
        let capture = capture_session(None, true, cx);
        let n = CAPTURED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        crate::editing::text::FAULT.set(Some(("write", std::io::ErrorKind::Other)));
        let result = with_writer(&path, |writer| write_session(writer, &path, n, &capture, false));
        crate::editing::text::FAULT.set(None);
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), json);
        with_writer(&path, |writer| write_session(writer, &path, n, &capture, false)).unwrap();
    });
    assert!(kept_texts().contains(&"unplaced draft b".into()));
}

#[gpui::test]
fn an_unreadable_session_is_not_written_over_and_keeps_its_drafts(cx: &mut TestAppContext) {
    // a session that cannot be read as text (a bad block, a sync client's half write)
    let dir = no_window(cx);
    std::fs::create_dir_all(dir.join("Session")).unwrap();
    let broken = [UNSAVED.as_bytes(), &[0xff, 0xfe]].concat();
    std::fs::write(session::path(), &broken).unwrap();
    std::fs::write(dir.join("Session/a.txt"), "draft a").unwrap();
    launch(cx);
    cx.executor().advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(std::fs::read(session::path()).unwrap(), broken, "not written over");
    assert!(kept_texts().contains(&"draft a".into()));
}

/// main.rs's launch.
fn launch(cx: &mut TestAppContext) {
    cx.update(|cx| {
        restore_session(cx);
    });
}

/// The test's support folder, with the window that set it up closed again.
fn no_window(cx: &mut TestAppContext) -> PathBuf {
    let (_ws, vcx) = window(cx);
    vcx.update(|window, _| window.remove_window());
    session::support_dir()
}

#[gpui::test]
fn the_session_is_written_within_30_s_however_long_typing_goes_on(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let mut written_at = None;
    for k in 0..20 {
        type_in(&ws, vcx, "x");
        advance(vcx, 2);
        if session::path().exists() {
            written_at = Some(2 * (k + 1));
            break;
        }
    }
    assert!(written_at.is_some_and(|s| s <= 32), "{written_at:?}");
}

#[gpui::test]
fn failed_autosaves_are_tried_again_and_shown_after_three(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    // a file where the folder of unsaved texts goes: every write fails
    let blocker = session::support_dir().join("Session");
    std::fs::write(&blocker, "").unwrap();
    type_in(&ws, vcx, "draft");
    for _ in 0..30 {
        advance(vcx, 1);
    }
    let status = |vcx: &mut VisualTestContext| {
        ws.update(vcx, |ws, cx| (session_unsaved(cx), ws.position_text(ws.active_editor().unwrap().read(cx), cx)))
    };
    assert!(status(vcx).0, "three failures are shown");
    std::fs::remove_file(&blocker).unwrap();
    for _ in 0..70 {
        advance(vcx, 1);
    }
    assert!(!status(vcx).0, "a write that works clears it");
    assert_eq!(saved_draft(), "draft");
}

#[gpui::test]
fn closing_a_window_or_changing_line_endings_reaches_the_session(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let w2 = vcx.cx.update(|cx| open_window(None, cx));
    let mut vcx2 = VisualTestContext::from_window(w2.into(), &vcx.cx);
    advance(&mut vcx2, 3);
    let windows = || session::load(&session::path()).windows.len();
    assert_eq!(windows(), 2);
    w2.update(&mut vcx2, |ws, window, cx| ws.close_window(&CloseWindow, window, cx)).unwrap();
    advance(vcx, 3);
    assert_eq!(windows(), 1, "the closed window left the session");
    let dir = temp("orlo-line-ending-session");
    let f = write_in(&dir, "a.txt", "a\n");
    ws.update_in(vcx, |ws, window, cx| ws.open_path(f.clone(), window, cx));
    advance(vcx, 3);
    ws.update_in(vcx, |ws, window, cx| ws.set_line_ending(&SetLineEnding { ending: "\r\n" }, window, cx));
    advance(vcx, 3);
    let saved = session::load(&session::path());
    let v = &saved.windows[0].groups[0].views[0];
    assert_eq!(v.unsaved.as_ref().map(|u| u.line_ending.as_str()), Some("\r\n"));
}

#[gpui::test]
fn after_an_edit_a_big_buffers_outlines_move_and_the_search_waits_for_a_pause(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    find::SEARCH_BG.set(Some(0));
    type_in(&ws, vcx, "xaa yaa");
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    ws.update_in(vcx, |ws, _, cx| set_query(ws, "aa", false, cx));
    vcx.run_until_parked();
    let outlines = |vcx: &mut VisualTestContext| on_editor(&ws, vcx, |e, _, _| e.find_ranges.clone());
    assert_eq!(outlines(vcx), [1..3, 5..7]);
    // "aa" overlaps itself: an edit needs a whole search, which waits for edits to pause
    on_editor(&ws, vcx, |e, _, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("z");
        })
    });
    vcx.run_until_parked();
    assert_eq!(outlines(vcx), [2..4, 6..8], "still on their text");
    let searched = |vcx: &mut VisualTestContext| {
        on_editor(&ws, vcx, |e, _, cx| e.read_state(cx).found("aa", &FindOpts::default()).is_some())
    };
    assert!(!searched(vcx), "not searched again before the pause");
    vcx.executor().advance_clock(std::time::Duration::from_millis(300));
    vcx.run_until_parked();
    assert!(searched(vcx));
    assert_eq!(outlines(vcx), [2..4, 6..8]);
    find::SEARCH_BG.set(None);
}

#[gpui::test]
fn a_search_landing_for_text_edited_since_starts_no_other_before_the_pause(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    find::SEARCH_BG.set(Some(0));
    type_in(&ws, vcx, "xaa yaa");
    ws.update_in(vcx, |ws, window, cx| {
        ws.find(&Find, window, cx);
        let q = ws.find_panel().unwrap().query.clone();
        // the search starts, and the text changes before it lands
        q.update(cx, |e, cx| e.set_text("aa", cx));
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("z");
            })
        });
    });
    vcx.run_until_parked();
    let searched = on_editor(&ws, vcx, |e, _, cx| e.read_state(cx).found("aa", &FindOpts::default()).is_some());
    assert!(!searched, "the next search waits for edits to pause");
    find::SEARCH_BG.set(None);
}

#[gpui::test]
fn goto_anythings_walk_keeps_the_highlighted_file_and_walks_again_when_asked(cx: &mut TestAppContext) {
    let dir = temp("orlo-goto-walk-again").canonicalize().unwrap();
    for n in ["b.txt", "c.txt"] {
        std::fs::write(dir.join(n), "").unwrap();
    }
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.clone(), window, cx));
    vcx.run_until_parked();
    ws.update_in(vcx, |ws, window, cx| {
        ws.goto_anything(&GotoAnything, window, cx);
        ws.quick_panel_next(&quick_panel::SelectNext, window, cx);
    });
    let highlighted = |vcx: &mut VisualTestContext| {
        ws.read_with(vcx, |ws, _| {
            let p = ws.quick_panel.as_ref().unwrap();
            p.items[p.filtered[p.selected]].label.clone()
        })
    };
    let before = highlighted(vcx);
    // a file sorting first appears, and the walk behind the panel lands
    std::fs::write(dir.join("a.txt"), "").unwrap();
    vcx.run_until_parked();
    assert_eq!(highlighted(vcx), before);
    // asked for twice at once: the second walk runs after the first
    let walks = quick_panel::WALKS.get();
    ws.update(vcx, |ws, cx| {
        ws.walk_files(cx);
        ws.walk_files(cx);
    });
    vcx.run_until_parked();
    assert_eq!(quick_panel::WALKS.get() - walks, 2);
}

#[gpui::test]
fn a_file_landing_on_the_blank_tab_that_had_the_keys_gives_them_to_the_file(cx: &mut TestAppContext) {
    let file = big_file(&temp("orlo-focus-blank-gave-way"));
    let (ws, vcx) = window(cx);
    // the file is read while the keys move to a new blank tab in another group, where it lands
    ws.update_in(vcx, |ws, window, cx| {
        ws.focus_editor(window, cx);
        ws.open_path(file.clone(), window, cx);
        ws.set_layout(&columns(2), window, cx);
        ws.focus_group(&FocusGroup { group: 1 }, window, cx);
        ws.new_file(&NewFile, window, cx);
    });
    let landed = |cx: &mut VisualTestContext| {
        ws.read_with(cx, |ws, cx| ws.views.tabs().any(|t| t.read(cx).doc(cx).path.is_some()))
    };
    settle_until(vcx, landed);
    let (in_front, file_has_keys) = ws.update_in(vcx, |ws, window, cx| {
        let e = ws.active_editor().unwrap();
        let in_front = e.read(cx).doc(cx).path.as_ref() == Some(&file);
        (in_front, e.read(cx).focus_handle(cx).is_focused(window))
    });
    assert_eq!((in_front, file_has_keys), (true, true));
}

#[gpui::test]
fn a_restored_file_landing_after_the_blank_tab_with_the_keys_gave_way_moves_the_keys(cx: &mut TestAppContext) {
    let big = big_file(&temp("orlo-focus-restore-gave-way"));
    let group = |views| GroupState { views, active: 0 };
    let view = ViewState { path: big.clone(), ..ViewState::default() };
    let st = WindowState {
        layout: Layout::preset(Preset::Columns(2)),
        groups: vec![group(vec![]), group(vec![view])],
        active_group: 0,
        ..WindowState::default()
    };
    let (ws, vcx) = window(cx);
    // read with the find panel up and the keys in a new blank tab, which gives way to the file
    let blank = ws.update_in(vcx, |ws, window, cx| {
        ws.restore(st, window, cx);
        ws.new_file(&NewFile, window, cx);
        ws.find(&Find, window, cx);
        ws.focus_editor(window, cx);
        ws.active_editor().unwrap().read(cx).focus_handle(cx)
    });
    settle_until(vcx, |cx| ws.read_with(cx, |ws, cx| ws.views.tabs().any(|t| t.read(cx).doc(cx).path.is_some())));
    let gone = ws.read_with(vcx, |ws, cx| !ws.views.tabs().any(|t| t.read(cx).focus_handle(cx) == blank));
    let kept = vcx.update(|window, _| blank.is_focused(window));
    assert_eq!((gone, kept), (true, false));
}

#[gpui::test]
fn a_save_while_the_text_is_compared_with_the_disk_is_not_asked_about(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let p = temp("orlo-compare-then-save").canonicalize().unwrap().join("f.txt");
    std::fs::write(&p, "old\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    type_in(&ws, vcx, "x");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    let s = crate::files::watch::stamp(&p).unwrap();
    // a chmod's comparison starts, and the buffer is saved before its read
    ws.update_in(vcx, |ws, window, cx| {
        let e = ws.active_editor().unwrap();
        ws.ask_if_text_changed(e, s, window, cx);
        ws.save(&Save, window, cx);
    });
    settle(vcx);
    assert!(!vcx.has_pending_prompt(), "the text just saved is no outside edit");
}

#[gpui::test]
fn a_session_write_a_crash_cut_short_is_swept_at_launch(cx: &mut TestAppContext) {
    let dir = no_window(cx);
    std::fs::create_dir_all(&dir).unwrap();
    let tmp = write_in(&dir, "Session.json.orlo-99999-1.tmp", "{");
    launch(cx);
    cx.run_until_parked();
    assert!(!tmp.exists());
}

#[gpui::test]
fn a_signal_writes_the_session_as_quit_does(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "draft");
    // what `quit_on_signals`'s thread hands the UI thread when a signal arrives
    let arrived = crate::app::handoff::Inbox::default();
    vcx.update(|_, cx| quit_on(arrived.clone(), cx));
    arrived.push(Vec::new());
    // a few seconds before the autosave would
    vcx.run_until_parked();
    assert_eq!(saved_draft(), "draft");
}

#[test]
fn a_second_signal_or_ten_seconds_more_end_orlo_as_the_signal_would() {
    use signal_hook::consts::{SIGINT, SIGTERM};
    // one signal, then the watchdog after `wait`: what it ended Orlo with
    let one = |wait| {
        let (tx, rx) = std::sync::mpsc::channel();
        let tell = crate::app::handoff::Inbox::default();
        let die = move |s| tx.send(s).unwrap();
        watch_signals([SIGTERM].into_iter(), tell, wait, die);
        rx
    };
    let minute = std::time::Duration::from_secs(60);
    assert!(one(minute).try_recv().is_err(), "not before its time");
    assert_eq!(one(std::time::Duration::ZERO).recv(), Ok(SIGTERM));
    // a second signal: at once, however long the wait
    let (tx, rx) = std::sync::mpsc::channel();
    let tell = crate::app::handoff::Inbox::default();
    watch_signals([SIGTERM, SIGINT].into_iter(), tell.clone(), minute, move |s| tx.send(s).unwrap());
    assert_eq!(rx.try_recv(), Ok(SIGINT));
    // told once
    use std::future::Future as _;
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::pin::pin!(tell.next()).poll(&mut cx).is_ready());
    assert!(std::pin::pin!(tell.next()).poll(&mut cx).is_pending());
}

#[gpui::test]
fn quit_waits_two_seconds_for_a_write_under_way_then_says_so(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "draft");
    let quit_says = |vcx: &mut VisualTestContext| {
        vcx.simulate_keystrokes("cmd-q");
        vcx.run_until_parked();
        let (_, detail) = vcx.pending_prompt().expect("an alert, not a quit");
        assert!(detail.contains("still being written after 2 s"), "{detail}");
        vcx.simulate_prompt_answer("Cancel");
    };
    // an autosave's write that does not end (a stuck disk), holding the session
    let held = writer(&session::path());
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let (locked, is_locked) = std::sync::mpsc::channel();
    let stuck = std::thread::spawn(move || {
        let _w = held.lock().unwrap();
        locked.send(()).unwrap();
        let _ = wait.recv();
    });
    is_locked.recv().unwrap();
    quit_says(vcx);
    drop(done);
    stuck.join().unwrap();
    // nothing under way, and Quit's own write sticks
    *STUCK.lock().unwrap() = Some(session::path());
    quit_says(vcx);
    *STUCK.lock().unwrap() = None;
}

#[gpui::test]
fn quit_takes_hand_offs_again_while_its_alert_is_open(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "draft");
    let sock = session::support_dir().join("orlo.sock");
    let inbox = crate::app::handoff::Inbox::default();
    assert!(crate::app::handoff::listen(&sock, inbox.clone()));
    // a file where the folder of unsaved texts goes: the write fails
    std::fs::write(session::support_dir().join("Session"), "").unwrap();
    vcx.simulate_keystrokes("cmd-q");
    vcx.run_until_parked();
    assert!(vcx.has_pending_prompt(), "an alert, not a quit");
    // `Orlo f` meanwhile: handed to this Orlo, not given up after five seconds of trying
    // a thread stands in for the UI: it accepts the delivery once it has taken it
    let receiver = std::thread::spawn(move || {
        use std::future::Future;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let mut task = std::task::Context::from_waker(std::task::Waker::noop());
            if let std::task::Poll::Ready(delivery) = std::pin::pin!(inbox.next_delivery()).poll(&mut task) {
                assert_eq!(delivery.paths, [PathBuf::from("/tmp/f")]);
                delivery.accept();
                return;
            }
            assert!(std::time::Instant::now() < deadline, "no handoff arrived");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    });
    assert!(crate::app::handoff::hand_off(&sock, &[PathBuf::from("/tmp/f")]));
    receiver.join().unwrap();
    vcx.simulate_prompt_answer("Cancel");
    crate::app::handoff::stop_listening();
}

/// A launch whose session file cannot be read, beside a draft of its own, then a draft typed
/// in this run: the window, and the unreadable file's bytes.
fn launch_blocked(cx: &mut TestAppContext) -> (Entity<Workspace>, VisualTestContext, Vec<u8>) {
    let dir = no_window(cx);
    std::fs::create_dir_all(dir.join("Session")).unwrap();
    let broken = [UNSAVED.as_bytes(), &[0xff, 0xfe]].concat();
    std::fs::write(session::path(), &broken).unwrap();
    std::fs::write(dir.join("Session/a.txt"), "draft a").unwrap();
    let w = cx.update(restore_session)[0];
    let mut vcx = VisualTestContext::from_window(w.into(), cx);
    vcx.run_until_parked();
    vcx.simulate_prompt_answer("Keep It Untouched");
    let ws = w.update(&mut vcx, |_, _, cx| cx.entity()).unwrap();
    type_in(&ws, &mut vcx, "this run");
    (ws, vcx, broken)
}

/// After a quit with the session blocked: this run's draft is in the session, and the file
/// that could not be read is kept byte for byte, with its draft.
#[track_caller]
fn kept_both(broken: &[u8]) {
    assert_eq!(saved_draft(), "this run");
    let bad = (std::fs::read_dir(session::support_dir()).unwrap().flatten())
        .map(|e| e.path())
        .find(|p| p.is_file() && p.to_string_lossy().ends_with(".bad"))
        .expect("the unreadable file kept");
    assert_eq!(std::fs::read(bad).unwrap(), broken);
    assert!(kept_texts().contains(&"draft a".into()));
}

#[gpui::test]
fn quit_with_the_session_unreadable_moves_it_aside_and_keeps_this_runs_drafts(cx: &mut TestAppContext) {
    let (_ws, vcx, broken) = launch_blocked(cx);
    // as the menu does it, outside any window's update
    vcx.cx.update(quit);
    vcx.run_until_parked();
    assert!(!vcx.has_pending_prompt(), "written, so no alert");
    kept_both(&broken);
}

#[gpui::test]
fn the_apps_quit_with_the_session_unreadable_keeps_this_runs_drafts(cx: &mut TestAppContext) {
    // the Dock's Quit, logout
    let (_ws, vcx, broken) = launch_blocked(cx);
    vcx.cx.update(app_quitting);
    kept_both(&broken);
}

#[gpui::test]
fn a_signal_with_the_session_unreadable_keeps_this_runs_drafts(cx: &mut TestAppContext) {
    let (_ws, mut vcx, broken) = launch_blocked(cx);
    let arrived = crate::app::handoff::Inbox::default();
    vcx.update(|_, cx| quit_on(arrived.clone(), cx));
    arrived.push(Vec::new());
    vcx.run_until_parked();
    kept_both(&broken);
}

#[gpui::test]
fn the_file_menu_says_again_that_the_session_could_not_be_read(cx: &mut TestAppContext) {
    let (ws, mut vcx, broken) = launch_blocked(cx);
    let item = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| {
        let name = "Session Could Not Be Read…";
        (menu(ws, vcx, "File").iter()).any(|i| matches!(i, MenuItem::Action { name: n, .. } if n == name))
    };
    assert!(item(&ws, &mut vcx));
    ws.update_in(&mut vcx, |ws, window, cx| ws.session_problem(&SessionProblem, window, cx));
    vcx.run_until_parked();
    vcx.simulate_prompt_answer("Start a New Session");
    vcx.run_until_parked();
    assert!(!item(&ws, &mut vcx), "gone once the file is moved aside");
    advance(&mut vcx, 3);
    kept_both(&broken);
}

/// The seconds of the test clock at which the session file was written while `each` ran once
/// a second (each write renames a new file in).
fn writes_over(
    vcx: &mut VisualTestContext,
    seconds: usize,
    mut each: impl FnMut(usize, &mut VisualTestContext),
) -> Vec<usize> {
    use std::os::unix::fs::MetadataExt;
    let ino = || std::fs::metadata(session::path()).map(|m| m.ino()).ok();
    let (mut last, mut at) = (ino(), Vec::new());
    for t in 1..=seconds {
        each(t, vcx);
        advance(vcx, 1);
        if ino() != last {
            last = ino();
            at.push(t);
        }
    }
    at
}

#[gpui::test]
fn steady_typing_writes_the_session_about_every_30_s(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let at = writes_over(vcx, 120, |t, vcx| {
        if t % 2 == 1 {
            type_in(&ws, vcx, "x");
        }
    });
    // 30 s after the oldest change not yet written, not on every keystroke after that
    assert_eq!(at, [30, 60, 90, 120], "written at {at:?}");
}

#[gpui::test]
fn failed_autosaves_wait_longer_each_time_however_much_typing_goes_on(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    // a file where the folder of unsaved texts goes: every write fails
    std::fs::write(session::support_dir().join("Session"), "").unwrap();
    let (mut tried, mut shown) = (Vec::new(), None);
    writes_over(vcx, 100, |t, vcx| {
        // what the second before did
        if AUTOSAVES.get() > tried.len() {
            tried.push(t - 1);
        }
        if shown.is_none() && vcx.update(|_, cx| session_unsaved(cx)) {
            shown = Some(t - 1);
        }
        if t % 2 == 1 {
            type_in(&ws, vcx, "x");
        }
    });
    // the typing holds the first for 30 s; then 2, 4, 8, 16 and 32 s after each failure
    assert_eq!(tried, [30, 32, 36, 44, 60, 92]);
    assert_eq!(shown, Some(36), "after the third");
}

#[gpui::test]
fn a_write_under_way_is_kept_and_what_came_meanwhile_follows_in_one_more(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    // Quit's write on a stuck disk holds the session meanwhile
    let held = writer(&session::path());
    let guard = held.lock().unwrap();
    type_in(&ws, vcx, "a");
    advance(vcx, 3);
    let started = AUTOSAVES.get();
    for t in ["b", "c"] {
        type_in(&ws, vcx, t);
        advance(vcx, 3);
    }
    assert_eq!(AUTOSAVES.get(), started, "one write at a time");
    assert!(!session::path().exists());
    drop(guard);
    advance(vcx, 3);
    assert_eq!(AUTOSAVES.get(), started + 1, "then one more, for both edits");
    assert_eq!(saved_draft(), "abc");
}

#[gpui::test]
fn closing_a_window_with_nothing_unsaved_leaves_the_session_soon_after(cx: &mut TestAppContext) {
    let (_ws, vcx) = window(cx);
    let w2 = vcx.cx.update(|cx| open_window(None, cx));
    let mut vcx2 = VisualTestContext::from_window(w2.into(), &vcx.cx);
    advance(&mut vcx2, 3);
    let windows = || session::load(&session::path()).windows.len();
    assert_eq!(windows(), 2);
    // its close button: nothing to ask, so the window just goes
    assert!(vcx2.simulate_close());
    vcx2.update(|window, _| window.remove_window());
    advance(vcx, 3);
    assert_eq!(windows(), 1);
}

#[gpui::test]
fn another_buffers_edit_leaves_the_active_views_search_going(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    find::SEARCH_BG.set(Some(0));
    // a view behind the active one, of another buffer (Find Results streaming in, a reload)
    let other = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, vcx, "xaa yaa");
    vcx.run_until_parked();
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    ws.update_in(vcx, |ws, _, cx| set_query(ws, "aa", false, cx));
    // the search is under way when the other buffer changes
    assert!(ws.read_with(vcx, |ws, _| ws.searching.is_some()));
    other.update(vcx, |e, cx| e.edit(cx, |s| s.insert("results")));
    vcx.run_until_parked();
    let outlines = on_editor(&ws, vcx, |e, _, _| e.find_ranges.clone());
    assert_eq!(outlines, [1..3, 5..7], "landed without waiting for a pause");
}

/// Release only, with the benchmarks: `cargo test --release bench_ -- --ignored`.
#[gpui::test]
#[ignore]
fn bench_a_keystroke_with_the_find_panel_open_on_10m_matches(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let text = std::fs::read_to_string(crate::testing::bench::big_file()).unwrap();
    type_in(&ws, vcx, &text);
    drop(text);
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    ws.update_in(vcx, |ws, _, cx| set_query(ws, "e", false, cx));
    let outlines = |vcx: &mut VisualTestContext| on_editor(&ws, vcx, |e, _, _| e.find_ranges.len());
    settle_until(vcx, |vcx| outlines(vcx) > 5_000_000);
    let n = outlines(vcx);
    on_editor(&ws, vcx, |e, _, cx| {
        let at = e.doc(cx).len() / 3;
        e.edit(cx, |s| s.set_cursor(at))
    });
    vcx.run_until_parked();
    crate::testing::bench::time("a keystroke, find panel open on 10M matches", 3, 300., || {
        type_in(&ws, vcx, "x");
        vcx.run_until_parked();
    });
    // the outlines kept up with the typing ("x" holds no "e")
    assert_eq!(outlines(vcx), n);
}

#[gpui::test]
fn a_launch_that_keeps_the_session_aside_says_where(cx: &mut TestAppContext) {
    launch_after(cx, "{oops", &[("a.txt", "draft a")]);
    let w = cx.windows()[0];
    let vcx = VisualTestContext::from_window(w, cx);
    let (title, detail) = vcx.pending_prompt().expect("said");
    assert_eq!(title, "The session was kept aside");
    let kept = (std::fs::read_dir(session::support_dir()).unwrap().flatten())
        .map(|e| e.file_name().into_string().unwrap())
        .find(|n| n.starts_with("Session.json.") && n.ends_with(".bad"))
        .unwrap();
    assert!(detail.contains(&kept), "{detail}");
    vcx.simulate_prompt_answer("Ok");
}

#[gpui::test]
fn quit_writes_the_session_once(cx: &mut TestAppContext) {
    use std::os::unix::fs::MetadataExt;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "draft");
    vcx.cx.update(quit);
    let ino = || std::fs::metadata(session::path()).unwrap().ino();
    let written = ino();
    // the app's own quit, which follows
    vcx.cx.update(app_quitting);
    assert_eq!(ino(), written, "not written again");
}

/// The status bar's left text.
fn left_text(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> String {
    ws.read_with(vcx, |ws, cx| ws.left_text(cx))
}

#[gpui::test]
fn status_message_shows_then_clears(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "abc");
    assert_eq!(left_text(&ws, vcx), "Line 1, Column 4    UTF-8");
    ws.update(vcx, |ws, cx| ws.status_message("No more occurrences", cx));
    assert_eq!(left_text(&ws, vcx), "No more occurrences");
    advance(vcx, 3);
    assert_eq!(left_text(&ws, vcx), "No more occurrences");
    // a newer message restarts the four seconds
    ws.update(vcx, |ws, cx| ws.status_message("SHA-256 copied", cx));
    advance(vcx, 3);
    assert_eq!(left_text(&ws, vcx), "SHA-256 copied");
    advance(vcx, 2);
    assert_eq!(left_text(&ws, vcx), "Line 1, Column 4    UTF-8");
}

#[gpui::test]
fn an_editors_status_reaches_the_status_bar(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    ws.update(vcx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        e.update(cx, |e, cx| e.status("No more occurrences", cx));
    });
    assert_eq!(left_text(&ws, vcx), "No more occurrences");
}

type Calls = std::rc::Rc<std::cell::RefCell<Vec<(String, Entity<Editor>)>>>;

/// An input on the active view whose callback records the text and view it gets, then answers.
fn open_input(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, answer: Result<(), String>) -> Calls {
    let calls = Calls::default();
    let sink = calls.clone();
    ws.update_in(vcx, |ws, window, cx| {
        ws.show_input(
            "Text:",
            "",
            Box::new(move |values, editor, _, _, _| {
                sink.borrow_mut().push((values.text("text").to_string(), editor));
                answer.clone()
            }),
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    calls
}

fn form_error(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Option<Option<String>> {
    ws.read_with(vcx, |ws, _| match &ws.panel {
        Some(Panel::Form(p)) => Some(p.error.clone()),
        _ => None,
    })
}

#[gpui::test]
fn form_submits_values_to_the_target_editor(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "doc");
    let calls = open_input(&ws, vcx, Ok(()));
    vcx.simulate_input("abc");
    vcx.simulate_keystrokes("enter");
    let target = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    assert_eq!(*calls.borrow(), vec![("abc".to_string(), target.clone())]);
    assert_eq!(form_error(&ws, vcx), None, "the panel closed");
    vcx.update(|window, cx| {
        assert!(target.read(cx).is_focused(window), "focus is back");
    });
    assert_eq!(text(&ws, vcx), "doc");
}

#[gpui::test]
fn form_error_keeps_panel_open(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "doc");
    let calls = open_input(&ws, vcx, Err("bad".into()));
    vcx.simulate_input("x");
    vcx.simulate_keystrokes("enter");
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(form_error(&ws, vcx), Some(Some("bad".to_string())));
    assert_eq!(text(&ws, vcx), "doc");
    // the panel still takes keys: a second try goes through the same callback
    vcx.simulate_keystrokes("enter");
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(calls.borrow()[1].0, "x");
}

#[gpui::test]
fn form_escape_closes_without_calling(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "doc");
    let calls = open_input(&ws, vcx, Ok(()));
    vcx.simulate_input("abc");
    vcx.simulate_keystrokes("escape");
    assert!(calls.borrow().is_empty());
    assert_eq!(form_error(&ws, vcx), None, "the panel closed");
    assert_eq!(text(&ws, vcx), "doc");
}

#[gpui::test]
fn form_closes_when_target_changes(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "doc");
    let calls = open_input(&ws, vcx, Ok(()));
    ws.update_in(vcx, |ws, window, cx| {
        ws.new_file(&NewFile, window, cx);
        // Enter in the form's field, as if the user clicked back into it
        ws.submit_form(&form::Submit, window, cx);
    });
    assert!(calls.borrow().is_empty());
    assert_eq!(form_error(&ws, vcx), None, "the panel closed");
}

#[gpui::test]
fn form_closes_when_target_is_closed(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let calls = open_input(&ws, vcx, Ok(()));
    ws.update_in(vcx, |ws, window, cx| {
        // a second tab stays, so the form has an active view that is not its target
        ws.new_file(&NewFile, window, cx);
        ws.views.select(0, 0);
        ws.close_tab(&CloseTab, window, cx);
        ws.submit_form(&form::Submit, window, cx);
    });
    assert!(calls.borrow().is_empty());
    assert_eq!(form_error(&ws, vcx), None, "the panel closed");
}

#[gpui::test]
fn form_fields_toggles_choices_and_tab(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "doc");
    let got = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = got.clone();
    let field = |key, kind, initial: &str| Field { key, label: key, kind, initial: initial.to_string() };
    ws.update_in(vcx, |ws, window, cx| {
        ws.show_form(
            "Process",
            vec![
                field("first", FieldKind::Text, "x"),
                field("case", FieldKind::Toggle(false), ""),
                field("mode", FieldKind::Choice(vec!["keep", "drop", "mark"]), "drop"),
                field("second", FieldKind::Text, "y"),
            ],
            Box::new(move |v, _, _, _, _| {
                *sink.borrow_mut() =
                    Some((v.text("first").to_string(), v.text("second").to_string(), v.flag("case"), v.choice("mode")));
                Ok(())
            }),
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    for button in ["case", "mode"] {
        let b = vcx.debug_bounds(button).expect(button);
        vcx.simulate_click(b.center(), gpui::Modifiers::default());
    }
    // the clicks left the keys in the first field, whose text starts selected
    vcx.simulate_input("1");
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("2");
    vcx.simulate_keystrokes("shift-tab");
    vcx.simulate_input("3");
    // Tab skips the buttons and wraps
    vcx.simulate_keystrokes("tab tab");
    vcx.simulate_input("4");
    vcx.simulate_keystrokes("enter");
    let want = ("134".to_string(), "2".to_string(), true, 2);
    assert_eq!(*got.borrow(), Some(want));
    assert_eq!(form_error(&ws, vcx), None, "the panel closed");
}

fn history(vcx: &mut VisualTestContext) -> Vec<(String, bool)> {
    vcx.update(|_, cx| {
        let h = cx.default_global::<crate::editing::clipboard_history::ClipboardHistory>();
        (h.entries().iter()).map(|e| (e.text.clone(), e.whole_lines)).collect()
    })
}

fn set_selections(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, ranges: &[(usize, usize)]) {
    let sels = ranges.iter().map(|&(a, b)| Selection::new(a, b)).collect();
    ws.update(vcx, |ws, cx| ws.with_active(cx, |e, cx| e.sel(cx).set_selections(sels)));
}

/// The labels of the rows the quick panel lists, if one is open.
fn panel_labels(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Option<Vec<String>> {
    ws.read_with(vcx, |ws, _| {
        let p = ws.quick_panel.as_ref()?;
        Some(p.filtered.iter().map(|&i| p.items[i].label.clone()).collect())
    })
}

#[gpui::test]
fn copy_and_cut_record_the_text_and_the_whole_lines_flag(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "one\ntwo");
    set_selections(&ws, vcx, &[(4, 4)]);
    vcx.simulate_keystrokes("cmd-c");
    assert_eq!(history(vcx), [("two\n".to_string(), true)]);
    set_selections(&ws, vcx, &[(0, 3)]);
    vcx.simulate_keystrokes("cmd-x");
    assert_eq!(text(&ws, vcx), "\ntwo");
    assert_eq!(history(vcx), [("one".to_string(), false), ("two\n".to_string(), true)]);
}

#[gpui::test]
fn paste_from_history_whole_line_entry_pastes_above_line(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "one\ntwo\nthree\nfour\nfive");
    set_selections(&ws, vcx, &[(4, 4)]);
    vcx.simulate_keystrokes("cmd-c");
    // line 4, with something else on the system clipboard
    set_selections(&ws, vcx, &[(14, 14)]);
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("other".into()));
    vcx.simulate_keystrokes("cmd-k cmd-v");
    assert!(ws.read_with(vcx, |ws, _| {
        (ws.quick_panel.as_ref()).is_some_and(|p| p.kind == quick_panel::Kind::ClipboardHistory)
    }));
    vcx.simulate_keystrokes("enter");
    let pasted = "one\ntwo\nthree\ntwo\nfour\nfive";
    assert_eq!(text(&ws, vcx), pasted);
    assert_eq!(sels(&ws, vcx), [Selection::new(18, 18)]);
    // one undo step restores text and caret; redo pastes again
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "one\ntwo\nthree\nfour\nfive");
    assert_eq!(sels(&ws, vcx), [Selection::new(14, 14)]);
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, vcx), pasted);
    // the system clipboard has the entry back, with its flag: cmd-v pastes it the same way
    vcx.simulate_keystrokes("cmd-v");
    assert_eq!(text(&ws, vcx), "one\ntwo\nthree\ntwo\ntwo\nfour\nfive");
}

#[gpui::test]
fn paste_from_history_distributes_over_carets(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a b c\n\n\n");
    set_selections(&ws, vcx, &[(0, 1), (2, 3), (4, 5)]);
    vcx.simulate_keystrokes("cmd-c");
    assert_eq!(history(vcx), [("a\nb\nc".to_string(), false)]);
    let carets = [(6, 6), (7, 7), (8, 8)];
    set_selections(&ws, vcx, &carets);
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("other".into()));
    vcx.simulate_keystrokes("cmd-k cmd-v enter");
    assert_eq!(text(&ws, vcx), "a b c\na\nb\nc");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "a b c\n\n\n");
    let carets: Vec<_> = carets.iter().map(|&(a, b)| Selection::new(a, b)).collect();
    assert_eq!(sels(&ws, vcx), carets);
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, vcx), "a b c\na\nb\nc");
}

#[gpui::test]
fn paste_from_history_lists_previews_and_filters_by_them(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    history_of_alpha_and_beta(&ws, vcx);
    assert_eq!(panel_labels(&ws, vcx).unwrap(), ["beta", "alpha"]);
    vcx.simulate_input("alp");
    assert_eq!(panel_labels(&ws, vcx).unwrap(), ["alpha"]);
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "alpha beta alpha");
    // the choice moved to the top and onto the system clipboard
    assert_eq!(history(vcx), [("alpha".to_string(), false), ("beta".to_string(), false)]);
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "alpha");
}

/// "alpha beta " typed, each word copied, and the history panel opened at the end.
fn history_of_alpha_and_beta(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    type_in(ws, vcx, "alpha beta ");
    for at in [(0, 5), (6, 10)] {
        set_selections(ws, vcx, &[at]);
        vcx.simulate_keystrokes("cmd-c");
    }
    set_selections(ws, vcx, &[(11, 11)]);
    vcx.simulate_keystrokes("cmd-k cmd-v");
}

/// "one\ntwo" in the history and on the clipboard, the document's caret at its start.
fn copy_two_lines(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    type_in(ws, vcx, "one\ntwo");
    set_selections(ws, vcx, &[(0, 7)]);
    vcx.simulate_keystrokes("cmd-c");
    set_selections(ws, vcx, &[(0, 0)]);
}

#[gpui::test]
fn paste_from_history_pastes_every_line_into_the_focused_find_field(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    copy_two_lines(&ws, vcx);
    ws.update_in(vcx, |ws, window, cx| ws.find(&Find, window, cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-k cmd-v enter");
    ws.update_in(vcx, |ws, window, cx| {
        let q = ws.find_panel().unwrap().query.clone();
        assert_eq!(q.read(cx).text(cx), "one\ntwo");
        assert!(q.read(cx).is_focused(window), "the keys are back in the field");
    });
    assert_eq!(text(&ws, vcx), "one\ntwo", "the document is untouched");
}

#[gpui::test]
fn paste_from_history_pastes_into_a_find_in_files_field(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    copy_two_lines(&ws, vcx);
    ws.update_in(vcx, |ws, window, cx| ws.find_in_files(&FindInFiles, window, cx));
    vcx.run_until_parked();
    // Tab moves to the Where field
    vcx.simulate_keystrokes("tab cmd-k cmd-v enter");
    ws.update_in(vcx, |ws, window, cx| {
        let p = ws.fif_panel().unwrap();
        assert_eq!(p.query.read(cx).text(cx), "");
        assert_eq!(p.where_.read(cx).text(cx), "one");
        assert!(p.where_.read(cx).is_focused(window));
    });
    assert_eq!(text(&ws, vcx), "one\ntwo");
}

#[gpui::test]
fn paste_from_history_pastes_into_a_form_field(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    copy_two_lines(&ws, vcx);
    let got = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = got.clone();
    let field = |key| Field { key, label: key, kind: FieldKind::Text, initial: String::new() };
    ws.update_in(vcx, |ws, window, cx| {
        ws.show_form(
            "Process",
            vec![field("first"), field("second")],
            Box::new(move |v, _, _, _, _| {
                *sink.borrow_mut() = Some((v.text("first").to_string(), v.text("second").to_string()));
                Ok(())
            }),
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("tab cmd-k cmd-v enter");
    // the form still has the keys: Enter submits it
    vcx.simulate_keystrokes("enter");
    assert_eq!(*got.borrow(), Some((String::new(), "one".to_string())));
    assert_eq!(text(&ws, vcx), "one\ntwo");
}

#[gpui::test]
fn copying_in_the_history_panels_own_input_does_not_change_the_entry_pasted(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    history_of_alpha_and_beta(&ws, vcx);
    vcx.simulate_input("alp");
    // the copy lands at the top of the history, a row ahead of the one on screen
    vcx.simulate_keystrokes("cmd-a cmd-c");
    assert_eq!(history(vcx)[0].0, "alp");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "alpha beta alpha");
}

#[gpui::test]
fn paste_from_history_with_nothing_copied_shows_one_row_that_does_nothing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "text");
    vcx.simulate_keystrokes("cmd-k cmd-v");
    assert_eq!(panel_labels(&ws, vcx).unwrap(), ["Clipboard history is empty"]);
    vcx.simulate_keystrokes("enter");
    assert_eq!(panel_labels(&ws, vcx), None, "the panel closed");
    assert_eq!(text(&ws, vcx), "text");
    assert!(history(vcx).is_empty());
}

#[gpui::test]
fn clear_clipboard_history_keeps_the_system_clipboard(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "text");
    vcx.simulate_keystrokes("cmd-a cmd-c");
    assert_eq!(history(vcx).len(), 1);
    vcx.dispatch_action(ClearClipboardHistory);
    assert!(history(vcx).is_empty());
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "text");
}

#[gpui::test]
fn clipboard_history_not_in_session(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "SECRET-CLIP");
    vcx.simulate_keystrokes("cmd-a cmd-x");
    assert_eq!(history(vcx), [("SECRET-CLIP".to_string(), false)]);
    type_in(&ws, vcx, "draft");
    vcx.simulate_keystrokes("cmd-q");
    vcx.run_until_parked();
    let session = std::fs::read_to_string(session::path()).unwrap();
    assert!(!session.contains("SECRET-CLIP"));
    let kept = kept_texts();
    assert!(kept.iter().any(|t| t == "draft"), "the draft was saved");
    assert!(kept.iter().all(|t| !t.contains("SECRET-CLIP")));
}

#[gpui::test]
fn cmd_u_after_cmd_d_chain(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "foo foo foo");
    let ranges = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| {
        let sels = self::sels(ws, vcx);
        sels.iter().map(|s| (s.anchor, s.head)).collect::<Vec<_>>()
    };
    vcx.simulate_keystrokes("cmd-up cmd-d cmd-d cmd-d");
    assert_eq!(ranges(&ws, vcx), [(0, 3), (4, 7), (8, 11)]);
    vcx.simulate_keystrokes("cmd-u");
    assert_eq!(ranges(&ws, vcx), [(0, 3), (4, 7)]);
    vcx.simulate_keystrokes("cmd-u cmd-u");
    assert_eq!(ranges(&ws, vcx), [(0, 0)]);
    vcx.simulate_keystrokes("cmd-u");
    assert_eq!(ranges(&ws, vcx), [(11, 11)], "back to where typing left the caret");
    assert_eq!(text(&ws, vcx), "foo foo foo", "never touches the text");
    vcx.simulate_keystrokes("cmd-u");
    assert_eq!(left_text(&ws, vcx), "No selection to undo");
    vcx.simulate_keystrokes("cmd-shift-u cmd-shift-u cmd-shift-u cmd-shift-u");
    assert_eq!(ranges(&ws, vcx), [(0, 3), (4, 7), (8, 11)]);
    vcx.simulate_keystrokes("cmd-shift-u");
    assert_eq!(left_text(&ws, vcx), "No selection to redo");
    // every occurrence is selected: nothing to skip to
    vcx.simulate_keystrokes("cmd-k cmd-d");
    assert_eq!(left_text(&ws, vcx), "No more occurrences");
    assert_eq!(ranges(&ws, vcx), [(0, 3), (4, 7), (8, 11)]);
    // a skip is a selection change, and cmd-u takes it back
    vcx.simulate_keystrokes("cmd-u cmd-k cmd-d");
    assert_eq!(ranges(&ws, vcx), [(0, 3), (8, 11)]);
    vcx.simulate_keystrokes("cmd-u");
    assert_eq!(ranges(&ws, vcx), [(0, 3), (4, 7)]);
}

#[gpui::test]
fn commands_within_one_frame_are_separate_selection_states(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "foo foo foo");
    vcx.simulate_keystrokes("cmd-up");
    // no frame is drawn between these three
    ws.update(vcx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            for _ in 0..3 {
                e.edit(cx, |s| s.select_next_occurrence());
            }
        })
    });
    vcx.simulate_keystrokes("cmd-u");
    assert_eq!(sels(&ws, vcx).len(), 2);
}

fn convert(vcx: &mut VisualTestContext, codec: Codec, decode: bool) {
    vcx.dispatch_action(editor::Convert { codec, decode });
}

#[gpui::test]
fn uri_component_requires_physical_selection_without_changing_history(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a b/é");
    for selections in [
        vec![Selection::caret(1)],
        vec![Selection::caret(0), Selection::caret(5)],
        vec![Selection::from_points((5, 1), (5, 4))],
    ] {
        on_editor(&ws, vcx, |e, _, cx| {
            e.state.virtual_space = true.into();
            e.edit(cx, |s| s.set_selections(selections.clone()));
        });
        let version = on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version);
        for decode in [false, true] {
            convert(vcx, Codec::UriComponent, decode);
            assert_eq!(text(&ws, vcx), "a b/é");
            assert_eq!(sels(&ws, vcx), selections);
            assert_eq!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version), version);
            assert!(!vcx.has_pending_prompt());
        }
    }
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "", "URI actions added no undo step");
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "a b/é");
    // Existing codecs retain whole-document fallback.
    set_selections(&ws, vcx, &[(1, 1)]);
    convert(vcx, Codec::Hex, false);
    assert_eq!(text(&ws, vcx), "6120622fc3a9");
}

#[gpui::test]
fn uri_component_catalog_action_maps_shared_views_and_respects_follow_lock(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let input = "a b/é | +😀\n行 | end";
    let encoded = "a%20b%2F%C3%A9 | %2B%F0%9F%98%80%0A%E8%A1%8C | end";
    type_in(&ws, vcx, input);
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    views[0].update(vcx, |e, cx| e.edit(cx, |s| s.set_cursor(18)));
    set_selections(&ws, vcx, &[(0, 5), (7, 7), (8, 12)]);
    on_editor(&ws, vcx, |e, _, cx| {
        e.state.virtual_space = true.into();
        let mut selections = e.state.selections.clone();
        selections.push(Selection::from_points((18, 1), (18, 4)));
        e.sel(cx).set_selections(selections);
    });
    on_editor(&ws, vcx, |e, _, cx| e.update_doc(cx, |d| d.read_only = true));
    let menu = submenu(&ws, vcx, "Edit", "Convert");
    let action = menu
        .into_iter()
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name.as_ref() == "URI Component Encode" => Some(action),
            _ => None,
        })
        .expect("URI encode in the native menu");
    vcx.update(|window, cx| window.dispatch_action(action, cx));
    vcx.run_until_parked();
    for view in &views {
        assert_eq!(view.read_with(vcx, |e, cx| e.doc(cx).text()), encoded);
    }
    assert_eq!(views[0].read_with(vcx, |e, _| e.state.cursor()), 50);
    assert_eq!(
        sels(&ws, vcx),
        [Selection::new(0, 14), Selection::caret(16), Selection::new(17, 44), Selection::from_points((50, 1), (50, 4))]
    );
    vcx.dispatch_action(editor::Undo);
    for view in &views {
        assert_eq!(view.read_with(vcx, |e, cx| e.doc(cx).text()), input);
    }
    assert_eq!(views[0].read_with(vcx, |e, _| e.state.cursor()), 18);
    assert_eq!(
        sels(&ws, vcx),
        [Selection::new(0, 5), Selection::caret(7), Selection::new(8, 12), Selection::from_points((18, 1), (18, 4))]
    );
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), encoded);
    on_editor(&ws, vcx, |e, _, cx| e.update_doc(cx, |d| d.set_following(true)));
    let before = (sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version));
    convert(vcx, Codec::UriComponent, true);
    assert_eq!(text(&ws, vcx), encoded);
    assert_eq!((sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version)), before);
    on_editor(&ws, vcx, |e, _, cx| e.update_doc(cx, |d| d.set_following(false)));
    convert(vcx, Codec::UriComponent, true);
    assert_eq!(text(&ws, vcx), input);
    assert!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).read_only));
}

#[gpui::test]
fn uri_component_checks_selection_after_a_shared_peer_deletes_its_text(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a b/é tail");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    set_selections(&ws, vcx, &[(0, 5)]);
    // Run before document-event delivery catches the active view up.
    ws.update_in(vcx, |_, window, cx| {
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_selection(0, 5);
                s.delete();
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.selections[0].range(), 0..5, "the action sees a stale selection");
            assert!(e.perform_action(&editor::Convert { codec: Codec::UriComponent, decode: false }, window, cx));
            assert_eq!(e.doc(cx).text(), " tail");
            assert_eq!(e.state.selections, [Selection::caret(0)]);
        });
    });
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "a b/é tail", "URI made no undo step after the peer's deletion");
}

#[gpui::test]
fn uri_component_invalid_decode_alerts_without_partial_edits(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    for bad in ["%", "%GG", "%FF"] {
        let input = format!("a%20b\n{bad}");
        set_state(&ws, vcx, &input);
        set_selections(&ws, vcx, &[(0, 5), (6, input.chars().count())]);
        let before = (sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version));
        convert(vcx, Codec::UriComponent, true);
        let (message, detail) = vcx.pending_prompt().expect("decode alert");
        assert_eq!(message, "Decode failed");
        assert!(detail.starts_with("Line 2:"), "{detail}");
        assert_eq!(text(&ws, vcx), input);
        assert_eq!((sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version)), before);
        vcx.simulate_prompt_answer("Ok");
    }
}

#[gpui::test]
fn html_entities_requires_physical_selection_without_changing_history(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "<&é'x&amp;");
    for selections in [
        vec![Selection::caret(1)],
        vec![Selection::caret(0), Selection::caret(5)],
        vec![Selection::from_points((10, 1), (10, 4))],
    ] {
        on_editor(&ws, vcx, |e, _, cx| {
            e.state.virtual_space = true.into();
            e.edit(cx, |s| s.set_selections(selections.clone()));
        });
        let version = on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version);
        for decode in [false, true] {
            convert(vcx, Codec::HtmlEntities, decode);
            assert_eq!(text(&ws, vcx), "<&é'x&amp;");
            assert_eq!(sels(&ws, vcx), selections);
            assert_eq!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version), version);
            assert!(!vcx.has_pending_prompt());
        }
    }
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "", "HTML entity actions added no undo step");
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "<&é'x&amp;");
    // Existing codecs retain whole-document fallback.
    set_selections(&ws, vcx, &[(1, 1)]);
    convert(vcx, Codec::Hex, false);
    assert_eq!(text(&ws, vcx), "3c26c3a9277826616d703b");
}

#[gpui::test]
fn html_entities_catalog_action_maps_shared_views_and_respects_follow_lock(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let input = "<& | é'\nend";
    let encoded = "&lt;&amp; | é&#39;\nend";
    type_in(&ws, vcx, input);
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    views[0].update(vcx, |e, cx| e.edit(cx, |s| s.set_cursor(11)));
    set_selections(&ws, vcx, &[(0, 2), (3, 3), (5, 7)]);
    on_editor(&ws, vcx, |e, _, cx| {
        e.state.virtual_space = true.into();
        let mut selections = e.state.selections.clone();
        selections.push(Selection::from_points((11, 1), (11, 4)));
        e.sel(cx).set_selections(selections);
    });
    on_editor(&ws, vcx, |e, _, cx| e.update_doc(cx, |d| d.read_only = true));
    let menu = submenu(&ws, vcx, "Edit", "Convert");
    let action = menu
        .into_iter()
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name.as_ref() == "HTML Entity Escape" => Some(action),
            _ => None,
        })
        .expect("HTML entity encode in the native menu");
    vcx.update(|window, cx| window.dispatch_action(action, cx));
    vcx.run_until_parked();
    for view in &views {
        assert_eq!(view.read_with(vcx, |e, cx| e.doc(cx).text()), encoded);
    }
    assert_eq!(views[0].read_with(vcx, |e, _| e.state.cursor()), 22);
    assert_eq!(
        sels(&ws, vcx),
        [Selection::new(0, 9), Selection::caret(10), Selection::new(12, 18), Selection::from_points((22, 1), (22, 4))]
    );
    vcx.dispatch_action(editor::Undo);
    for view in &views {
        assert_eq!(view.read_with(vcx, |e, cx| e.doc(cx).text()), input);
    }
    assert_eq!(views[0].read_with(vcx, |e, _| e.state.cursor()), 11);
    assert_eq!(
        sels(&ws, vcx),
        [Selection::new(0, 2), Selection::caret(3), Selection::new(5, 7), Selection::from_points((11, 1), (11, 4))]
    );
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), encoded);
    on_editor(&ws, vcx, |e, _, cx| e.update_doc(cx, |d| d.set_following(true)));
    let before = (sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version));
    convert(vcx, Codec::HtmlEntities, true);
    assert_eq!(text(&ws, vcx), encoded);
    assert_eq!((sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version)), before);
    on_editor(&ws, vcx, |e, _, cx| e.update_doc(cx, |d| d.set_following(false)));
    convert(vcx, Codec::HtmlEntities, true);
    assert_eq!(text(&ws, vcx), input);
    assert!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).read_only));
}

#[gpui::test]
fn html_entities_checks_selection_after_a_shared_peer_deletes_its_text(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "<&é'x &tail");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    set_selections(&ws, vcx, &[(0, 5)]);
    // Run before document-event delivery catches the active view up.
    ws.update_in(vcx, |_, window, cx| {
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_selection(0, 5);
                s.delete();
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.selections[0].range(), 0..5, "the action sees a stale selection");
            assert!(e.perform_action(&editor::Convert { codec: Codec::HtmlEntities, decode: false }, window, cx));
            assert_eq!(e.doc(cx).text(), " &tail");
            assert_eq!(e.state.selections, [Selection::caret(0)]);
        });
    });
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "<&é'x &tail", "HTML entity made no undo step after the peer's deletion");
}

#[gpui::test]
fn html_entities_preserves_immutable_viewer_text_and_history(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "<&amp;&#xD800;");
    set_selections(&ws, vcx, &[(0, 13)]);
    on_editor(&ws, vcx, |e, _, _| e.input_enabled = false);
    let before = (sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version));
    for decode in [false, true] {
        convert(vcx, Codec::HtmlEntities, decode);
        assert_eq!(text(&ws, vcx), "<&amp;&#xD800;");
        assert_eq!((sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version)), before);
        assert!(!vcx.has_pending_prompt());
    }
    on_editor(&ws, vcx, |e, _, _| e.input_enabled = true);
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "", "viewer conversions added no history");
}

#[gpui::test]
fn html_entities_invalid_decode_alerts_without_partial_edits(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    for bad in ["&#", "&#65", "&#xD800;", "&#x110000;", "&#999999999999999999999;"] {
        let input = format!("&lt;\n{bad}");
        set_state(&ws, vcx, &input);
        set_selections(&ws, vcx, &[(0, 4), (5, input.chars().count())]);
        let before = (sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version));
        convert(vcx, Codec::HtmlEntities, true);
        let (message, detail) = vcx.pending_prompt().expect("decode alert");
        assert_eq!(message, "Decode failed");
        assert!(detail.starts_with("Line 2:"), "{detail}");
        assert_eq!(text(&ws, vcx), input);
        assert_eq!((sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version)), before);
        vcx.simulate_prompt_answer("Ok");
    }
}

/// Dispatch a named literal operation from the real shared native Convert menu.
fn n03_convert(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, label: &str) {
    let action = submenu(ws, vcx, "Edit", "Convert")
        .into_iter()
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name.as_ref() == label => Some(action),
            _ => None,
        })
        .expect(label);
    vcx.update(|window, cx| window.dispatch_action(action, cx));
}

#[gpui::test]
fn n03_literal_actions_ignore_carets_and_pure_virtual_targets(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a.b\\n\"😀\"");
    for selections in [
        vec![Selection::caret(1)],
        vec![Selection::caret(0), Selection::caret(8)],
        vec![Selection::from_points((8, 1), (8, 4)).in_rectangle()],
    ] {
        on_editor(&ws, vcx, |e, _, cx| {
            e.state.virtual_space = true.into();
            e.sel(cx).set_selections(selections.clone());
            e.sel(cx).sel_checkpoint();
        });
        let before = on_editor(&ws, vcx, |e, _, cx| {
            (e.read_state(cx).selection_generation(), e.doc(cx).version, e.doc(cx).save_point(), e.marked.clone())
        });
        for label in ["Escape Regex", "Quote JSON String", "Unquote JSON String", "Decode Text Escapes"] {
            n03_convert(&ws, vcx, label);
            assert_eq!(text(&ws, vcx), "a.b\\n\"😀\"");
            assert_eq!(sels(&ws, vcx), selections);
            assert_eq!(
                on_editor(&ws, vcx, |e, _, cx| (
                    e.read_state(cx).selection_generation(),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.marked.clone(),
                )),
                before
            );
            assert!(!vcx.has_pending_prompt());
        }
    }
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "", "literal operations added no history");
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "a.b\\n\"😀\"");
}

#[gpui::test]
fn n03_json_quotes_mixed_physical_targets_once_preserving_shared_origins_and_undo(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let input = "a\"😀 | \n雪 | tail";
    let quoted = "\"a\\\"😀\" | \"\\n雪\" | tail";
    type_in(&ws, vcx, input);
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    views[0].update(vcx, |e, cx| e.sel(cx).set_cursor(15));
    on_editor(&ws, vcx, |e, _, cx| {
        e.update_doc(cx, |d| {
            d.read_only = true;
            d.line_ending = "\r\n";
            d.bom = true;
        });
        e.state.virtual_space = true.into();
        e.sel(cx).set_selections(vec![
            Selection::new(3, 0).in_rectangle(),
            Selection::caret(4),
            Selection::new(6, 8),
            Selection::from_points((15, 1), (15, 4)).in_rectangle(),
        ]);
    });
    let before = sels(&ws, vcx);
    let primary = on_editor(&ws, vcx, |e, _, _| e.state.primary());
    n03_convert(&ws, vcx, "Quote JSON String");
    for view in &views {
        assert_eq!(view.read_with(vcx, |e, cx| e.doc(cx).text()), quoted);
    }
    let after = sels(&ws, vcx);
    assert_eq!(after[0], Selection::new(6, 0).in_rectangle());
    assert_eq!(after[1], Selection::caret(7), "empty target receives no quotes");
    assert_eq!(after[2], Selection::new(9, 14));
    assert_eq!(after[3], Selection::from_points((21, 1), (21, 4)).in_rectangle());
    assert_eq!(on_editor(&ws, vcx, |e, _, _| e.state.primary()), primary);
    assert_eq!(views[0].read_with(vcx, |e, _| e.state.cursor()), 21);
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), input);
    assert_eq!(sels(&ws, vcx), before);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), quoted);
    assert_eq!(sels(&ws, vcx), after);
    n03_convert(&ws, vcx, "Unquote JSON String");
    assert_eq!(text(&ws, vcx), input);
    assert_eq!(sels(&ws, vcx), before);
    assert_eq!(
        on_editor(&ws, vcx, |e, _, cx| (e.doc(cx).read_only, e.doc(cx).line_ending, e.doc(cx).bom)),
        (true, "\r\n", true)
    );
}

#[gpui::test]
fn n03_invalid_later_target_preserves_selection_generation_marked_text_save_identity_and_redo(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    for (label, first, bad) in [
        ("Unquote JSON String", "\"ok\"", "null"),
        ("Unquote JSON String", "\"ok\"", "\"\\uD800\""),
        ("Decode Text Escapes", r"\xE9", r"\uD800"),
        ("Decode Text Escapes", r"\uD83D\uDE00", r"\U00110000"),
        ("Decode Text Escapes", r"\n", "\\"),
    ] {
        let input = format!("{first}\n{bad}");
        set_state(&ws, vcx, &input);
        on_editor(&ws, vcx, |e, _, cx| {
            e.edit(cx, |s| {
                s.set_cursor(s.len());
                s.insert("!");
                s.undo();
            })
        });
        set_selections(&ws, vcx, &[(first.chars().count(), 0), (first.chars().count() + 1, input.chars().count())]);
        on_editor(&ws, vcx, |e, _, cx| {
            e.sel(cx).sel_checkpoint();
            e.marked = std::iter::once(0..first.chars().count()).collect();
        });
        let before = on_editor(&ws, vcx, |e, _, cx| {
            (
                e.read_state(cx).selection_generation(),
                e.doc(cx).version,
                e.doc(cx).dirty,
                e.doc(cx).save_point(),
                e.marked.clone(),
            )
        });
        let selections = sels(&ws, vcx);
        let selection_state = on_editor(&ws, vcx, |e, _, cx| e.read_state(cx).selection_state());
        n03_convert(&ws, vcx, label);
        let (message, detail) = vcx.pending_prompt().expect("invalid literal alert");
        assert_eq!(message, "Decode failed");
        assert!(detail.starts_with("Line 2:"), "{detail}");
        assert_eq!(text(&ws, vcx), input);
        assert_eq!(sels(&ws, vcx), selections);
        assert!(on_editor(&ws, vcx, |e, _, cx| e.read_state(cx).selection_state() == selection_state));
        assert_eq!(
            on_editor(&ws, vcx, |e, _, cx| (
                e.read_state(cx).selection_generation(),
                e.doc(cx).version,
                e.doc(cx).dirty,
                e.doc(cx).save_point(),
                e.marked.clone(),
            )),
            before
        );
        vcx.simulate_prompt_answer("Ok");
        on_editor(&ws, vcx, |e, _, _| e.marked.clear());
        vcx.dispatch_action(editor::Redo);
        assert_eq!(text(&ws, vcx), input + "!", "preflight failure retained redo");
    }
}

#[gpui::test]
fn n03_literal_actions_follow_viewer_and_noop_preserve_source_and_history(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, r#"a.b "\q" \uD800"#);
    set_selections(&ws, vcx, &[(0, 16)]);
    for (follow, viewer) in [(true, false), (false, true)] {
        on_editor(&ws, vcx, |e, _, cx| {
            e.update_doc(cx, |d| d.set_following(follow));
            e.input_enabled = !viewer;
            e.sel(cx).sel_checkpoint();
        });
        let before = on_editor(&ws, vcx, |e, _, cx| {
            (e.read_state(cx).selection_generation(), e.doc(cx).version, e.doc(cx).save_point())
        });
        let selections = sels(&ws, vcx);
        for label in ["Escape Regex", "Quote JSON String", "Unquote JSON String", "Decode Text Escapes"] {
            n03_convert(&ws, vcx, label);
            assert_eq!(text(&ws, vcx), r#"a.b "\q" \uD800"#);
            assert_eq!(sels(&ws, vcx), selections);
            assert_eq!(
                on_editor(&ws, vcx, |e, _, cx| (
                    e.read_state(cx).selection_generation(),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                )),
                before
            );
            assert!(!vcx.has_pending_prompt());
        }
    }
    on_editor(&ws, vcx, |e, _, _| e.input_enabled = true);
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "");
    type_in(&ws, vcx, "plain é😀");
    set_selections(&ws, vcx, &[(8, 0)]);
    let before = on_editor(&ws, vcx, |e, _, cx| (e.read_state(cx).selection_generation(), e.doc(cx).version));
    n03_convert(&ws, vcx, "Escape Regex");
    n03_convert(&ws, vcx, "Decode Text Escapes");
    assert_eq!(text(&ws, vcx), "plain é😀");
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| (e.read_state(cx).selection_generation(), e.doc(cx).version)), before);
}

#[gpui::test]
fn n03_literal_actions_catch_up_shared_selection_before_preflight(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a.b tail");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    set_selections(&ws, vcx, &[(3, 0)]);
    ws.update_in(vcx, |_, window, cx| {
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_selection(0, 3);
                s.delete();
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.selections[0].range(), 0..3);
            assert!(e.perform_action(&editor::Convert { codec: Codec::JsonString, decode: false }, window, cx));
            assert_eq!(e.doc(cx).text(), " tail");
            assert_eq!(e.state.selections, [Selection::caret(0)]);
        });
    });
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "a.b tail", "no edit after peer deleted the selected target");
}

#[gpui::test]
fn n03_native_menu_text_escape_success_commits_ime_and_preserves_virtual_caret(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    let input = r"\uD83D\uDE00 | \xE9\n";
    type_in(&ws, vcx, input);
    on_editor(&ws, vcx, |e, w, cx| {
        let len = e.doc(cx).len();
        e.sel(cx).set_cursor(len);
        e.replace_and_mark_text_in_range(None, "中", Some(1..1), w, cx);
        e.state.virtual_space = true.into();
        e.sel(cx).set_selections(vec![
            Selection::new(12, 0),
            Selection::new(15, 21),
            Selection::from_points((22, 1), (22, 4)),
        ]);
    });
    n03_convert(&ws, vcx, "Decode Text Escapes");
    assert_eq!(text(&ws, vcx), "😀 | é\n中");
    assert!(on_editor(&ws, vcx, |e, _, _| e.marked.is_empty()));
    assert_eq!(sels(&ws, vcx), [Selection::new(1, 0), Selection::new(4, 6), Selection::from_points((7, 1), (7, 4))]);
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), input.to_string() + "中");
}

#[gpui::test]
fn n03_literal_custom_bindings_record_and_reconstruct_macro_actions(cx: &mut TestAppContext) {
    use crate::keybindings::CommandAction;
    let (ws, vcx) = window(cx);
    let keymap = serde_json::json!([
        {"keys":["cmd-alt-j"],"command":"editor::Convert","args":{"codec":"json_string","decode":false},"context":"Editor"},
        {"keys":["cmd-alt-h"],"command":"editor::DecodeTextEscapes","context":"Editor"},
    ]);
    std::fs::write(crate::config::customization::keymap_path(), keymap.to_string()).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "é\"😀\n");
    set_selections(&ws, vcx, &[(0, 4)]);
    vcx.update(|_, cx| cx.default_global::<crate::keybindings::Recorder>().recording = true);
    vcx.simulate_keystrokes("cmd-alt-j");
    assert_eq!(text(&ws, vcx), "\"é\\\"😀\\n\"");
    set_state(&ws, vcx, r"\uD83D\uDE00");
    set_selections(&ws, vcx, &[(0, 12)]);
    vcx.simulate_keystrokes("cmd-alt-h");
    assert_eq!(text(&ws, vcx), "😀");
    let commands = vcx.update(|_, cx| {
        let recorder = cx.global_mut::<crate::keybindings::Recorder>();
        recorder.recording = false;
        recorder.commands.clone()
    });
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].command, "editor::Convert");
    assert_eq!(commands[0].args, serde_json::json!({"codec":"json_string","decode":false}));
    assert_eq!(commands[1].command, "editor::DecodeTextEscapes");
    for (command, input, want) in [(&commands[0], "a\"😀", "\"a\\\"😀\""), (&commands[1], r"\xFF", "ÿ")] {
        set_state(&ws, vcx, input);
        set_selections(&ws, vcx, &[(0, input.chars().count())]);
        on_editor(&ws, vcx, |e, w, cx| assert!(e.perform_action(command.build(cx).unwrap().as_ref(), w, cx)));
        assert_eq!(text(&ws, vcx), want);
    }
}

#[gpui::test]
fn n03_escape_regex_menu_changes_selected_literals_and_undo_redo_once(cx: &mut TestAppContext) {
    use crate::editing::text::FindOpts;
    use orlo_editor::search::pattern::Pattern;
    let (ws, vcx) = window(cx);
    let input = "a.b+ | (?=😀)\\1";
    type_in(&ws, vcx, input);
    set_selections(&ws, vcx, &[(4, 0), (7, 14)]);
    let before = sels(&ws, vcx);
    n03_convert(&ws, vcx, "Escape Regex");
    assert_eq!(text(&ws, vcx), r"a\.b\+ | \(\?=😀\)\\1");
    let outputs = on_editor(&ws, vcx, |e, _, cx| {
        e.state.selections.iter().map(|s| e.doc(cx).rope.slice(s.range()).to_string()).collect::<Vec<_>>()
    });
    let opts = FindOpts { regex: true, case_sensitive: true, ..FindOpts::default() };
    for (query, original) in outputs.iter().zip(["a.b+", "(?=😀)\\1"]) {
        let p = Pattern::new(&format!(r"\A(?:{query})\z"), &opts).unwrap();
        let matches = p.find_all(original, &|| false).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], 0..original.len());
    }
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), input);
    assert_eq!(sels(&ws, vcx), before);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), r"a\.b\+ | \(\?=😀\)\\1");
    let before = on_editor(&ws, vcx, |e, _, cx| (e.read_state(cx).selection_generation(), e.doc(cx).version));
    convert(vcx, Codec::RegexLiteral, true);
    assert!(vcx.pending_prompt().is_some(), "unsupported direction is an error");
    assert_eq!(text(&ws, vcx), r"a\.b\+ | \(\?=😀\)\\1");
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| (e.read_state(cx).selection_generation(), e.doc(cx).version)), before);
}

#[gpui::test]
fn n03_literal_range_edit_keeps_snippet_mirrors_and_field_navigation(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_snippet("literal", "${1:a.b} / $1 / ${2:end}$0", "", "", "Literal");
    vcx.dispatch_action(InsertSnippet);
    choose_pipeline(vcx, "Literal");
    assert_eq!(text(&ws, vcx), "a.b / a.b / end");
    assert_eq!(snippet_fields(&ws, vcx), (vec!["a.b".into(), "a.b".into()], true));
    n03_convert(&ws, vcx, "Escape Regex");
    assert_eq!(text(&ws, vcx), r"a\.b / a\.b / end");
    assert_eq!(snippet_fields(&ws, vcx), (vec![r"a\.b".into(), r"a\.b".into()], true));
    vcx.simulate_keystrokes("tab");
    assert_eq!(snippet_fields(&ws, vcx), (vec!["end".into()], true));
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "a.b / a.b / end");
}

#[gpui::test]
fn n03_literal_pipeline_actions_reach_selection_ui_and_fail_atomically(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_pipeline("literal", r#"{ "op": "decode_text_escapes" }, { "op": "encode", "codec": "json_string" }"#);
    let input = r"\xE9 | \uD83D\uDE00";
    type_in(&ws, vcx, input);
    set_selections(&ws, vcx, &[(0, 4), (7, 19)]);
    vcx.dispatch_action(RunPipeline);
    choose_pipeline(vcx, "literal");
    assert_eq!(text(&ws, vcx), "\"é\" | \"😀\"");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), input);
    write_pipeline("literal", r#"{ "op": "decode", "codec": "json_string" }"#);
    set_state(&ws, vcx, "\"ok\" | null");
    set_selections(&ws, vcx, &[(0, 4), (7, 11)]);
    vcx.dispatch_action(RunPipeline);
    choose_pipeline(vcx, "literal");
    assert_eq!(text(&ws, vcx), "\"ok\" | null");
    assert!(vcx.pending_prompt().is_some());
}

#[gpui::test]
fn base64_encode_converts_the_selection_and_undo_redo_follow(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "foo bar");
    set_selections(&ws, vcx, &[(0, 3)]);
    convert(vcx, Codec::Base64, false);
    assert_eq!(text(&ws, vcx), "Zm9v bar");
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 4)]);
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "foo bar");
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 3)]);
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, vcx), "Zm9v bar");
    // hex, and the URL-safe form without padding, on the whole document
    set_selections(&ws, vcx, &[(0, 0)]);
    convert(vcx, Codec::Hex, false);
    assert_eq!(text(&ws, vcx), "5a6d397620626172");
    vcx.simulate_keystrokes("cmd-z");
    convert(vcx, Codec::Base64Url, false);
    assert_eq!(text(&ws, vcx), "Wm05diBiYXI");
    convert(vcx, Codec::Base64Url, true);
    assert_eq!(text(&ws, vcx), "Zm9v bar");
}

#[gpui::test]
fn a_failed_decode_alerts_with_the_line_and_changes_nothing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "Zm9v\nZm9v!");
    set_selections(&ws, vcx, &[(0, 4), (5, 10)]);
    convert(vcx, Codec::Base64, true);
    let (message, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "Decode failed");
    assert!(detail.starts_with("Line 2: Invalid Base64"), "{detail}");
    assert_eq!(text(&ws, vcx), "Zm9v\nZm9v!");
    vcx.simulate_prompt_answer("Ok");
    // bytes that are not text
    set_selections(&ws, vcx, &[(0, 0)]);
    ws.update(vcx, |ws, cx| ws.with_active(cx, |e, cx| e.set_text("/w==", cx)));
    convert(vcx, Codec::Base64, true);
    let (_, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(detail, "Line 1: The decoded data is not UTF-8 text");
    assert_eq!(text(&ws, vcx), "/w==");
}

#[gpui::test]
fn copy_sha256_copies_the_digest_reports_it_and_keeps_it_out_of_the_history(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "abc");
    vcx.dispatch_action(editor::CopyHash { hash: Hash::Sha256 });
    let want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), want);
    assert_eq!(left_text(&ws, vcx), "SHA-256 copied");
    assert!(history(vcx).is_empty(), "a digest is not a Copy");
    assert_eq!(text(&ws, vcx), "abc");
    // one digest a line for several selections
    type_in(&ws, vcx, " abc");
    set_selections(&ws, vcx, &[(0, 3), (4, 7)]);
    vcx.dispatch_action(editor::CopyHash { hash: Hash::Md5 });
    let md5 = "900150983cd24fb0d6963f7d28e17f72";
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), format!("{md5}\n{md5}"));
    assert_eq!(left_text(&ws, vcx), "MD5 of 2 selections copied");
}

#[gpui::test]
fn hash_file_shows_every_digest_and_copy_puts_them_on_the_clipboard(cx: &mut TestAppContext) {
    let dir = temp("orlo-hash-file");
    let (a, b, gone) = (dir.join("a.bin"), dir.join("b.txt"), dir.join("gone"));
    std::fs::write(&a, [0u8, 255, 1, 254]).unwrap();
    std::fs::write(&b, "abc").unwrap();
    let (ws, vcx) = window(cx);
    let paths = vec![a.clone(), gone.clone(), b.clone()];
    ws.update_in(vcx, |ws, window, cx| ws.hash_paths(paths, window, cx));
    vcx.run_until_parked();
    let (message, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "Hash Files");
    let sha = |bytes: &[u8]| convert::digest(Hash::Sha256, bytes);
    assert!(detail.contains(&format!("SHA-256  {}", sha(b"abc"))), "{detail}");
    assert!(detail.contains(&format!("MD5  {}", convert::digest(Hash::Md5, &[0, 255, 1, 254]))));
    assert!(detail.contains(&format!("{}\nCould not read:", gone.display())), "{detail}");
    vcx.simulate_prompt_answer("Copy");
    vcx.run_until_parked();
    let copied = vcx.read_from_clipboard().unwrap().text().unwrap();
    let lines: Vec<_> = copied.lines().collect();
    assert_eq!(lines.len(), 8, "four algorithms for each of the two files read");
    assert_eq!(lines[0], format!("MD5  {}  {}", convert::digest(Hash::Md5, &[0, 255, 1, 254]), a.display()));
    assert_eq!(lines[6], format!("SHA-256  {}  {}", sha(b"abc"), b.display()));
    assert!(history(vcx).is_empty());
}

#[gpui::test]
fn edit_menu_has_a_convert_submenu(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    assert_eq!(
        names(&submenu(&ws, vcx, "Edit", "Convert")),
        [
            "Base64 Encode",
            "Base64 Decode",
            "Base64 URL-safe Encode",
            "Base64 URL-safe Decode",
            "Hex Encode",
            "Hex Decode",
            "URI Component Encode",
            "URI Component Decode",
            "HTML Entity Escape",
            "HTML Entity Unescape",
            "Escape Regex",
            "Quote JSON String",
            "Unquote JSON String",
            "Decode Text Escapes",
            "-",
            "Copy MD5",
            "Copy SHA-1",
            "Copy SHA-256",
            "Copy SHA-512",
            "-",
            "Hash File…",
        ]
    );
}

fn set_state(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, text: &str) {
    ws.update(vcx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.set_text(text, cx);
            e.sel(cx).set_cursor(0);
        })
    });
}

#[gpui::test]
fn document_statistics_shows_the_selection_column_only_for_a_selection(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "ab cd\nef gh");
    ws.update(vcx, |ws, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| s.convert_line_endings("\r\n"));
            e.update_doc(cx, |d| d.bom = true);
        })
    });
    vcx.dispatch_action(DocumentStatistics);
    let (message, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "Document Statistics");
    assert_eq!(
        detail,
        "Characters: 11\nGrapheme clusters: 11\nWords: 4\nLines: 2\n\
         UTF-8 bytes as saved: 15"
    );
    vcx.simulate_prompt_answer("Ok");
    set_selections(&ws, vcx, &[(0, 2), (6, 11), (4, 4)]);
    vcx.dispatch_action(DocumentStatistics);
    let (_, detail) = vcx.pending_prompt().expect("an alert");
    assert!(detail.starts_with("Characters: 11 (selection 7 in 2 selections)\n"), "{detail}");
    assert!(detail.ends_with("UTF-8 bytes as saved: 15"), "{detail}");
}

#[gpui::test]
fn document_statistics_of_a_huge_buffer_are_counted_off_the_ui_thread(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let text = "ab cd\n".repeat(stats::BIG / 6 + 1);
    let chars = text.chars().count();
    assert!(chars > stats::BIG);
    set_state(&ws, vcx, &text);
    ws.update_in(vcx, |ws, window, cx| ws.document_statistics(&DocumentStatistics, window, cx));
    // nothing was counted on this thread: the alert waits for the background task
    assert!(vcx.pending_prompt().is_none());
    vcx.run_until_parked();
    let (_, detail) = vcx.pending_prompt().expect("an alert once counted");
    let utf8 = crate::editing::encoding::Encoding::Utf8;
    let want = stats::stats(&ropey::Rope::from_str(&text), &[], "\n", false, utf8);
    assert_eq!(detail, want.detail());
    assert!(detail.starts_with("Characters: 4,000,002\n"), "{detail}");
}

#[gpui::test]
fn character_info_describes_the_cluster_after_the_primary_caret(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    set_state(&ws, vcx, "e\u{301}👩\u{200d}👧\n");
    let info = |vcx: &mut VisualTestContext, at: usize| {
        set_selections(&ws, vcx, &[(at, at)]);
        vcx.dispatch_action(CharacterInfo);
        let (message, detail) = vcx.pending_prompt().expect("an alert");
        assert_eq!(message, "Character Info");
        vcx.simulate_prompt_answer("Ok");
        detail
    };
    let accent = info(vcx, 0);
    assert!(accent.contains("U+0065  UTF-8 65  UTF-16 0065  letter"), "{accent}");
    assert!(accent.contains("U+0301  UTF-8 CC 81  UTF-16 0301  combining mark"));
    assert!(accent.ends_with("Cluster: 2 scalars, 3 UTF-8 bytes, 2 UTF-16 units"));
    let family = info(vcx, 2);
    assert!(family.contains("U+200D  UTF-8 E2 80 8D  UTF-16 200D  joiner"), "{family}");
    assert!(family.contains("U+1F467  UTF-8 F0 9F 91 A7  UTF-16 D83D DC67"));
    assert!(family.ends_with("Cluster: 3 scalars, 11 UTF-8 bytes, 5 UTF-16 units"));
    // the line break says how a save writes it
    ws.update(vcx, |ws, cx| ws.with_active(cx, |e, cx| e.update_doc(cx, |d| d.line_ending = "\r\n")));
    let end_of_line = info(vcx, 5);
    assert!(
        end_of_line.contains("U+000A  UTF-8 0A  UTF-16 000A  line break (saved as Windows (CRLF))"),
        "{end_of_line}"
    );
    assert_eq!(info(vcx, 6), "End of document");
}

#[gpui::test]
fn edit_menu_ends_with_the_statistics_commands(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let names: Vec<_> = (menu(&ws, vcx, "Edit").into_iter())
        .filter_map(|i| match i {
            MenuItem::Action { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(names[names.len() - 2..], ["Document Statistics", "Character Info"]);
}

/// Every save-time cleanup on, trimming as `trim` says.
fn clean_on_save(vcx: &mut VisualTestContext, trim: crate::config::settings::Trim) {
    vcx.update(|_, cx| {
        cx.set_global(Settings {
            trim_trailing_white_space_on_save: trim,
            remove_trailing_blank_lines_on_save: true,
            ensure_newline_at_eof_on_save: true,
            ..Settings::default()
        })
    });
}

#[gpui::test]
fn a_cleaned_save_keeps_the_files_crlf_and_byte_order_mark(cx: &mut TestAppContext) {
    let (ws, vcx, p) = window_on(cx, "orlo-save-cleanup-crlf", "\u{feff}a  \r\nb \t\r\n\r\n\r\n");
    clean_on_save(vcx, crate::config::settings::Trim::All);
    ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
    assert_eq!(std::fs::read(&p).unwrap(), "\u{feff}a\r\nb\r\n".as_bytes());
    assert_eq!(text(&ws, vcx), "a\nb\n");
    assert!(!on_editor(&ws, vcx, |e, _, cx| e.doc(cx).dirty));
}

#[gpui::test]
fn a_save_that_cleans_nothing_leaves_the_composition_alone(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx, p) = window_on(cx, "orlo-save-cleanup-ime", "x\n");
    // default settings, then every cleanup on over text that needs none
    for on in [false, true] {
        if on {
            clean_on_save(vcx, crate::config::settings::Trim::All);
        }
        on_editor(&ws, vcx, |e, w, cx| e.replace_and_mark_text_in_range(None, "k", None, w, cx));
        ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
        assert!(on_editor(&ws, vcx, |e, w, cx| e.marked_text_range(w, cx)).is_some(), "cleanup on: {on}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "kx\n");
        on_editor(&ws, vcx, |e, w, cx| e.unmark_text(w, cx));
        on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.undo()));
        ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "x\n");
    }
}

#[gpui::test]
fn a_cancelled_save_as_changes_nothing_and_a_chosen_path_cleans(cx: &mut TestAppContext) {
    let dir = temp("orlo-save-cleanup-cancel").canonicalize().unwrap();
    let (ws, vcx) = window(cx);
    clean_on_save(vcx, crate::config::settings::Trim::All);
    type_in(&ws, vcx, "u  \n\n");
    ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
    vcx.simulate_new_path_selection(|_| None);
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "u  \n\n", "the panel was cancelled");
    ws.update_in(vcx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
    let p = dir.join("u.txt");
    let chosen = p.clone();
    vcx.simulate_new_path_selection(move |_| Some(chosen.clone()));
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "u\n");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "u\n");
    // the file is saved: Save As cancelled again leaves it alone
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.insert("v  ")));
    ws.update_in(vcx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
    vcx.simulate_new_path_selection(|_| None);
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "u\nv  ", "the caret went with the cleanup");
}

#[gpui::test]
fn a_failed_write_keeps_the_cleanup_edit_dirty_and_undoable(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let (ws, vcx, p) = window_on(cx, "orlo-save-cleanup-fails", "old  \n");
    let dir = p.parent().unwrap();
    clean_on_save(vcx, crate::config::settings::Trim::All);
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
    let prompt = vcx.pending_prompt();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(prompt.map(|p| p.0), Some("Could not save file".into()));
    vcx.simulate_prompt_answer("Ok");
    assert_eq!(text(&ws, vcx), "old\n");
    assert!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).dirty));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "old  \n");
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.undo()));
    assert_eq!(text(&ws, vcx), "old  \n");
}

#[gpui::test]
fn the_other_view_follows_a_cleaned_save_and_one_undo_takes_it_back(cx: &mut TestAppContext) {
    let (ws, vcx, p) = window_on(cx, "orlo-save-cleanup-views", "a  \nb  ");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    assert_eq!(views.len(), 2);
    // the other view's caret is at the end, where the cleanup trims and adds a newline
    views[0].update(vcx, |e, cx| e.edit(cx, |s| s.set_cursor(7)));
    clean_on_save(vcx, crate::config::settings::Trim::All);
    ws.update_in(vcx, |ws, window, cx| ws.save(&Save, window, cx));
    let states = |vcx: &mut VisualTestContext| -> Vec<(String, usize, bool)> {
        let state = |e: &Editor, cx: &App| (e.doc(cx).text(), e.state.cursor(), e.doc(cx).dirty);
        views.iter().map(|v| v.read_with(vcx, state)).collect()
    };
    assert_eq!(states(vcx), vec![("a\nb\n".into(), 3, false), ("a\nb\n".into(), 0, false)]);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a\nb\n");
    for _ in 0..2 {
        views[1].update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(
            states(vcx).into_iter().map(|s| s.0).collect::<Vec<_>>(),
            ["a  \nb  ", "a  \nb  "],
            "one step: the second undo has nothing left"
        );
    }
    assert!(states(vcx).iter().all(|s| s.2), "undone past the save");
}

// ---- Edit > Columns ----

use crate::editing::csv::{ColumnOp, Delim};

fn columns_cmd(vcx: &mut VisualTestContext, op: ColumnOp) {
    vcx.dispatch_action(editor::ColumnCommand { op });
}

fn open_column_editor(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    ws.update_in(vcx, |ws, window, cx| ws.column_editor(&ColumnEditor, window, cx));
    vcx.run_until_parked();
}

fn click_field(vcx: &mut VisualTestContext, key: &'static str) {
    let b = vcx.debug_bounds(key).expect(key);
    vcx.simulate_click(b.center(), gpui::Modifiers::default());
}

fn caret_list(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, at: &[usize]) {
    set_selections(ws, vcx, &at.iter().map(|&o| (o, o)).collect::<Vec<_>>());
}

#[gpui::test]
fn the_column_editor_inserts_its_text_at_every_caret_in_one_undo_step(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "ab\ncd\nef");
    caret_list(&ws, vcx, &[1, 4, 7]);
    open_column_editor(&ws, vcx);
    vcx.simulate_input("X");
    vcx.simulate_keystrokes("enter");
    assert_eq!(form_error(&ws, vcx), None, "the panel closed");
    assert_eq!(text(&ws, vcx), "aXb\ncXd\neXf");
    assert_eq!(sels(&ws, vcx), [2, 6, 10].map(Selection::caret), "a caret after each");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "ab\ncd\nef");
    assert_eq!(sels(&ws, vcx), [1, 4, 7].map(Selection::caret));
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, vcx), "aXb\ncXd\neXf");
}

#[gpui::test]
fn the_column_editor_numbers_the_carets_and_pads_short_lines(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "abcdef\nab\na");
    caret_list(&ws, vcx, &[3, 9, 11]);
    open_column_editor(&ws, vcx);
    click_field(vcx, "mode");
    // text, initial, increment: the first field is focused
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("10");
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("5");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "abc10def\nab 15\na  20");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "abcdef\nab\na");
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, vcx), "abc10def\nab 15\na  20");
}

#[gpui::test]
fn the_column_editor_pads_hex_numbers_and_can_leave_short_lines_alone(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "abcdef\nab");
    caret_list(&ws, vcx, &[3, 9]);
    open_column_editor(&ws, vcx);
    click_field(vcx, "mode");
    click_field(vcx, "base");
    click_field(vcx, "uppercase");
    click_field(vcx, "pad_short");
    // tab: initial, increment, repeat, width
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("254");
    vcx.simulate_keystrokes("tab tab tab");
    vcx.simulate_input("4");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "abc00FEdef\nab00FF");
}

#[gpui::test]
fn a_bad_number_or_an_overflow_keeps_the_form_open_and_the_text_as_it_was(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a\nb");
    caret_list(&ws, vcx, &[1, 3]);
    let version = on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version);
    open_column_editor(&ws, vcx);
    click_field(vcx, "mode");
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("x");
    vcx.simulate_keystrokes("enter");
    assert_eq!(form_error(&ws, vcx), Some(Some("Initial must be an integer".to_string())));
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("9223372036854775807");
    vcx.simulate_keystrokes("enter");
    let error = form_error(&ws, vcx).flatten().expect("open, with a message");
    assert!(error.contains("64 bits"), "{error}");
    assert_eq!(text(&ws, vcx), "a\nb");
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version), version);
    // fix it and the same form goes through
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("1");
    vcx.simulate_keystrokes("enter");
    assert_eq!(form_error(&ws, vcx), None);
    assert_eq!(text(&ws, vcx), "a1\nb2");
}

#[gpui::test]
fn pad_to_column_reaches_the_rightmost_caret_or_a_typed_column(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "abcd\nab\nabcdef");
    caret_list(&ws, vcx, &[4, 7, 10]);
    ws.update_in(vcx, |ws, window, cx| ws.pad_to_column(&PadToColumn, window, cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "abcd\nab  \nab  cdef");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "abcd\nab\nabcdef");
    // the number is the 1-based column the status bar would show
    ws.update_in(vcx, |ws, window, cx| ws.pad_to_column(&PadToColumn, window, cx));
    vcx.run_until_parked();
    vcx.simulate_input("7");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "abcd  \nab    \nab    cdef");
    vcx.simulate_keystrokes("cmd-z cmd-shift-z");
    assert_eq!(text(&ws, vcx), "abcd  \nab    \nab    cdef");
}

#[gpui::test]
fn pad_to_column_rejects_a_bad_column_and_changes_nothing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "ab\nc");
    caret_list(&ws, vcx, &[2, 4]);
    ws.update_in(vcx, |ws, window, cx| ws.pad_to_column(&PadToColumn, window, cx));
    vcx.run_until_parked();
    for bad in ["abc", "0", "-3", "99999999999"] {
        vcx.simulate_input(bad);
        vcx.simulate_keystrokes("enter");
        let error = form_error(&ws, vcx).flatten();
        assert_eq!(error.as_deref(), Some("Column must be a whole number from 1 to 1000"), "{bad}");
        assert_eq!(text(&ws, vcx), "ab\nc");
    }
}

const TABLE: &str = "a,\"x,\"\"y\"\"\",c\n1,,3\nshort\n\"multi\nline\",2,3\n";

#[gpui::test]
fn each_column_command_edits_in_one_undo_step_and_undo_and_redo_follow(cx: &mut TestAppContext) {
    for (op, want) in [
        (ColumnOp::MoveRight, "a,c,\"x,\"\"y\"\"\"\n1,3,\nshort\n\"multi\nline\",3,2\n"),
        (ColumnOp::MoveLeft, "\"x,\"\"y\"\"\",a,c\n,1,3\nshort\n2,\"multi\nline\",3\n"),
        (ColumnOp::Delete, "a,c\n1,3\nshort\n\"multi\nline\",3\n"),
        (ColumnOp::Clear, "a,,c\n1,,3\nshort\n\"multi\nline\",,3\n"),
    ] {
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, TABLE);
        caret_list(&ws, vcx, &[3]);
        columns_cmd(vcx, op);
        assert_eq!(text(&ws, vcx), want, "{op:?}");
        vcx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&ws, vcx), TABLE, "undo {op:?}");
        assert_eq!(sels(&ws, vcx), [Selection::caret(3)]);
        vcx.simulate_keystrokes("cmd-shift-z");
        assert_eq!(text(&ws, vcx), want, "redo {op:?}");
    }
}

#[gpui::test]
fn select_column_selects_the_raw_fields_and_empty_ones_become_carets(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, TABLE);
    caret_list(&ws, vcx, &[3]);
    columns_cmd(vcx, ColumnOp::Select);
    let got: Vec<String> = on_editor(&ws, vcx, |e, _, cx| {
        e.state.selections.iter().map(|s| e.doc(cx).rope.slice(s.range()).to_string()).collect()
    });
    assert_eq!(got, ["\"x,\"\"y\"\"\"", "", "2"]);
    assert_eq!(text(&ws, vcx), TABLE);
}

#[gpui::test]
fn copy_column_copies_the_values_and_records_them_in_the_clipboard_history(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, TABLE);
    caret_list(&ws, vcx, &[3]);
    columns_cmd(vcx, ColumnOp::Copy);
    let want = "x,\"y\"\n\n\n2";
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), want);
    assert_eq!(history(vcx), [(want.to_string(), false)]);
    assert_eq!(left_text(&ws, vcx), "Column copied");
    assert_eq!(text(&ws, vcx), TABLE);
    // pasting it back brings the same lines
    vcx.simulate_keystrokes("cmd-a cmd-v");
    assert_eq!(text(&ws, vcx), want);
}

#[gpui::test]
fn an_unclosed_quote_alerts_with_the_line_and_every_column_command_changes_nothing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let broken = "a,b\n1,\"two\n3,4\n";
    type_in(&ws, vcx, broken);
    caret_list(&ws, vcx, &[1]);
    history(vcx);
    for op in
        [ColumnOp::Select, ColumnOp::Copy, ColumnOp::MoveLeft, ColumnOp::MoveRight, ColumnOp::Delete, ColumnOp::Clear]
    {
        columns_cmd(vcx, op);
        let (_, detail) = vcx.pending_prompt().expect("an alert");
        assert_eq!(detail, "Unclosed quote in the record starting on line 2", "{op:?}");
        vcx.simulate_prompt_answer("Ok");
        assert_eq!(text(&ws, vcx), broken);
        assert_eq!(sels(&ws, vcx), [Selection::caret(1)]);
        assert!(history(vcx).is_empty(), "nothing copied");
    }
}

#[gpui::test]
fn the_delimiter_menu_choice_decides_the_columns_and_a_new_view_shares_it(cx: &mut TestAppContext) {
    let dir = temp("orlo-columns-delim").canonicalize().unwrap();
    let p = dir.join("t.txt");
    std::fs::write(&p, "a;b\n1;2\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    new_view(&ws, vcx);
    caret_list(&ws, vcx, &[2]);
    // detected: a semicolon table
    columns_cmd(vcx, ColumnOp::MoveLeft);
    assert_eq!(text(&ws, vcx), "b;a\n2;1\n");
    vcx.simulate_keystrokes("cmd-z");
    vcx.dispatch_action(SetDelimiter { delim: Some(Delim::Comma) });
    columns_cmd(vcx, ColumnOp::MoveLeft);
    assert_eq!(text(&ws, vcx), "a;b\n1;2\n", "one field a record under comma: nothing to move");
    // the other view of the buffer has the same choice
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    let choices: Vec<_> = views.iter().map(|v| v.read_with(vcx, |e, cx| e.doc(cx).delimiter_override())).collect();
    assert_eq!(choices, [Some(Delim::Comma); 2]);
    vcx.dispatch_action(SetDelimiter { delim: None });
    columns_cmd(vcx, ColumnOp::MoveLeft);
    assert_eq!(text(&ws, vcx), "b;a\n2;1\n");
}

fn names(items: &[MenuItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| match i {
            MenuItem::Action { name, .. } => name.to_string(),
            MenuItem::Submenu(m) => format!("{} >", m.name),
            _ => "-".into(),
        })
        .collect()
}

/// The items of the menu bar's menu `name`.
fn menu(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, name: &str) -> Vec<MenuItem> {
    ws.read_with(vcx, |ws, cx| {
        let menu = ws.menus(cx).into_iter().find(|m| m.name == name);
        menu.unwrap_or_else(|| panic!("{name}")).items
    })
}

/// The items of the submenu `name` of the menu bar's `menu`.
fn submenu(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, menu: &str, name: &str) -> Vec<MenuItem> {
    (self::menu(ws, vcx, menu).into_iter())
        .find_map(|i| match i {
            MenuItem::Submenu(m) if m.name == name => Some(m.items),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{menu} > {name}"))
}

#[gpui::test]
fn edit_columns_menu_lists_the_commands_and_checks_the_active_delimiter(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let items = submenu(&ws, vcx, "Edit", "Columns");
    assert_eq!(
        names(&items),
        [
            "Column Editor…",
            "Pad to Column…",
            "Align on Pattern…",
            "Align Selected Fields",
            "-",
            "Select Column",
            "Copy Column",
            "Move Column Left",
            "Move Column Right",
            "Delete Column",
            "Clear Column",
            "-",
            "Delimiter >",
        ]
    );
    let delimiter = |items: &[MenuItem]| match items.last() {
        Some(MenuItem::Submenu(m)) => names(&m.items),
        _ => panic!("Delimiter"),
    };
    assert_eq!(delimiter(&items), ["✓ Automatic", "Comma", "Tab", "Semicolon", "Pipe"]);
    vcx.dispatch_action(SetDelimiter { delim: Some(Delim::Semicolon) });
    assert_eq!(delimiter(&submenu(&ws, vcx, "Edit", "Columns")), ["Automatic", "Comma", "Tab", "✓ Semicolon", "Pipe"]);
    assert_eq!(left_text(&ws, vcx), "Delimiter: Semicolon");
}

#[gpui::test]
fn the_menu_bar_follows_the_delimiter_across_tabs(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    vcx.update(|window, _| window.activate_window());
    vcx.dispatch_action(SetDelimiter { delim: Some(Delim::Pipe) });
    vcx.run_until_parked();
    let built = ws.read_with(vcx, |ws, _| ws.menus_built);
    // a second tab has no choice: the menu bar is rebuilt for it, not left checking Pipe
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    vcx.run_until_parked();
    assert!(ws.read_with(vcx, |ws, _| ws.menus_built) > built);
    let items = submenu(&ws, vcx, "Edit", "Columns");
    let Some(MenuItem::Submenu(delims)) = items.last() else { panic!("Delimiter") };
    assert_eq!(names(&delims.items)[0], "✓ Automatic");
}

#[gpui::test]
fn the_other_view_follows_a_column_editor_insert_and_a_column_move(cx: &mut TestAppContext) {
    let dir = temp("orlo-columns-views").canonicalize().unwrap();
    let p = write_in(&dir, "t.csv", "a,b\n1,2\nx,y\n");
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    // the first view's caret is on the last line, which a move at the second view's caret shifts
    views[0].update(vcx, |e, cx| e.edit(cx, |s| s.set_cursor(9)));
    caret_list(&ws, vcx, &[0, 4]);
    open_column_editor(&ws, vcx);
    vcx.simulate_input("#");
    vcx.simulate_keystrokes("enter");
    let both = |vcx: &mut VisualTestContext| -> Vec<(String, usize)> {
        views.iter().map(|v| v.read_with(vcx, |e, cx| (e.doc(cx).text(), e.state.cursor()))).collect()
    };
    assert_eq!(both(vcx), [("#a,b\n#1,2\nx,y\n".to_string(), 11), ("#a,b\n#1,2\nx,y\n".to_string(), 6)]);
    caret_list(&ws, vcx, &[1]);
    columns_cmd(vcx, ColumnOp::MoveRight);
    let texts: Vec<_> = both(vcx).into_iter().map(|b| b.0).collect();
    assert_eq!(texts, ["b,#a\n2,#1\ny,x\n"; 2]);
    vcx.simulate_keystrokes("cmd-z");
    let texts: Vec<_> = both(vcx).into_iter().map(|b| b.0).collect();
    assert_eq!(texts, ["#a,b\n#1,2\nx,y\n"; 2]);
}

/// A CSV just over the size where column commands leave the UI thread: three records with
/// columns, then many with one field, so the parse is big but the edit is small (a debug build
/// takes seconds to apply hundreds of thousands of changes).
fn huge_csv(first: &str) -> String {
    let text = first.repeat(3) + &"dddd\n".repeat(stats::BIG / 5 + 1);
    assert!(text.chars().count() > stats::BIG);
    text
}

/// `columns_cmd` without letting the background executor run: dispatching an action parks.
fn columns_cmd_unparked(vcx: &mut VisualTestContext, op: ColumnOp) {
    vcx.update(|window, cx| window.dispatch_action(Box::new(editor::ColumnCommand { op }), cx));
}

fn version(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> u64 {
    on_editor(ws, vcx, |e, _, cx| e.doc(cx).version)
}

#[gpui::test]
fn a_huge_csv_is_parsed_off_the_ui_thread_and_the_move_lands_when_done(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let text = huge_csv("aaaa,bbbb,cccc\n");
    set_state(&ws, vcx, &text);
    set_selections(&ws, vcx, &[(5, 5)]);
    let before = version(&ws, vcx);
    columns_cmd_unparked(vcx, ColumnOp::MoveRight);
    // nothing was parsed on this thread: the text waits for the background task
    assert_eq!(version(&ws, vcx), before);
    assert_eq!(left_text(&ws, vcx), "Working…");
    vcx.run_until_parked();
    let moved = huge_csv("aaaa,cccc,bbbb\n");
    assert!(text_is(&ws, vcx, &moved));
    assert_eq!(left_text(&ws, vcx), "Done");
    vcx.simulate_keystrokes("cmd-z");
    assert!(text_is(&ws, vcx, &text), "one undo step");
    vcx.simulate_keystrokes("cmd-shift-z");
    assert!(text_is(&ws, vcx, &moved));
}

fn text_is(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, want: &str) -> bool {
    on_editor(ws, vcx, |e, _, cx| e.doc(cx).rope == want)
}

#[gpui::test]
fn a_huge_csv_changed_while_it_is_parsed_is_left_alone(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let text = huge_csv("aaaa,bbbb,cccc\n");
    set_state(&ws, vcx, &text);
    set_selections(&ws, vcx, &[(5, 5)]);
    columns_cmd_unparked(vcx, ColumnOp::Delete);
    type_in(&ws, vcx, "X");
    let typed = version(&ws, vcx);
    vcx.run_until_parked();
    assert_eq!(version(&ws, vcx), typed, "the late result was dropped");
    assert!(self::text(&ws, vcx).starts_with("aaaa,Xbbbb,cccc"));
    assert_eq!(left_text(&ws, vcx), "The document changed; run the command again");
    // a caret moved meanwhile picked another column: also dropped
    columns_cmd_unparked(vcx, ColumnOp::Delete);
    set_selections(&ws, vcx, &[(0, 0)]);
    vcx.run_until_parked();
    assert_eq!(version(&ws, vcx), typed);
    assert_eq!(left_text(&ws, vcx), "The document changed; run the command again");
}

#[gpui::test]
fn an_unclosed_quote_in_a_huge_csv_alerts_from_the_background_parse(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let mut text = huge_csv("aaaa,bbbb,cccc\n");
    let records = text.matches('\n').count();
    text.push_str("\"oops,1\n");
    set_state(&ws, vcx, &text);
    set_selections(&ws, vcx, &[(5, 5)]);
    columns_cmd_unparked(vcx, ColumnOp::Delete);
    assert!(vcx.pending_prompt().is_none(), "not before the parse is done");
    vcx.run_until_parked();
    let (_, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(detail, format!("Unclosed quote in the record starting on line {}", records + 1));
    assert!(text_is(&ws, vcx, &text));
}

// ---- Edit > Transform ----

use crate::editing::transform::{NormForm, TextOp};

/// The focused form's first text field gets `typed`, then Enter.
fn submit_form(vcx: &mut VisualTestContext, typed: &str) {
    if !typed.is_empty() {
        vcx.simulate_input(typed);
    }
    vcx.simulate_keystrokes("enter");
}

fn undo_redo(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, before: &str, after: &str) {
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(ws, vcx), before, "undo");
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(ws, vcx), after, "redo");
}

#[gpui::test]
fn sort_lines_by_pattern_sorts_by_the_key_in_one_undo_step(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "b 2\na 10\nc 1\nx");
    vcx.dispatch_action(SortLinesByPattern);
    click_field(vcx, "numeric");
    submit_form(vcx, r"(\d+)");
    assert_eq!(form_error(&ws, vcx), None, "the panel closed");
    assert_eq!(text(&ws, vcx), "c 1\nb 2\na 10\nx");
    undo_redo(&ws, vcx, "b 2\na 10\nc 1\nx", "c 1\nb 2\na 10\nx");
    // only the selected lines, widened to whole lines
    vcx.simulate_keystrokes("cmd-z");
    set_selections(&ws, vcx, &[(1, 5)]);
    vcx.dispatch_action(SortLinesByPattern);
    click_field(vcx, "numeric");
    click_field(vcx, "reverse");
    submit_form(vcx, r"\d+");
    assert_eq!(text(&ws, vcx), "a 10\nb 2\nc 1\nx");
}

#[gpui::test]
fn n30_sort_form_exposes_natural_capture_and_header_with_one_undo(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let before = "H\nb:v2-part10\nx\na:v2-part9\nc:v2-part9\nz";
    type_in(&ws, vcx, before);
    vcx.dispatch_action(SortLinesByPattern);
    click_field(vcx, "order"); // lexical -> natural
    click_field(vcx, "keep_header");
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("2");
    vcx.simulate_keystrokes("shift-tab");
    submit_form(vcx, r"^([^:]+):(.+)$");
    let after = "H\na:v2-part9\nc:v2-part9\nb:v2-part10\nx\nz";
    assert_eq!(form_error(&ws, vcx), None);
    assert_eq!(text(&ws, vcx), after);
    undo_redo(&ws, vcx, before, after);
}

#[gpui::test]
fn n30_sort_form_invalid_capture_keeps_source_and_shows_error(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "b2\na1");
    vcx.dispatch_action(SortLinesByPattern);
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("2");
    vcx.simulate_keystrokes("shift-tab");
    submit_form(vcx, r"(\d+)");
    assert!(form_error(&ws, vcx).flatten().unwrap().contains("capture"));
    assert_eq!(text(&ws, vcx), "b2\na1");
    vcx.simulate_keystrokes("tab");
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("-1");
    vcx.simulate_keystrokes("enter");
    assert!(form_error(&ws, vcx).flatten().unwrap().contains("capture"));
    assert_eq!(text(&ws, vcx), "b2\na1");
}

#[gpui::test]
fn n30_sort_form_offers_locale_and_grapheme_length(cx: &mut TestAppContext) {
    for (clicks, before, after) in [(2, "é\ne\u{301}", "é\ne\u{301}"), (3, "bb\ne\u{301}\n👩‍💻", "e\u{301}\n👩‍💻\nbb")]
    {
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, before);
        vcx.dispatch_action(SortLinesByPattern);
        for _ in 0..clicks {
            click_field(vcx, "order");
        }
        submit_form(vcx, ".*");
        assert_eq!(form_error(&ws, vcx), None);
        assert_eq!(text(&ws, vcx), after);
    }
}

#[gpui::test]
fn a_bad_sort_pattern_is_said_and_changes_nothing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "b\na");
    vcx.dispatch_action(SortLinesByPattern);
    submit_form(vcx, "*");
    let error = form_error(&ws, vcx).flatten().expect("the panel stays");
    assert!(error.contains("repetition"), "{error}");
    assert_eq!(text(&ws, vcx), "b\na");
    // an empty pattern is also refused
    vcx.update(|window, cx| window.dispatch_action(Box::new(ClosePanel), cx));
    vcx.dispatch_action(SortLinesByPattern);
    submit_form(vcx, "");
    assert!(form_error(&ws, vcx).flatten().is_some());
    assert_eq!(text(&ws, vcx), "b\na");
}

#[gpui::test]
fn process_duplicate_lines_deletes_and_copies_the_removed_lines(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a\nb\na\nc\nb");
    vcx.dispatch_action(ProcessDuplicateLines);
    // keep first -> keep last
    click_field(vcx, "action");
    click_field(vcx, "copy_removed");
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "a\nc\nb");
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "a\nb\n");
    assert_eq!(history(vcx)[0], ("a\nb\n".to_string(), false), "in the history too");
    undo_redo(&ws, vcx, "a\nb\na\nc\nb", "a\nc\nb");
    // delete every duplicated line, comparing without case
    vcx.simulate_keystrokes("cmd-z");
    type_in(&ws, vcx, "\nA");
    vcx.dispatch_action(ProcessDuplicateLines);
    click_field(vcx, "compare");
    click_field(vcx, "action");
    click_field(vcx, "action");
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "c");
}

#[gpui::test]
fn process_lines_matching_keeps_deletes_copies_or_extracts(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let doc = "apple\nbanana\ncherry\navocado";
    type_in(&ws, vcx, doc);
    let run = |vcx: &mut VisualTestContext, pattern: &str, picks: &[&'static str]| {
        vcx.dispatch_action(ProcessLinesMatching);
        for key in picks {
            click_field(vcx, key);
        }
        submit_form(vcx, pattern);
    };
    // keep (the first action)
    run(vcx, "^a", &[]);
    assert_eq!(text(&ws, vcx), "apple\navocado");
    undo_redo(&ws, vcx, doc, "apple\navocado");
    vcx.simulate_keystrokes("cmd-z");
    // delete, inverted
    run(vcx, "^a", &["invert", "action"]);
    assert_eq!(text(&ws, vcx), "apple\navocado");
    vcx.simulate_keystrokes("cmd-z");
    run(vcx, "^a", &["action"]);
    assert_eq!(text(&ws, vcx), "banana\ncherry");
    vcx.simulate_keystrokes("cmd-z");
    // copy: the document is untouched
    run(vcx, "^a", &["action", "action"]);
    assert_eq!(text(&ws, vcx), doc);
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "apple\navocado\n");
    assert_eq!(history(vcx)[0].0, "apple\navocado\n");
    // extract: a new untitled tab, the document untouched
    let tabs = tab_count(&ws, vcx);
    run(vcx, "an", &["regex", "action", "action", "action"]);
    assert_eq!(tab_count(&ws, vcx), tabs + 1);
    assert_eq!(text(&ws, vcx), "banana\n", "literal 'an' (regex off)");
    assert_eq!(active_path(&ws, vcx), None);
}

#[gpui::test]
fn lines_emptied_by_a_command_take_their_line_breaks_with_them(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a\nb\nc");
    // delete the lines matching, in a selection of the middle line
    set_selections(&ws, vcx, &[(2, 3)]);
    vcx.dispatch_action(ProcessLinesMatching);
    click_field(vcx, "action");
    submit_form(vcx, "b");
    assert_eq!(text(&ws, vcx), "a\nc");
    undo_redo(&ws, vcx, "a\nb\nc", "a\nc");
    // keep the lines matching nothing: the selected block goes, the others stay
    vcx.simulate_keystrokes("cmd-z");
    set_selections(&ws, vcx, &[(2, 3)]);
    vcx.dispatch_action(ProcessLinesMatching);
    submit_form(vcx, "zzz");
    assert_eq!(text(&ws, vcx), "a\nc");
    undo_redo(&ws, vcx, "a\nb\nc", "a\nc");
    // the last line, which has no break of its own
    vcx.simulate_keystrokes("cmd-z");
    set_selections(&ws, vcx, &[(4, 5)]);
    vcx.dispatch_action(ProcessLinesMatching);
    click_field(vcx, "action");
    submit_form(vcx, "c");
    assert_eq!(text(&ws, vcx), "a\nb");
    // every selected line duplicated: all of them go
    set_state(&ws, vcx, "x\nd\nd\ny");
    type_in(&ws, vcx, "");
    set_selections(&ws, vcx, &[(2, 5)]);
    vcx.dispatch_action(ProcessDuplicateLines);
    click_field(vcx, "action");
    click_field(vcx, "action");
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "x\ny");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "x\nd\nd\ny");
}

#[gpui::test]
fn blank_lines_can_be_matched_and_copied_as_removed_duplicates(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a\n\nb\n\nc");
    // copy the lines matching ^$
    vcx.dispatch_action(ProcessLinesMatching);
    click_field(vcx, "action");
    click_field(vcx, "action");
    submit_form(vcx, "^$");
    assert_eq!(form_error(&ws, vcx), None, "the blank lines matched");
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "\n\n");
    assert_eq!(text(&ws, vcx), "a\n\nb\n\nc");
    // the second blank line is the duplicate that is copied
    vcx.dispatch_action(ProcessDuplicateLines);
    click_field(vcx, "copy_removed");
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "a\n\nb\nc");
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "\n");
    assert_eq!(history(vcx)[0], ("\n".to_string(), false));
    undo_redo(&ws, vcx, "a\n\nb\n\nc", "a\n\nb\nc");
}

#[gpui::test]
fn nothing_to_copy_or_extract_keeps_the_panel_open(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "one\ntwo");
    vcx.dispatch_action(ProcessLinesMatching);
    click_field(vcx, "action");
    click_field(vcx, "action");
    submit_form(vcx, "zzz");
    let error = form_error(&ws, vcx).flatten().expect("the panel stays");
    assert!(error.contains("No lines match"), "{error}");
    assert_eq!(tab_count(&ws, vcx), 1);
}

#[gpui::test]
fn line_prefix_and_suffix_add_and_remove(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a\n\nb\n");
    let affix = |vcx: &mut VisualTestContext, suffix, remove, typed: &str| {
        vcx.dispatch_action(AffixLines { suffix, remove });
        submit_form(vcx, typed);
    };
    affix(vcx, false, false, "> ");
    assert_eq!(text(&ws, vcx), "> a\n\n> b\n", "blank lines are skipped, the final break stays");
    undo_redo(&ws, vcx, "a\n\nb\n", "> a\n\n> b\n");
    affix(vcx, true, false, ";");
    assert_eq!(text(&ws, vcx), "> a;\n\n> b;\n");
    affix(vcx, false, true, "> ");
    affix(vcx, true, true, ";");
    assert_eq!(text(&ws, vcx), "a\n\nb\n");
    // blank lines can be given the text too
    vcx.dispatch_action(AffixLines { suffix: false, remove: false });
    click_field(vcx, "skip_blank");
    submit_form(vcx, "-");
    assert_eq!(text(&ws, vcx), "-a\n-\n-b\n");
    // without text there is nothing to do
    vcx.dispatch_action(AffixLines { suffix: true, remove: true });
    submit_form(vcx, "");
    assert!(form_error(&ws, vcx).flatten().is_some());
}

#[gpui::test]
fn quote_and_unicode_commands_work_on_selections_or_the_document(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let run = |vcx: &mut VisualTestContext, op| vcx.dispatch_action(editor::TransformText { op });
    type_in(&ws, vcx, "say \"hi\" it's");
    run(vcx, TextOp::SmartQuotes);
    assert_eq!(text(&ws, vcx), "say “hi” it’s");
    undo_redo(&ws, vcx, "say \"hi\" it's", "say “hi” it’s");
    run(vcx, TextOp::StraightenQuotes);
    assert_eq!(text(&ws, vcx), "say \"hi\" it's");
    // only the selections
    vcx.simulate_keystrokes("cmd-z cmd-z");
    set_selections(&ws, vcx, &[(0, 3), (4, 8)]);
    run(vcx, TextOp::SmartQuotes);
    assert_eq!(text(&ws, vcx), "say “hi” it's");
    // Unicode forms and diacritics, on the whole document
    set_state(&ws, vcx, "caf\u{e9}");
    run(vcx, TextOp::Normalize(NormForm::Nfd));
    assert_eq!(text(&ws, vcx), "cafe\u{301}");
    undo_redo(&ws, vcx, "caf\u{e9}", "cafe\u{301}");
    run(vcx, TextOp::StripDiacritics);
    assert_eq!(text(&ws, vcx), "cafe");
}

fn open_table(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, rows: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = temp(&format!("orlo-table-{}-{n}", std::process::id())).join("table.tsv");
    std::fs::write(&path, rows).unwrap();
    let file = path.clone();
    ws.update_in(vcx, |ws, window, cx| ws.substitution_form(file, window, cx));
    vcx.run_until_parked();
    path
}

#[gpui::test]
fn a_substitution_table_applies_literally_in_one_pass_and_one_undo_step(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "ab Ab ab");
    open_table(&ws, vcx, "# swap\na\tb\nb\tc\n\n");
    vcx.simulate_keystrokes("enter");
    assert_eq!(form_error(&ws, vcx), None, "the panel closed");
    assert_eq!(text(&ws, vcx), "bc Ac bc");
    undo_redo(&ws, vcx, "ab Ab ab", "bc Ac bc");
    // case-insensitive, whole word, and only the selection
    vcx.simulate_keystrokes("cmd-z");
    set_selections(&ws, vcx, &[(0, 5)]);
    open_table(&ws, vcx, "ab\tX");
    click_field(vcx, "case_insensitive");
    click_field(vcx, "whole_word");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "X X ab");
}

#[gpui::test]
fn a_regex_substitution_table_applies_its_rows_in_order_with_captures(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "me@home");
    open_table(&ws, vcx, "(\\w+)@(\\w+)\t$2 at $1\nat\tAT\n");
    click_field(vcx, "mode");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "home AT me");
}

#[gpui::test]
fn a_bad_substitution_table_lists_every_problem_and_changes_nothing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "foo");
    let path = open_table(&ws, vcx, "x\nfoo\tbar\nfoo\tbaz\n\tq\n");
    vcx.simulate_keystrokes("enter");
    let error = form_error(&ws, vcx).flatten().expect("the panel stays");
    let lines: Vec<_> = error.lines().collect();
    assert_eq!(lines.len(), 3, "{error}");
    assert!(lines[0].starts_with("Line 1:"), "{error}");
    assert!(lines[1].starts_with("Line 3:"), "{error}");
    assert!(lines[2].starts_with("Line 4:"), "{error}");
    assert_eq!(text(&ws, vcx), "foo");
    // fixed on disk, the same panel's next Enter reads the file again
    std::fs::write(&path, "foo\tbar\n").unwrap();
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "bar");
    // a file that cannot be read says so
    let gone = path.with_file_name("gone.tsv");
    ws.update_in(vcx, |ws, window, cx| ws.substitution_form(gone, window, cx));
    vcx.simulate_keystrokes("enter");
    let error = form_error(&ws, vcx).flatten().expect("the panel stays");
    assert!(error.contains("gone.tsv"), "{error}");
}

#[gpui::test]
fn edit_transform_menu_lists_the_commands(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    assert_eq!(
        names(&submenu(&ws, vcx, "Edit", "Transform")),
        [
            "Sort Lines by Pattern…",
            "Process Duplicate Lines…",
            "Process Lines Matching…",
            "-",
            "Add Line Prefix…",
            "Add Line Suffix…",
            "Remove Line Prefix…",
            "Remove Line Suffix…",
            "-",
            "Straighten Quotes",
            "Smart Quotes",
            "-",
            "Unicode Normalization >",
            "Strip Diacritics",
            "-",
            "Apply Substitution Table…",
        ]
    );
}

// ---- Tools > Text Pipelines ----

use crate::searching::pipeline;

/// A saved pipeline of `steps` (the JSON of each) in this test's Pipelines folder.
fn write_pipeline(name: &str, steps: &str) -> PathBuf {
    std::fs::create_dir_all(pipeline::dir()).unwrap();
    let path = pipeline::dir().join(format!("{name}.orlo-pipeline"));
    let json = format!(r#"{{ "name": "{name}", "steps": [ {steps} ] }}"#);
    std::fs::write(&path, json).unwrap();
    path
}

const SORT: &str = r#"{ "op": "sort_lines" }"#;

/// The pipeline list's rows.
fn pipeline_rows(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Vec<String> {
    ws.read_with(vcx, |ws, _| {
        let panel = ws.quick_panel.as_ref().expect("the pipeline list");
        panel.items.iter().map(|i| i.label.clone()).collect()
    })
}

/// Type `name` in the pipeline list and press Enter.
fn choose_pipeline(vcx: &mut VisualTestContext, name: &str) {
    vcx.simulate_input(name);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
}

/// Every tab's title, text and whether it is unsaved, in tab order.
fn tab_list(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Vec<(String, String, bool)> {
    ws.read_with(vcx, |ws, cx| {
        (ws.views.tabs())
            .map(|t| {
                let e = t.read(cx);
                (e.title(cx), e.doc(cx).text(), e.doc(cx).unsaved())
            })
            .collect()
    })
}

fn set_text(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, text: &str) {
    ws.update(vcx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        editor.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert(text)
            })
        });
    });
}

fn new_pipeline_named(vcx: &mut VisualTestContext, name: &str) {
    vcx.dispatch_action(NewPipeline);
    vcx.simulate_input(name);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
}

#[gpui::test]
fn run_pipeline_changes_the_document_in_one_undo_step(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_pipeline("tidy", &format!(r#"{SORT}, {{ "op": "add_line_prefix", "text": "- " }}"#));
    type_in(&ws, vcx, "b\na");
    vcx.dispatch_action(RunPipeline);
    assert_eq!(pipeline_rows(&ws, vcx), ["tidy"]);
    choose_pipeline(vcx, "tidy");
    assert_eq!(text(&ws, vcx), "- a\n- b");
    undo_redo(&ws, vcx, "b\na", "- a\n- b");
    // a selection: its lines only
    vcx.simulate_keystrokes("cmd-z");
    set_selections(&ws, vcx, &[(0, 1)]);
    vcx.dispatch_action(RunPipeline);
    choose_pipeline(vcx, "tidy");
    assert_eq!(text(&ws, vcx), "- b\na");
    undo_redo(&ws, vcx, "b\na", "- b\na");
    assert!(!vcx.has_pending_prompt());
}

#[gpui::test]
fn a_failing_step_aborts_the_run_with_its_number_and_changes_nothing(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_pipeline("bad", r#"{ "op": "convert_case", "case": "upper" }, { "op": "decode", "codec": "base64" }"#);
    type_in(&ws, vcx, "not base64!");
    vcx.dispatch_action(RunPipeline);
    choose_pipeline(vcx, "bad");
    let (message, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "Could not run pipeline");
    assert!(detail.starts_with("Step 2 (decode): "), "{detail}");
    assert_eq!(text(&ws, vcx), "not base64!");
    // nothing to undo to: the typing is the only step
    vcx.simulate_prompt_answer("Ok");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "");
}

#[gpui::test]
fn preview_opens_a_scratch_tab_and_leaves_the_document(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_pipeline("tidy", SORT);
    type_in(&ws, vcx, "b\na");
    vcx.dispatch_action(PreviewPipeline);
    choose_pipeline(vcx, "tidy");
    assert_eq!(
        tab_list(&ws, vcx),
        [("untitled".to_string(), "b\na".to_string(), true), ("Preview: tidy".to_string(), "a\nb".to_string(), false),]
    );
    let scratch = ws.read_with(vcx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        let e = e.read(cx);
        (e.doc(cx).scratch, e.doc(cx).path.clone())
    });
    assert_eq!(scratch, (true, None));
    // closing the preview asks nothing
    vcx.dispatch_action(CloseTab);
    assert!(!vcx.has_pending_prompt());
    assert_eq!(tab_list(&ws, vcx).len(), 1);
    // a selection is what is previewed; a failing step previews nothing
    set_selections(&ws, vcx, &[(0, 1)]);
    vcx.dispatch_action(PreviewPipeline);
    choose_pipeline(vcx, "tidy");
    assert_eq!(tab_list(&ws, vcx)[1].1, "b\n");
    write_pipeline("bad", r#"{ "op": "decode", "codec": "hex" }"#);
    vcx.dispatch_action(PreviewPipeline);
    choose_pipeline(vcx, "bad");
    let (message, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "Could not run pipeline");
    assert!(detail.starts_with("Step 1 (decode): "), "{detail}");
    assert_eq!(tab_list(&ws, vcx).len(), 2);
}

#[gpui::test]
fn the_pipeline_list_says_so_when_there_are_none(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    for action in [
        Box::new(RunPipeline) as Box<dyn Action>,
        Box::new(PreviewPipeline),
        Box::new(EditPipeline),
        Box::new(RunPipelineOnFiles),
    ] {
        vcx.update(|window, cx| window.dispatch_action(action, cx));
        vcx.run_until_parked();
        let (message, detail) = vcx.pending_prompt().expect("an alert");
        assert_eq!(message, "No pipelines");
        assert!(detail.contains("New Pipeline"), "{detail}");
        vcx.simulate_prompt_answer("Ok");
    }
    assert_eq!(text(&ws, vcx), "");
}

#[gpui::test]
fn new_pipeline_opens_the_template_in_a_tab_named_for_it(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    new_pipeline_named(vcx, "Tidy up");
    let path = crate::editing::text::resolve(&pipeline::dir().join("Tidy up.orlo-pipeline"));
    assert_eq!(active_path(&ws, vcx), Some(path.clone()));
    // the template is a buffer, written only by the first save, so closing it asks nothing
    assert_eq!(tab_list(&ws, vcx), [("Tidy up.orlo-pipeline".to_string(), pipeline::template(), false)]);
    assert!(!path.exists());
    // a name that is not a file name is said, and nothing opens
    vcx.dispatch_action(NewPipeline);
    vcx.simulate_input("a/b");
    vcx.simulate_keystrokes("enter");
    let error = form_error(&ws, vcx).flatten().expect("the panel stays");
    assert!(error.contains("/"), "{error}");
    assert_eq!(tab_list(&ws, vcx).len(), 1);
    vcx.update(|window, cx| window.dispatch_action(Box::new(ClosePanel), cx));
    // a name already taken opens that pipeline, not the template over it
    let kept = write_pipeline("kept", SORT);
    new_pipeline_named(vcx, "kept");
    assert_eq!(active_path(&ws, vcx), Some(crate::editing::text::resolve(&kept)));
    assert!(text(&ws, vcx).contains("sort_lines"));
    assert_eq!(tab_list(&ws, vcx).len(), 2);
}

#[gpui::test]
fn edit_pipeline_opens_the_chosen_file(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_pipeline("one", SORT);
    let two = write_pipeline("two", r#"{ "op": "reverse_lines" }"#);
    vcx.dispatch_action(EditPipeline);
    assert_eq!(pipeline_rows(&ws, vcx), ["one", "two"]);
    choose_pipeline(vcx, "two");
    assert_eq!(active_path(&ws, vcx), Some(crate::editing::text::resolve(&two)));
    assert!(text(&ws, vcx).contains("reverse_lines"));
}

#[gpui::test]
fn saving_a_pipeline_file_alerts_with_the_problems_and_their_lines(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    new_pipeline_named(vcx, "mine");
    set_text(
        &ws,
        vcx,
        "{\n  \"steps\": [\n    { \"op\": \"nope\" },\n    { \"op\": \"sort_lines\", \"cse\": 1 }\n  ]\n}",
    );
    vcx.dispatch_action(Save);
    vcx.run_until_parked();
    let (message, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "Pipeline has problems");
    let lines: Vec<_> = detail.lines().collect();
    assert_eq!(lines.len(), 2, "{detail}");
    assert!(lines[0].starts_with("Line 3: step 1: "), "{detail}");
    assert!(lines[1].starts_with("Line 4: step 2: "), "{detail}");
    // saved all the same: the alert is advice, the file is the user's
    let path = pipeline::dir().join("mine.orlo-pipeline");
    assert!(std::fs::read_to_string(&path).unwrap().contains("nope"));
    vcx.simulate_prompt_answer("Ok");
    // a good one saves quietly, and so does any other file
    set_text(&ws, vcx, r#"{ "steps": [ { "op": "sort_lines" } ] }"#);
    vcx.dispatch_action(Save);
    vcx.run_until_parked();
    assert!(!vcx.has_pending_prompt());
    let json = temp("orlo-not-a-pipeline").join("x.json");
    ws.update_in(vcx, |ws, window, cx| ws.open_path(json.clone(), window, cx));
    set_text(&ws, vcx, "{ nope");
    vcx.dispatch_action(Save);
    vcx.run_until_parked();
    assert!(!vcx.has_pending_prompt());
}

#[gpui::test]
fn a_pipeline_with_problems_is_refused_before_a_file_is_chosen(cx: &mut TestAppContext) {
    // the file chooser is not in gpui's test platform: reaching it would panic
    let (_ws, vcx) = window(cx);
    let path = write_pipeline("broken", r#"{ "op": "nope" }"#);
    for how in [PipelineUse::RunOnFiles, PipelineUse::Run, PipelineUse::Preview] {
        vcx.dispatch_action(UsePipeline { path: path.clone(), how });
        let (message, detail) = vcx.pending_prompt().expect("an alert");
        assert_eq!(message, "Pipeline has problems");
        assert!(detail.starts_with("Line 1: step 1: "), "{detail}");
        vcx.simulate_prompt_answer("Ok");
    }
    vcx.dispatch_action(UsePipeline { path: path.with_file_name("gone.orlo-pipeline"), how: PipelineUse::Run });
    let (_, detail) = vcx.pending_prompt().expect("an alert");
    assert!(detail.contains("gone.orlo-pipeline"), "{detail}");
}

fn tidy() -> std::sync::Arc<pipeline::Pipeline> {
    let json = r#"{ "name": "tidy", "steps": [ { "op": "sort_lines" } ] }"#;
    std::sync::Arc::new(pipeline::parse(json).unwrap())
}

/// The active tab as the batch report left it: its title and text.
fn report_tab(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> (String, String) {
    ws.read_with(vcx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        let e = e.read(cx);
        assert!(e.doc(cx).scratch && e.doc(cx).path.is_none());
        (e.title(cx), e.doc(cx).text())
    })
}

#[gpui::test]
fn a_batch_changes_open_buffers_in_place_and_closed_files_on_disk(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp("orlo-pipeline-batch").canonicalize().unwrap();
    let file = |name: &str, bytes: &[u8]| {
        std::fs::write(dir.join(name), bytes).unwrap();
        dir.join(name)
    };
    let open = file("open.txt", b"b\na\n");
    let closed = file("closed.txt", b"\xEF\xBB\xBFb\r\na\r\n");
    let latin1 = file("latin1.txt", b"caf\xE9\nb\na\n");
    let locked = file("locked.txt", b"b\na\n");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o444)).unwrap();
    let same = file("same.txt", b"a\nb\n");
    let md = file("notes.md", b"b\na\n");
    std::fs::create_dir(dir.join("sub")).unwrap();
    let nested = file("sub/nested.txt", b"z\ny");
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(open.clone(), window, cx));
    settle(vcx);
    // an edit the buffer has and the disk does not
    type_in(&ws, vcx, "z\n");
    let same_time = std::fs::metadata(&same).unwrap().modified().unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.start_batch(tidy(), vec![dir.clone()], Some("*.txt".into()), window, cx));
    let progress = ws.read_with(vcx, |ws, _| ws.pipeline_progress());
    assert!(progress.unwrap().starts_with("Pipeline tidy"), "running");
    settle(vcx);
    assert_eq!(ws.read_with(vcx, |ws, _| ws.pipeline_progress()), None);

    // the open buffer: transformed with its edit, unsaved, the file as it was
    let tabs = tab_list(&ws, vcx);
    assert_eq!(tabs[0], ("open.txt".to_string(), "a\nb\nz\n".to_string(), true));
    assert_eq!(std::fs::read(&open).unwrap(), b"b\na\n");
    // closed files: written with their line ending and BOM kept
    assert_eq!(std::fs::read(&closed).unwrap(), b"\xEF\xBB\xBFa\r\nb\r\n".to_vec());
    assert_eq!(std::fs::read(&nested).unwrap(), b"y\nz");
    // the rest as they were
    assert_eq!(std::fs::read(&latin1).unwrap(), b"caf\xE9\nb\na\n");
    assert_eq!(std::fs::read(&locked).unwrap(), b"b\na\n");
    assert_eq!(std::fs::read(&md).unwrap(), b"b\na\n");
    assert_eq!(std::fs::metadata(&same).unwrap().modified().unwrap(), same_time);

    // the report
    let (title, report) = report_tab(&ws, vcx);
    assert_eq!(title, "Pipeline Report: tidy");
    let want = |heading: &str, path: &Path, why: &str| {
        let entry = format!("{heading}:\n    {}{why}\n", path.display());
        assert!(report.contains(&entry), "{entry:?} in\n{report}");
    };
    want("Changed (2)", &closed, "");
    want("Unchanged (1)", &same, "");
    want("Changed in open buffer, not saved (1)", &open, "");
    want("Skipped (1)", &latin1, ": malformed text; reopen with the correct encoding");
    assert!(report.contains(&format!("{}\n", nested.display())));
    let failed = format!("Failed (1):\n    {}: ", locked.display());
    assert!(report.contains(&failed), "{report}");
    assert!(report.contains("read-only"), "{report}");
    assert!(!report.contains("notes.md"));

    // one undo step takes the pipeline back and leaves the earlier edit
    vcx.dispatch_action(SelectTab { index: 0 });
    undo_redo(&ws, vcx, "z\nb\na\n", "a\nb\nz\n");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "z\nb\na\n");
    // the report asks nothing when it is closed
    vcx.dispatch_action(SelectTab { index: 1 });
    vcx.dispatch_action(CloseTab);
    assert!(!vcx.has_pending_prompt());
}

#[gpui::test]
fn a_run_and_a_batch_reach_every_view_of_a_buffer(cx: &mut TestAppContext) {
    let dir = temp("orlo-pipeline-views").canonicalize().unwrap();
    let file = write_in(&dir, "v.txt", "b\na\n");
    let (ws, vcx) = window(cx);
    write_pipeline("tidy", SORT);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(file.clone(), window, cx));
    settle(vcx);
    new_view(&ws, vcx);
    let texts = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| -> Vec<String> {
        tab_list(ws, vcx).into_iter().map(|t| t.1).collect()
    };
    vcx.dispatch_action(RunPipeline);
    choose_pipeline(vcx, "tidy");
    assert_eq!(texts(&ws, vcx), ["a\nb\n", "a\nb\n"]);
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(texts(&ws, vcx), ["b\na\n", "b\na\n"]);
    ws.update_in(vcx, |ws, window, cx| ws.start_batch(tidy(), vec![file.clone()], None, window, cx));
    settle(vcx);
    assert_eq!(texts(&ws, vcx)[..2], ["a\nb\n", "a\nb\n"]);
    assert_eq!(std::fs::read(&file).unwrap(), b"b\na\n");
}

#[gpui::test]
fn with_no_tab_open_the_pipeline_commands_say_so_and_no_batch_runs_unfiltered(cx: &mut TestAppContext) {
    let dir = temp("orlo-pipeline-no-tab").canonicalize().unwrap();
    let (txt, md) = (dir.join("a.txt"), dir.join("b.md"));
    std::fs::write(&txt, "b\na").unwrap();
    std::fs::write(&md, "b\na").unwrap();
    let (ws, vcx) = window(cx);
    write_pipeline("tidy", SORT);
    vcx.dispatch_action(CloseTab);
    assert!(tab_list(&ws, vcx).is_empty());
    for action in [
        Box::new(NewPipeline) as Box<dyn Action>,
        Box::new(PreviewPipeline),
        Box::new(RunPipeline),
        Box::new(RunPipelineOnFiles),
    ] {
        vcx.update(|window, cx| window.dispatch_action(action, cx));
        vcx.run_until_parked();
        let (message, detail) = vcx.pending_prompt().expect("an alert");
        assert_eq!(message, "No open tab");
        assert!(detail.contains("Open a tab"), "{detail}");
        vcx.simulate_prompt_answer("Ok");
        assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
    }
    // a folder chosen while the tab went away: the glob cannot be asked, so nothing starts
    ws.update_in(vcx, |ws, window, cx| ws.ask_include(tidy(), vec![dir.clone()], window, cx));
    let (message, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "No open tab");
    assert!(detail.contains("files"), "{detail}");
    vcx.simulate_prompt_answer("Ok");
    settle(vcx);
    assert!(ws.read_with(vcx, |ws, _| ws.pipeline_run.is_none()));
    assert!(tab_list(&ws, vcx).is_empty(), "no report");
    assert_eq!(std::fs::read(&txt).unwrap(), b"b\na");
    assert_eq!(std::fs::read(&md).unwrap(), b"b\na");
    // files named one by one need no glob
    ws.update_in(vcx, |ws, window, cx| ws.ask_include(tidy(), vec![txt.clone()], window, cx));
    settle(vcx);
    assert_eq!(std::fs::read(&txt).unwrap(), b"a\nb");
    assert_eq!(std::fs::read(&md).unwrap(), b"b\na");
}

#[gpui::test]
fn files_named_one_by_one_are_run_whatever_the_glob_says(cx: &mut TestAppContext) {
    let dir = temp("orlo-pipeline-explicit").canonicalize().unwrap();
    let (md, txt) = (dir.join("a.md"), dir.join("b.txt"));
    std::fs::write(&md, "b\na").unwrap();
    std::fs::write(&txt, "b\na").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.start_batch(tidy(), vec![md.clone(), txt.clone()], Some("*.nothing".into()), window, cx)
    });
    settle(vcx);
    assert_eq!(std::fs::read(&md).unwrap(), b"a\nb");
    assert_eq!(std::fs::read(&txt).unwrap(), b"a\nb");
}

#[gpui::test]
fn a_batch_with_no_matching_files_says_so_and_touches_nothing(cx: &mut TestAppContext) {
    let dir = temp("orlo-pipeline-none").canonicalize().unwrap();
    std::fs::write(dir.join("a.md"), "b\na").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.start_batch(tidy(), vec![dir.clone()], Some("*.txt".into()), window, cx));
    settle(vcx);
    assert_eq!(std::fs::read(dir.join("a.md")).unwrap(), b"b\na");
    let (title, report) = report_tab(&ws, vcx);
    assert_eq!(title, "Pipeline Report: tidy");
    assert!(report.contains("No files matched"), "{report}");
}

#[gpui::test]
fn cancelling_a_batch_stops_it_and_reports_what_was_done(cx: &mut TestAppContext) {
    let dir = temp("orlo-pipeline-cancel").canonicalize().unwrap();
    for n in ["a", "b", "c"] {
        std::fs::write(dir.join(format!("{n}.txt")), "b\na").unwrap();
    }
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.start_batch(tidy(), vec![dir.clone()], None, window, cx);
        ws.cancel_pipeline_run(&CancelPipelineRun, window, cx);
    });
    settle(vcx);
    for n in ["a", "b", "c"] {
        assert_eq!(std::fs::read(dir.join(format!("{n}.txt"))).unwrap(), b"b\na");
    }
    let (_, report) = report_tab(&ws, vcx);
    assert!(report.contains("Cancelled"), "{report}");
    assert_eq!(ws.read_with(vcx, |ws, _| ws.pipeline_progress()), None);
    // with nothing running, Cancel says so
    ws.update_in(vcx, |ws, window, cx| ws.cancel_pipeline_run(&CancelPipelineRun, window, cx));
    assert_eq!(left_text(&ws, vcx), "No pipeline is running");
}

#[gpui::test]
fn a_second_batch_waits_for_the_first(cx: &mut TestAppContext) {
    let dir = temp("orlo-pipeline-twice").canonicalize().unwrap();
    std::fs::write(dir.join("a.txt"), "b\na").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.start_batch(tidy(), vec![dir.clone()], None, window, cx);
        ws.start_batch(tidy(), vec![dir.clone()], None, window, cx);
    });
    let (message, _) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "A pipeline is already running");
    vcx.simulate_prompt_answer("Ok");
    settle(vcx);
    assert_eq!(tab_list(&ws, vcx).len(), 2, "one report");
}

#[gpui::test]
fn a_batch_reaches_buffers_open_in_other_windows(cx: &mut TestAppContext) {
    let dir = temp("orlo-pipeline-windows").canonicalize().unwrap();
    let file = write_in(&dir, "x.txt", "b\na\n");
    let (ws1, ws2, mut vcx1, mut vcx2) = two_windows(cx);
    ws2.update_in(&mut vcx2, |ws, window, cx| ws.open_path(file.clone(), window, cx));
    settle(&mut vcx2);
    type_in(&ws2, &mut vcx2, "z\n");
    ws1.update_in(&mut vcx1, |ws, window, cx| ws.start_batch(tidy(), vec![file.clone()], None, window, cx));
    settle(&mut vcx1);
    assert_eq!(tab_list(&ws2, &mut vcx2), [("x.txt".to_string(), "a\nb\nz\n".to_string(), true)]);
    assert_eq!(std::fs::read(&file).unwrap(), b"b\na\n");
    let (_, report) = report_tab(&ws1, &mut vcx1);
    assert!(report.contains(&format!("(1):\n    {}\n", file.display())), "{report}");
    assert!(report.contains("open buffer"), "{report}");
}

#[gpui::test]
fn tools_text_pipelines_menu_lists_the_commands(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    assert_eq!(
        names(&submenu(&ws, vcx, "Tools", "Text Pipelines")),
        [
            "New Pipeline…",
            "Edit Pipeline…",
            "-",
            "Preview Pipeline…",
            "Run Pipeline…",
            "Run Pipeline on Files…",
            "Cancel Pipeline Run",
        ]
    );
}

#[gpui::test]
fn a_batch_uses_a_buffer_opened_after_it_started(cx: &mut TestAppContext) {
    let dir = temp("orlo-pipeline-late-open").canonicalize().unwrap();
    let file = write_in(&dir, "data.txt", "b\na\n");
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.start_batch(tidy(), vec![file.clone()], None, window, cx);
        ws.open_path(file.clone(), window, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("z\n")));
    });
    settle(vcx);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "b\na\n");
    assert_eq!(tab_list(&ws, vcx)[0].1, "a\nb\nz\n");
}

#[gpui::test]
fn project_switch_discards_old_background_opens(cx: &mut TestAppContext) {
    let d = temp("orlo-project-read-restored").canonicalize().unwrap();
    let one = d.join("one.sublime-project");
    let two = d.join("two.sublime-project");
    let old = d.join("old.txt");
    std::fs::write(&one, "{}").unwrap();
    std::fs::write(&two, "{}").unwrap();
    std::fs::write(&old, "old project file").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_project_path(&OpenProjectPath { path: one, here: false }, window, cx);
        ws.read_in_background(old.clone(), window, cx);
        ws.open_project_path(&OpenProjectPath { path: two, here: true }, window, cx);
        let state = ws.capture(window, true, &mut Texts::new(), cx);
        assert!(
            state.groups.iter().flat_map(|g| &g.views).all(|v| v.path != old),
            "the new project's recovery state must not contain the old read"
        );
    });
    settle(vcx);
    ws.read_with(vcx, |ws, cx| {
        assert!(
            ws.views.tabs().all(|e| e.read(cx).doc(cx).path.as_ref() != Some(&old)),
            "the old read must not land in the new project"
        );
    });
}

#[gpui::test]
fn background_restore_keeps_preview_tabs(cx: &mut TestAppContext) {
    let d = temp("orlo-restore-preview").canonicalize().unwrap();
    let large = big_file(&d);
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.restore(
            WindowState {
                groups: vec![GroupState {
                    views: vec![ViewState { path: large.clone(), preview: true, ..Default::default() }],
                    active: 0,
                }],
                ..Default::default()
            },
            window,
            cx,
        );
    });
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
    ws.read_with(vcx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        assert_eq!(e.read(cx).doc(cx).path.as_ref(), Some(&large));
        assert!(ws.views.is_preview(&e), "large and small files restore the same preview state");
    });
}

#[gpui::test]
fn backward_tab_and_group_commands_keep_dirty_views_and_panel_state(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    vcx.simulate_resize(gpui::size(px(1000.), px(800.)));
    vcx.run_until_parked();
    ws.update_in(vcx, |ws, window, cx| {
        let mut original = Vec::new();
        for (i, text) in ["first draft", "second draft", "third draft"].into_iter().enumerate() {
            if i > 0 {
                ws.new_file(&NewFile, window, cx);
            }
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert(text)));
            original.push(ws.active_editor().unwrap());
        }
        let second = original[1].clone();
        ws.select_tab(&SelectTab { index: 1 }, window, cx);
        ws.add_tab_to_selection(&AddTabToSelection { forward: false }, window, cx);
        assert_eq!(ws.active_editor(), Some(original[0].clone()));
        assert_eq!(ws.views.shown(0), original[..2]);
        ws.focus_selected_tab(&FocusSelectedTab { forward: false }, window, cx);
        assert_eq!(ws.active_editor(), Some(second.clone()), "backward focus wraps");
        ws.deselect_tabs(&DeselectTabs, window, cx);
        assert_eq!(ws.views.shown(0), std::slice::from_ref(&second));
        ws.next_tab(&NextTab, window, cx);
        assert_eq!(ws.active_editor(), Some(original[2].clone()));
        ws.prev_tab(&PrevTab, window, cx);
        assert_eq!(ws.active_editor(), Some(second.clone()));
        ws.move_to_new_group(&MoveToNewGroup { axis: Axis::Rows }, window, cx);
        assert_eq!(ws.views.groups.len(), 2);
        assert_eq!(ws.views.find(&second).unwrap().0, 1);
        ws.focus_neighbour(&FocusNeighbour { forward: false }, window, cx);
        assert_eq!(ws.views.active, 0);
        ws.focus_neighbour(&FocusNeighbour { forward: true }, window, cx);
        assert_eq!(ws.active_editor(), Some(second.clone()));
        ws.move_to_neighbour(&MoveToNeighbour { forward: false }, window, cx);
        assert_eq!(ws.views.find(&second).unwrap().0, 0);
        ws.move_to_neighbour(&MoveToNeighbour { forward: true }, window, cx);
        assert_eq!(ws.views.find(&second).unwrap().0, 1);
        for group in [0, 1] {
            ws.move_to_group(&MoveToGroup { group }, window, cx);
            assert_eq!(ws.views.find(&second).unwrap().0, group);
        }
        ws.focus_group(&FocusGroup { group: 99 }, window, cx);
        ws.move_to_group(&MoveToGroup { group: 99 }, window, cx);
        assert_eq!(ws.views.active, 1, "invalid group commands leave the view in place");
        ws.toggle_pin(&TogglePin, window, cx);
        assert!(ws.views.is_pinned(&second));
        ws.toggle_pin(&TogglePin, window, cx);
        assert!(!ws.views.is_pinned(&second));
        assert_eq!(ws.views.tabs().count(), original.len());
        ws.open_playground(&OpenPlayground, window, cx);
        let query = match &ws.panel {
            Some(Panel::Playground(p)) => p.pattern.clone(),
            _ => panic!("playground"),
        };
        query.update(cx, |e, cx| e.set_text("draft", cx));
        for dock in [crate::app::session::Dock::Right, crate::app::session::Dock::Bottom] {
            ws.move_panel(&MovePanel { dock }, window, cx);
            assert_eq!(ws.panel_places.playground.dock, dock);
            assert_eq!(query.read(cx).text(cx), "draft");
        }
        assert_eq!(ws.views.tabs().count(), original.len() + 1, "Playground owns a sample tab");
        assert!(ws.active_editor().unwrap().read(cx).playground);
        for (e, expected) in original.iter().zip(["first draft", "second draft", "third draft"]) {
            assert!(ws.views.find(e).is_some());
            assert_eq!(e.read(cx).doc(cx).text(), expected);
            assert!(e.read(cx).doc(cx).dirty);
        }
    });
    assert!(!vcx.has_pending_prompt());
}

#[gpui::test]
fn explicit_tab_semicolon_and_pipe_delimiters_change_columns(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    for (delim, separator) in [(Delim::Tab, "\t"), (Delim::Semicolon, ";"), (Delim::Pipe, "|")] {
        let before = format!("a{separator}b\n1{separator}2\n");
        ws.update_in(vcx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert(&before);
                    s.set_cursor(2);
                })
            });
            ws.set_delimiter(&SetDelimiter { delim: Some(delim) }, window, cx);
        });
        vcx.dispatch_action(editor::ColumnCommand { op: crate::editing::csv::ColumnOp::MoveLeft });
        assert_eq!(text(&ws, vcx), format!("b{separator}a\n2{separator}1\n"));
        vcx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&ws, vcx), before);
    }
}

#[gpui::test]
fn font_ruler_and_wrap_choices_preserve_shared_text_and_view_local_wrap(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "shared text stays unchanged");
    ws.update_in(vcx, |ws, window, cx| {
        cx.global_mut::<Settings>().font_size = 14.;
        cx.global_mut::<Settings>().wrap_width = 88;
        ws.apply_view(cx);
        let first = ws.active_editor().unwrap();
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        let second = ws.active_editor().unwrap();
        ws.increase_font_size(&IncreaseFontSize, window, cx);
        assert_eq!(ws.view.font_size, 15.);
        ws.decrease_font_size(&DecreaseFontSize, window, cx);
        assert_eq!(ws.view.font_size, 14.);
        for _ in 0..100 {
            ws.decrease_font_size(&DecreaseFontSize, window, cx);
        }
        assert_eq!(ws.view.font_size, 6.);
        for _ in 0..100 {
            ws.increase_font_size(&IncreaseFontSize, window, cx);
        }
        assert_eq!(ws.view.font_size, 72.);
        cx.global_mut::<Settings>().font_size = 18.;
        ws.reset_font_size(&ResetFontSize, window, cx);
        assert_eq!(ws.view.font_size, 18., "Reset follows the latest settings");
        for col in [None, Some(70), Some(72), Some(78), Some(80), Some(100), Some(120)] {
            ws.set_ruler(&SetRuler { col }, window, cx);
            let expected: Vec<_> = col.into_iter().collect();
            for e in [&first, &second] {
                assert_eq!(e.read(cx).view.rulers, expected);
                assert_eq!(e.read(cx).view.font_size, 18.);
                assert_eq!(e.read(cx).doc(cx).text(), "shared text stays unchanged");
            }
        }
        first.update(cx, |e, _| e.wrap_override = Some(true));
        second.update(cx, |e, _| e.wrap_override = Some(true));
        for col in [None, Some(0), Some(70), Some(72), Some(78), Some(80), Some(100), Some(120)] {
            ws.set_wrap_column(&SetWrapColumn { col }, window, cx);
            assert_eq!(second.read(cx).wrap_cols(), col.unwrap_or(88));
            assert_eq!(first.read(cx).wrap_cols(), 88, "wrap choice belongs to its view");
        }
        ws.toggle_word_wrap(&ToggleWordWrap, window, cx);
        assert!(!second.read(cx).wraps());
        assert!(first.read(cx).wraps());
        ws.set_wrap_column(&SetWrapColumn { col: Some(80) }, window, cx);
        ws.toggle_word_wrap(&ToggleWordWrap, window, cx);
        assert!(second.read(cx).wraps());
        assert_eq!(second.read(cx).wrap_cols(), 80);
        assert_eq!(first.read(cx).doc(cx).text(), "shared text stays unchanged");
    });
}

#[gpui::test]
fn escape_closes_a_context_menu_before_the_editor_sees_it(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "one two");
    set_selections(&ws, cx, &[(0, 0), (4, 4)]);
    ws.update_in(cx, |ws, window, cx| {
        ws.focus_editor(window, cx);
        let items = vec![context_menu::item("Close", CloseTabTarget)];
        ws.open_context_menu(gpui::point(px(40.), px(40.)), items, cx);
    });
    cx.run_until_parked();
    // the editor binds Escape (to one selection); the open menu takes it first
    cx.simulate_keystrokes("escape");
    ws.read_with(cx, |ws, _| assert!(ws.context_menu.is_none()));
    assert_eq!(sels(&ws, cx).len(), 2);
    cx.simulate_keystrokes("escape");
    assert_eq!(sels(&ws, cx).len(), 1);
}

// ---- Edit > Code Folding ----

fn hidden(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Vec<std::ops::Range<usize>> {
    on_editor(ws, vcx, |e, _, cx| e.hidden_lines(cx))
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)]
fn the_code_folding_keys_and_the_edit_menu(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    // regions: "a" 0..=2 holding "b" 1..=2, "d" 3..=4; offset 11 is in "        c"
    type_in(&ws, vcx, "a\n    b\n        c\nd\n    e\n");
    set_selections(&ws, vcx, &[(11, 11)]);
    vcx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(hidden(&ws, vcx), [2..3]);
    vcx.simulate_keystrokes("cmd-alt-]");
    assert!(hidden(&ws, vcx).is_empty());
    vcx.simulate_keystrokes("cmd-k cmd-1");
    assert_eq!(hidden(&ws, vcx), [1..3, 4..5]);
    vcx.simulate_keystrokes("cmd-k cmd-2");
    assert_eq!(hidden(&ws, vcx), [2..3]);
    vcx.simulate_keystrokes("cmd-k cmd-0");
    assert!(hidden(&ws, vcx).is_empty());
    let edit = menu(&ws, vcx, "Edit");
    let at = names(&edit).iter().position(|n| n == "Mark >").expect("Edit > Mark");
    let Some(MenuItem::Submenu(folding)) = edit.get(at + 1) else {
        panic!("Edit > Code Folding after Mark");
    };
    assert_eq!(folding.name, "Code Folding");
    let mut want = ["Fold", "Unfold", "Unfold All", "-", "Fold All"].map(String::from).to_vec();
    want.extend((2..=9).map(|n| format!("Fold Level {n}")));
    assert_eq!(names(&folding.items), want);
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)]
fn find_into_a_fold_opens_it(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "fn a() {\n    needle();\n}\nfn b() {\n    needle();\n}\n");
    set_selections(&ws, vcx, &[(0, 0)]);
    vcx.simulate_keystrokes("cmd-k cmd-1");
    assert_eq!(hidden(&ws, vcx), [1..2, 4..5]);
    // the find as typed selects the first match, in the first fold
    vcx.simulate_keystrokes("cmd-f");
    vcx.simulate_input("needle");
    assert_eq!(hidden(&ws, vcx), [4..5]);
    // Find All selects the other one too
    ws.update_in(vcx, |ws, window, cx| ws.find_all(&FindAll, window, cx));
    vcx.run_until_parked();
    assert!(hidden(&ws, vcx).is_empty());
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)]
fn a_macro_types_where_its_keys_did_in_a_fold_a_command_opened(cx: &mut TestAppContext) {
    // the second cmd-d selects the "zebra" in the folded body, Right goes to its end and ";"
    // is typed there; played back with no frame between the commands, it is typed there too
    let (ws, vcx) = window(cx);
    let folded = |vcx: &mut VisualTestContext| {
        on_editor(&ws, vcx, |e, window, cx| {
            e.set_text("zebra\nfn a() {\n    zebra();\n}", cx);
            e.sel(cx).set_cursor(6);
            assert!(e.perform_action(&editor::Fold, window, cx));
            e.sel(cx).set_cursor(0);
        });
        vcx.run_until_parked();
        assert_eq!(hidden(&ws, vcx), [2..3]);
    };
    let typed = "zebra;\nfn a() {\n    zebra;();\n}";
    folded(vcx);
    vcx.simulate_keystrokes("ctrl-q cmd-d cmd-d right");
    vcx.simulate_input(";");
    vcx.simulate_keystrokes("ctrl-q");
    assert_eq!(text(&ws, vcx), typed, "typed, a frame after each key");
    folded(vcx);
    ws.update_in(vcx, |ws, window, cx| ws.replay_macro(&ReplayMacro, window, cx));
    assert_eq!(text(&ws, vcx), typed, "played back");
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)]
fn a_batch_types_where_its_keys_did_after_find_next_went_into_a_fold(cx: &mut TestAppContext) {
    // Find Next selects the "zebra" in the folded body, Right goes to its end and ";" is typed
    // there; run as one batch (an extension's commands), with no frame after Find Next, the
    // editor command after it finds the fold open, so ";" is typed there too
    let (ws, vcx) = window(cx);
    let folded = |vcx: &mut VisualTestContext| {
        ws.update(vcx, |ws, _| ws.last_query = "zebra".into());
        on_editor(&ws, vcx, |e, window, cx| {
            e.set_text("zebra\nfn a() {\n    zebra();\n}", cx);
            e.sel(cx).set_cursor(6);
            assert!(e.perform_action(&editor::Fold, window, cx));
            e.sel(cx).set_selection(0, 5);
        });
        vcx.run_until_parked();
        assert_eq!(hidden(&ws, vcx), [2..3]);
    };
    let typed = "zebra\nfn a() {\n    zebra;();\n}";
    folded(vcx);
    vcx.simulate_keystrokes("cmd-g right");
    vcx.simulate_input(";");
    assert_eq!(text(&ws, vcx), typed, "typed, a frame after each key");
    folded(vcx);
    let batch = [
        ("workspace::FindNext", serde_json::json!({})),
        ("editor::Move", serde_json::json!({"motion": "right", "select": false})),
        ("insert", serde_json::json!({"text": ";"})),
    ];
    ws.update_in(vcx, |ws, window, cx| {
        for (command, args) in batch {
            let command = crate::config::customization::Command { command: command.into(), args };
            ws.execute_command(&command, window, cx).unwrap();
        }
    });
    assert_eq!(text(&ws, vcx), typed, "run as a batch");
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)]
fn two_views_of_a_file_keep_their_own_folds(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    // "a" heads line 1, "c" (at 8) heads line 3
    type_in(&ws, vcx, "a\n    b\nc\n    d\n");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    for (view, at) in views.iter().zip([0, 8]) {
        view.update_in(vcx, |e, window, cx| {
            e.sel(cx).set_cursor(at);
            assert!(e.perform_action(&editor::Fold, window, cx));
        });
    }
    vcx.run_until_parked();
    let folds = |vcx: &mut VisualTestContext| -> Vec<Vec<std::ops::Range<usize>>> {
        (views.iter()).map(|v| v.read_with(vcx, |e, cx| e.hidden_lines(cx))).collect()
    };
    assert_eq!(folds(vcx), [vec![1..2], vec![3..4]]);
    // typed above both in one view: each view's fold moves with its text
    views[0].update(vcx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("x\n");
        })
    });
    vcx.run_until_parked();
    assert_eq!(folds(vcx), [vec![2..3], vec![4..5]]);
    views[1].update_in(vcx, |e, window, cx| {
        assert!(e.perform_action(&editor::UnfoldAll, window, cx));
    });
    vcx.run_until_parked();
    assert_eq!(folds(vcx), [vec![2..3], vec![]]);
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)]
fn a_click_on_the_palette_over_a_folds_marker_runs_the_command_and_keeps_the_fold(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    vcx.run_until_parked();
    let (view, text_left, space_w) = on_editor(&ws, vcx, |e, _, _| e.drawn_frame()).unwrap();
    // a header on line 3, long enough that its marker is 100 pt left of the middle, where the
    // palette's first row covers it
    let middle = view.left() + view.size.width / 2.;
    let n = ((middle - px(100.) - text_left) / space_w) as usize;
    let text = format!("a\nb\nc\n{}\n    body\nend\n", "h".repeat(n));
    type_in(&ws, vcx, &text);
    set_selections(&ws, vcx, &[(6, 6)]);
    vcx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(hidden(&ws, vcx), [4..5]);
    let marker = gpui::point(text_left + space_w * n as f32 + px(12.), view.top() + px(15. * 3.5));
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("Quick Find All");
    let first = panel_labels(&ws, vcx).unwrap().remove(0);
    assert_eq!(first, "Selection: Quick Find All");
    vcx.simulate_click(marker, gpui::Modifiers::none());
    assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()), "the row took the click");
    assert_eq!(hidden(&ws, vcx), [4..5], "the fold under it stays");
    // Quick Find All ran: the header's word, the caret's, selected
    assert_eq!(sels(&ws, vcx), [Selection::new(6, 6 + n)]);
}

#[gpui::test]
fn the_fold_button_settings_reach_the_views(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let buttons = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| {
        on_editor(ws, vcx, |e, _, _| (e.view.fold_buttons, e.view.fade_fold_buttons))
    };
    assert_eq!(buttons(&ws, vcx), (true, true));
    vcx.update(|_, cx| {
        cx.set_global(Settings {
            fold_buttons: false,
            fade_fold_buttons: false,
            lsp_enabled: false,
            ..Settings::default()
        })
    });
    vcx.run_until_parked();
    assert_eq!(buttons(&ws, vcx), (false, false));
}

#[gpui::test]
fn the_drag_text_setting_reaches_the_views(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    assert!(on_editor(&ws, vcx, |e, _, _| e.view.drag_text));
    vcx.update(|_, cx| cx.set_global(Settings { drag_text: false, lsp_enabled: false, ..Settings::default() }));
    vcx.run_until_parked();
    assert!(!on_editor(&ws, vcx, |e, _, _| e.view.drag_text));
}

/// The View menu's items, as `names` writes them.
fn view_items(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Vec<String> {
    names(&menu(ws, vcx, "View"))
}

fn white_space(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> settings::WhiteSpace {
    on_editor(ws, vcx, |e, _, _| e.view.white_space)
}

fn toggle_white_space(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    ws.update_in(vcx, |ws, window, cx| ws.toggle_white_space(&ToggleWhiteSpace, window, cx));
}

/// View > Show White Space as the menu words it.
fn white_space_item(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> String {
    let items = view_items(ws, vcx);
    let item = items.into_iter().find(|n| n.ends_with("Show White Space"));
    item.expect("View > Show White Space")
}

fn set_settings(vcx: &mut VisualTestContext, settings: Settings) {
    vcx.update(|_, cx| cx.set_global(settings));
    vcx.run_until_parked();
}

#[gpui::test]
fn show_white_space_toggles_this_window_between_all_and_the_setting(cx: &mut TestAppContext) {
    use settings::WhiteSpace;
    let (ws, vcx) = window(cx);
    // Sublime's default is the white space inside selections
    assert_eq!(white_space(&ws, vcx), WhiteSpace::SELECTION);
    assert_eq!(white_space_item(&ws, vcx), "Show White Space");
    toggle_white_space(&ws, vcx);
    assert_eq!(white_space(&ws, vcx), WhiteSpace::ALL);
    assert_eq!(white_space_item(&ws, vcx), "✓ Show White Space");
    toggle_white_space(&ws, vcx);
    assert_eq!(white_space(&ws, vcx), WhiteSpace::SELECTION, "the setting again");
    assert_eq!(white_space_item(&ws, vcx), "Show White Space");
    // a setting of "all" has the check on already: the toggle hides, then shows
    let setting = |draw_white_space| Settings { draw_white_space, lsp_enabled: false, ..Settings::default() };
    set_settings(vcx, setting(WhiteSpace::ALL));
    assert_eq!(white_space(&ws, vcx), WhiteSpace::ALL);
    assert_eq!(white_space_item(&ws, vcx), "✓ Show White Space");
    toggle_white_space(&ws, vcx);
    assert_eq!(white_space(&ws, vcx), WhiteSpace::NONE);
    assert_eq!(white_space_item(&ws, vcx), "Show White Space");
    toggle_white_space(&ws, vcx);
    assert_eq!(white_space(&ws, vcx), WhiteSpace::ALL);
    // a saved settings file takes the window's choice back
    toggle_white_space(&ws, vcx);
    assert_eq!(white_space(&ws, vcx), WhiteSpace::NONE);
    let trailing = WhiteSpace { trailing: true, ..WhiteSpace::NONE };
    set_settings(vcx, setting(trailing));
    assert_eq!(white_space(&ws, vcx), trailing);
    toggle_white_space(&ws, vcx);
    assert_eq!(white_space(&ws, vcx), WhiteSpace::ALL);
    set_settings(vcx, setting(WhiteSpace::SELECTION));
    assert_eq!(white_space(&ws, vcx), WhiteSpace::SELECTION);
}

#[gpui::test]
fn show_white_space_reaches_every_view_of_a_file_and_the_tabs_opened_after(cx: &mut TestAppContext) {
    use settings::WhiteSpace;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a  b\n");
    new_view(&ws, vcx);
    let drawn = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| -> Vec<WhiteSpace> {
        ws.read_with(vcx, |ws, cx| (ws.views.tabs()).map(|t| t.read(cx).view.white_space).collect())
    };
    assert_eq!(drawn(&ws, vcx), [WhiteSpace::SELECTION; 2]);
    toggle_white_space(&ws, vcx);
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    assert_eq!(drawn(&ws, vcx), [WhiteSpace::ALL; 3]);
    toggle_white_space(&ws, vcx);
    assert_eq!(drawn(&ws, vcx), [WhiteSpace::SELECTION; 3]);
}

#[gpui::test]
fn the_white_space_choice_is_saved_with_the_window(cx: &mut TestAppContext) {
    use settings::WhiteSpace;
    let (ws, vcx) = window(cx);
    assert_eq!(capture(&ws, vcx).white_space, None);
    toggle_white_space(&ws, vcx);
    let saved = capture(&ws, vcx);
    assert_eq!(saved.white_space, Some(true));
    // through the session file's JSON and into another window
    let json = serde_json::to_string(&saved).unwrap();
    let st: WindowState = serde_json::from_str(&json).unwrap();
    let (ws2, vcx2) = window(vcx);
    assert_eq!(white_space(&ws2, vcx2), WhiteSpace::SELECTION);
    ws2.update_in(vcx2, |ws, window, cx| ws.restore(st, window, cx));
    assert_eq!(white_space(&ws2, vcx2), WhiteSpace::ALL);
    assert_eq!(white_space_item(&ws2, vcx2), "✓ Show White Space");
    // a session from before it has none
    let old: WindowState = serde_json::from_str("{}").unwrap();
    assert_eq!(old.white_space, None);
}

// ---- Edit > Copy as HTML ----

use crate::editor::rich;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;

/// Keeps the highlighting thread busy until dropped: what is queued behind it waits.
struct Busy(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Busy {
    fn new() -> Self {
        use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
        let nap = || std::thread::sleep(std::time::Duration::from_millis(1));
        let (open, running) =
            (std::sync::Arc::new(AtomicBool::new(false)), std::sync::Arc::new(AtomicBool::new(false)));
        let (o, r) = (open.clone(), running.clone());
        drop(highlight::on_worker(move || {
            r.store(true, Relaxed);
            while !o.load(Relaxed) {
                nap();
            }
        }));
        while !running.load(Relaxed) {
            nap();
        }
        Self(open)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Wait for one Copy as HTML to reach the pasteboard: its HTML and plain text.
fn html_written(vcx: &mut VisualTestContext) -> (String, String) {
    let mut got = Vec::new();
    settle_until(vcx, |_| {
        got.extend(rich::take());
        !got.is_empty()
    });
    assert_eq!(got.len(), 1, "one write");
    got.remove(0)
}

fn copy_as_html(vcx: &mut VisualTestContext) -> (String, String) {
    rich::take();
    vcx.dispatch_action(editor::CopyAsHtml);
    html_written(vcx)
}

/// What is inside the `<pre>` of a copy.
fn pre_body(html: &str) -> &str {
    let pre = html.find("<pre ").expect("a <pre>");
    let open = pre + html[pre..].find('>').expect("its end") + 1;
    html[open..].strip_suffix("</pre>").expect("closed")
}

/// `html` without its tags: the text they hold, escaped as it is.
fn untagged(html: &str) -> String {
    let (mut out, mut inside) = (String::new(), false);
    for c in html.chars() {
        match (c, inside) {
            ('<', _) => inside = true,
            ('>', true) => inside = false,
            (c, false) => out.push(c),
            _ => {}
        }
    }
    out
}

fn status_text(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Option<String> {
    ws.read_with(vcx, |ws, _| ws.status.as_ref().map(|s| s.0.clone()))
}

#[gpui::test]
fn copy_as_html_takes_the_selections_joined_by_newlines_and_records_their_text(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "alpha beta\ngamma delta\n");
    set_selections(&ws, vcx, &[(0, 5), (7, 7), (11, 16)]);
    let version = on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version);
    let (html, plain) = copy_as_html(vcx);
    assert_eq!(plain, "alpha\ngamma");
    assert_eq!(pre_body(&html), "alpha\ngamma");
    // the default size, 12, as CSS pt that AppKit renders at 12
    assert!(html.contains(";font-size:9pt;tab-size:4;"), "{html}");
    // the caret between them adds nothing; the plain text is history's, not whole lines
    assert_eq!(history(vcx), [("alpha\ngamma".to_string(), false)]);
    assert_eq!(status_text(&ws, vcx).as_deref(), Some("Copied as HTML"));
    // not an edit: the text, the selections and the undo steps are as they were
    assert_eq!(text(&ws, vcx), "alpha beta\ngamma delta\n");
    assert_eq!(sels(&ws, vcx).len(), 3);
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version), version);
    // the pasteboard text is what Copy puts on the clipboard
    vcx.simulate_keystrokes("cmd-c");
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), plain);
}

#[gpui::test]
fn with_every_selection_empty_copy_as_html_takes_the_whole_document(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "one\n\n  two\t\n");
    set_selections(&ws, vcx, &[(1, 1), (6, 6)]);
    let (html, plain) = copy_as_html(vcx);
    assert_eq!(plain, "one\n\n  two\t\n");
    assert_eq!(pre_body(&html), "one\n\n  two\t\n");
    assert_eq!(history(vcx), [("one\n\n  two\t\n".to_string(), false)]);
}

#[gpui::test]
fn the_pasteboard_html_says_it_is_utf_8_and_carries_the_views_font_size_and_tabs(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "café\t中\n");
    on_editor(&ws, vcx, |e, _, cx| {
        e.view.font_face = "Fira Code".into();
        e.view.font_size = 14.;
        e.update_doc(cx, |d| d.tab_size = 8);
    });
    let (html, _) = copy_as_html(vcx);
    // AppKit's HTML importer reads UTF-8 with no charset as Windows-1252
    assert!(html.starts_with("<meta charset=\"utf-8\"><pre style=\"background-color:#"), "{html}");
    // 14 is 10.5 pt in CSS, which AppKit renders at 14
    assert!(
        html.contains(";font-family:'Fira Code',monospace;font-size:10.5pt;tab-size:8;white-space:pre\">"),
        "{html}"
    );
    assert_eq!(pre_body(&html), "café\t中\n");
}

#[gpui::test]
fn lines_not_coloured_yet_are_coloured_for_the_copy(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "fn main() { let s = \"a<b\"; }\n// done\n");
    // a highlighter that has not coloured a line yet
    on_editor(&ws, vcx, |e, _, _| {
        e.highlighter = Highlighter::new(Some(std::path::Path::new("x.rs")));
    });
    let (html, plain) = copy_as_html(vcx);
    assert_eq!(plain, "fn main() { let s = \"a<b\"; }\n// done\n");
    let body = pre_body(&html);
    // tokens have colours of their own, and the text inside them is escaped
    assert!(body.starts_with("<span style=\"color:#"), "{body}");
    assert_eq!(untagged(body), "fn main() { let s = &quot;a&lt;b&quot;; }\n// done\n");
    // the comment is one run in Mariana's comment colour
    let comment = "<span style=\"color:#a6acb9\">// done</span>\n";
    assert!(body.ends_with(comment), "{body}");
}

#[gpui::test]
fn a_line_over_the_colouring_limit_keeps_the_default_foreground(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let long = format!("let s = \"{}\";", "x".repeat(20_000));
    type_in(&ws, vcx, &format!("{long}\nfn f() {{}}\n"));
    on_editor(&ws, vcx, |e, _, _| {
        e.highlighter = Highlighter::new(Some(std::path::Path::new("x.rs")));
    });
    let (html, _) = copy_as_html(vcx);
    let mut lines = pre_body(&html).lines();
    let escaped = long.replace('"', "&quot;");
    assert_eq!(lines.next(), Some(escaped.as_str()), "no span at all");
    assert!(lines.next().unwrap().contains("<span "), "the next line is coloured");
}

#[gpui::test]
fn a_huge_document_is_coloured_and_built_off_the_ui_thread(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let line = "x".repeat(99_999);
    let text = format!("{line}\n").repeat(42);
    assert!(text.chars().count() > stats::BIG);
    set_state(&ws, vcx, &text);
    let busy = Busy::new();
    rich::take();
    vcx.dispatch_action(editor::CopyAsHtml);
    vcx.run_until_parked();
    // nothing was built here: the pasteboard waits for the highlighting thread
    assert!(rich::take().is_empty());
    assert_eq!(status_text(&ws, vcx).as_deref(), Some("Working…"));
    drop(busy);
    let (html, plain) = html_written(vcx);
    assert_eq!(plain, text);
    assert_eq!(pre_body(&html), text);
    assert_eq!(status_text(&ws, vcx).as_deref(), Some("Copied as HTML"));
    // 4 MB is over the history's cap: it is not kept
    assert!(history(vcx).is_empty());
}

#[gpui::test]
fn copy_as_html_refuses_more_than_16_mib_of_text_before_doing_any_work(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    // one byte over the limit: 16,384 lines of 1,023 characters, then one more
    let text = format!("{}\n", "x".repeat(1023)).repeat(16 * 1024) + "x";
    assert_eq!(text.len(), 16 * 1024 * 1024 + 1);
    set_state(&ws, vcx, &text);
    on_editor(&ws, vcx, |e, _, _| e.wrap_override = Some(false));
    let refused = |vcx: &mut VisualTestContext| {
        rich::take();
        vcx.dispatch_action(editor::CopyAsHtml);
        vcx.run_until_parked();
        // no job was started and nothing was written or recorded, only said
        assert!(!on_editor(&ws, vcx, |e, _, _| e.html_copying()));
        assert!(rich::take().is_empty());
        assert!(history(vcx).is_empty());
        assert!(vcx.read_from_clipboard().is_none());
        assert_eq!(status_text(&ws, vcx).as_deref(), Some("Copy as HTML is limited to 16 MB of text"));
    };
    // the whole document, every selection empty
    refused(vcx);
    // selections that add up to it, the line break joining them included
    let half = 8 * 1024 * 1024;
    set_selections(&ws, vcx, &[(0, half), (half + 1, 2 * half + 1)]);
    refused(vcx);
    // the limit is on the text to copy, not on the buffer
    set_selections(&ws, vcx, &[(0, 5)]);
    let (_, plain) = copy_as_html(vcx);
    assert_eq!(plain, "xxxxx");
    assert_eq!(status_text(&ws, vcx).as_deref(), Some("Copied as HTML"));
}

#[gpui::test]
fn a_newer_copy_as_html_replaces_one_still_being_built(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "first second\n");
    let busy = Busy::new();
    rich::take();
    for (from, to) in [(0, 5), (6, 12)] {
        set_selections(&ws, vcx, &[(from, to)]);
        vcx.dispatch_action(editor::CopyAsHtml);
    }
    drop(busy);
    let (_, plain) = html_written(vcx);
    assert_eq!(plain, "second");
    // the first one finished on the thread too, and was not written
    vcx.run_until_parked();
    assert!(rich::take().is_empty());
    assert_eq!(history(vcx), [("second".to_string(), false)]);
}

/// Makes the active view's copies count the lines they ask colours for, and hold the first one
/// until the guard is dropped (so a failing test lets the thread go too): (lines asked for, a
/// copy is at its first line, the guard).
fn hold_the_colouring(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
) -> (Arc<AtomicUsize>, Arc<AtomicBool>, Busy) {
    use std::sync::atomic::Ordering::Relaxed;
    let (asked, at_first, go) =
        (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let (a, f, g) = (asked.clone(), at_first.clone(), go.clone());
    on_editor(ws, vcx, |e, _, _| {
        e.highlighter.probe = Some(Arc::new(move |_| {
            a.fetch_add(1, Relaxed);
            f.store(true, Relaxed);
            while !g.load(Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }));
    });
    (asked, at_first, Busy(go))
}

#[gpui::test]
fn a_copy_replaced_by_a_newer_one_stops_at_its_next_line(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, vcx) = window(cx);
    set_state(&ws, vcx, &"x\n".repeat(5_000));
    let (asked, at_first, held) = hold_the_colouring(&ws, vcx);
    rich::take();
    // the whole document: 5,001 lines, the thread held at the first
    vcx.dispatch_action(editor::CopyAsHtml);
    settle_until(vcx, |_| at_first.load(Relaxed));
    // a newer copy of one line replaces it
    set_selections(&ws, vcx, &[(0, 1)]);
    vcx.dispatch_action(editor::CopyAsHtml);
    drop(held);
    let (_, plain) = html_written(vcx);
    assert_eq!(plain, "x", "the newer copy's");
    // the first asked for one line before it was told, the second for its one
    assert_eq!(asked.load(Relaxed), 2);
}

#[gpui::test]
fn a_copy_whose_view_closes_stops_at_its_next_line_and_writes_nothing(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, vcx) = window(cx);
    set_state(&ws, vcx, &"x\n".repeat(5_000));
    let (asked, at_first, held) = hold_the_colouring(&ws, vcx);
    rich::take();
    vcx.dispatch_action(editor::CopyAsHtml);
    settle_until(vcx, |_| at_first.load(Relaxed));
    // the view is clean, so closing it asks nothing
    ws.update_in(vcx, |ws, window, cx| ws.close_tab(&CloseTab, window, cx));
    vcx.run_until_parked();
    drop(held);
    // the thread is free again once the copy has ended
    let freed = Arc::new(AtomicBool::new(false));
    let done = freed.clone();
    drop(highlight::on_worker_urgent(move || done.store(true, Relaxed)));
    settle_until(vcx, |_| freed.load(Relaxed));
    vcx.run_until_parked();
    assert_eq!(asked.load(Relaxed), 1);
    assert!(rich::take().is_empty());
}

#[gpui::test]
fn a_pasteboard_that_refuses_still_gets_the_plain_text(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "plain old text\n");
    set_selections(&ws, vcx, &[(6, 9)]);
    rich::refuse(true);
    vcx.dispatch_action(editor::CopyAsHtml);
    settle_until(vcx, |cx| cx.read_from_clipboard().is_some());
    rich::refuse(false);
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "old");
    assert_eq!(history(vcx), [("old".to_string(), false)]);
    assert_eq!(status_text(&ws, vcx).as_deref(), Some("Copied the text; the pasteboard took no HTML"));
}

#[gpui::test]
fn edit_copy_as_html_follows_copy_in_the_menu(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let names = names(&menu(&ws, vcx, "Edit"));
    let at = names.iter().position(|n| n == "Copy").expect("Edit > Copy");
    assert_eq!(names[at + 1], "Copy as HTML");
}

/// `window`, made the active window: a completion list closes in one that is not.
fn active_window(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (ws, vcx) = window(cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    (ws, vcx)
}

/// The completion list in the active view as shown: its labels and the selected row.
fn completions(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Option<(Vec<String>, usize)> {
    on_editor(ws, vcx, |e, _, _| e.completion_list())
}

fn labels(l: &[&str]) -> Vec<String> {
    l.iter().map(|l| l.to_string()).collect()
}

#[gpui::test]
fn other_buffers_of_the_syntax_offer_their_words_read_once_a_version(cx: &mut TestAppContext) {
    let (ws, vcx) = active_window(cx);
    type_in(&ws, vcx, "zebra_one\n");
    let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    // a Rust buffer's words are not offered in plain text
    ws.update_in(vcx, |ws, window, cx| {
        ws.new_file(&NewFile, window, cx);
        ws.set_syntax(&syntax("Rust"), window, cx);
    });
    type_in(&ws, vcx, "zebra_rust\n");
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, vcx, "zebra_two\n");
    vcx.simulate_input("ze");
    let shown = completions(&ws, vcx).map(|l| l.0);
    assert_eq!(shown, Some(labels(&["zebra_one", "zebra_two"])));
    // the first buffer's words were read for its version: the next list takes them from its view
    let read = first.read_with(vcx, |e, _| e.words_read()).expect("kept");
    vcx.simulate_input(" ze");
    assert!(completions(&ws, vcx).is_some());
    let again = first.read_with(vcx, |e, _| e.words_read()).unwrap();
    assert!(Arc::ptr_eq(&read, &again), "not read again");
    // edited, it is read again
    first.update(vcx, |e, cx| e.edit(cx, |s| s.insert("zebra_three ")));
    vcx.simulate_input(" ze");
    let shown = completions(&ws, vcx).map(|l| l.0);
    assert_eq!(shown, Some(labels(&["zebra_one", "zebra_two", "zebra_three"])));
    let edited = first.read_with(vcx, |e, _| e.words_read()).unwrap();
    assert!(!Arc::ptr_eq(&read, &edited));
}

#[gpui::test]
fn with_the_list_open_its_keys_come_before_the_editors(cx: &mut TestAppContext) {
    let (ws, vcx) = active_window(cx);
    type_in(&ws, vcx, "alpha\nalpine\n");
    vcx.simulate_input("al");
    assert_eq!(completions(&ws, vcx), Some((labels(&["alpine", "alpha"]), 0)));
    vcx.simulate_keystrokes("down");
    assert_eq!(completions(&ws, vcx).unwrap().1, 1);
    vcx.simulate_keystrokes("up");
    assert_eq!(completions(&ws, vcx).unwrap().1, 0);
    vcx.simulate_keystrokes("down enter");
    assert_eq!(text(&ws, vcx), "alpha\nalpine\nalpha", "Enter took the row, no line break");
    vcx.simulate_keystrokes("enter");
    vcx.simulate_input("al");
    vcx.simulate_keystrokes("tab");
    assert_eq!(text(&ws, vcx), "alpha\nalpine\nalpha\nalpha", "Tab took it");
    // Escape closes the list and leaves the carets; with it closed, it leaves one
    vcx.simulate_keystrokes("enter");
    let n = text(&ws, vcx).chars().count();
    set_selections(&ws, vcx, &[(0, 0), (n, n)]);
    vcx.simulate_input("al");
    assert!(completions(&ws, vcx).is_some());
    vcx.simulate_keystrokes("escape");
    assert_eq!(completions(&ws, vcx), None);
    assert_eq!(sels(&ws, vcx).len(), 2);
    vcx.simulate_keystrokes("escape");
    assert_eq!(sels(&ws, vcx).len(), 1);
    // Shift-Tab closes the list, then unindents
    type_in(&ws, vcx, "\n    ");
    vcx.simulate_input("al");
    assert!(completions(&ws, vcx).is_some());
    vcx.simulate_keystrokes("shift-tab");
    assert_eq!(completions(&ws, vcx), None);
    assert!(text(&ws, vcx).ends_with("\nal"), "{:?}", text(&ws, vcx));
}

#[gpui::test]
fn ctrl_space_opens_the_list_at_once_and_closed_its_keys_are_the_editors(cx: &mut TestAppContext) {
    let (ws, vcx) = active_window(cx);
    type_in(&ws, vcx, "alpha\nalpine\n");
    // an empty prefix: everything on offer
    vcx.simulate_keystrokes("ctrl-space");
    assert_eq!(completions(&ws, vcx), Some((labels(&["alpine", "alpha"]), 0)));
    vcx.simulate_keystrokes("escape");
    // closed: Tab indents, Up moves the caret and Enter breaks the line
    vcx.simulate_keystrokes("tab");
    assert!(text(&ws, vcx).ends_with("\n    "), "{:?}", text(&ws, vcx));
    vcx.simulate_keystrokes("up");
    assert_eq!(sels(&ws, vcx)[0].head, 10, "column 4 of the line above");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "alpha\nalpi\nne\n    ");
}

#[gpui::test]
fn alt_escape_opens_the_list_too_for_when_macos_takes_ctrl_space(cx: &mut TestAppContext) {
    let (ws, vcx) = active_window(cx);
    type_in(&ws, vcx, "alpha\nalpine\n");
    vcx.simulate_keystrokes("alt-escape");
    assert_eq!(completions(&ws, vcx), Some((labels(&["alpine", "alpha"]), 0)));
    // the list's own escape closes it
    vcx.simulate_keystrokes("escape");
    assert_eq!(completions(&ws, vcx), None);
}

#[gpui::test]
fn an_edit_from_another_view_closes_the_list(cx: &mut TestAppContext) {
    let (ws, vcx) = active_window(cx);
    type_in(&ws, vcx, "alpha\n");
    new_view(&ws, vcx);
    vcx.simulate_input("al");
    assert!(completions(&ws, vcx).is_some());
    let other = ws.read_with(vcx, |ws, _| ws.views.tabs().next().cloned().unwrap());
    other.update(vcx, |e, cx| e.edit(cx, |s| s.insert("x")));
    vcx.run_until_parked();
    assert_eq!(completions(&ws, vcx), None);
    assert_eq!(text(&ws, vcx), "alpha\nxal");
}

#[gpui::test]
fn a_list_left_open_in_another_tab_is_gone_when_the_tab_comes_back(cx: &mut TestAppContext) {
    let (ws, vcx) = active_window(cx);
    type_in(&ws, vcx, "alpha\n");
    vcx.simulate_input("al");
    assert!(completions(&ws, vcx).is_some());
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("ctrl-shift-tab");
    assert_eq!(text(&ws, vcx), "alpha\nal", "the first tab again");
    assert_eq!(completions(&ws, vcx), None);
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "alpha\nal\n", "Enter breaks the line");
}

#[gpui::test]
fn the_auto_complete_settings_reach_the_views(cx: &mut TestAppContext) {
    let (ws, vcx) = active_window(cx);
    type_in(&ws, vcx, "alpha alpine\n");
    vcx.update(|_, cx| cx.set_global(Settings { auto_complete: false, lsp_enabled: false, ..Settings::default() }));
    vcx.run_until_parked();
    vcx.simulate_input("alp");
    assert_eq!(completions(&ws, vcx), None, "typing does not open it");
    vcx.simulate_keystrokes("ctrl-space");
    assert_eq!(completions(&ws, vcx).map(|l| l.0), Some(labels(&["alpha", "alpine"])));
    vcx.simulate_keystrokes("escape");
    vcx.update(|_, cx| {
        cx.set_global(Settings { auto_complete_min_chars: 4, lsp_enabled: false, ..Settings::default() })
    });
    vcx.run_until_parked();
    vcx.simulate_input(" alp");
    assert_eq!(completions(&ws, vcx), None, "three letters are not four");
    vcx.simulate_input("h");
    assert_eq!(completions(&ws, vcx).map(|l| l.0), Some(labels(&["alpha"])));
}

#[gpui::test]
fn edit_show_completions_follows_wrap_in_the_menu(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let names = names(&menu(&ws, vcx, "Edit"));
    let at = names.iter().position(|n| n == "Wrap >").expect("Edit > Wrap");
    assert_eq!(names[at + 1], "Show Completions");
}

// ---- snippets in the window ----

/// A snippet file in this test's Snippets folder.
fn write_snippet(name: &str, content: &str, trigger: &str, scope: &str, description: &str) {
    let dir = crate::editing::snippet::dir();
    std::fs::create_dir_all(&dir).unwrap();
    let xml = format!(
        "<snippet><content><![CDATA[{content}]]></content><tabTrigger>{trigger}</tabTrigger>\
         <scope>{scope}</scope><description>{description}</description></snippet>"
    );
    std::fs::write(dir.join(format!("{name}.sublime-snippet")), xml).unwrap();
}

/// The active view's selected texts, and whether a snippet session is on there.
fn snippet_fields(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> (Vec<String>, bool) {
    on_editor(ws, vcx, |e, _, cx| {
        let s = e.read_state(cx);
        let sel = (s.view.selections.iter()).map(|x| s.doc.rope.slice(x.range()).to_string());
        (sel.collect(), s.in_snippet())
    })
}

#[gpui::test]
fn insert_snippet_lists_the_carets_scopes_snippets_and_expands_the_one_chosen(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_snippet("bold", "<b>${1:$SELECTION}</b>$0", "", "", "Bold");
    write_snippet("fn", "fn ${1:f}() {}", "fn", "source.rust", "Function");
    // an untitled tab is plain text: the Rust snippet is not for it
    type_in(&ws, vcx, "one two");
    set_selections(&ws, vcx, &[(0, 3), (4, 7)]);
    vcx.dispatch_action(InsertSnippet);
    assert_eq!(pipeline_rows(&ws, vcx), ["Bold"]);
    choose_pipeline(vcx, "Bold");
    assert_eq!(text(&ws, vcx), "<b>one</b> <b>two</b>");
    assert_eq!(snippet_fields(&ws, vcx), (vec!["one".into(), "two".into()], true));
    undo_redo(&ws, vcx, "one two", "<b>one</b> <b>two</b>");
    // nothing for the scope: said, nothing changed
    std::fs::remove_file(crate::editing::snippet::dir().join("bold.sublime-snippet")).unwrap();
    let store = crate::editing::snippet::Snippets::new(crate::editing::snippet::dir());
    vcx.update(|_, cx| cx.set_global(editor::SnippetStore(store)));
    vcx.dispatch_action(InsertSnippet);
    let (message, detail) = vcx.pending_prompt().expect("an alert");
    assert_eq!(message, "No snippets here");
    assert!(detail.contains("New Snippet"), "{detail}");
}

#[gpui::test]
fn a_macro_replays_a_snippet_chosen_in_insert_snippet(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_snippet("bold", "<b>${1:$SELECTION}</b>$0", "", "", "Bold");
    vcx.simulate_keystrokes("ctrl-q");
    vcx.dispatch_action(InsertSnippet);
    choose_pipeline(vcx, "Bold");
    vcx.simulate_input("x");
    vcx.simulate_keystrokes("ctrl-q");
    assert_eq!(text(&ws, vcx), "<b>x</b>");
    ws.update_in(vcx, |ws, window, cx| {
        ws.new_file(&NewFile, window, cx);
        ws.replay_macro(&ReplayMacro, window, cx)
    });
    assert_eq!(vcx.pending_prompt(), None);
    assert_eq!(text(&ws, vcx), "<b>x</b>");
}

#[gpui::test]
fn keymap_entries_for_expand_snippet_and_completion_close_load_and_the_snippet_one_runs(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_snippet("bold", "<b>${1:x}</b>$0", "", "", "Bold");
    let path = crate::editing::snippet::dir().join("bold.sublime-snippet");
    let keymap = serde_json::json!([
        {"keys":["cmd-j"],"command":"editor::ExpandSnippet",
         "args":{"path":path,"trigger":""},"context":"Editor"},
        {"keys":["cmd-alt-j"],"command":"editor::CompletionClose",
         "args":{"fall_through":false},"context":"Editor"},
    ]);
    std::fs::write(crate::config::customization::keymap_path(), keymap.to_string()).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    assert_eq!(vcx.pending_prompt(), None, "both entries load");
    vcx.simulate_keystrokes("cmd-j");
    assert_eq!(text(&ws, vcx), "<b>x</b>");
    assert_eq!(snippet_fields(&ws, vcx), (vec!["x".into()], true));
}

#[gpui::test]
fn several_snippets_for_the_trigger_are_chosen_in_the_quick_panel(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_snippet("a", "first(${1:x})", "go", "", "Go first");
    write_snippet("b", "second(${1:x})", "go", "", "Go second");
    type_in(&ws, vcx, "go");
    vcx.simulate_keystrokes("tab");
    assert_eq!(pipeline_rows(&ws, vcx), ["Go first", "Go second"]);
    assert_eq!(text(&ws, vcx), "go", "nothing yet");
    choose_pipeline(vcx, "second");
    assert_eq!(text(&ws, vcx), "second(x)");
    assert_eq!(snippet_fields(&ws, vcx), (vec!["x".into()], true));
    undo_redo(&ws, vcx, "go", "second(x)");
}

#[gpui::test]
fn new_snippet_opens_the_template_as_xml_and_a_name_taken_opens_that_snippet(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let new_snippet = |vcx: &mut VisualTestContext, name: &str| {
        vcx.dispatch_action(NewSnippet);
        vcx.simulate_input(name);
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
    };
    new_snippet(vcx, "hello");
    let path = crate::editing::text::resolve(&crate::editing::snippet::dir().join("hello.sublime-snippet"));
    assert_eq!(active_path(&ws, vcx), Some(path.clone()));
    // the template is a buffer, written by its first save, so closing it asks nothing
    assert_eq!(
        tab_list(&ws, vcx),
        [("hello.sublime-snippet".to_string(), crate::editing::snippet::template().to_string(), false)]
    );
    assert!(!path.exists());
    let syntax = on_editor(&ws, vcx, |e, _, _| e.highlighter.syntax_name().to_string());
    assert_eq!(syntax, "XML");
    // a name that is not a file name is said, and nothing opens
    vcx.dispatch_action(NewSnippet);
    vcx.simulate_input("a/b");
    vcx.simulate_keystrokes("enter");
    let error = form_error(&ws, vcx).flatten().expect("the panel stays");
    assert!(error.contains("/"), "{error}");
    vcx.update(|window, cx| window.dispatch_action(Box::new(ClosePanel), cx));
    // a name already taken opens that snippet
    write_snippet("kept", "kept here", "k", "", "");
    new_snippet(vcx, "kept.sublime-snippet");
    let kept = crate::editing::text::resolve(&crate::editing::snippet::dir().join("kept.sublime-snippet"));
    assert_eq!(active_path(&ws, vcx), Some(kept));
    assert!(text(&ws, vcx).contains("kept here"));
}

#[gpui::test]
fn another_view_follows_an_expansion_and_its_mirrors_and_its_edit_ends_the_session(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_snippet("let", "let ${1:name} = ${1/(.*)/\\U$1/};", "let", "", "");
    type_in(&ws, vcx, "let");
    new_view(&ws, vcx);
    let texts = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| -> Vec<String> {
        ws.read_with(vcx, |ws, cx| ws.views.tabs().map(|t| t.read(cx).doc(cx).text()).collect())
    };
    on_editor(&ws, vcx, |e, window, cx| e.perform_action(&editor::Tab, window, cx));
    assert_eq!(texts(&ws, vcx), ["let name = NAME;", "let name = NAME;"]);
    vcx.simulate_input("ab");
    assert_eq!(texts(&ws, vcx), ["let ab = AB;", "let ab = AB;"]);
    assert_eq!(snippet_fields(&ws, vcx), (vec!["".into()], true));
    // an edit in the first view lies in none of the second's fields, so that session ends
    ws.update(vcx, |ws, cx| {
        let first = ws.views.tabs().next().unwrap();
        first.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(s.len());
                s.insert("!")
            })
        });
    });
    assert_eq!(texts(&ws, vcx), ["let ab = AB;!", "let ab = AB;!"]);
    assert!(!snippet_fields(&ws, vcx).1);
}

#[gpui::test]
fn tools_snippets_has_insert_and_new_snippet(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let tools = names(&menu(&ws, vcx, "Tools"));
    assert_eq!(tools[..2], ["Command Palette…", "Snippets >"]);
    assert_eq!(names(&submenu(&ws, vcx, "Tools", "Snippets")), ["Insert Snippet…", "New Snippet…"]);
}

struct SpellProbe {
    calls: AtomicUsize,
    entered: AtomicBool,
    released: Arc<AtomicBool>,
}
impl crate::languages::spell::Checker for SpellProbe {
    fn misspelled(&self, text: &str, _stop: &AtomicBool) -> Vec<crate::languages::spell::Misspelling> {
        use std::sync::atomic::Ordering::Relaxed;
        assert!(crate::languages::spell::is_owner_thread());
        self.calls.fetch_add(1, Relaxed);
        self.entered.store(true, Relaxed);
        while !self.released.load(Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        text.match_indices("mispelled")
            .map(|(b, w)| crate::languages::spell::Misspelling { range: b..b + w.len(), source: b..b + w.len() })
            .collect()
    }
}
fn spell_probe(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, released: bool) -> Arc<SpellProbe> {
    let probe = Arc::new(SpellProbe {
        calls: AtomicUsize::new(0),
        entered: AtomicBool::new(false),
        released: Arc::new(AtomicBool::new(released)),
    });
    on_editor(ws, vcx, |e, _, _| e.spell_checker = Some(probe.clone()));
    probe
}

#[gpui::test]
fn keystrokes_do_not_wait_for_checker_and_stale_completion_is_dropped(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "mispelled");
    let probe = spell_probe(&ws, vcx, false);
    let held = Busy(probe.released.clone());
    vcx.simulate_keystrokes("f6");
    assert!(on_editor(&ws, vcx, |e, _, _| e.spell_check()));
    settle_until(vcx, |_| probe.entered.load(Relaxed));
    vcx.simulate_keystrokes("x");
    assert!(text(&ws, vcx).contains('x'));
    assert!(on_editor(&ws, vcx, |e, _, _| e.spelling_ranges(0)).is_empty());
    // F6 off while the stale check runs: its result draws no underline
    vcx.simulate_keystrokes("f6");
    drop(held);
    settle(vcx);
    assert!(on_editor(&ws, vcx, |e, _, _| e.spelling_drawn()).is_empty());
    assert!(!on_editor(&ws, vcx, |e, _, _| e.spell_check()));
}

#[gpui::test]
fn visible_waves_are_cached_and_toggle_stays_per_shared_view(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "😀 café mispelled");
    let probe = spell_probe(&ws, vcx, true);
    let action = (submenu(&ws, vcx, "View", "Spelling").into_iter())
        .find_map(|item| match item {
            gpui::MenuItem::Action { name, action, .. } if name == "Spell Check" => Some(action),
            _ => None,
        })
        .expect("View > Spelling > Spell Check");
    assert!(action.as_any().is::<editor::ToggleSpellCheck>());
    vcx.update(|_, cx| {
        let bindings = crate::config::customization::bindings(
            r#"[{"keys":["cmd-shift-9"],"command":"editor::ToggleSpellCheck"}]"#,
            cx,
        )
        .unwrap();
        assert_eq!(bindings.len(), 1);
    });
    vcx.simulate_keystrokes("f6");
    settle_until(vcx, |cx| !on_editor(&ws, cx, |e, _, _| e.spelling_drawn()).is_empty());
    assert_eq!(on_editor(&ws, vcx, |e, _, _| e.spelling_drawn()), std::slice::from_ref(&(7..16)));
    let count = probe.calls.load(Relaxed);
    on_editor(&ws, vcx, |_, _, cx| cx.notify());
    vcx.run_until_parked();
    assert_eq!(probe.calls.load(Relaxed), count, "an unchanged frame uses its cache");
    let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    new_view(&ws, vcx);
    vcx.simulate_keystrokes("f6");
    assert!(!on_editor(&ws, vcx, |e, _, _| e.spell_check()));
    assert!(first.read_with(vcx, |e, _| e.spell_check()));
    vcx.simulate_keystrokes("x");
    vcx.run_until_parked();
    assert_eq!(first.read_with(vcx, |e, cx| e.doc(cx).text()), text(&ws, vcx));
    assert!(
        first.read_with(vcx, |e, _| e.spelling_ranges(0)).is_empty(),
        "sibling edits invalidate the old view's ranges"
    );
}

#[gpui::test]
fn settings_selector_and_syntax_changes_refresh_visible_spelling(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "let mispelled = 1; // mispelled");
    let probe = spell_probe(&ws, vcx, true);
    ws.update_in(vcx, |ws, window, cx| {
        ws.apply_settings(r#"{"spell_check":true}"#, window, cx);
    });
    settle_until(vcx, |cx| on_editor(&ws, cx, |e, _, _| e.spelling_ranges(0)).len() == 2);
    ws.update_in(vcx, |ws, window, cx| ws.set_syntax(&syntax("Rust"), window, cx));
    settle_until(vcx, |cx| on_editor(&ws, cx, |e, _, _| e.spelling_ranges(0)) == std::slice::from_ref(&(22..31)));
    ws.update_in(vcx, |ws, window, cx| {
        ws.set_syntax(&syntax("Rust"), window, cx);
        let e = ws.active_editor().unwrap();
        assert!(e.read(cx).spelling_ranges(0).is_empty());
    });
    settle_until(vcx, |cx| on_editor(&ws, cx, |e, _, _| e.spelling_ranges(0)) == std::slice::from_ref(&(22..31)));
    let count = probe.calls.load(Relaxed);
    on_editor(&ws, vcx, |e, _, cx| {
        e.view.spelling_selector = "keyword".into();
        cx.notify();
    });
    settle_until(vcx, |cx| {
        probe.calls.load(Relaxed) > count && on_editor(&ws, cx, |e, _, _| e.spelling_drawn()).is_empty()
    });
}

// the dictionary and the blocked probes are the whole process's: these tests take turns
static SPELL_ACTION_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[gpui::test]
fn navigation_checks_distant_lines_wraps_and_skips_selected_word(cx: &mut TestAppContext) {
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    let (ws, vcx) = window(cx);
    let document = format!("mispelled\n{}😀 café mispelled\n", "good\n".repeat(5_000));
    type_in(&ws, vcx, &document);
    spell_probe(&ws, vcx, true);
    on_editor(&ws, vcx, |e, _, cx| e.select_range(0..9, cx));
    vcx.simulate_keystrokes("ctrl-f6");
    settle_until(vcx, |cx| on_editor(&ws, cx, |e, _, _| e.state.cursor()) > 20_000);
    let distant = on_editor(&ws, vcx, |e, _, _| e.state.selections[e.state.primary()].range());
    assert_eq!(slice(&ws, vcx, distant.clone()), "mispelled");
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).point(distant.start)), (5_001, 7));
    vcx.simulate_keystrokes("ctrl-f6");
    settle_until(vcx, |cx| first_range(&ws, cx) == (0..9));
    vcx.simulate_keystrokes("ctrl-shift-f6");
    settle_until(vcx, |cx| first_range(&ws, cx) == distant);
    vcx.simulate_keystrokes("ctrl-shift-f6");
    settle_until(vcx, |cx| first_range(&ws, cx) == (0..9));
    on_editor(&ws, vcx, |e, _, cx| e.select_range(0..0, cx));
    vcx.simulate_keystrokes("ctrl-f6");
    settle_until(vcx, |cx| first_range(&ws, cx) == (0..9));
    on_editor(&ws, vcx, |e, _, cx| e.select_range(2..2, cx));
    vcx.simulate_keystrokes("ctrl-f6");
    settle_until(vcx, |cx| first_range(&ws, cx) == distant);
    on_editor(&ws, vcx, |e, _, cx| e.select_range(distant.start + 2..distant.start + 2, cx));
    vcx.simulate_keystrokes("ctrl-shift-f6");
    settle_until(vcx, |cx| first_range(&ws, cx) == (0..9));
    on_editor(&ws, vcx, |e, _, cx| e.set_text("correct prose", cx));
    vcx.simulate_keystrokes("ctrl-f6");
    settle_until(vcx, |cx| left_text(&ws, cx) == "No misspellings");
}

/// The active view's first selection.
fn first_range(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> std::ops::Range<usize> {
    on_editor(ws, cx, |e, _, _| e.state.selections[0].range())
}

/// The active view's text in `range`.
fn slice(ws: &Entity<Workspace>, cx: &mut VisualTestContext, range: std::ops::Range<usize>) -> String {
    on_editor(ws, cx, |e, _, cx| e.doc(cx).rope.slice(range).to_string())
}

struct DictionaryProbe {
    learned: std::sync::Mutex<std::collections::HashSet<String>>,
    changes: std::sync::Mutex<Vec<(String, String)>>,
    entered: AtomicBool,
    released: Arc<AtomicBool>,
}
impl crate::languages::spell::Checker for DictionaryProbe {
    fn misspelled(&self, text: &str, _stop: &AtomicBool) -> Vec<crate::languages::spell::Misspelling> {
        assert!(crate::languages::spell::is_owner_thread());
        use unicode_segmentation::UnicodeSegmentation;
        let learned = self.learned.lock().unwrap();
        text.unicode_word_indices()
            .filter(|(_, w)| w.starts_with("qzx") && !learned.contains(*w))
            .map(|(b, w)| crate::languages::spell::Misspelling {
                range: b..b + w.trim_end_matches("'s").len(),
                source: b..b + w.len(),
            })
            .collect()
    }
    fn word_at(&self, text: &str, at: usize, stop: &AtomicBool) -> Option<std::ops::Range<usize>> {
        use unicode_segmentation::UnicodeSegmentation;
        assert!(crate::languages::spell::is_owner_thread());
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        text.unicode_word_indices().map(|(b, w)| b..b + w.len()).find(|r| r.start <= at && at <= r.end)
    }
    fn guesses(&self, _: &str) -> Vec<String> {
        use std::sync::atomic::Ordering::Relaxed;
        assert!(crate::languages::spell::is_owner_thread());
        self.entered.store(true, Relaxed);
        while !self.released.load(Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        vec!["corrected".into()]
    }
    fn has_learned_word(&self, word: &str) -> bool {
        self.learned.lock().unwrap().contains(word)
    }
    fn learn_word(&self, word: &str) {
        self.record("Learn Word", word);
        self.learned.lock().unwrap().insert(word.into());
    }
    fn unlearn_word(&self, word: &str) {
        self.record("Unlearn Word", word);
        self.learned.lock().unwrap().remove(word);
    }
    fn ignore_word(&self, word: &str) {
        self.record("Ignore Word", word);
    }
}
impl DictionaryProbe {
    /// A dictionary change `action` made to `word`, on the checker's own thread.
    fn record(&self, action: &str, word: &str) {
        assert!(crate::languages::spell::is_owner_thread());
        let change = (action.into(), word.into());
        self.changes.lock().unwrap().push(change);
    }
}
fn dictionary_probe(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Arc<DictionaryProbe> {
    let probe = Arc::new(DictionaryProbe {
        learned: Default::default(),
        changes: Default::default(),
        entered: AtomicBool::new(false),
        released: Arc::new(AtomicBool::new(true)),
    });
    on_editor(ws, cx, |e, _, _| e.spell_checker = Some(probe.clone()));
    probe
}
fn suggestions(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    on_editor(ws, cx, |e, w, cx| {
        assert!(e.perform_action(&editor::SpellingSuggestions, w, cx));
    });
    settle_until(cx, |cx| {
        ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.loaded && !p.items.is_empty()))
    });
}
fn spelling_choice(ws: &Entity<Workspace>, cx: &mut VisualTestContext, label: &str) {
    ws.update_in(cx, |ws, w, cx| {
        let p = ws.quick_panel.as_mut().unwrap();
        p.selected = p.filtered.iter().position(|i| p.items[*i].label == label).expect(label);
        ws.confirm_quick_panel(w, cx);
    });
}

#[gpui::test]
fn accepted_dictionary_changes_survive_edits_caret_requests_and_each_other(cx: &mut TestAppContext) {
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, other_ws, mut main, mut other) = two_windows(cx);
    let probe = dictionary_probe(&ws, &mut main);
    on_editor(&other_ws, &mut other, |e, _, _| e.spell_checker = Some(probe.clone()));
    for (action, change, word) in [
        ("Ignore Word", "edit", "qzxacceptedignore"),
        ("Learn Word", "caret", "qzxacceptedlearn"),
        ("Unlearn Word", "request", "qzxacceptedunlearn"),
    ] {
        let words = [format!("{word}a"), format!("{word}b")];
        on_editor(&ws, &mut main, |e, _, cx| e.set_text(&words[0], cx));
        on_editor(&other_ws, &mut other, |e, _, cx| e.set_text(&words[1], cx));
        if action == "Unlearn Word" {
            probe.learned.lock().unwrap().extend(words.clone());
        }
        suggestions(&ws, &mut main);
        suggestions(&other_ws, &mut other);
        let released = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(AtomicBool::new(false));
        let held = Busy(released.clone());
        let running = entered.clone();
        let blocker = std::thread::spawn(move || {
            crate::languages::spell::test_on_worker(move || {
                running.store(true, Relaxed);
                while !released.load(Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            })
        });
        settle_until(&mut main, |_| entered.load(Relaxed));
        let generation = crate::languages::spell::DICTIONARY_GENERATION.load(Relaxed);
        let before = probe.changes.lock().unwrap().len();
        spelling_choice(&ws, &mut main, action);
        main.run_until_parked();
        spelling_choice(&other_ws, &mut other, action);
        other.run_until_parked();
        assert_eq!(probe.changes.lock().unwrap().len(), before, "native owner is blocked");
        on_editor(&ws, &mut main, |e, w, cx| match change {
            "edit" => e.edit(cx, |s| s.insert("x")),
            "caret" => e.move_caret(crate::editing::text::Motion::Left, w, cx),
            "request" => {
                e.perform_action(&editor::SpellingSuggestions, w, cx);
            }
            _ => unreachable!(),
        });
        main.run_until_parked();
        let revisions = [
            on_editor(&ws, &mut main, |e, _, _| e.spelling_revision()),
            on_editor(&other_ws, &mut other, |e, _, _| e.spelling_revision()),
        ];
        drop(held);
        blocker.join().unwrap();
        crate::languages::spell::test_on_worker(|| {});
        assert_eq!(
            &probe.changes.lock().unwrap()[before..],
            &[(action.to_string(), words[0].clone()), (action.to_string(), words[1].clone())]
        );
        settle_until(&mut main, |cx| on_editor(&ws, cx, |e, _, _| e.spelling_revision()) == revisions[0] + 2);
        settle_until(&mut other, |cx| on_editor(&other_ws, cx, |e, _, _| e.spelling_revision()) == revisions[1] + 2);
        assert_eq!(crate::languages::spell::DICTIONARY_GENERATION.load(Relaxed), generation + 2);
        for word in &words {
            assert_eq!(probe.learned.lock().unwrap().contains(word), action == "Learn Word");
        }
        close_quick_panel(&ws, &mut main);
    }
}

/// The quick panel closed, and the keys back in the active view.
fn close_quick_panel(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, w, cx| {
        ws.cancel_quick_panel(cx);
        ws.focus_editor(w, cx);
    });
}
#[gpui::test]
fn suggestion_replaces_exact_unicode_word_in_one_shared_undo(cx: &mut TestAppContext) {
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "😀 café qzxsuggest's tail");
    dictionary_probe(&ws, vcx);
    new_view(&ws, vcx);
    let views = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect::<Vec<_>>());
    on_editor(&ws, vcx, |e, _, cx| e.select_range(9..9, cx));
    suggestions(&ws, vcx);
    spelling_choice(&ws, vcx, "corrected");
    settle(vcx);
    assert_eq!(text(&ws, vcx), "😀 café corrected's tail");
    for view in &views {
        assert_eq!(view.read_with(vcx, |e, cx| e.text(cx)), text(&ws, vcx));
    }
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "😀 café qzxsuggest's tail");
    assert_eq!(on_editor(&ws, vcx, |e, _, _| e.state.cursor()), 9);
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, vcx), "😀 café corrected's tail");
    for view in &views {
        assert_eq!(view.read_with(vcx, |e, cx| e.text(cx)), text(&ws, vcx));
    }
}

#[gpui::test]
fn scan_expires_on_edits_caret_source_syntax_selector_and_context(cx: &mut TestAppContext) {
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, vcx) = window(cx);
    for change in ["edit", "caret", "source", "syntax", "selector", "panel", "window", "dictionary"] {
        on_editor(&ws, vcx, |e, _, cx| {
            e.set_text("mispelled good mispelled", cx);
            e.select_range(0..0, cx);
        });
        let probe = spell_probe(&ws, vcx, false);
        let held = Busy(probe.released.clone());
        vcx.simulate_keystrokes("ctrl-f6");
        settle_until(vcx, |_| probe.entered.load(Relaxed));
        ws.update_in(vcx, |ws, w, cx| match change {
            "edit" => {
                ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
            }
            "caret" => {
                ws.with_active(cx, |e, cx| e.select_range(12..12, cx));
            }
            "source" => ws.new_file(&NewFile, w, cx),
            "syntax" => ws.set_syntax(&syntax("Rust"), w, cx),
            "selector" => ws.apply_settings(r#"{"spelling_selector":"keyword"}"#, w, cx),
            "panel" => ws.command_palette(&CommandPalette, w, cx),
            "window" => w.activate_window(),
            "dictionary" => {
                crate::languages::spell::DICTIONARY_GENERATION.fetch_add(1, Relaxed);
            }
            _ => unreachable!(),
        });
        let expected = on_editor(&ws, vcx, |e, _, cx| (e.text(cx), e.state.selections.clone()));
        drop(held);
        settle(vcx);
        assert_eq!(on_editor(&ws, vcx, |e, _, cx| (e.text(cx), e.state.selections.clone())), expected, "{change}");
        ws.update_in(vcx, |ws, w, cx| {
            ws.cancel_quick_panel(cx);
            ws.focus_editor(w, cx);
            ws.apply_settings("{}", w, cx);
            ws.set_syntax(&syntax("Plain Text"), w, cx);
        });
    }
}

#[gpui::test]
fn delayed_suggestions_preserve_changed_words_new_panels_and_stale_choices(cx: &mut TestAppContext) {
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "qzxstale");
    let probe = dictionary_probe(&ws, vcx);
    probe.released.store(false, Relaxed);
    let held = Busy(probe.released.clone());
    on_editor(&ws, vcx, |e, w, cx| {
        e.perform_action(&editor::SpellingSuggestions, w, cx);
    });
    settle_until(vcx, |_| probe.entered.load(Relaxed));
    vcx.simulate_keystrokes("escape");
    assert!(
        ws.read_with(vcx, |ws, _| ws.spelling_job.is_none()),
        "Escape cancels pending guesses before another native stage"
    );
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.insert("x")));
    ws.update_in(vcx, |ws, w, cx| ws.command_palette(&CommandPalette, w, cx));
    let input =
        |vcx: &mut VisualTestContext| ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().unwrap().input.entity_id());
    let newer = input(vcx);
    drop(held);
    settle(vcx);
    assert_eq!(input(vcx), newer);
    assert_eq!(text(&ws, vcx), "qzxstalex");
    close_quick_panel(&ws, vcx);
    suggestions(&ws, vcx);
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.insert("y")));
    spelling_choice(&ws, vcx, "corrected");
    assert_eq!(text(&ws, vcx), "qzxstalexy");
    close_quick_panel(&ws, vcx);
    suggestions(&ws, vcx);
    ws.update(vcx, |ws, _| ws.invalidate_navigation());
    spelling_choice(&ws, vcx, "corrected");
    assert_eq!(text(&ws, vcx), "qzxstalexy");
}

#[gpui::test]
fn dictionary_actions_are_session_wide_and_learned_words_can_be_unlearned(cx: &mut TestAppContext) {
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    use std::sync::atomic::Ordering::Relaxed;
    let (ws, other_ws, mut main, mut other) = two_windows(cx);
    let vcx = &mut main;
    type_in(&ws, vcx, "qzxsessionignore");
    let probe = dictionary_probe(&ws, vcx);
    type_in(&other_ws, &mut other, "qzxsessionignore");
    on_editor(&other_ws, &mut other, |e, _, _| e.spell_checker = Some(probe.clone()));
    other.simulate_keystrokes("f6");
    settle_until(&mut other, |cx| !on_editor(&other_ws, cx, |e, _, _| e.spelling_ranges(0)).is_empty());
    let other_view = other_ws.read_with(&other, |ws, _| ws.active_editor().unwrap());
    vcx.simulate_keystrokes("f6");
    settle_until(vcx, |cx| !on_editor(&ws, cx, |e, _, _| e.spelling_ranges(0)).is_empty());
    let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(vcx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
    type_in(&ws, vcx, "qzxsessionignore");
    on_editor(&ws, vcx, |e, _, _| e.spell_checker = Some(probe.clone()));
    let generation = crate::languages::spell::DICTIONARY_GENERATION.load(Relaxed);
    suggestions(&ws, vcx);
    spelling_choice(&ws, vcx, "Ignore Word");
    settle_until(vcx, |_| crate::languages::spell::DICTIONARY_GENERATION.load(Relaxed) > generation);
    settle_until(vcx, |cx| first.read_with(cx, |e, _| e.spelling_ranges(0)).is_empty());
    settle_until(&mut other, |cx| other_view.read_with(cx, |e, _| e.spelling_ranges(0)).is_empty());
    vcx.simulate_keystrokes("ctrl-f6");
    settle_until(vcx, |cx| left_text(&ws, cx) == "No misspellings");
    on_editor(&ws, vcx, |e, _, cx| e.set_text("qzxlearnmutable", cx));
    suggestions(&ws, vcx);
    spelling_choice(&ws, vcx, "Learn Word");
    settle_until(vcx, |_| probe.learned.lock().unwrap().contains("qzxlearnmutable"));
    suggestions(&ws, vcx);
    spelling_choice(&ws, vcx, "Unlearn Word");
    settle_until(vcx, |_| !probe.learned.lock().unwrap().contains("qzxlearnmutable"));
    settle(vcx);
    on_editor(&ws, vcx, |e, _, cx| e.select_range(0..0, cx));
    vcx.simulate_keystrokes("ctrl-f6");
    settle_until(vcx, |cx| first_range(&ws, cx) == (0..15));
}

#[gpui::test]
fn scan_covers_late_sub_batches_and_a_giant_line(cx: &mut TestAppContext) {
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    let (ws, vcx) = window(cx);
    spell_probe(&ws, vcx, true);
    for document in [format!("{}mispelled", "good\n".repeat(437)), format!("{} café mispelled", "x".repeat(150_000))] {
        on_editor(&ws, vcx, |e, _, cx| {
            e.set_text(&document, cx);
            e.select_range(0..0, cx);
        });
        vcx.simulate_keystrokes("ctrl-f6");
        settle_until(vcx, |cx| on_editor(&ws, cx, |e, _, _| e.state.selections[0].range().len()) == 9);
        let range = first_range(&ws, vcx);
        assert_eq!(slice(&ws, vcx, range.clone()), "mispelled");
        assert_eq!(range.end, document.chars().count());
    }
}

#[gpui::test]
fn right_click_and_command_customization_reach_spelling_choices(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "😀 café qzxright");
    dictionary_probe(&ws, vcx);
    vcx.update(|_, cx| {
        let bindings = crate::config::customization::bindings(r#"[{"keys":["cmd-shift-9"],"command":"editor::SpellingSuggestions"},{"keys":["cmd-shift-8"],"command":"editor::NextMisspelling"},{"keys":["cmd-shift-7"],"command":"editor::PrevMisspelling"}]"#, cx).unwrap();
        assert_eq!(bindings.len(), 3);
    });
    vcx.simulate_keystrokes("f6");
    settle_until(vcx, |cx| !on_editor(&ws, cx, |e, _, _| e.spelling_drawn()).is_empty());
    let at = on_editor(&ws, vcx, |e, window, cx| {
        e.bounds_for_range(8..9, gpui::Bounds::default(), window, cx).unwrap().center()
    });
    vcx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::none());
    settle_until(vcx, |cx| {
        ws.read_with(cx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.kind == Kind::Spelling && p.loaded))
    });
    spelling_choice(&ws, vcx, "corrected");
    assert_eq!(text(&ws, vcx), "😀 café corrected");
    let view = submenu(&ws, vcx, "View", "Spelling");
    for name in ["Next Misspelling", "Previous Misspelling", "Spelling Suggestions…"] {
        assert!(
            view.iter().any(|m| matches!(m, gpui::MenuItem::Action { name: n, .. } if n.as_ref() == name)),
            "{name}"
        );
    }
}

#[gpui::test]
fn navigation_checks_boundary_spanning_words_intact_in_both_directions(cx: &mut TestAppContext) {
    let _dictionary = SPELL_ACTION_TEST.lock().unwrap();
    struct LongWord {
        word: String,
        calls: AtomicUsize,
    }
    impl crate::languages::spell::Checker for LongWord {
        fn misspelled(&self, text: &str, _stop: &AtomicBool) -> Vec<crate::languages::spell::Misspelling> {
            assert!(crate::languages::spell::is_owner_thread());
            let ranges: Vec<_> = text
                .match_indices(&self.word)
                .map(|(at, word)| crate::languages::spell::Misspelling {
                    range: at..at + word.len(),
                    source: at..at + word.len(),
                })
                .collect();
            if !ranges.is_empty() {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            ranges
        }
    }
    let (ws, vcx) = window(cx);
    for (word, prefix, suffix) in [("mispelled".repeat(400), "", ""), ("mispelled".repeat(20_000), "😀 café ", " fin")]
    {
        let probe = Arc::new(LongWord { word: word.clone(), calls: AtomicUsize::new(0) });
        let document = format!("{prefix}{word}{suffix}");
        let expected = prefix.chars().count()..prefix.chars().count() + word.chars().count();
        on_editor(&ws, vcx, |e, _, cx| {
            e.spell_checker = Some(probe.clone());
            e.set_text(&document, cx);
            e.select_range(0..0, cx);
        });
        for (key, caret) in [("ctrl-f6", 0), ("ctrl-shift-f6", document.chars().count())] {
            on_editor(&ws, vcx, |e, _, cx| e.select_range(caret..caret, cx));
            vcx.simulate_keystrokes(key);
            settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.spelling_job.is_none()));
            assert_eq!(first_range(&ws, vcx), expected, "{key}: eligible word must be checked intact");
            assert_ne!(left_text(&ws, vcx), "No misspellings");
        }
        assert_eq!(
            probe.calls.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "one intact input per direction, without repeating the giant word per region"
        );
    }
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)]
fn folding_never_changes_the_saved_text_or_the_undo_history(cx: &mut TestAppContext) {
    let path = temp(&format!("orlo-fold-save-{}", std::process::id())).join("folded.rs");
    // "a, b" is 29..33, inside the block "fn a" heads
    let text = "fn a() {\n    let x = compute(a, b);\n}\n";
    std::fs::write(&path, text).unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    vcx.run_until_parked();
    // This assertion exercises indentation folds; a completed Rust worker has a different
    // valid syntax-node boundary. Select the fallback explicitly instead of racing that job.
    ws.update_in(vcx, |ws, window, cx| ws.set_syntax(&syntax("Plain Text"), window, cx));
    let state = |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| {
        on_editor(ws, vcx, |e, _, cx| {
            let doc = e.doc(cx);
            (doc.text(), doc.version, doc.dirty, e.folds().chars())
        })
    };
    let (_, version, _, _) = state(&ws, vcx);
    // a fold inside a line, then the block around it
    set_selections(&ws, vcx, &[(29, 33)]);
    vcx.simulate_keystrokes("cmd-alt-[");
    set_selections(&ws, vcx, &[(2, 2)]);
    vcx.simulate_keystrokes("cmd-alt-[");
    let (now, v, dirty, folds) = state(&ws, vcx);
    assert_eq!((now.as_str(), v, dirty), (text, version, false), "folding is no edit");
    // the block: the break after "{" (8) through the end of line 1 (35)
    assert_eq!(folds, [8..35, 29..33]);
    // an edit after "}" (36), saved: the file has the text, every char of it, no placeholder
    set_selections(&ws, vcx, &[(37, 37)]);
    vcx.simulate_keystrokes("x cmd-s");
    vcx.run_until_parked();
    let edited = "fn a() {\n    let x = compute(a, b);\n}x\n";
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    // undo takes back the edit, and only it: no fold was an undo step
    vcx.simulate_keystrokes("cmd-z");
    let (now, _, _, folds) = state(&ws, vcx);
    assert_eq!(now, text);
    assert_eq!(folds, [8..35, 29..33], "the folds are not undone");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(state(&ws, vcx).0, text, "nothing before the edit to undo");
    let _ = std::fs::remove_file(&path);
}

#[gpui::test]
fn unicode_warning_range_character_info_uses_configured_description(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    set_state(&ws, vcx, "e\u{301}\u{e000}");
    ws.update_in(vcx, |ws, window, cx| {
        ws.apply_settings(
            r#"{"unicode_warnings":false,"unicode_warning_ranges":{"U+0300..U+036F":{"glyph":"?","description":"Configured accents"},
        "U+E000":{"glyph":"!","description":"Private marker"}}}"#,
            window,
            cx,
        )
    });
    set_selections(&ws, vcx, &[(0, 0)]);
    vcx.dispatch_action(CharacterInfo);
    let (_, detail) = vcx.pending_prompt().unwrap();
    assert!(detail.contains("U+0301 — Configured accents"), "{detail}");
    assert!(detail.contains("marks disabled"), "{detail}");
    vcx.simulate_prompt_answer("Ok");
    set_selections(&ws, vcx, &[(2, 2)]);
    vcx.dispatch_action(CharacterInfo);
    let (_, detail) = vcx.pending_prompt().unwrap();
    assert!(detail.contains("U+E000 — Private marker"), "{detail}");
    vcx.simulate_prompt_answer("Ok");
    ws.update_in(vcx, |ws, window, cx| {
        ws.apply_settings(
            r#"{"unicode_warnings":true,"unicode_warning_ranges":{"U+D800":{"glyph":"!","description":"Bad range"}}}"#,
            window,
            cx,
        )
    });
    let (title, detail) = vcx.pending_prompt().unwrap();
    assert_eq!(title, "Could not apply settings");
    assert!(detail.contains("unicode_warning_ranges"), "{detail}");
    vcx.simulate_prompt_answer("Ok");
    vcx.run_until_parked();
    on_editor(&ws, vcx, |e, _, cx| {
        assert!(e.view.unicode_warnings);
        assert!(e.view.unicode_warning_ranges.is_empty());
        assert!(e.char_bounds(0, cx).is_some());
        assert_eq!(e.doc(cx).text(), "e\u{301}\u{e000}");
    });
}

#[gpui::test]
fn unicode_warning_ranges_preserve_shared_copy_save_bytes_and_undo(cx: &mut TestAppContext) {
    let dir = temp(&format!("orlo-unicode-range-bytes-{}", std::process::id()));
    let path = dir.join("warnings.txt");
    let original = "\u{a0}\u{200b}\u{202e}e\u{301}\u{e000}\n";
    std::fs::write(&path, original.as_bytes()).unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.open_path(path.clone(), window, cx));
    vcx.run_until_parked();
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    assert_eq!(views.len(), 2);
    for enabled in [true, false, true] {
        let version = on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version);
        let settings = format!(
            r#"{{"unicode_warnings":{enabled},"unicode_warning_ranges":{{
            "U+0300..U+036F":{{"glyph":"?","description":"Accents"}},
            "U+E000":{{"glyph":"!","description":"Private marker"}}}}}}"#
        );
        ws.update_in(vcx, |ws, window, cx| ws.apply_settings(&settings, window, cx));
        vcx.run_until_parked();
        for view in &views {
            view.read_with(vcx, |e, cx| {
                assert_eq!(e.view.unicode_warnings, enabled);
                assert_eq!(e.view.unicode_warning_ranges.len(), 2);
                assert_eq!(e.doc(cx).version, version);
                assert_eq!(e.doc(cx).text(), original);
                assert!(!e.doc(cx).dirty);
            });
        }
        set_selections(&ws, vcx, &[(0, original.chars().count())]);
        vcx.simulate_keystrokes("cmd-c");
        assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap().as_bytes(), original.as_bytes());
        set_selections(&ws, vcx, &[(original.chars().count(), original.chars().count())]);
        vcx.simulate_keystrokes("z cmd-s");
        settle_until(vcx, |_| std::fs::read(&path).unwrap() == format!("{original}z").as_bytes());
        vcx.simulate_keystrokes("cmd-z cmd-s");
        settle_until(vcx, |_| std::fs::read(&path).unwrap() == original.as_bytes());
        for view in &views {
            view.read_with(vcx, |e, cx| assert_eq!(e.doc(cx).text(), original));
        }
    }
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), original, "settings create no undo entry");
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui::test]
fn unicode_warning_character_info_and_live_settings_follow_shared_views(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    set_state(&ws, vcx, "\u{a0}e\u{301}👩\u{200d}👧");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    assert_eq!(views.len(), 2);
    let original = text(&ws, vcx);
    set_selections(&ws, vcx, &[(0, 0)]);
    vcx.dispatch_action(CharacterInfo);
    let (_, detail) = vcx.pending_prompt().unwrap();
    assert!(detail.contains("U+00A0 — Non-breaking space"), "{detail}");
    vcx.simulate_prompt_answer("Ok");
    ws.update_in(vcx, |ws, window, cx| ws.apply_settings(r#"{"unicode_warnings":false}"#, window, cx));
    for view in &views {
        view.read_with(vcx, |e, cx| {
            assert!(!e.view.unicode_warnings);
            assert_eq!(e.doc(cx).text(), original);
        });
    }
    vcx.dispatch_action(CharacterInfo);
    let (_, detail) = vcx.pending_prompt().unwrap();
    assert!(detail.contains("marks disabled"), "{detail}");
    vcx.simulate_prompt_answer("Ok");
    ws.update_in(vcx, |ws, window, cx| {
        ws.apply_settings(
            r#"{"unicode_warnings":true,"unicode_warning_characters":{"\u00a0":{"glyph":"?","description":"Project spacing convention"}}}"#,
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    vcx.dispatch_action(CharacterInfo);
    let (_, detail) = vcx.pending_prompt().unwrap();
    assert!(detail.contains("Project spacing convention"), "{detail}");
    vcx.simulate_prompt_answer("Ok");
    for view in &views {
        view.read_with(vcx, |e, cx| {
            assert!(e.view.unicode_warnings);
            assert_eq!(e.view.unicode_warning_characters.len(), 1);
            assert_eq!(e.view.unicode_warning_characters[&'\u{a0}'].glyph, '?');
            assert_eq!(e.doc(cx).text(), original);
        });
    }
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.insert("z")));
    vcx.run_until_parked();
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.undo()));
    for view in &views {
        view.read_with(vcx, |e, cx| assert_eq!(e.doc(cx).text(), original));
    }
}

#[gpui::test]
fn overtype_keyboard_action_and_clickable_status_indicator(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "abcd");
    on_editor(&ws, vcx, |e, _, cx| e.sel(cx).set_cursor(0));
    vcx.simulate_keystrokes("x");
    assert_eq!(text(&ws, vcx), "xabcd");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("overtype-ins").is_some());
    vcx.dispatch_action(editor::ToggleOvertype);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("overtype-ovr").is_some());
    vcx.simulate_keystrokes("y");
    assert_eq!(text(&ws, vcx), "xybcd");
    ws.update(vcx, |ws, cx| ws.status_message("Saved", cx));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("overtype-ovr").is_some(), "mode is visible during temporary messages");
    let button = vcx.debug_bounds("overtype-toggle").unwrap();
    vcx.simulate_click(button.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("overtype-ins").is_some());
    vcx.simulate_keystrokes("z");
    assert_eq!(text(&ws, vcx), "xyzbcd");
    vcx.dispatch_action(editor::ToggleOvertype);
    vcx.dispatch_action(crate::config::customization::InsertText { text: "Q".into(), replace: None });
    assert_eq!(text(&ws, vcx), "xyzQcd");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "xyzbcd");
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, vcx), "xyzQcd");
}

#[gpui::test]
fn overtype_menu_palette_and_new_shared_view(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    assert!(menu(&ws, vcx, "Edit").iter().any(|item| {
        matches!(item, MenuItem::Action { name, action, .. }
            if name == "Toggle Overtype" && action.as_any().is::<editor::ToggleOvertype>())
    }));
    assert!(quick_panel::command_items().iter().any(|item| {
        item.label == "Edit: Toggle Overtype"
            && matches!(&item.action, ItemAction::Dispatch(action) if action.as_any().is::<editor::ToggleOvertype>())
    }));
    type_in(&ws, vcx, "abc");
    vcx.dispatch_action(editor::ToggleOvertype);
    let first = ws.update(vcx, |ws, _| ws.active_editor().unwrap());
    new_view(&ws, vcx);
    on_editor(&ws, vcx, |e, _, cx| {
        assert!(!e.state.overtype);
        assert!(e.same_document(first.read(cx)));
        assert!(first.read(cx).state.overtype);
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("overtype-ins").is_some());
}

#[gpui::test]
fn overtype_mode_and_indicator_follow_tabs_and_stay_local_across_windows(cx: &mut TestAppContext) {
    let path = temp("orlo-overtype-windows").join("source.txt");
    std::fs::write(&path, "abc").unwrap();
    let (owner, peer, mut a, mut b) = two_windows(cx);
    owner.update_in(&mut a, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    peer.update_in(&mut b, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    a.dispatch_action(editor::ToggleOvertype);
    a.run_until_parked();
    b.run_until_parked();
    assert!(a.debug_bounds("overtype-ovr").is_some());
    assert!(b.debug_bounds("overtype-ins").is_some());
    on_editor(&owner, &mut a, |e, _, cx| e.sel(cx).set_cursor(0));
    a.simulate_keystrokes("X");
    assert_eq!(text(&owner, &mut a), "Xbc");
    on_editor(&peer, &mut b, |e, _, cx| {
        assert!(!e.state.overtype);
        e.sel(cx).set_cursor(0);
    });
    b.simulate_keystrokes("Y");
    assert_eq!(text(&owner, &mut a), "YXbc");
    owner.update_in(&mut a, |ws, w, cx| ws.new_file(&NewFile, w, cx));
    // GPUI retains old selector keys. Resize so stale bounds cannot stand in for this label.
    a.simulate_resize(gpui::size(px(1100.), px(800.)));
    a.run_until_parked();
    let before_toggle = a.debug_bounds("overtype-toggle").unwrap();
    let label = a.debug_bounds("overtype-ins").unwrap();
    assert!(a.debug_bounds("overtype-toggle").unwrap().contains(&label.center()));
    on_editor(&owner, &mut a, |e, _, _| assert!(!e.state.overtype));
    a.simulate_keystrokes("new");
    a.dispatch_action(SelectTab { index: 0 });
    a.simulate_resize(gpui::size(px(1400.), px(800.)));
    a.run_until_parked();
    let after_toggle = a.debug_bounds("overtype-toggle").unwrap();
    assert_ne!(before_toggle.origin.x, after_toggle.origin.x, "the control must actually move between frames");
    let label = a.debug_bounds("overtype-ovr").unwrap();
    assert!(a.debug_bounds("overtype-toggle").unwrap().contains(&label.center()));
    on_editor(&owner, &mut a, |e, _, cx| {
        assert!(e.state.overtype);
        e.sel(cx).set_cursor(0);
    });
    a.simulate_keystrokes("Z");
    assert_eq!(text(&peer, &mut b), "ZXbc");
    b.dispatch_action(editor::ToggleOvertype);
    let toggle = a.debug_bounds("overtype-toggle").unwrap();
    a.simulate_click(toggle.center(), gpui::Modifiers::default());
    a.simulate_resize(gpui::size(px(1200.), px(800.)));
    a.run_until_parked();
    b.run_until_parked();
    let label = a.debug_bounds("overtype-ins").unwrap();
    assert!(a.debug_bounds("overtype-toggle").unwrap().contains(&label.center()));
    assert!(b.debug_bounds("overtype-ovr").is_some());
    on_editor(&owner, &mut a, |e, _, _| assert!(!e.state.overtype));
    on_editor(&peer, &mut b, |e, _, _| assert!(e.state.overtype));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "abc");
}

#[gpui::test]
fn overtype_source_mode_does_not_reach_live_find_and_replace_fields(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "abc");
    vcx.dispatch_action(editor::ToggleOvertype);
    ws.update_in(vcx, |ws, w, cx| replace_text(ws, "ab", "cd", false, w, cx));
    let fields = ws.read_with(vcx, |ws, _| {
        let p = ws.find_panel().unwrap();
        [p.query.clone(), p.replace.clone()]
    });
    for (field, expected) in fields.into_iter().zip(["aXb", "cXd"]) {
        field.update_in(vcx, |e, w, cx| {
            assert!(!e.state.overtype);
            e.sel(cx).set_cursor(1);
            e.focus(w);
        });
        vcx.dispatch_action(editor::ToggleOvertype);
        vcx.simulate_keystrokes("X");
        field.read_with(vcx, |e, cx| {
            assert!(!e.state.overtype);
            assert_eq!(e.text(cx), expected);
        });
        on_editor(&ws, vcx, |e, _, cx| {
            assert!(e.state.overtype);
            assert_eq!(e.text(cx), "abc");
        });
        vcx.run_until_parked();
        assert!(vcx.debug_bounds("overtype-ovr").is_some());
    }
    vcx.simulate_keystrokes("escape");
    on_editor(&ws, vcx, |e, _, cx| e.sel(cx).set_cursor(0));
    vcx.simulate_keystrokes("Z");
    assert_eq!(text(&ws, vcx), "Zbc");
}

#[gpui::test]
fn overtype_paste_tab_and_newline_use_insertion(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "abcd");
    on_editor(&ws, vcx, |e, _, cx| e.sel(cx).set_cursor(0));
    vcx.dispatch_action(editor::ToggleOvertype);
    vcx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("XY".into())));
    vcx.simulate_keystrokes("cmd-v");
    assert_eq!(text(&ws, vcx), "XYabcd");
    vcx.simulate_keystrokes("tab");
    assert_eq!(text(&ws, vcx), "XY  abcd");
    vcx.simulate_keystrokes("enter");
    assert_eq!(text(&ws, vcx), "XY  \nabcd");
}

#[gpui::test]
fn overtype_is_not_persisted_in_session_or_reopened_views(cx: &mut TestAppContext) {
    let (ws, vcx, path) = window_on(cx, "orlo-overtype-restore", "abc\r\n");
    vcx.dispatch_action(editor::ToggleOvertype);
    let saved = ws.update_in(vcx, |ws, window, cx| {
        assert!(ws.active_editor().unwrap().read(cx).state.overtype);
        let state = ws.capture(window, false, &mut Texts::new(), cx);
        let json = serde_json::to_string(&state).unwrap();
        assert!(!json.contains("\"overtype\""));
        serde_json::from_str::<WindowState>(&json).unwrap()
    });
    ws.update_in(vcx, |ws, window, cx| ws.restore(saved, window, cx));
    settle(vcx);
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
    on_editor(&ws, vcx, |e, window, cx| {
        assert!(!e.state.overtype);
        e.perform_action(&editor::ToggleOvertype, window, cx);
    });
    ws.update_in(vcx, |ws, window, cx| {
        let editor = ws.active_editor().unwrap();
        ws.close_editor(&editor, window, cx);
        ws.reopen_closed_file(&ReopenClosedFile, window, cx);
        assert!(!ws.active_editor().unwrap().read(cx).state.overtype);
    });
    assert_eq!(std::fs::read(path).unwrap(), b"abc\r\n");
}

#[gpui::test]
fn virtual_space_settings_shared_views_and_session_offsets_remain_compatible(cx: &mut TestAppContext) {
    assert_virtual_space_session_roundtrip("true", false, cx);
}

#[gpui::test]
fn virtual_space_rectangular_settings_and_session_offsets_remain_compatible(cx: &mut TestAppContext) {
    assert_virtual_space_session_roundtrip("\"rectangular\"", true, cx);
}

fn assert_virtual_space_session_roundtrip(policy: &str, rectangular: bool, cx: &mut TestAppContext) {
    let (ws, vcx, path) = window_on(cx, &format!("orlo-virtual-space-restore-{rectangular}"), "word\r\n");
    ws.update_in(vcx, |ws, window, cx| ws.apply_settings(&format!(r#"{{"virtual_space":{}}}"#, policy), window, cx));
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    for view in &views {
        view.update(vcx, |e, cx| {
            assert!(e.state.virtual_space.enabled());
            e.edit(cx, |s| s.set_selections(vec![Selection { rectangular, ..Selection::from_points((4, 3), (4, 3)) }]));
            assert!(!e.doc(cx).unsaved());
        });
    }
    let saved = ws.update_in(vcx, |ws, window, cx| {
        let state = ws.capture(window, false, &mut Texts::new(), cx);
        let json = serde_json::to_string(&state).unwrap();
        assert!(!json.contains("anchor_virtual") && !json.contains("head_virtual") && !json.contains("rectangular"));
        serde_json::from_str::<WindowState>(&json).unwrap()
    });
    ws.update_in(vcx, |ws, window, cx| ws.restore(saved, window, cx));
    settle(vcx);
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
    on_editor(&ws, vcx, |e, _, cx| {
        assert!(e.state.virtual_space.enabled());
        assert!(e.state.selections.iter().all(|s| !s.has_virtual()));
        e.edit(cx, |s| s.set_selections(vec![Selection { rectangular, ..Selection::from_points((4, 2), (4, 2)) }]));
    });
    vcx.simulate_keystrokes("X");
    assert_eq!(text(&ws, vcx), "word  X\n");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "word\n");
    ws.update_in(vcx, |ws, window, cx| ws.apply_settings("{}", window, cx));
    on_editor(&ws, vcx, |e, _, cx| {
        assert!(!e.state.virtual_space.enabled());
        assert!(e.state.selections.iter().all(|s| !s.has_virtual()));
        assert!(!e.doc(cx).unsaved());
    });
    assert_eq!(std::fs::read(path).unwrap(), b"word\r\n");
}

#[gpui::test]
fn subword_menu_exposes_all_six_commands(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    assert_eq!(
        names(&submenu(&ws, vcx, "Edit", "Subwords")),
        [
            "Previous Subword",
            "Next Subword",
            "Select Previous Subword",
            "Select Next Subword",
            "Delete Previous Subword",
            "Delete Next Subword",
        ]
    );
}

#[gpui::test]
fn subword_custom_keymap_runs_selection_history_delete_and_preserves_word_bindings(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "camelCase item42Value");
    let keymap = serde_json::json!([
        {"keys":["cmd-j"],"command":"editor::Move",
         "args":{"motion":"subword_right","select":false},"context":"Editor"},
        {"keys":["cmd-alt-j"],"command":"editor::Move",
         "args":{"motion":"subword_right","select":true},"context":"Editor"},
        {"keys":["cmd-alt-backspace"],"command":"editor::DeleteSubword",
         "args":{"forward":false},"context":"Editor"},
    ]);
    std::fs::write(crate::config::customization::keymap_path(), keymap.to_string()).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    assert_eq!(vcx.pending_prompt(), None);
    vcx.simulate_keystrokes("cmd-up alt-right");
    assert_eq!(sels(&ws, vcx), [Selection::caret(9)], "ordinary word still consumes camelCase");
    vcx.simulate_keystrokes("ctrl-left cmd-j");
    assert_eq!(sels(&ws, vcx), [Selection::caret(5)]);
    vcx.simulate_keystrokes("cmd-alt-j");
    assert_eq!(sels(&ws, vcx), [Selection::new(5, 9)]);
    vcx.dispatch_action(editor::UndoSelection);
    assert_eq!(sels(&ws, vcx), [Selection::caret(5)]);
    vcx.simulate_keystrokes("cmd-alt-j cmd-alt-backspace");
    assert_eq!(text(&ws, vcx), "camel item42Value");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "camelCase item42Value");
    assert_eq!(sels(&ws, vcx), [Selection::new(5, 9)]);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "camel item42Value");
}

#[gpui::test]
fn subword_native_menu_delete_maps_shared_views_and_respects_locked_sources(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "camelCase item42Value");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    views[0].update(vcx, |e, cx| e.edit(cx, |s| s.set_cursor(20)));
    set_selections(&ws, vcx, &[(1, 1), (3, 3), (16, 16)]);
    on_editor(&ws, vcx, |e, _, cx| e.update_doc(cx, |d| d.read_only = true));
    let action = submenu(&ws, vcx, "Edit", "Subwords")
        .into_iter()
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name.as_ref() == "Delete Next Subword" => Some(action),
            _ => None,
        })
        .expect("native subword deletion");
    vcx.update(|window, cx| window.dispatch_action(action, cx));
    vcx.run_until_parked();
    for view in &views {
        assert_eq!(view.read_with(vcx, |e, cx| e.text(cx)), "cCase item42");
    }
    assert_eq!(views[0].read_with(vcx, |e, _| e.state.cursor()), 12);
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "camelCase item42Value");
    assert_eq!(sels(&ws, vcx), [Selection::caret(1), Selection::caret(3), Selection::caret(16)]);
    for (follow, viewer) in [(true, false), (false, true)] {
        on_editor(&ws, vcx, |e, _, cx| {
            e.update_doc(cx, |d| d.set_following(follow));
            e.input_enabled = !viewer;
        });
        let before = (sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version));
        for forward in [false, true] {
            vcx.dispatch_action(editor::DeleteSubword { forward });
            assert_eq!(text(&ws, vcx), "camelCase item42Value");
            assert_eq!((sels(&ws, vcx), on_editor(&ws, vcx, |e, _, cx| e.doc(cx).version)), before);
        }
    }
    on_editor(&ws, vcx, |e, _, _| e.input_enabled = true);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "cCase item42", "locked deletions added no history");
}

#[gpui::test]
fn subword_action_catches_up_before_calculating_deletion_targets(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "camelCase");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    set_selections(&ws, vcx, &[(5, 5)]);
    ws.update_in(vcx, |_, window, cx| {
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("prefix ");
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.cursor(), 5, "event delivery has not caught this view up");
            assert!(e.perform_action(&editor::DeleteSubword { forward: true }, window, cx));
            assert_eq!(e.text(cx), "prefix camel");
            assert_eq!(e.state.cursor(), 12);
        });
    });
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "prefix camelCase");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "camelCase");
}

#[gpui::test]
fn syntax_selection_native_menu_custom_keymap_macro_and_lexical_binding(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "fn main() { café + 雪; }");
    vcx.dispatch_action(SetSyntax { name: "Rust".into() });
    on_editor(&ws, vcx, |e, _, cx| {
        let rope = e.doc(cx).rope.clone();
        let job = highlight::run_job(e.highlighter.job(0), &rope, &[0]);
        e.highlighter.adopt(job, &rope);
        e.sel(cx).set_cursor(13);
    });
    let native = menu(&ws, vcx, "Selection");
    for label in ["Expand Syntax Selection", "Shrink Syntax Selection"] {
        assert!(names(&native).contains(&label.to_string()));
    }
    let keymap = serde_json::json!([
        {"keys":["cmd-alt-j"],"command":"editor::ExpandSyntaxSelection","context":"Editor"},
        {"keys":["cmd-alt-h"],"command":"editor::ShrinkSyntaxSelection","context":"Editor"},
    ]);
    std::fs::write(crate::config::customization::keymap_path(), keymap.to_string()).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    on_editor(&ws, vcx, |e, _, cx| {
        let rope = e.doc(cx).rope.clone();
        let job = highlight::run_job(e.highlighter.job(0), &rope, &[0]);
        e.highlighter.adopt(job, &rope);
    });
    vcx.update(|_, cx| cx.default_global::<crate::keybindings::Recorder>().recording = true);
    vcx.simulate_keystrokes("cmd-alt-j cmd-alt-j cmd-alt-h");
    assert_eq!(sels(&ws, vcx), [Selection::new(12, 16)]);
    vcx.update(|_, cx| {
        let recorded = &cx.global::<crate::keybindings::Recorder>().commands;
        assert_eq!(
            recorded.iter().map(|c| c.command.as_str()).collect::<Vec<_>>(),
            ["editor::ExpandSyntaxSelection", "editor::ExpandSyntaxSelection", "editor::ShrinkSyntaxSelection"]
        );
    });
    vcx.simulate_keystrokes("cmd-alt-h");
    assert_eq!(sels(&ws, vcx), [Selection::caret(13)]);
    vcx.simulate_keystrokes("cmd-shift-a");
    assert_eq!(sels(&ws, vcx), [Selection::new(12, 16)], "original lexical binding");
    assert_eq!(text(&ws, vcx), "fn main() { café + 雪; }");
}

#[gpui::test]
fn syntax_selection_shared_view_catches_up_and_has_independent_shrink_history(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "fn main() { café + 雪; }");
    vcx.dispatch_action(SetSyntax { name: "Rust".into() });
    on_editor(&ws, vcx, |e, _, cx| {
        let rope = e.doc(cx).rope.clone();
        let job = highlight::run_job(e.highlighter.job(0), &rope, &[0]);
        e.highlighter.adopt(job, &rope);
        e.sel(cx).set_cursor(13);
    });
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    ws.update_in(vcx, |_, window, cx| {
        views[0].update(cx, |e, cx| {
            e.perform_action(&editor::ExpandSyntaxSelection, window, cx);
            assert_eq!(e.state.selections, [Selection::new(12, 16)]);
        });
        views[1].update(cx, |e, cx| {
            e.perform_action(&editor::ShrinkSyntaxSelection, window, cx);
            assert_eq!(e.state.selections, [Selection::caret(13)], "a clone has no shrink stack");
        });
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("// new\n");
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.cursor(), 13, "document event is still queued");
            e.perform_action(&editor::ExpandSyntaxSelection, window, cx);
            assert_eq!(e.state.selections, [Selection::caret(20)], "catch up before parser query");
            let rope = e.doc(cx).rope.clone();
            let job = highlight::run_job(e.highlighter.job(0), &rope, &[0, 1]);
            e.highlighter.adopt(job, &rope);
            e.perform_action(&editor::ExpandSyntaxSelection, window, cx);
            assert_eq!(e.state.selections, [Selection::new(19, 23)]);
            e.perform_action(&editor::ShrinkSyntaxSelection, window, cx);
            assert_eq!(e.state.selections, [Selection::caret(20)]);
        });
    });
}

#[gpui::test]
fn syntax_selection_background_file_open_reuses_detected_parser_warmup(cx: &mut TestAppContext) {
    let path = temp("orlo-syntax-selection-warm-open").canonicalize().unwrap().join("warm.rs");
    std::fs::write(&path, "fn main() { café + 雪; }").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| ws.read_in_background(path.clone(), window, cx));
    settle_until(vcx, |cx| ws.read_with(cx, |ws, cx| ws.editor_for_path(&path, cx).is_some()));
    let e = ws.read_with(vcx, |ws, cx| ws.editor_for_path(&path, cx).expect("background file opened"));
    // Attachment can adopt partial warmup. Use the ordinary cooperative retry path until
    // both current-tree queries needed below are ready.
    settle_until(vcx, |cx| {
        e.update(cx, |e, cx| {
            let ready = e.highlighter.enclosing_named_range(&e.doc(cx).rope, 13..13).is_some()
                && e.highlighter.fold_regions(&e.doc(cx).rope).is_some();
            if !ready {
                e.highlight_in_background(0, 50, cx);
            }
            ready
        })
    });
    e.update_in(vcx, |e, window, cx| {
        e.sel(cx).set_cursor(13);
        assert_eq!(e.highlighter.enclosing_named_range(&e.doc(cx).rope, 13..13), Some(Some(12..16)));
        assert!(e.highlighter.fold_regions(&e.doc(cx).rope).is_some());
        assert!(e.perform_action(&editor::ExpandSyntaxSelection, window, cx));
        assert_eq!(e.state.selections, [Selection::new(12, 16)]);
    });
}

#[gpui::test]
fn reindent_menu_palette_and_custom_keymap(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    assert!(submenu(&ws, vcx, "Edit", "Line").iter().any(|item| {
        matches!(item, MenuItem::Action { name, action, .. } if name == "Reindent Selection" && action.as_any().is::<editor::ReindentSelection>())
    }));
    assert!(quick_panel::command_items().iter().any(|item| {
        item.label == "Edit: Reindent Selection"
            && matches!(&item.action, ItemAction::Dispatch(action) if action.as_any().is::<editor::ReindentSelection>())
    }));
    type_in(&ws, vcx, "if ok:\nwork()");
    vcx.dispatch_action(SetSyntax { name: "Python".into() });
    vcx.simulate_keystrokes("cmd-a");
    let keymap = serde_json::json!([
        {"keys":["cmd-alt-j"],"command":"editor::ReindentSelection","context":"Editor"}
    ]);
    std::fs::write(crate::config::customization::keymap_path(), keymap.to_string()).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    vcx.simulate_keystrokes("cmd-alt-j");
    assert_eq!(text(&ws, vcx), "if ok:\n    work()");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "if ok:\nwork()");
}

#[gpui::test]
fn reindent_shared_view_catches_up_and_read_only_working_copy_edits(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "if ok:\nwork()");
    vcx.dispatch_action(SetSyntax { name: "Python".into() });
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    on_editor(&ws, vcx, |e, _, cx| {
        e.highlighter = highlight::Highlighter::by_name("Python");
        e.sel(cx).select_all();
    });
    ws.update_in(vcx, |_, window, cx| {
        views[0].update(cx, |e, cx| {
            e.update_doc(cx, |d| d.read_only = true);
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("# header\n");
            });
        });
        views[1].update(cx, |e, cx| {
            assert!(e.perform_action(&editor::ReindentSelection, window, cx));
            assert_eq!(e.text(cx), "# header\nif ok:\n    work()");
        });
    });
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "# header\nif ok:\nwork()");
}

#[gpui::test]
fn virtual_space_fix1_insert_snippet_tab_and_shift_tab_keep_mixed_origins(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    write_snippet("origins", "${1:x}-$1-${2:y}$0", "", "", "Origins");
    type_in(&ws, vcx, "a\nb");
    ws.update_in(vcx, |ws, window, cx| ws.apply_settings(r#"{"virtual_space":"rectangular"}"#, window, cx));
    let original = vec![Selection::from_points((1, 2), (1, 2)).in_rectangle(), Selection::caret(3)];
    on_editor(&ws, vcx, |e, _, cx| e.sel(cx).set_selections(original.clone()));
    vcx.dispatch_action(InsertSnippet);
    choose_pipeline(vcx, "Origins");
    let expanded = "a  x-x-y\nbx-x-y";
    assert_eq!(text(&ws, vcx), expanded);
    let origins = |cx: &mut VisualTestContext| {
        on_editor(&ws, cx, |e, _, _| e.state.selections.iter().map(|sel| sel.rectangular).collect::<Vec<_>>())
    };
    assert_eq!(origins(vcx), [true, true, false, false]);
    vcx.simulate_keystrokes("tab");
    assert_eq!(snippet_fields(&ws, vcx), (vec!["y".into(), "y".into()], true));
    assert_eq!(origins(vcx), [true, false]);
    vcx.simulate_keystrokes("shift-tab");
    assert_eq!(origins(vcx), [true, true, false, false]);
    vcx.simulate_keystrokes("tab tab right");
    let after = on_editor(&ws, vcx, |e, _, _| {
        assert_eq!(e.state.selections.iter().map(|sel| sel.head_virtual).collect::<Vec<_>>(), [1, 0]);
        assert_eq!(e.state.primary(), 0);
        e.state.selections.clone()
    });
    assert_eq!(origins(vcx), [true, false]);
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, vcx), "a\nb");
    assert_eq!(on_editor(&ws, vcx, |e, _, _| e.state.selections.clone()), original);
    vcx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&ws, vcx), expanded);
    assert_eq!(on_editor(&ws, vcx, |e, _, _| e.state.selections.clone()), after);
}

#[gpui::test]
fn prose_native_menu_exposes_ten_commands_and_runs_selection_delete_undo(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "One. Two! Three tail");
    let items = submenu(&ws, vcx, "Edit", "Prose");
    assert_eq!(
        names(&items),
        [
            "Previous Paragraph",
            "Next Paragraph",
            "Select Previous Paragraph",
            "Select Next Paragraph",
            "Previous Sentence",
            "Next Sentence",
            "Select Previous Sentence",
            "Select Next Sentence",
            "Select Sentence",
            "Delete Sentence",
        ]
    );
    set_selections(&ws, vcx, &[(0, 0)]);
    for label in ["Next Sentence", "Select Sentence", "Delete Sentence"] {
        let action = submenu(&ws, vcx, "Edit", "Prose")
            .into_iter()
            .find_map(|item| match item {
                MenuItem::Action { name, action, .. } if name.as_ref() == label => Some(action),
                _ => None,
            })
            .unwrap();
        vcx.update(|window, cx| window.dispatch_action(action, cx));
        vcx.run_until_parked();
        match label {
            "Next Sentence" => assert_eq!(sels(&ws, vcx), [Selection::caret(5)]),
            "Select Sentence" => assert_eq!(sels(&ws, vcx), [Selection::new(5, 10)]),
            _ => assert_eq!(text(&ws, vcx), "One. Three tail"),
        }
    }
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "One. Two! Three tail");
    assert_eq!(sels(&ws, vcx), [Selection::new(5, 10)]);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "One. Three tail");
}

#[gpui::test]
fn prose_custom_keymap_records_reconstructs_and_replays_all_ten_commands(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, Recorder};
    let (ws, vcx) = window(cx);
    let source = "One. Two!\n\nThree tail";
    type_in(&ws, vcx, source);
    let definitions = [
        ("1", "editor::Move", serde_json::json!({"motion":"paragraph_next","select":false})),
        ("2", "editor::Move", serde_json::json!({"motion":"paragraph_previous","select":false})),
        ("3", "editor::Move", serde_json::json!({"motion":"paragraph_next","select":true})),
        ("4", "editor::Move", serde_json::json!({"motion":"paragraph_previous","select":true})),
        ("5", "editor::Move", serde_json::json!({"motion":"sentence_next","select":false})),
        ("6", "editor::Move", serde_json::json!({"motion":"sentence_previous","select":false})),
        ("7", "editor::Move", serde_json::json!({"motion":"sentence_next","select":true})),
        ("8", "editor::Move", serde_json::json!({"motion":"sentence_previous","select":true})),
        ("9", "editor::SelectSentence", serde_json::json!({})),
        ("0", "editor::DeleteSentence", serde_json::json!({})),
    ];
    let bindings: Vec<_> = definitions
        .iter()
        .map(|(key, command, args)| {
            serde_json::json!({
                "keys":[format!("cmd-alt-{key}")],"command":command,"args":args,"context":"Editor"
            })
        })
        .collect();
    std::fs::write(crate::config::customization::keymap_path(), serde_json::to_string(&bindings).unwrap()).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    assert_eq!(vcx.pending_prompt(), None);
    vcx.simulate_keystrokes("cmd-up alt-right");
    assert_eq!(sels(&ws, vcx), [Selection::caret(3)], "default word motion remains unchanged");
    vcx.simulate_keystrokes("cmd-up");
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    for (key, _, _) in &definitions {
        vcx.simulate_keystrokes(&format!("cmd-alt-{key}"));
    }
    assert_eq!(text(&ws, vcx), "Two!\n\nThree tail");
    let recorded = vcx.update(|_, cx| {
        let recorder = cx.default_global::<Recorder>();
        recorder.recording = false;
        recorder.commands.clone()
    });
    let expected: Vec<_> =
        definitions.into_iter().map(|(_, command, args)| Command { command: command.into(), args }).collect();
    assert_eq!(recorded, expected);
    let saved = serde_json::to_string(&recorded).unwrap();
    let loaded = vcx.update(|_, cx| crate::config::customization::parse_macro(&saved, cx).unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, vcx, source);
    set_selections(&ws, vcx, &[(0, 0)]);
    ws.update_in(vcx, |ws, window, cx| {
        cx.default_global::<Recorder>().last = loaded;
        ws.replay_macro(&ReplayMacro, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "Two!\n\nThree tail");
    assert_eq!(sels(&ws, vcx), [Selection::caret(0)]);
}

#[gpui::test]
fn prose_actions_catch_up_before_motion_selection_and_delete_in_same_update(cx: &mut TestAppContext) {
    use crate::editing::text::Motion;
    for operation in ["motion", "selection", "delete"] {
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, "One. Two! Three tail");
        new_view(&ws, vcx);
        let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
        set_selections(&ws, vcx, &[(6, 6)]);
        ws.update_in(vcx, |_, window, cx| {
            views[0].update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.set_cursor(0);
                    s.insert("New. ");
                })
            });
            views[1].update(cx, |e, cx| {
                assert_eq!(e.state.cursor(), 6, "event delivery is still queued");
                let action: Box<dyn gpui::Action> = match operation {
                    "motion" => Box::new(editor::Move { motion: Motion::SentencePrevious, select: false }),
                    "selection" => Box::new(editor::SelectSentence),
                    _ => Box::new(editor::DeleteSentence),
                };
                assert!(e.perform_action(action.as_ref(), window, cx));
                match operation {
                    "motion" => assert_eq!(e.state.selections, [Selection::caret(10)]),
                    "selection" => assert_eq!(e.state.selections, [Selection::new(10, 15)]),
                    _ => assert_eq!(e.text(cx), "New. One. Three tail"),
                }
            });
        });
        if operation == "delete" {
            assert_eq!(views[0].read_with(vcx, |e, cx| e.text(cx)), "New. One. Three tail");
            vcx.dispatch_action(editor::Undo);
            assert_eq!(text(&ws, vcx), "New. One. Two! Three tail");
            assert_eq!(sels(&ws, vcx), [Selection::caret(11)]);
            vcx.dispatch_action(editor::Undo);
            assert_eq!(text(&ws, vcx), "One. Two! Three tail");
        }
    }
}

#[gpui::test]
fn n06_transpose_words_registered_action_swaps_words_and_preserves_separator(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "alpha, beta");
    set_selections(&ws, vcx, &[(2, 2)]);
    let action = vcx.update(|_, cx| cx.build_action("editor::TransposeWords", Some(serde_json::json!({}))));
    assert!(action.is_ok(), "N06 is missing its distinct recordable Transpose Words action");
    vcx.update(|window, cx| window.dispatch_action(action.unwrap(), cx));
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "beta, alpha");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "alpha, beta");
    assert_eq!(sels(&ws, vcx), [Selection::caret(2)]);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "beta, alpha");
}

#[gpui::test]
fn n06_transpose_words_native_menu_custom_binding_and_real_macro_replay(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, Recorder};
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "alpha, beta");
    set_selections(&ws, vcx, &[(2, 2)]);
    let action = submenu(&ws, vcx, "Edit", "Text")
        .into_iter()
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name.as_ref() == "Transpose Words" => Some(action),
            _ => None,
        })
        .unwrap();
    assert!(action.as_any().is::<editor::TransposeWords>());
    vcx.update(|window, cx| window.dispatch_action(action, cx));
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "beta, alpha");
    vcx.dispatch_action(editor::Undo);
    let binding = serde_json::json!([
        {"keys":["cmd-alt-t"],"command":"editor::TransposeWords","context":"Editor"}
    ]);
    std::fs::write(crate::config::customization::keymap_path(), binding.to_string()).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    assert_eq!(vcx.pending_prompt(), None);
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    vcx.simulate_keystrokes("cmd-alt-t");
    assert_eq!(text(&ws, vcx), "beta, alpha");
    let recorded = vcx.update(|_, cx| {
        let recorder = cx.default_global::<Recorder>();
        recorder.recording = false;
        recorder.commands.clone()
    });
    assert_eq!(recorded, [Command { command: "editor::TransposeWords".into(), args: serde_json::json!({}) }]);
    let saved = serde_json::to_string(&recorded).unwrap();
    let loaded = vcx.update(|_, cx| crate::config::customization::parse_macro(&saved, cx).unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, vcx, "one\t,\n二三");
    set_selections(&ws, vcx, &[(1, 1)]);
    ws.update_in(vcx, |_, _, cx| cx.default_global::<Recorder>().last = loaded);
    vcx.dispatch_action(ReplayMacro);
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "二三\t,\none");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "one\t,\n二三");
}

#[gpui::test]
fn n06_transpose_words_catches_up_shared_revision_before_preparing_targets(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "alpha beta gamma");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    set_selections(&ws, vcx, &[(2, 2)]);
    ws.update_in(vcx, |_, window, cx| {
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("new ");
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.cursor(), 2, "revision publication is still queued");
            assert!(e.perform_action(&editor::TransposeWords, window, cx));
            assert_eq!(e.text(cx), "new beta alpha gamma");
        });
    });
    assert_eq!(views[0].read_with(vcx, |e, cx| e.text(cx)), "new beta alpha gamma");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "new alpha beta gamma");
    assert_eq!(sels(&ws, vcx), [Selection::caret(6)]);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "new beta alpha gamma");
}

#[gpui::test]
fn n07_registered_surround_actions_replace_and_remove_existing_pairs(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, CommandAction};
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "(value)");
    set_selections(&ws, vcx, &[(3, 3)]);
    for (name, args, expected) in [
        ("editor::ChangeSurround", serde_json::json!({"target":"(","replacement":"["}), "[value]"),
        ("editor::DeleteSurround", serde_json::json!({"target":"["}), "value"),
    ] {
        let action = vcx.update(|_, cx| Command { command: name.into(), args }.build(cx));
        assert!(action.is_ok(), "N07 is missing its typed recordable {name} action");
        vcx.update(|window, cx| window.dispatch_action(action.unwrap(), cx));
        vcx.run_until_parked();
        assert_eq!(text(&ws, vcx), expected);
    }
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "[value]");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "(value)");
    assert_eq!(sels(&ws, vcx), [Selection::caret(3)]);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "[value]");
}

#[gpui::test]
fn n07_surrounds_native_forms_validate_count_record_action_and_refuse_stale(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, Recorder};
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "((value))");
    set_selections(&ws, vcx, &[(4, 4)]);
    let action = submenu(&ws, vcx, "Edit", "Text")
        .into_iter()
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name.as_ref() == "Change Surround…" => Some(action),
            _ => None,
        })
        .unwrap();
    assert!(action.as_any().is::<ChangeSurroundPrompt>());
    vcx.update(|window, cx| window.dispatch_action(action, cx));
    vcx.run_until_parked();
    for invalid in ["0", "-1", "1.5", "18446744073709551616"] {
        vcx.simulate_keystrokes("cmd-a");
        submit_form(vcx, invalid);
        assert!(form_error(&ws, vcx).unwrap().is_some());
        assert_eq!(text(&ws, vcx), "((value))");
    }
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("2");
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    submit_form(vcx, "");
    assert_eq!(form_error(&ws, vcx), None);
    assert_eq!(text(&ws, vcx), "[(value)]");
    let recorded = vcx.update(|_, cx| {
        let recorder = cx.default_global::<Recorder>();
        recorder.recording = false;
        recorder.commands.clone()
    });
    assert_eq!(
        recorded,
        [Command {
            command: "editor::ChangeSurround".into(),
            args: serde_json::json!({"target":"(","replacement":"[","count":2})
        }]
    );
    vcx.dispatch_action(editor::Undo);
    vcx.dispatch_action(DeleteSurroundPrompt);
    vcx.run_until_parked();
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "(value)", "default count1 removes one innermost pair");
    vcx.dispatch_action(editor::Undo);
    for source_change in [false, true] {
        vcx.dispatch_action(ChangeSurroundPrompt);
        vcx.run_until_parked();
        on_editor(&ws, vcx, |e, _, cx| {
            if source_change {
                e.edit(cx, |s| {
                    s.set_cursor(0);
                    s.insert("new ");
                });
            } else {
                let at = e.state.cursor();
                e.sel(cx).set_cursor(0);
                e.sel(cx).set_cursor(at);
            }
        });
        let before = text(&ws, vcx);
        submit_form(vcx, "");
        assert_eq!(text(&ws, vcx), before);
        assert!(form_error(&ws, vcx).unwrap().unwrap().contains("changed"));
        vcx.simulate_keystrokes("escape");
    }
}

#[gpui::test]
fn n07_surrounds_custom_bindings_and_actual_replay_validate_typed_parameters(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, CommandAction, Recorder};
    let (ws, vcx) = window(cx);
    for args in [
        serde_json::json!({"target":"x"}),
        serde_json::json!({"target":"(","count":0}),
        serde_json::json!({"target":"(","count":-1}),
        serde_json::json!({"target":"(","count":1.5}),
    ] {
        assert!(vcx.update(|_, cx| Command { command: "editor::DeleteSurround".into(), args }.build(cx)).is_err());
    }
    assert!(vcx
        .update(|_, cx| Command { command: "editor::DeleteSurround".into(), args: serde_json::json!({"target":")"}) }
            .build(cx))
        .is_ok());
    let binding = serde_json::json!([
        {"keys":["cmd-alt-t"],"command":"editor::ChangeSurround","args":{"target":"(","replacement":"["},"context":"Editor"},
        {"keys":["cmd-alt-d"],"command":"editor::DeleteSurround","args":{"target":"["},"context":"Editor"}
    ]);
    std::fs::write(crate::config::customization::keymap_path(), binding.to_string()).unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
    assert_eq!(vcx.pending_prompt(), None);
    type_in(&ws, vcx, "(value)");
    set_selections(&ws, vcx, &[(3, 3)]);
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    vcx.simulate_keystrokes("cmd-alt-t cmd-alt-d");
    assert_eq!(text(&ws, vcx), "value");
    let recorded = vcx.update(|_, cx| {
        let r = cx.default_global::<Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].command, "editor::ChangeSurround");
    assert_eq!(recorded[1].command, "editor::DeleteSurround");
    let loaded = vcx.update(|_, cx| {
        crate::config::customization::parse_macro(&serde_json::to_string(&recorded).unwrap(), cx).unwrap()
    });
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, vcx, "(二三)");
    set_selections(&ws, vcx, &[(2, 2)]);
    vcx.update(|_, cx| cx.default_global::<Recorder>().last = loaded);
    vcx.dispatch_action(ReplayMacro);
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "二三");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "[二三]");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "(二三)");
}

#[gpui::test]
fn n07_surrounds_shared_revision_catches_up_before_preparation(cx: &mut TestAppContext) {
    use crate::editing::text::Surround;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "(value)");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    set_selections(&ws, vcx, &[(3, 3)]);
    ws.update_in(vcx, |_, window, cx| {
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("new ");
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.cursor(), 3);
            e.perform_action(
                &editor::ChangeSurround {
                    target: Surround::Round,
                    replacement: Surround::Square,
                    count: std::num::NonZeroUsize::MIN,
                },
                window,
                cx,
            );
            assert_eq!(e.text(cx), "new [value]");
        });
    });
    assert_eq!(views[0].read_with(vcx, |e, cx| e.text(cx)), "new [value]");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "new (value)");
    assert_eq!(sels(&ws, vcx), [Selection::caret(7)]);
}

#[gpui::test]
fn n08_selection_set_actual_commands_and_selection_undo_preserve_text(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, CommandAction};
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a,b,c");
    set_selections(&ws, vcx, &[(5, 0)]);
    for (command, args, expected) in [
        ("editor::SplitSelectionsByRegex", serde_json::json!({"pattern":","}), vec![(1, 0), (3, 2), (5, 4)]),
        ("editor::KeepSelectionsMatchingRegex", serde_json::json!({"pattern":"b"}), vec![(3, 2)]),
        ("editor::RemoveMatchingSelections", serde_json::json!({"pattern":"b"}), vec![(2, 2)]),
        ("editor::InvertSelections", serde_json::json!({}), vec![(0, 5)]),
    ] {
        let cmd = Command { command: command.into(), args };
        assert!(vcx.update(|_, cx| cmd.build(cx)).is_ok(), "{command} must use actual catalog");
        ws.update_in(vcx, |ws, window, cx| ws.execute_command(&cmd, window, cx).unwrap());
        let expected: Vec<_> = expected.into_iter().map(|(a, h)| Selection::new(a, h)).collect();
        assert_eq!(sels(&ws, vcx), expected, "{command}");
        assert_eq!(text(&ws, vcx), "a,b,c");
    }
    vcx.dispatch_action(editor::UndoSelection);
    assert_eq!(sels(&ws, vcx), [Selection::caret(2)]);
    vcx.dispatch_action(editor::RedoSelection);
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 5)]);
}

#[gpui::test]
fn n08_selection_regex_forms_are_reachable_and_record_resolved_actions(cx: &mut TestAppContext) {
    use crate::keybindings::{CommandAction, Recorder};
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a,b,c");
    set_selections(&ws, vcx, &[(0, 5)]);
    let prompt = vcx.update(|_, cx| {
        crate::keybindings::Command {
            command: "workspace::SplitSelectionsByRegexPrompt".into(),
            args: serde_json::json!({}),
        }
        .build(cx)
        .unwrap()
    });
    vcx.update(|window, cx| window.dispatch_action(prompt, cx));
    vcx.run_until_parked();
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    submit_form(vcx, ",");
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 1), Selection::new(2, 3), Selection::new(4, 5)]);
    let commands = vcx.update(|_, cx| {
        let r = cx.default_global::<Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].command, "editor::SplitSelectionsByRegex");
    assert_eq!(commands[0].args["pattern"], ",");
}

#[gpui::test]
fn n08_native_menu_forms_bindings_and_actual_replay(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    assert_eq!(
        names(&submenu(&ws, vcx, "Selection", "Selection Sets")),
        [
            "Invert Selections",
            "Split Selections by Regex…",
            "Keep Selections Matching Regex…",
            "Remove Matching Selections…"
        ]
    );
    type_in(&ws, vcx, "a,b,c");
    set_selections(&ws, vcx, &[(0, 5)]);
    for (label, pattern, expected) in [
        ("Split Selections by Regex…", ",", vec![(0, 1), (2, 3), (4, 5)]),
        ("Keep Selections Matching Regex…", "b|c", vec![(2, 3), (4, 5)]),
        ("Remove Matching Selections…", "c", vec![(2, 3)]),
    ] {
        let action = submenu(&ws, vcx, "Selection", "Selection Sets")
            .into_iter()
            .find_map(|m| match m {
                MenuItem::Action { name, action, .. } if name.as_ref() == label => Some(action),
                _ => None,
            })
            .unwrap();
        vcx.update(|w, cx| w.dispatch_action(action, cx));
        vcx.run_until_parked();
        submit_form(vcx, pattern);
        assert_eq!(sels(&ws, vcx), expected.into_iter().map(|(a, h)| Selection::new(a, h)).collect::<Vec<_>>());
    }
    let invert = submenu(&ws, vcx, "Selection", "Selection Sets")
        .into_iter()
        .find_map(|m| match m {
            MenuItem::Action { name, action, .. } if name == "Invert Selections" => Some(action),
            _ => None,
        })
        .unwrap();
    vcx.update(|w, cx| w.dispatch_action(invert, cx));
    vcx.run_until_parked();
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 2), Selection::new(3, 5)]);
    let binding = serde_json::json!([
        {"keys":["cmd-alt-i"],"command":"editor::InvertSelections","context":"Editor"},
        {"keys":["cmd-alt-k"],"command":"editor::KeepSelectionsMatchingRegex","args":{"pattern":"^a$"},"context":"Editor"},
        {"keys":["cmd-alt-r"],"command":"editor::RemoveMatchingSelections","args":{"pattern":"a"},"context":"Editor"},
        {"keys":["cmd-alt-s"],"command":"editor::SplitSelectionsByRegex","args":{"pattern":","},"context":"Editor"}
    ]);
    std::fs::write(crate::config::customization::keymap_path(), binding.to_string()).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    set_selections(&ws, vcx, &[(0, 5)]);
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    vcx.simulate_keystrokes("cmd-alt-s cmd-alt-k cmd-alt-r cmd-alt-i");
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 5)]);
    let commands = vcx.update(|_, cx| {
        let r = cx.default_global::<Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(commands.len(), 4);
    let loaded = vcx.update(|_, cx| {
        crate::config::customization::parse_macro(&serde_json::to_string(&commands).unwrap(), cx).unwrap()
    });
    set_selections(&ws, vcx, &[(2, 3)]);
    vcx.update(|_, cx| cx.default_global::<Recorder>().last = loaded);
    vcx.dispatch_action(ReplayMacro);
    vcx.run_until_parked();
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 5)]);
    assert_eq!(text(&ws, vcx), "a,b,c");
}

#[gpui::test]
fn n08_regex_form_rejects_syntax_source_and_away_back_staleness(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a,b,c");
    set_selections(&ws, vcx, &[(0, 5)]);
    vcx.dispatch_action(SplitSelectionsByRegexPrompt);
    vcx.run_until_parked();
    submit_form(vcx, "[");
    assert!(form_error(&ws, vcx).unwrap().is_some());
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 5)]);
    vcx.simulate_keystrokes("escape");
    for source_change in [false, true] {
        vcx.dispatch_action(SplitSelectionsByRegexPrompt);
        vcx.run_until_parked();
        on_editor(&ws, vcx, |e, _, cx| {
            if source_change {
                e.edit(cx, |s| s.insert("x"));
            } else {
                e.sel(cx).set_cursor(0);
                e.sel(cx).set_selection(0, 5);
            }
        });
        let before = (text(&ws, vcx), sels(&ws, vcx));
        submit_form(vcx, ",");
        assert_eq!((text(&ws, vcx), sels(&ws, vcx)), before);
        assert!(form_error(&ws, vcx).unwrap().unwrap().contains("changed"));
        vcx.simulate_keystrokes("escape");
    }
}

#[gpui::test]
fn n08_shared_composition_refuses_native_and_bound_selection_commands(cx: &mut TestAppContext) {
    use crate::keybindings::Command;
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a,b,c");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    views[0].update_in(vcx, |e, w, cx| e.replace_and_mark_text_in_range(None, "中", None, w, cx));
    let before = text(&ws, vcx);
    for command in [
        "editor::InvertSelections",
        "editor::SplitSelectionsByRegex",
        "editor::KeepSelectionsMatchingRegex",
        "editor::RemoveMatchingSelections",
    ] {
        let old = sels(&ws, vcx);
        let error = ws
            .update_in(vcx, |ws, w, cx| {
                ws.execute_command(
                    &Command {
                        command: command.into(),
                        args: if command.ends_with("InvertSelections") {
                            serde_json::json!({})
                        } else {
                            serde_json::json!({"pattern":","})
                        },
                    },
                    w,
                    cx,
                )
            })
            .unwrap_err();
        assert!(error.contains("composition"));
        assert_eq!(sels(&ws, vcx), old);
    }
    vcx.dispatch_action(SplitSelectionsByRegexPrompt);
    vcx.run_until_parked();
    assert!(!ws.read_with(vcx, |ws, _| matches!(ws.panel, Some(Panel::Form(_)))));
    assert_eq!(text(&ws, vcx), before);
    assert!(!views[0].read_with(vcx, |e, _| e.marked.is_empty()));
}

#[gpui::test]
fn n08_pending_form_refuses_later_shared_edit_and_ime(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a,b,c");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    set_selections(&ws, vcx, &[(0, 5)]);
    for ime in [true, false] {
        vcx.dispatch_action(SplitSelectionsByRegexPrompt);
        vcx.run_until_parked();
        views[0].update_in(vcx, |e, w, cx| {
            if ime {
                e.replace_and_mark_text_in_range(None, "中", None, w, cx);
            } else {
                e.edit(cx, |s| {
                    s.set_cursor(0);
                    s.insert("x");
                });
            }
        });
        let before = text(&ws, vcx);
        submit_form(vcx, ",");
        assert_eq!(text(&ws, vcx), before);
        let error = form_error(&ws, vcx).unwrap().unwrap();
        assert!(error.contains(if ime { "composition" } else { "changed" }));
        vcx.simulate_keystrokes("escape");
        if ime {
            views[0].update(vcx, |e, _| e.marked.clear());
        }
    }
}

#[gpui::test]
fn n08_shared_revision_catches_up_and_selection_changes_keep_real_text_redo(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a,b,c");
    vcx.dispatch_action(editor::Undo);
    vcx.dispatch_action(editor::InvertSelections);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "a,b,c", "selection command preserves the actual source redo branch");
    new_view(&ws, vcx);
    set_selections(&ws, vcx, &[(0, 5)]);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    ws.update_in(vcx, |_, w, cx| {
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("new ");
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.selections, [Selection::new(0, 5)], "revision publication is queued");
            e.perform_action(&editor::SplitSelectionsByRegex { pattern: ",".into(), ignore_case: false }, w, cx);
            assert_eq!(e.state.selections, [Selection::new(0, 5), Selection::new(6, 7), Selection::new(8, 9)]);
            assert_eq!(e.text(cx), "new a,b,c");
        });
    });
    vcx.dispatch_action(editor::UndoSelection);
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 9)]);
    assert_eq!(text(&ws, vcx), "new a,b,c");
}

#[gpui::test]
fn n08_crlf_split_form_reports_atomic_failure_and_accepts_whole_line_ending(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a\r\nb");
    assert_eq!(text(&ws, vcx), "a\r\nb");
    set_selections(&ws, vcx, &[(0, 4)]);
    for pattern in [r"\r", "", r"(?=\n)"] {
        vcx.dispatch_action(SplitSelectionsByRegexPrompt);
        vcx.run_until_parked();
        let before = on_editor(&ws, vcx, |e, _, cx| {
            (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.state.selections.clone(),
                e.read_state(cx).selection_generation(),
            )
        });
        submit_form(vcx, pattern);
        vcx.run_until_parked();
        assert_eq!(
            on_editor(&ws, vcx, |e, _, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.state.selections.clone(),
                e.read_state(cx).selection_generation()
            )),
            before
        );
        assert!(ws.read_with(vcx, |ws, _| ws.status.as_ref().unwrap().0.contains("CRLF")));
    }
    vcx.dispatch_action(SplitSelectionsByRegexPrompt);
    vcx.run_until_parked();
    submit_form(vcx, r"\r\n");
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 1), Selection::new(3, 4)]);
    assert_eq!(text(&ws, vcx), "a\r\nb");
    vcx.dispatch_action(editor::UndoSelection);
    assert_eq!(sels(&ws, vcx), [Selection::new(0, 4)]);
}

#[gpui::test]
fn n09_actual_command_build_adjusts_all_original_values_in_one_undo(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, CommandAction};
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "009 0x0f -001");
    set_selections(&ws, vcx, &[(3, 0), (4, 8), (9, 13)]);
    let original = sels(&ws, vcx);
    let command = Command { command: "editor::AdjustValues".into(), args: serde_json::json!({"step":"1"}) };
    assert!(vcx.update(|_, cx| command.build(cx)).is_ok(), "N09 typed action is missing");
    ws.update_in(vcx, |ws, w, cx| ws.execute_command(&command, w, cx).unwrap());
    assert_eq!(text(&ws, vcx), "010 0x10 000");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "009 0x0f -001");
    assert_eq!(sels(&ws, vcx), original);
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "010 0x10 000");
}

#[gpui::test]
fn n09_native_palette_and_form_defaults_validate_and_record_resolved_step(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "009");
    set_selections(&ws, vcx, &[(0, 3)]);
    for label in ["Increment Values", "Decrement Values", "Adjust Values…"] {
        assert!(quick_panel::command_items()
            .iter()
            .any(|item| item.label == format!("Edit: {label}") && item.hint.is_empty()));
        let action = submenu(&ws, vcx, "Edit", "Text")
            .into_iter()
            .find_map(|item| match item {
                MenuItem::Action { name, action, .. } if name.as_ref() == label => Some(action),
                _ => None,
            })
            .unwrap();
        vcx.update(|w, cx| w.dispatch_action(action, cx));
        vcx.run_until_parked();
        if label == "Increment Values" {
            assert_eq!(text(&ws, vcx), "010");
        }
        if label == "Decrement Values" {
            assert_eq!(text(&ws, vcx), "009");
        }
    }
    for invalid in ["", "1.5", " 1", "170141183460469231731687303715884105728"] {
        vcx.simulate_keystrokes("cmd-a backspace");
        submit_form(vcx, invalid);
        assert!(form_error(&ws, vcx).flatten().is_some());
        assert_eq!(text(&ws, vcx), "009");
    }
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("-2");
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "007");
    assert_eq!(form_error(&ws, vcx), None);
    let recorded = vcx.update(|_, cx| {
        let r = cx.default_global::<Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].command, "editor::AdjustValues");
    assert_eq!(recorded[0].args, serde_json::json!({"step":"-2"}));
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "009");
    vcx.dispatch_action(AdjustValuesPrompt);
    vcx.run_until_parked();
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "010", "form default is +1");
}

#[gpui::test]
fn n09_form_refuses_source_changes_and_selection_away_back(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "009");
    set_selections(&ws, vcx, &[(0, 3)]);
    for change_text in [false, true] {
        vcx.dispatch_action(AdjustValuesPrompt);
        vcx.run_until_parked();
        on_editor(&ws, vcx, |e, _, cx| {
            if change_text {
                e.edit(cx, |s| {
                    s.set_cursor(3);
                    s.insert("0");
                });
            } else {
                e.sel(cx).set_cursor(0);
                e.sel(cx).set_selection(0, 3);
            }
        });
        let before = text(&ws, vcx);
        submit_form(vcx, "");
        assert_eq!(text(&ws, vcx), before);
        assert!(form_error(&ws, vcx).flatten().unwrap().contains("changed"));
        vcx.simulate_keystrokes("escape");
    }
}

#[gpui::test]
fn n09_custom_bindings_build_and_public_macro_replay_keep_extreme_steps_lossless(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, CommandAction, Recorder};
    let (ws, vcx) = window(cx);
    for args in [
        serde_json::json!({"step":1}),
        serde_json::json!({"step":1.5}),
        serde_json::json!({"step":"1.0"}),
        serde_json::json!({"step":"170141183460469231731687303715884105728"}),
    ] {
        assert!(vcx.update(|_, cx| Command { command: "editor::AdjustValues".into(), args }.build(cx)).is_err());
    }
    let binding = serde_json::json!([
        {"keys":["cmd-alt-i"],"command":"editor::AdjustValues","args":{"step":i128::MAX.to_string()},"context":"Editor"},
        {"keys":["cmd-alt-d"],"command":"editor::AdjustValues","args":{"step":i128::MIN.to_string()},"context":"Editor"}
    ]);
    std::fs::write(crate::config::customization::keymap_path(), binding.to_string()).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "0");
    set_selections(&ws, vcx, &[(0, 1)]);
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    vcx.simulate_keystrokes("cmd-alt-i");
    assert_eq!(text(&ws, vcx), i128::MAX.to_string());
    vcx.simulate_keystrokes("cmd-alt-d");
    assert_eq!(text(&ws, vcx), format!("-{:039}", 1));
    let recorded = vcx.update(|_, cx| {
        let r = cx.default_global::<Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].args["step"], i128::MAX.to_string());
    assert_eq!(recorded[1].args["step"], i128::MIN.to_string());
    let loaded = vcx.update(|_, cx| {
        crate::config::customization::parse_macro(&serde_json::to_string(&recorded).unwrap(), cx).unwrap()
    });
    ws.update_in(vcx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
    type_in(&ws, vcx, "0");
    set_selections(&ws, vcx, &[(0, 1)]);
    vcx.update(|_, cx| cx.default_global::<Recorder>().last = loaded);
    vcx.dispatch_action(ReplayMacro);
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), format!("-{:039}", 1));
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), i128::MAX.to_string());
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "0");
    ws.update_in(vcx, |ws, w, cx| {
        ws.execute_command(&Command { command: "editor::AdjustValues".into(), args: serde_json::json!({}) }, w, cx)
            .unwrap()
    });
    assert_eq!(text(&ws, vcx), "1", "typed default is +1");
    vcx.dispatch_action(editor::Undo);
    ws.update_in(vcx, |ws, w, cx| {
        ws.execute_command(
            &Command {
                command: "editor::AdjustValues".into(),
                args: serde_json::json!({"step":i128::MIN.to_string()}),
            },
            w,
            cx,
        )
        .unwrap()
    });
    assert_eq!(text(&ws, vcx), i128::MIN.to_string(), "MIN step itself executes losslessly");
}

#[gpui::test]
fn n09_shared_revision_is_caught_up_and_shared_ime_refuses_all_entry_points(cx: &mut TestAppContext) {
    use crate::keybindings::Command;
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a 009");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    set_selections(&ws, vcx, &[(2, 5)]);
    ws.update_in(vcx, |_, w, cx| {
        views[0].update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("x ");
            })
        });
        views[1].update(cx, |e, cx| {
            assert_eq!(e.state.selections[0].range(), 2..5);
            e.perform_action(&editor::AdjustValues { step: crate::editing::columns::ValueStep(1) }, w, cx);
            assert_eq!(e.text(cx), "x a 010");
        });
    });
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "x a 009");
    views[0].update_in(vcx, |e, w, cx| {
        let len = e.doc(cx).len();
        e.sel(cx).set_cursor(len);
        e.replace_and_mark_text_in_range(None, "中", None, w, cx);
    });
    let before = text(&ws, vcx);
    let selections = sels(&ws, vcx);
    let error = ws
        .update_in(vcx, |ws, w, cx| {
            ws.execute_command(
                &Command { command: "editor::AdjustValues".into(), args: serde_json::json!({"step":"1"}) },
                w,
                cx,
            )
        })
        .unwrap_err();
    assert!(error.contains("composition"));
    vcx.dispatch_action(IncrementValues);
    vcx.dispatch_action(DecrementValues);
    vcx.dispatch_action(AdjustValuesPrompt);
    vcx.run_until_parked();
    assert!(!ws.read_with(vcx, |ws, _| matches!(ws.panel, Some(Panel::Form(_)))));
    assert_eq!(text(&ws, vcx), before);
    assert_eq!(sels(&ws, vcx), selections);
    assert!(!views[0].read_with(vcx, |e, _| e.marked.is_empty()));
}

#[gpui::test]
fn n09_pending_form_refuses_later_shared_ime(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "009");
    new_view(&ws, vcx);
    set_selections(&ws, vcx, &[(0, 3)]);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    vcx.dispatch_action(AdjustValuesPrompt);
    vcx.run_until_parked();
    views[0].update_in(vcx, |e, w, cx| {
        let len = e.doc(cx).len();
        e.sel(cx).set_cursor(len);
        e.replace_and_mark_text_in_range(None, "中", None, w, cx);
    });
    let before = text(&ws, vcx);
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), before);
    assert!(form_error(&ws, vcx).flatten().unwrap().contains("composition"));
    assert!(!views[0].read_with(vcx, |e, _| e.marked.is_empty()));
}

#[gpui::test]
fn n09_pipeline_failure_prepares_before_ime_history_or_selection_changes(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "009 bad");
    let path = write_pipeline("adjust", r#"{"op":"adjust_values","step":"1"}"#);
    on_editor(&ws, vcx, |e, w, cx| {
        let len = e.doc(cx).len();
        e.sel(cx).set_cursor(len);
        e.replace_and_mark_text_in_range(None, "中", None, w, cx);
        e.sel(cx).set_selections(vec![Selection::new(0, 3), Selection::new(4, 7)]);
    });
    let before = on_editor(&ws, vcx, |e, _, cx| {
        (
            e.text(cx),
            e.read_state(cx).selection_state(),
            e.read_state(cx).selection_generation(),
            e.doc(cx).version,
            e.doc(cx).save_point(),
            e.marked.clone(),
        )
    });
    ws.update_in(vcx, |ws, w, cx| ws.use_pipeline(&UsePipeline { path, how: PipelineUse::Run }, w, cx));
    assert!(
        on_editor(&ws, vcx, |e, _, cx| (
            e.text(cx),
            e.read_state(cx).selection_state(),
            e.read_state(cx).selection_generation(),
            e.doc(cx).version,
            e.doc(cx).save_point(),
            e.marked.clone()
        )) == before,
        "N09 pipeline failure must leave local composition and full state intact"
    );
}

#[gpui::test]
fn n09_pipeline_preserves_scope_order_forward_targets_lf_and_empty_last_line_rule(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a 0x0f b 009");
    set_selections(&ws, vcx, &[(6, 2), (12, 9)]);
    let path = write_pipeline("adjust", r#"{"op":"convert_case","case":"upper"},{"op":"adjust_values","step":"1"}"#);
    ws.update_in(vcx, |ws, w, cx| ws.use_pipeline(&UsePipeline { path, how: PipelineUse::Run }, w, cx));
    assert_eq!(text(&ws, vcx), "a 0X10 b 010");
    assert_eq!(sels(&ws, vcx), [Selection::new(2, 6), Selection::new(9, 12)]);
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "a 0x0f b 009");
    assert_eq!(sels(&ws, vcx), [Selection::new(6, 2), Selection::new(12, 9)]);
    ws.update_in(vcx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
    type_in(&ws, vcx, "first\n0x0f");
    set_selections(&ws, vcx, &[(8, 9)]);
    let path = write_pipeline(
        "empty",
        r#"{"op":"remove_line_prefix","text":"0x"},{"op":"add_line_prefix","text":"0x"},{"op":"adjust_values","step":"1"},{"op":"apply_substitution_table","rows":[["0x10",""]]}"#,
    );
    ws.update_in(vcx, |ws, w, cx| ws.use_pipeline(&UsePipeline { path, how: PipelineUse::Run }, w, cx));
    assert_eq!(text(&ws, vcx), "first", "widened last line deletion removes its preceding newline");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "first\n0x0f");
    assert_eq!(sels(&ws, vcx), [Selection::new(8, 9)]);
    ws.update_in(vcx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
    type_in(&ws, vcx, "009");
    let path = write_pipeline(
        "lf",
        r#"{"op":"adjust_values","step":"1"},{"op":"apply_substitution_table","rows":[["010","a\r\nb"]]}"#,
    );
    ws.update_in(vcx, |ws, w, cx| ws.use_pipeline(&UsePipeline { path, how: PipelineUse::Run }, w, cx));
    assert_eq!(text(&ws, vcx), "a\nb");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "009");
}

#[gpui::test]
fn n09_pipeline_noop_preserves_real_redo_and_virtual_only_targets(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "009");
    set_selections(&ws, vcx, &[(0, 3)]);
    vcx.dispatch_action(IncrementValues);
    vcx.dispatch_action(editor::Undo);
    let before = on_editor(&ws, vcx, |e, _, cx| {
        (
            e.read_state(cx).selection_state(),
            e.read_state(cx).selection_generation(),
            e.doc(cx).version,
            e.doc(cx).save_point(),
        )
    });
    let path = write_pipeline("zero", r#"{"op":"adjust_values","step":"0"}"#);
    ws.update_in(vcx, |ws, w, cx| ws.use_pipeline(&UsePipeline { path, how: PipelineUse::Run }, w, cx));
    assert!(
        on_editor(&ws, vcx, |e, _, cx| (
            e.read_state(cx).selection_state(),
            e.read_state(cx).selection_generation(),
            e.doc(cx).version,
            e.doc(cx).save_point()
        )) == before
    );
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "010");
    on_editor(&ws, vcx, |e, _, cx| {
        e.sel(cx).set_virtual_space(true);
        e.sel(cx).set_selections(vec![Selection::from_points((3, 2), (3, 4))]);
    });
    let before = sels(&ws, vcx);
    let path = write_pipeline("virtual", r#"{"op":"adjust_values"}"#);
    ws.update_in(vcx, |ws, w, cx| ws.use_pipeline(&UsePipeline { path, how: PipelineUse::Run }, w, cx));
    assert_eq!(text(&ws, vcx), "010");
    assert_eq!(sels(&ws, vcx), before);
}

#[gpui::test]
fn n09_typed_binding_and_native_action_refuse_shared_ime_without_losing_redo(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    std::fs::write(
        crate::config::customization::keymap_path(),
        r#"[{"keys":["cmd-alt-i"],"command":"editor::AdjustValues","args":{"step":"1"},"context":"Editor"}]"#,
    )
    .unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "009中");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    on_editor(&ws, vcx, |e, w, cx| {
        let end = e.doc(cx).len();
        e.edit(cx, |s| {
            s.replace_ranges(vec![(end..end, "!".into())]);
        });
        e.perform_action(&editor::Undo, w, cx);
        assert_eq!(e.text(cx), "009中");
    });
    // Native composition of identical text keeps a real source redo branch available.
    views[0].update_in(vcx, |e, w, cx| {
        e.replace_and_mark_text_in_range(Some(3..4), "中", None, w, cx);
        assert!(!e.marked.is_empty());
    });
    on_editor(&ws, vcx, |e, w, cx| {
        e.sel(cx).set_virtual_space(true);
        let mut value = Selection::new(3, 0);
        value.rectangular = true;
        e.sel(cx).set_selections(vec![value, Selection::from_points((4, 2), (4, 5))]);
        e.focus(w);
    });
    vcx.run_until_parked();
    let before = on_editor(&ws, vcx, |e, _, cx| {
        e.sel(cx).sel_checkpoint();
        (
            e.text(cx),
            e.read_state(cx).buffer_id(),
            e.doc(cx).version,
            e.doc(cx).save_point(),
            e.doc(cx).unsaved(),
            e.read_state(cx).selection_state(),
            e.read_state(cx).selection_generation(),
            e.marked.clone(),
        )
    });
    let peer = views[0].read_with(vcx, |e, cx| {
        (e.marked.clone(), e.read_state(cx).selection_state(), e.read_state(cx).selection_generation())
    });
    for keybinding in [true, false] {
        if keybinding {
            vcx.simulate_keystrokes("cmd-alt-i");
        } else {
            vcx.dispatch_action(editor::AdjustValues { step: crate::editing::columns::ValueStep(1) });
        }
        assert!(
            on_editor(&ws, vcx, |e, _, cx| (
                e.text(cx),
                e.read_state(cx).buffer_id(),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.doc(cx).unsaved(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
                e.marked.clone(),
            )) == before,
            "shared composition must refuse native dispatch (keybinding={keybinding})"
        );
        assert!(
            views[0].read_with(vcx, |e, cx| (
                e.marked.clone(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
            )) == peer,
            "the composing peer must stay intact"
        );
    }
    views[0].update_in(vcx, |e, w, cx| e.unmark_text(w, cx));
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "009中!", "a genuine source redo survives refusal");
}

#[gpui::test]
fn n09_native_binding_keeps_focus_scope_and_records_once(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    std::fs::write(
        crate::config::customization::keymap_path(),
        r#"[{"keys":["cmd-alt-i"],"command":"editor::AdjustValues","args":{"step":"1"}}]"#,
    )
    .unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "009");
    set_selections(&ws, vcx, &[(0, 3)]);
    ws.update_in(vcx, |ws, w, _| w.focus(&ws.focus_handle));
    vcx.simulate_keystrokes("cmd-alt-i");
    assert_eq!(text(&ws, vcx), "009", "a globally bound action must not edit an unfocused source");
    on_editor(&ws, vcx, |e, w, _| e.focus(w));
    vcx.run_until_parked();
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    vcx.simulate_keystrokes("cmd-alt-i");
    assert_eq!(text(&ws, vcx), "010");
    let recorded = vcx.update(|_, cx| {
        let recorder = cx.default_global::<Recorder>();
        recorder.recording = false;
        recorder.commands.clone()
    });
    assert_eq!(recorded.len(), 1, "native dispatch retains one resolved macro command");
    assert_eq!(recorded[0].command, "editor::AdjustValues");
    assert_eq!(recorded[0].args, serde_json::json!({"step":"1"}));
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "009");
}

#[gpui::test]
fn n08_native_selection_bindings_refuse_shared_composition_with_full_state_and_redo(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, CommandAction};
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    let commands: Vec<_> = [
        ("cmd-alt-1", "editor::InvertSelections"),
        ("cmd-alt-2", "editor::SplitSelectionsByRegex"),
        ("cmd-alt-3", "editor::KeepSelectionsMatchingRegex"),
        ("cmd-alt-4", "editor::RemoveMatchingSelections"),
    ]
    .into_iter()
    .map(|(key, name)| {
        (
            key,
            Command {
                command: name.into(),
                args: if name.ends_with("InvertSelections") {
                    serde_json::json!({})
                } else {
                    serde_json::json!({"pattern":","})
                },
            },
        )
    })
    .collect();
    let bindings: Vec<_> = commands.iter().map(|(key, command)| {
        serde_json::json!({"keys":[key],"command":command.command,"args":command.args,"context":"Editor"})
    }).collect();
    std::fs::write(crate::config::customization::keymap_path(), serde_json::to_string(&bindings).unwrap()).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "ab,cd中");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    on_editor(&ws, vcx, |e, w, cx| {
        let end = e.doc(cx).len();
        e.edit(cx, |s| {
            s.replace_ranges(vec![(end..end, "!".into())]);
        });
        e.perform_action(&editor::Undo, w, cx);
    });
    views[0].update_in(vcx, |e, w, cx| e.replace_and_mark_text_in_range(Some(5..6), "中", None, w, cx));
    assert!(!views[0].read_with(vcx, |e, _| e.marked.is_empty()));
    let peer = views[0].read_with(vcx, |e, cx| {
        (e.marked.clone(), e.read_state(cx).selection_state(), e.read_state(cx).selection_generation())
    });
    let mut outcomes = Vec::new();
    for (key, command) in &commands {
        for keybinding in [true, false] {
            on_editor(&ws, vcx, |e, w, cx| {
                e.sel(cx).set_virtual_space(true);
                let mut value = Selection::new(5, 0);
                value.rectangular = true;
                e.sel(cx).set_selections(vec![value, Selection::from_points((6, 2), (6, 5))]);
                e.focus(w);
            });
            vcx.run_until_parked();
            let before = on_editor(&ws, vcx, |e, _, cx| {
                e.sel(cx).sel_checkpoint();
                (
                    e.text(cx),
                    e.read_state(cx).buffer_id(),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.doc(cx).unsaved(),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation(),
                    e.marked.clone(),
                )
            });
            if keybinding {
                vcx.simulate_keystrokes(key);
            } else {
                let action = vcx.update(|_, cx| command.build(cx).unwrap());
                vcx.update(|w, cx| w.dispatch_action(action, cx));
                vcx.run_until_parked();
            }
            let intact = on_editor(&ws, vcx, |e, _, cx| {
                (
                    e.text(cx),
                    e.read_state(cx).buffer_id(),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.doc(cx).unsaved(),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation(),
                    e.marked.clone(),
                ) == before
            });
            outcomes.push((*key, keybinding, intact));
            assert!(
                views[0].read_with(vcx, |e, cx| (
                    e.marked.clone(),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation()
                )) == peer
            );
        }
    }
    assert!(outcomes.iter().all(|(_, _, intact)| *intact), "every native route must refuse intact: {outcomes:?}");
    views[0].update_in(vcx, |e, w, cx| e.unmark_text(w, cx));
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "ab,cd中!", "refusal must retain a genuine source redo");
}

#[gpui::test]
fn n08_native_selection_bindings_keep_focus_recording_and_read_only_selection_access(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    let cases = [
        ("cmd-alt-1", "editor::InvertSelections", serde_json::json!({}), vec![(0, 1)], vec![(1, 3)]),
        (
            "cmd-alt-2",
            "editor::SplitSelectionsByRegex",
            serde_json::json!({"pattern":","}),
            vec![(0, 3)],
            vec![(0, 1), (2, 3)],
        ),
        (
            "cmd-alt-3",
            "editor::KeepSelectionsMatchingRegex",
            serde_json::json!({"pattern":"a"}),
            vec![(0, 1), (2, 3)],
            vec![(0, 1)],
        ),
        (
            "cmd-alt-4",
            "editor::RemoveMatchingSelections",
            serde_json::json!({"pattern":"a"}),
            vec![(0, 1), (2, 3)],
            vec![(2, 3)],
        ),
    ];
    let bindings: Vec<_> = cases
        .iter()
        .map(|(key, command, args, _, _)| serde_json::json!({"keys":[key],"command":command,"args":args}))
        .collect();
    std::fs::write(crate::config::customization::keymap_path(), serde_json::to_string(&bindings).unwrap()).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "a,b");
    for mode in ["ordinary", "viewer", "follow", "readonly"] {
        on_editor(&ws, vcx, |e, _, cx| {
            e.input_enabled = mode != "viewer";
            e.update_doc(cx, |d| {
                d.set_following(mode == "follow");
                d.read_only = mode == "readonly";
            });
        });
        for (key, command, _, before, expected) in &cases {
            set_selections(&ws, vcx, before);
            ws.update_in(vcx, |ws, w, _| w.focus(&ws.focus_handle));
            vcx.simulate_keystrokes(key);
            assert_eq!(
                sels(&ws, vcx),
                before.iter().map(|(a, h)| Selection::new(*a, *h)).collect::<Vec<_>>(),
                "wrong focus: {command}/{mode}"
            );
            on_editor(&ws, vcx, |e, w, _| e.focus(w));
            vcx.run_until_parked();
            vcx.update(|_, cx| {
                let recorder = cx.default_global::<Recorder>();
                recorder.commands.clear();
                recorder.recording = true;
            });
            vcx.simulate_keystrokes(key);
            assert_eq!(
                sels(&ws, vcx),
                expected.iter().map(|(a, h)| Selection::new(*a, *h)).collect::<Vec<_>>(),
                "focused selection access: {command}/{mode}"
            );
            let recorded = vcx.update(|_, cx| {
                let recorder = cx.default_global::<Recorder>();
                recorder.recording = false;
                recorder.commands.clone()
            });
            assert_eq!(recorded.len(), 1, "one native capture: {command}/{mode}");
            assert_eq!(&recorded[0].command, command);
            vcx.dispatch_action(editor::UndoSelection);
            assert_eq!(
                sels(&ws, vcx),
                before.iter().map(|(a, h)| Selection::new(*a, *h)).collect::<Vec<_>>(),
                "selection Undo: {command}/{mode}"
            );
            assert_eq!(text(&ws, vcx), "a,b");
        }
    }
}

#[gpui::test]
fn n10_actual_command_build_aligns_pattern_in_one_undo(cx: &mut TestAppContext) {
    use crate::keybindings::{Command, CommandAction};
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a=1\nlong=2");
    let command =
        Command { command: "editor::AlignOnPattern".into(), args: serde_json::json!({"pattern":"=","capture":0}) };
    assert!(vcx.update(|_, cx| command.build(cx)).is_ok(), "N10 typed command is missing");
    ws.update_in(vcx, |ws, w, cx| ws.execute_command(&command, w, cx).unwrap());
    assert_eq!(text(&ws, vcx), "a   =1\nlong=2");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "a=1\nlong=2");
}

#[gpui::test]
fn n10_catalog_form_defaults_validate_capture_and_record_resolved_action(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a=1\nlong=2");
    for label in ["Align on Pattern…", "Align Selected Fields"] {
        assert!(quick_panel::command_items()
            .iter()
            .any(|item| item.label == format!("Edit: {label}") && item.hint.is_empty()));
        assert!(submenu(&ws, vcx, "Edit", "Columns")
            .into_iter()
            .any(|item| matches!(item,MenuItem::Action{name,..} if name.as_ref()==label)));
    }
    vcx.dispatch_action(AlignOnPatternPrompt);
    vcx.run_until_parked();
    vcx.simulate_input("*");
    submit_form(vcx, "");
    assert!(form_error(&ws, vcx).flatten().is_some());
    assert_eq!(text(&ws, vcx), "a=1\nlong=2");
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("=");
    vcx.simulate_keystrokes("tab cmd-a");
    submit_form(vcx, "1");
    assert!(form_error(&ws, vcx).flatten().unwrap().contains("Capture index"));
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("0");
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "a   =1\nlong=2");
    let recorded = vcx.update(|_, cx| {
        let r = cx.default_global::<Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].command, "editor::AlignOnPattern");
    assert_eq!(recorded[0].args, serde_json::json!({"pattern":"=","capture":0,"ignore_case":false}));
    vcx.dispatch_action(editor::Undo);
    vcx.dispatch_action(AlignOnPatternPrompt);
    vcx.run_until_parked();
    submit_form(vcx, "");
    assert_eq!(text(&ws, vcx), "a   =1\nlong=2", "default regex is = and capture is 0");
}

#[gpui::test]
fn n10_form_refuses_source_selection_away_back_and_later_shared_composition(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a=1\nlong=2中");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    for kind in 0..3 {
        set_selections(&ws, vcx, &[(0, 11)]);
        vcx.dispatch_action(AlignOnPatternPrompt);
        vcx.run_until_parked();
        if kind == 2 {
            views[0].update_in(vcx, |e, w, cx| {
                let end = e.doc(cx).len();
                e.replace_and_mark_text_in_range(Some(end - 1..end), "中", None, w, cx);
            });
        } else {
            on_editor(&ws, vcx, |e, _, cx| {
                if kind == 0 {
                    e.edit(cx, |s| {
                        s.set_cursor(s.len());
                        s.insert("!");
                    });
                } else {
                    e.sel(cx).set_cursor(0);
                    e.sel(cx).set_selection(0, 11);
                }
            });
        }
        let before = text(&ws, vcx);
        submit_form(vcx, "");
        assert_eq!(text(&ws, vcx), before);
        let error = form_error(&ws, vcx).flatten().unwrap();
        assert!(error.contains(if kind == 2 { "composition" } else { "changed" }), "{error}");
        vcx.simulate_keystrokes("escape");
    }
    assert!(!views[0].read_with(vcx, |e, _| e.marked.is_empty()));
}

#[gpui::test]
fn n10_actual_keys_native_actions_and_json_refuse_shared_ime_preserving_all_state_and_redo(cx: &mut TestAppContext) {
    use crate::keybindings::Command;
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    std::fs::write(
        crate::config::customization::keymap_path(),
        r#"[
      {"keys":["cmd-alt-1"],"command":"editor::AlignOnPattern","args":{"pattern":"="},"context":"Editor"},
      {"keys":["cmd-alt-2"],"command":"editor::AlignSelectedFields","context":"Editor"} ]"#,
    )
    .unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "a=1\nlong=2中");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    on_editor(&ws, vcx, |e, w, cx| {
        let end = e.doc(cx).len();
        e.edit(cx, |s| {
            s.replace_ranges(vec![(end..end, "!".into())]);
        });
        e.perform_action(&editor::Undo, w, cx);
    });
    views[0].update_in(vcx, |e, w, cx| {
        let end = e.doc(cx).len();
        e.replace_and_mark_text_in_range(Some(end - 1..end), "中", None, w, cx);
    });
    on_editor(&ws, vcx, |e, w, cx| {
        e.sel(cx).set_virtual_space(true);
        e.sel(cx).set_selections(vec![
            Selection::new(1, 0).in_rectangle(),
            Selection::new(4, 8),
            Selection::from_points((11, 2), (11, 5)).in_rectangle(),
        ]);
        e.focus(w);
    });
    vcx.run_until_parked();
    let before = on_editor(&ws, vcx, |e, _, cx| {
        e.sel(cx).sel_checkpoint();
        (
            e.text(cx),
            e.doc(cx).version,
            e.doc(cx).save_point(),
            e.doc(cx).unsaved(),
            e.read_state(cx).selection_state(),
            e.read_state(cx).selection_generation(),
            e.marked.clone(),
        )
    });
    let peer = views[0].read_with(vcx, |e, cx| {
        (e.marked.clone(), e.read_state(cx).selection_state(), e.read_state(cx).selection_generation())
    });
    for kind in 0..6 {
        match kind {
            0 => vcx.simulate_keystrokes("cmd-alt-1"),
            1 => vcx.simulate_keystrokes("cmd-alt-2"),
            2 => vcx.dispatch_action(editor::AlignOnPattern { pattern: "=".into(), capture: 0, ignore_case: false }),
            3 => vcx.dispatch_action(editor::AlignSelectedFields),
            _ => {
                let command = Command {
                    command: if kind == 4 { "editor::AlignOnPattern" } else { "editor::AlignSelectedFields" }.into(),
                    args: if kind == 4 { serde_json::json!({"pattern":"="}) } else { serde_json::json!({}) },
                };
                assert!(ws
                    .update_in(vcx, |ws, w, cx| ws.execute_command(&command, w, cx))
                    .unwrap_err()
                    .contains("composition"));
            }
        }
        assert!(
            (on_editor(&ws, vcx, |e, _, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.doc(cx).unsaved(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
                e.marked.clone()
            ))) == before,
            "route {kind}"
        );
        assert!(
            views[0].read_with(vcx, |e, cx| (
                e.marked.clone(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation()
            )) == peer
        );
    }
    vcx.dispatch_action(AlignOnPatternPrompt);
    vcx.run_until_parked();
    assert!(!ws.read_with(vcx, |ws, _| matches!(ws.panel, Some(Panel::Form(_)))));
    views[0].update_in(vcx, |e, w, cx| e.unmark_text(w, cx));
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "a=1\nlong=2中!");
}

#[gpui::test]
fn n10_native_keys_keep_focus_record_once_and_public_macro_replay(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    std::fs::write(
        crate::config::customization::keymap_path(),
        r#"[
      {"keys":["cmd-alt-1"],"command":"editor::AlignOnPattern","args":{"pattern":"="}},
      {"keys":["cmd-alt-2"],"command":"editor::AlignSelectedFields"} ]"#,
    )
    .unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    for (key, name) in [("cmd-alt-1", "editor::AlignOnPattern"), ("cmd-alt-2", "editor::AlignSelectedFields")] {
        ws.update_in(vcx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
        type_in(&ws, vcx, "a=1\nlong=2");
        set_selections(&ws, vcx, &[(1, 0), (4, 8)]);
        ws.update_in(vcx, |ws, w, _| w.focus(&ws.focus_handle));
        vcx.simulate_keystrokes(key);
        assert_eq!(text(&ws, vcx), "a=1\nlong=2");
        on_editor(&ws, vcx, |e, w, _| e.focus(w));
        vcx.run_until_parked();
        vcx.update(|_, cx| {
            let r = cx.default_global::<Recorder>();
            r.commands.clear();
            r.recording = true;
        });
        vcx.simulate_keystrokes(key);
        assert_eq!(text(&ws, vcx), "a   =1\nlong=2");
        let recorded = vcx.update(|_, cx| {
            let r = cx.default_global::<Recorder>();
            r.recording = false;
            r.commands.clone()
        });
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].command, name);
        vcx.simulate_keystrokes(key);
        assert_eq!(text(&ws, vcx), "a   =1\nlong=2");
        vcx.dispatch_action(editor::Undo);
        assert_eq!(text(&ws, vcx), "a=1\nlong=2");
        let loaded = vcx.update(|_, cx| {
            crate::config::customization::parse_macro(&serde_json::to_string(&recorded).unwrap(), cx).unwrap()
        });
        vcx.update(|_, cx| cx.default_global::<Recorder>().last = loaded);
        vcx.dispatch_action(ReplayMacro);
        vcx.run_until_parked();
        assert_eq!(text(&ws, vcx), "a   =1\nlong=2");
        vcx.dispatch_action(editor::Undo);
        assert_eq!(text(&ws, vcx), "a=1\nlong=2");
    }
}

#[gpui::test]
fn n10_viewer_follow_refusal_filesystem_readonly_and_shared_revision_mapping(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a=1\nlong=2");
    set_selections(&ws, vcx, &[(1, 0), (4, 8)]);
    for follow in [false, true] {
        on_editor(&ws, vcx, |e, _, cx| {
            if follow {
                e.document.update(cx, |d, _| {
                    d.model.set_following(true);
                });
            } else {
                e.input_enabled = false;
            }
        });
        let before = on_editor(&ws, vcx, |e, _, cx| {
            (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
            )
        });
        vcx.dispatch_action(editor::AlignSelectedFields);
        vcx.dispatch_action(editor::AlignOnPattern { pattern: "=".into(), capture: 0, ignore_case: false });
        assert!(
            on_editor(&ws, vcx, |e, _, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation()
            )) == before
        );
        on_editor(&ws, vcx, |e, _, cx| {
            e.input_enabled = true;
            e.document.update(cx, |d, _| {
                d.model.set_following(false);
            });
        });
    }
    new_view(&ws, vcx);
    set_selections(&ws, vcx, &[(1, 0), (4, 8)]);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    views[0].update(vcx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(0);
            s.insert("x");
        });
    });
    on_editor(&ws, vcx, |e, _, cx| e.document.update(cx, |d, _| d.model.read_only = true));
    vcx.dispatch_action(editor::AlignSelectedFields);
    assert_eq!(text(&ws, vcx), "xa  =1\nlong=2");
    assert_eq!(views[0].read_with(vcx, |e, cx| e.text(cx)), "xa  =1\nlong=2");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "xa=1\nlong=2");
}

#[gpui::test]
fn n10_actual_native_keys_refuse_composition_in_another_window_and_locally(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let path = temp("orlo-n10-native-windows").join("source.txt");
    std::fs::write(&path, "a=1\nlong=2中").unwrap();
    let (owner, peer, mut a, mut b) = two_windows(cx);
    owner.update_in(&mut a, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    peer.update_in(&mut b, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    std::fs::write(
        crate::config::customization::keymap_path(),
        r#"[
       {"keys":["cmd-alt-1"],"command":"editor::AlignOnPattern","args":{"pattern":"="}},
       {"keys":["cmd-alt-2"],"command":"editor::AlignSelectedFields"}]"#,
    )
    .unwrap();
    owner.update_in(&mut a, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    set_selections(&owner, &mut a, &[(1, 0), (4, 8)]);
    on_editor(&peer, &mut b, |e, w, cx| {
        let end = e.doc(cx).len();
        e.replace_and_mark_text_in_range(Some(end - 1..end), "中", None, w, cx);
    });
    for local in [false, true] {
        if local {
            on_editor(&peer, &mut b, |e, w, cx| e.unmark_text(w, cx));
            on_editor(&owner, &mut a, |e, w, cx| {
                let end = e.doc(cx).len();
                e.replace_and_mark_text_in_range(Some(end - 1..end), "中", None, w, cx);
            });
        }
        on_editor(&owner, &mut a, |e, w, _| e.focus(w));
        a.run_until_parked();
        let before = on_editor(&owner, &mut a, |e, _, cx| {
            e.sel(cx).sel_checkpoint();
            (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
                e.marked.clone(),
            )
        });
        for key in ["cmd-alt-1", "cmd-alt-2"] {
            a.simulate_keystrokes(key);
            assert!(
                on_editor(&owner, &mut a, |e, _, cx| (
                    e.text(cx),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation(),
                    e.marked.clone()
                )) == before
            );
        }
    }
}

#[gpui::test]
fn n10_pattern_form_reports_unstable_columns_and_accepts_whitespace_tolerant_capture(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "a= a=\nxa= a=");
    vcx.dispatch_action(AlignOnPatternPrompt);
    vcx.run_until_parked();
    vcx.simulate_input(r"a(=)");
    vcx.simulate_keystrokes("tab cmd-a");
    submit_form(vcx, "1");
    assert!(form_error(&ws, vcx).flatten().unwrap().contains("field columns"));
    assert_eq!(text(&ws, vcx), "a= a=\nxa= a=");
    vcx.simulate_keystrokes("shift-tab cmd-a");
    submit_form(vcx, r"a\s*(=)");
    assert_eq!(form_error(&ws, vcx), None);
    assert_eq!(text(&ws, vcx), "a = a=\nxa= a=");
    vcx.dispatch_action(editor::Undo);
    assert_eq!(text(&ws, vcx), "a= a=\nxa= a=");
}

#[gpui::test]
fn n11_syntax_filter_control_is_reachable_and_replaces_comments_only(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "let hit = \"hit\"; // hit\n");
    ws.update_in(vcx, |ws, window, cx| {
        ws.set_syntax(&syntax("Rust"), window, cx);
        replace_text(ws, "hit", "done", false, window, cx);
        ws.refresh_matches(cx);
    });
    vcx.run_until_parked();
    let button = vcx.debug_bounds("syntax-filter").expect("Find must expose syntax-class eligibility");
    vcx.simulate_click(button.center(), gpui::Modifiers::default());
    settle_until(vcx, |cx| ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).find_ranges.len() == 1));
    ws.update_in(vcx, |ws, window, cx| ws.replace_all(&ReplaceAll, window, cx));
    assert_eq!(text(&ws, vcx), "let hit = \"hit\"; // done\n");
}

fn n11_wait(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    settle(cx);
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.find_panel().is_some_and(|p| p.error.is_none())));
}

#[gpui::test]
fn n11_filter_masks_options_selection_and_replace_next_keep_one_undo_step(cx: &mut TestAppContext) {
    use orlo_editor::search::model::SyntaxMask;
    let (ws, vcx) = window(cx);
    let original = "let hit = \"HIT hitx\"; // hit\n// HIT\n";
    type_in(&ws, vcx, original);
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("Rust"), w, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_selection(20, 33));
        replace_text(ws, "(hit)", "new${1}", true, w, cx);
        ws.toggle_find_selection(&ToggleFindSelection, w, cx);
        let p = ws.find_panel_mut().unwrap();
        p.syntax = SyntaxMask::Comments;
        p.opts.whole_word = true;
        ws.refresh_matches(cx);
    });
    n11_wait(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).find_ranges, vec![25..28]);
        ws.with_active(cx, |e, cx| e.sel(cx).set_selection(25, 28));
        ws.replace_one(&find_panel::ReplaceOne, w, cx);
    });
    assert_eq!(text(&ws, vcx), "let hit = \"HIT hitx\"; // newhit\n// HIT\n");
    on_editor(&ws, vcx, |e, _, cx| {
        e.edit(cx, |s| {
            s.undo();
        })
    });
    assert_eq!(text(&ws, vcx), original);
    on_editor(&ws, vcx, |e, _, cx| {
        e.edit(cx, |s| {
            s.redo();
        })
    });
    assert_eq!(text(&ws, vcx), "let hit = \"HIT hitx\"; // newhit\n// HIT\n");
}

#[gpui::test]
fn n11_exclude_and_multiline_start_only_keep_string_interpolation_and_code_distinct(cx: &mut TestAppContext) {
    use orlo_editor::search::model::SyntaxMask;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "let hit = `hit ${hit}`; // hit\n/* hit\nend */ hit");
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("JavaScript"), w, cx);
        replace_text(ws, "hit", "X", false, w, cx);
        ws.find_panel_mut().unwrap().syntax = SyntaxMask::CommentsOrStrings;
        ws.find_panel_mut().unwrap().exclude_syntax = true;
        ws.refresh_matches(cx);
    });
    n11_wait(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| ws.replace_all(&ReplaceAll, w, cx));
    assert_eq!(text(&ws, vcx), "let X = `hit ${X}`; // hit\n/* hit\nend */ X");
    n11_wait(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| {
        let p = ws.find_panel_mut().unwrap();
        p.syntax = SyntaxMask::Comments;
        p.exclude_syntax = false;
        set_query(ws, "hit\\nend \\*/ X", true, cx);
        ws.refresh_matches(cx);
        ws.replace_all(&ReplaceAll, w, cx);
    });
    assert_eq!(text(&ws, vcx), "let X = `hit ${X}`; // hit\n/* X");
}

#[gpui::test]
fn n11_source_edit_syntax_switch_pending_and_unsupported_cannot_broaden(cx: &mut TestAppContext) {
    use orlo_editor::search::model::SyntaxMask;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "// hit");
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("Rust"), w, cx);
        replace_text(ws, "hit", "X", false, w, cx);
        ws.find_panel_mut().unwrap().syntax = SyntaxMask::Comments;
        ws.refresh_matches(cx);
        assert!(ws.find_panel().unwrap().error.as_ref().unwrap().contains("preparing"));
        ws.replace_all(&ReplaceAll, w, cx);
    });
    assert_eq!(text(&ws, vcx), "// hit");
    n11_wait(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.replace_ranges(vec![(0..2, "  ".into())]);
            })
        });
        ws.replace_all(&ReplaceAll, w, cx);
    });
    assert_eq!(text(&ws, vcx), "   hit", "equal-length revision must invalidate the old comment class");
    n11_wait(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| {
        assert!(ws.active_editor().unwrap().read(cx).find_ranges.is_empty());
        ws.set_syntax(&syntax("Plain Text"), w, cx);
        ws.find_panel_mut().unwrap().exclude_syntax = true;
        ws.find_panel_mut().unwrap().highlight = false;
        ws.refresh_matches(cx);
        assert!(ws.find_panel().unwrap().error.as_ref().unwrap().contains("unavailable"));
        ws.replace_all(&ReplaceAll, w, cx);
        ws.find_all(&FindAll, w, cx);
        assert!(ws.find_panel().is_some(), "keep unavailable reason visible");
    });
    assert_eq!(text(&ws, vcx), "   hit");
}

#[gpui::test]
fn n11_replace_refuses_viewer_follow_and_shared_ime_before_marked_history_or_redo_changes(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    use orlo_editor::search::model::SyntaxMask;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "// hit");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("Rust"), w, cx);
        replace_text(ws, "hit", "X", false, w, cx);
        ws.find_panel_mut().unwrap().syntax = SyntaxMask::Comments;
        ws.refresh_matches(cx);
    });
    n11_wait(&ws, vcx);
    for follow in [false, true] {
        on_editor(&ws, vcx, |e, _, cx| {
            if follow {
                e.document.update(cx, |d, _| {
                    d.model.set_following(true);
                });
            } else {
                e.input_enabled = false;
            }
        });
        // Follow changes the source revision; prepare that snapshot before testing refusal.
        ws.update(vcx, |ws, cx| ws.refresh_matches(cx));
        n11_wait(&ws, vcx);
        let before = on_editor(&ws, vcx, |e, _, cx| {
            (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
            )
        });
        ws.update_in(vcx, |ws, w, cx| {
            ws.replace_all(&ReplaceAll, w, cx);
            ws.replace_one(&find_panel::ReplaceOne, w, cx);
        });
        assert!(
            on_editor(&ws, vcx, |e, _, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation()
            )) == before
        );
        on_editor(&ws, vcx, |e, _, cx| {
            e.input_enabled = true;
            e.document.update(cx, |d, _| {
                d.model.set_following(false);
            });
        });
    }
    views[0].update_in(vcx, |e, w, cx| {
        let len = e.doc(cx).len();
        e.sel(cx).set_cursor(len);
        e.replace_and_mark_text_in_range(None, "中", None, w, cx);
    });
    ws.update(vcx, |ws, cx| ws.refresh_matches(cx));
    n11_wait(&ws, vcx);
    let before = text(&ws, vcx);
    let marked = views[0].read_with(vcx, |e, _| e.marked.clone());
    let state = on_editor(&ws, vcx, |e, _, cx| {
        (
            e.doc(cx).version,
            e.doc(cx).save_point(),
            e.read_state(cx).selection_state(),
            e.read_state(cx).selection_generation(),
        )
    });
    vcx.dispatch_action(ReplaceAll);
    ws.update_in(vcx, |ws, w, cx| ws.replace_one(&find_panel::ReplaceOne, w, cx));
    assert_eq!(text(&ws, vcx), before);
    assert_eq!(views[0].read_with(vcx, |e, _| e.marked.clone()), marked);
    assert!(
        on_editor(&ws, vcx, |e, _, cx| (
            e.doc(cx).version,
            e.doc(cx).save_point(),
            e.read_state(cx).selection_state(),
            e.read_state(cx).selection_generation()
        )) == state
    );
}

#[gpui::test]
fn n11_actual_controls_cycle_all_masks_and_exclusion_without_losing_query_focus(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "hit \"hit\" // hit\n");
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("Rust"), w, cx);
        find_text(ws, "hit", false, w, cx);
    });
    vcx.run_until_parked();
    for count in [1, 1, 2, 3] {
        let bounds = vcx.debug_bounds("syntax-filter").unwrap();
        vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
        n11_wait(&ws, vcx);
        ws.update_in(vcx, |ws, w, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).find_ranges.len(), count);
            assert!(ws.find_panel().unwrap().query.read(cx).is_focused(w));
        });
    }
    let bounds = vcx.debug_bounds("syntax-filter").unwrap();
    vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
    n11_wait(&ws, vcx);
    let bounds = vcx.debug_bounds("syntax-exclude").unwrap();
    vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
    n11_wait(&ws, vcx);
    assert_eq!(on_editor(&ws, vcx, |e, _, _| e.find_ranges.clone()), [0..3, 5..8]);
}

#[gpui::test]
fn n11_zero_width_regex_next_and_all_replacements_use_eligible_starts(cx: &mut TestAppContext) {
    use orlo_editor::search::model::SyntaxMask;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "hit // hit hit");
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("Rust"), w, cx);
        replace_text(ws, "(?=hit)", "X", true, w, cx);
        ws.find_panel_mut().unwrap().syntax = SyntaxMask::Comments;
        ws.refresh_matches(cx);
    });
    n11_wait(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(7));
        ws.find_next(&FindNext, w, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 11);
        ws.replace_all(&ReplaceAll, w, cx);
    });
    assert_eq!(text(&ws, vcx), "hit // Xhit Xhit");
    on_editor(&ws, vcx, |e, _, cx| {
        e.edit(cx, |s| {
            s.undo();
        })
    });
    assert_eq!(text(&ws, vcx), "hit // hit hit");
}

#[gpui::test]
fn n11_pending_grammar_switch_and_closed_panel_discard_late_classification(cx: &mut TestAppContext) {
    use orlo_editor::search::model::SyntaxMask;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "// hit");
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("Rust"), w, cx);
        replace_text(ws, "hit", "X", false, w, cx);
        ws.find_panel_mut().unwrap().syntax = SyntaxMask::Comments;
        ws.refresh_matches(cx);
        ws.set_syntax(&syntax("Plain Text"), w, cx);
    });
    settle(vcx);
    ws.update_in(vcx, |ws, w, cx| {
        assert!(ws.find_panel().unwrap().error.as_ref().unwrap().contains("unavailable"));
        assert!(ws.active_editor().unwrap().read(cx).find_ranges.is_empty());
        ws.replace_all(&ReplaceAll, w, cx);
        ws.set_syntax(&syntax("Rust"), w, cx);
        ws.close_panel(&ClosePanel, w, cx);
        ws.find(&Find, w, cx);
    });
    settle(vcx);
    assert_eq!(text(&ws, vcx), "// hit");
    ws.read_with(vcx, |ws, _| assert_eq!(ws.find_panel().unwrap().syntax, SyntaxMask::Any));
}

#[gpui::test]
fn n11_native_replace_all_key_refuses_other_window_and_local_ime_preserving_real_redo(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    use orlo_editor::search::model::SyntaxMask;
    let path = temp("orlo-n11-native-windows").join("source.rs");
    std::fs::write(&path, "// hit中").unwrap();
    let (owner, peer, mut a, mut b) = two_windows(cx);
    owner.update_in(&mut a, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    peer.update_in(&mut b, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    on_editor(&owner, &mut a, |e, _, cx| {
        e.edit(cx, |s| {
            s.set_cursor(s.len());
            s.insert("!");
            s.undo();
        })
    });
    owner.update_in(&mut a, |ws, w, cx| {
        replace_text(ws, "hit", "X", false, w, cx);
        ws.find_panel_mut().unwrap().syntax = SyntaxMask::Comments;
        ws.refresh_matches(cx);
    });
    n11_wait(&owner, &mut a);
    for local in [false, true] {
        if local {
            on_editor(&peer, &mut b, |e, w, cx| e.unmark_text(w, cx));
        }
        let (target, context) = if local { (&owner, &mut a) } else { (&peer, &mut b) };
        on_editor(target, context, |e, w, cx| {
            let len = e.doc(cx).len();
            e.replace_and_mark_text_in_range(Some(len - 1..len), "中", None, w, cx);
        });
        owner.update_in(&mut a, |ws, w, cx| ws.find_panel().unwrap().query.read(cx).focus(w));
        let before = on_editor(&owner, &mut a, |e, _, cx| {
            (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
                e.marked.clone(),
            )
        });
        a.simulate_keystrokes("cmd-alt-enter");
        assert!(
            on_editor(&owner, &mut a, |e, _, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
                e.marked.clone()
            )) == before
        );
    }
    on_editor(&owner, &mut a, |e, w, cx| {
        e.unmark_text(w, cx);
        e.edit(cx, |s| {
            s.redo();
        });
    });
    assert_eq!(text(&owner, &mut a), "// hit中!", "refused native replacement must retain the real redo stack");
}

#[gpui::test]
fn n11_review_find_all_uses_ready_query_matches_despite_an_invalid_replacement(cx: &mut TestAppContext) {
    use orlo_editor::search::model::SyntaxMask;
    let (ws, vcx) = window(cx);
    let original = "hit\n// hit\nlet s = \"hit\";\n// hit";
    type_in(&ws, vcx, original);
    on_editor(&ws, vcx, |e, _, cx| {
        e.edit(cx, |s| {
            s.set_cursor(s.len());
            s.insert("!");
            s.undo();
        })
    });
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("Rust"), w, cx);
        replace_text(ws, "hit", "$1", true, w, cx);
        ws.find_panel_mut().unwrap().syntax = SyntaxMask::Comments;
        ws.refresh_matches(cx);
    });
    settle(vcx);
    settle_until(vcx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).find_ranges == vec![7..10, 29..32])
    });
    assert!(ws.read_with(vcx, |ws, _| ws.find_panel().unwrap().error.is_some()));
    let before = on_editor(&ws, vcx, |e, _, cx| (e.text(cx), e.doc(cx).version, e.doc(cx).save_point()));
    ws.update_in(vcx, |ws, w, cx| ws.find_all(&FindAll, w, cx));
    assert!(ws.read_with(vcx, |ws, _| ws.find_panel().is_none()));
    assert_eq!(
        on_editor(&ws, vcx, |e, _, cx| e.read_state(cx).view.selections.iter().map(|s| s.range()).collect::<Vec<_>>()),
        vec![7..10, 29..32]
    );
    assert!(on_editor(&ws, vcx, |e, _, cx| (e.text(cx), e.doc(cx).version, e.doc(cx).save_point())) == before);
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.redo()));
    assert_eq!(text(&ws, vcx), format!("{original}!"));
}

#[gpui::test]
fn n11_review_nonmutating_consumers_keep_eligible_matches_and_the_replacement_error(cx: &mut TestAppContext) {
    use orlo_editor::search::model::SyntaxMask;
    let (ws, vcx) = window(cx);
    let original = "hit\n// hit\nlet s = \"hit\";\n// hit";
    type_in(&ws, vcx, original);
    on_editor(&ws, vcx, |e, _, cx| {
        e.edit(cx, |s| {
            s.set_cursor(s.len());
            s.insert("!");
            s.undo();
        })
    });
    ws.update_in(vcx, |ws, w, cx| {
        ws.set_syntax(&syntax("Rust"), w, cx);
        replace_text(ws, "hit", "$1", true, w, cx);
        ws.find_panel_mut().unwrap().syntax = SyntaxMask::Comments;
        ws.refresh_matches(cx);
    });
    settle(vcx);
    settle_until(vcx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).find_ranges == vec![7..10, 29..32])
    });
    let error = ws.read_with(vcx, |ws, _| ws.find_panel().unwrap().error.clone());
    assert!(error.is_some());
    let before = on_editor(&ws, vcx, |e, _, cx| (e.text(cx), e.doc(cx).version, e.doc(cx).save_point()));
    ws.update_in(vcx, |ws, w, cx| {
        ws.bookmark_matches(&BookmarkMatches, w, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).read_state(cx).bookmarked_lines(), [1, 3]);
        ws.highlight_matches(&HighlightMatches { style: 2 }, w, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).read_state(cx).view.highlights[2], vec![7..10, 29..32]);
        ws.extract(
            &Extract {
                what: crate::searching::extract::What::Matches,
                from: ExtractFrom::Document,
                to_clipboard: true,
                locations: false,
            },
            w,
            cx,
        );
        assert_eq!(ws.find_panel().unwrap().error, error);
        ws.replace_all(&ReplaceAll, w, cx);
        assert_eq!(ws.find_panel().unwrap().error, error);
    });
    assert_eq!(vcx.read_from_clipboard().unwrap().text().unwrap(), "hit\nhit\n");
    assert!(on_editor(&ws, vcx, |e, _, cx| (e.text(cx), e.doc(cx).version, e.doc(cx).save_point())) == before);
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.redo()));
    assert_eq!(text(&ws, vcx), format!("{original}!"));
}

#[gpui::test]
fn previous_matching_lines_distinguishes_files_and_browser_filter(cx: &mut TestAppContext) {
    let dir = temp("orlo-previous-lines-causal").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "seed é target\nother target\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_path(dir, window, cx);
        ws.run_search("seed", path.to_str().unwrap(), None, FindOpts::default(), Default::default(), window, cx)
            .unwrap();
    });
    settle_previous_line_search(&ws, vcx);
    ws.update_in(vcx, |ws, window, cx| {
        assert_eq!(ws.result_set.as_ref().unwrap().counts().matches, 1);
        ws.show_results_panel(window, cx);
        let filter = ws.results_panel().unwrap().filter.clone();
        filter.update(cx, |e, cx| e.set_text("absent-path", cx));
        assert!(ws.results_rows(cx).is_empty());
        ws.run_search("target", "<previous matching lines>", None, FindOpts::default(), Default::default(), window, cx)
            .unwrap();
    });
    settle_previous_line_search(&ws, vcx);
    ws.update_in(vcx, |ws, window, cx| {
        let result = ws.result_set.as_ref().unwrap();
        assert_eq!(result.counts().matches, 1, "the second query searches only the prior source hit line");
        assert_eq!(result.files[0].entries()[0].hit.chars, 7..13);
        ws.run_search("target", "<previous results>", None, FindOpts::default(), Default::default(), window, cx)
            .unwrap();
    });
    settle_previous_line_search(&ws, vcx);
    ws.read_with(vcx, |ws, _| assert_eq!(ws.result_set.as_ref().unwrap().counts().matches, 2));
}

fn settle_previous_line_search(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    settle_until(vcx, |cx| {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        ws.read_with(cx, |ws, _| ws.search.is_none())
    });
}

fn previous_line_query(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    query: &str,
    where_: &str,
    replacement: Option<&str>,
) {
    ws.update_in(vcx, |ws, window, cx| {
        ws.run_search(query, where_, replacement, FindOpts::default(), Default::default(), window, cx).unwrap();
    });
    settle_previous_line_search(ws, vcx);
}

#[gpui::test]
fn previous_matching_lines_chooser_retains_original_scope_across_query_and_refresh(cx: &mut TestAppContext) {
    let dir = temp("orlo-previous-lines-chooser").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "seed first\nseed second\nelse second\n").unwrap();
    let (ws, vcx) = window(cx);
    previous_line_query(&ws, vcx, "seed", path.to_str().unwrap(), None);
    ws.update_in(vcx, |ws, window, cx| {
        ws.find_in_files(&FindInFiles, window, cx);
        ws.choose_where(&ChooseWhere, window, cx);
        let p = ws.quick_panel.as_mut().unwrap();
        let at = p.items.iter().position(|item| item.label == "Previous Matching Lines").unwrap();
        p.selected = p.filtered.iter().position(|i| *i == at).unwrap();
        ws.confirm_quick_panel(window, cx);
        let p = ws.fif_panel().unwrap();
        assert_eq!(p.where_.read(cx).text(cx), "<previous matching lines>");
        assert_eq!(ws.previous_lines.as_ref().unwrap()[0].ranges, std::iter::once(0..23).collect::<Vec<_>>());
    });
    previous_line_query(&ws, vcx, "first", "<previous matching lines>", None);
    previous_line_query(&ws, vcx, "second", "<previous matching lines>", None);
    ws.update_in(vcx, |ws, window, cx| {
        assert_eq!(
            ws.result_set.as_ref().unwrap().counts().matches,
            1,
            "query changes do not recapture only the first line"
        );
        ws.refresh_results(&RefreshResults, window, cx);
    });
    settle_previous_line_search(&ws, vcx);
    ws.read_with(vcx, |ws, _| {
        let r = ws.result_set.as_ref().unwrap();
        assert_eq!(r.counts().matches, 1);
        assert_eq!(r.previous_lines.as_ref().unwrap()[0].ranges, std::iter::once(0..23).collect::<Vec<_>>());
    });
}

#[gpui::test]
fn previous_matching_lines_unsaved_shared_alias_replace_is_one_undo_step(cx: &mut TestAppContext) {
    let dir = temp("orlo-previous-lines-unsaved").canonicalize().unwrap();
    let path = dir.join("source.txt");
    let alias = dir.join("alias.txt");
    std::fs::write(&path, "old\n").unwrap();
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let (peer, pcx) = window(cx);
    peer.update_in(pcx, |ws, window, cx| {
        ws.open_path(path.clone(), window, cx);
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert("seed é target\nother target\n");
            })
        });
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
    });
    let document = peer.read_with(pcx, |ws, _| ws.active_editor().unwrap());
    let (ws, vcx) = window(pcx);
    previous_line_query(&ws, vcx, "seed", alias.to_str().unwrap(), None);
    previous_line_query(&ws, vcx, "target", "<previous matching lines>", Some("中"));
    ws.update_in(vcx, |ws, window, cx| ws.apply_replacements(&ApplyReplacements, window, cx));
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
    document.update(vcx, |e, cx| {
        assert_eq!(e.text(cx), "seed é 中\nother target\n");
        e.edit(cx, |s| s.undo());
        assert_eq!(e.text(cx), "seed é target\nother target\n");
        e.edit(cx, |s| s.redo());
        assert_eq!(e.text(cx), "seed é 中\nother target\n");
    });
    peer.read_with(vcx, |ws, cx| {
        for view in ws.views.tabs() {
            assert_eq!(view.read(cx).text(cx), "seed é 中\nother target\n");
        }
    });
}

#[gpui::test]
fn previous_matching_lines_stale_sources_fail_per_file_without_replacement(cx: &mut TestAppContext) {
    use crate::searching::results::Body;
    for mode in ["disk-edit", "missing", "buffer-edit", "closed", "rebound", "alias-rebound"] {
        let dir = temp(&format!("orlo-previous-lines-stale-{mode}")).canonicalize().unwrap();
        let path = dir.join("source.txt");
        let valid = dir.join("valid.txt");
        let alias = dir.join("alias.txt");
        std::fs::write(&path, "seed target\nother target\n").unwrap();
        std::fs::write(&valid, "seed target\n").unwrap();
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        let (ws, vcx) = window(cx);
        let buffer = ws.update_in(vcx, |ws, window, cx| {
            if matches!(mode, "buffer-edit" | "closed" | "rebound") {
                ws.open_path(path.clone(), window, cx);
                Some(ws.active_editor().unwrap())
            } else {
                None
            }
        });
        let where_ = format!("{}, {}", if mode == "alias-rebound" { &alias } else { &path }.display(), valid.display());
        previous_line_query(&ws, vcx, "seed", &where_, None);
        ws.update_in(vcx, |ws, window, cx| match mode {
            "disk-edit" => std::fs::write(&path, "changed target\n").unwrap(),
            "missing" => std::fs::remove_file(&path).unwrap(),
            "buffer-edit" => buffer.as_ref().unwrap().update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.set_cursor(0);
                    s.insert("edit ");
                })
            }),
            "closed" => ws.close_editor(buffer.as_ref().unwrap(), window, cx),
            "rebound" => {
                buffer.as_ref().unwrap().update(cx, |e, cx| e.update_doc(cx, |d| d.path = Some(dir.join("new.txt"))))
            }
            "alias-rebound" => {
                std::fs::remove_file(&alias).unwrap();
                std::os::unix::fs::symlink(&valid, &alias).unwrap();
            }
            _ => unreachable!(),
        });
        previous_line_query(&ws, vcx, "target", "<previous matching lines>", Some("X"));
        ws.update_in(vcx, |ws, window, cx| {
            let set = ws.result_set.as_ref().unwrap();
            assert_eq!(set.counts().matches, 1, "only the unchanged file remains: {mode}");
            assert!(
                set.files
                    .iter()
                    .any(|f| matches!(&f.body, Body::Failed(why) if why.contains("stale") && why.contains("refresh"))),
                "{mode}"
            );
            ws.apply_replacements(&ApplyReplacements, window, cx);
        });
        settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
        assert_eq!(std::fs::read_to_string(&valid).unwrap(), "seed X\n");
        if path.exists() {
            assert!(!std::fs::read_to_string(&path).unwrap().contains('X'), "{mode}");
        }
    }
}

#[gpui::test]
fn previous_matching_lines_reject_source_changes_and_completed_writes_before_delivery(cx: &mut TestAppContext) {
    use crate::searching::results::Body;
    for mode in ["buffer", "disk", "ticket", "hardlink-ticket"] {
        let dir = temp(&format!("orlo-previous-lines-delivery-{mode}")).canonicalize().unwrap();
        let path = dir.join("source.txt");
        let alias = dir.join("hard.txt");
        std::fs::write(&path, "seed target\n").unwrap();
        std::fs::hard_link(&path, &alias).unwrap();
        let (ws, vcx) = window(cx);
        let buffer = ws.update_in(vcx, |ws, window, cx| {
            if mode == "buffer" {
                ws.open_path(path.clone(), window, cx);
                Some(ws.active_editor().unwrap())
            } else {
                None
            }
        });
        previous_line_query(&ws, vcx, "seed", path.to_str().unwrap(), None);
        ws.update_in(vcx, |ws, window, cx| {
            ws.run_search(
                "target",
                "<previous matching lines>",
                Some("X"),
                FindOpts::default(),
                Default::default(),
                window,
                cx,
            )
            .unwrap();
            match mode {
                "buffer" => buffer.as_ref().unwrap().update(cx, |e, cx| {
                    e.edit(cx, |s| {
                        s.set_cursor(0);
                        s.insert("edit ");
                    })
                }),
                "disk" => std::fs::write(&path, "changed\n").unwrap(),
                "ticket" => drop(pipelines::lease(&path, cx)),
                "hardlink-ticket" => drop(pipelines::lease_file(&alias, pipelines::file_id(&alias), cx)),
                _ => unreachable!(),
            }
        });
        settle_previous_line_search(&ws, vcx);
        ws.read_with(vcx, |ws, _| {
            let result = ws.result_set.as_ref().unwrap();
            assert_eq!(result.counts().matches, 0, "{mode}");
            assert!(matches!(&result.files[0].body, Body::Failed(why) if why.contains("refresh")), "{mode}");
        });
    }
}

#[gpui::test]
fn previous_matching_lines_keep_decoded_codec_bom_and_crlf_on_disk_apply(cx: &mut TestAppContext) {
    use crate::editing::encoding::{self, Encoding};
    for (codec, bom) in [(Encoding::Windows1252, false), (Encoding::Utf16Le, true)] {
        let dir = temp(&format!("orlo-previous-lines-codec-{codec:?}")).canonicalize().unwrap();
        let path = dir.join("source.txt");
        let original = encoding::encode_owned("seed café\r\nother café\r\n".into(), codec, bom).unwrap();
        std::fs::write(&path, &original).unwrap();
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            let loaded = Loaded::read_encoding(&path, Some(codec)).unwrap();
            ws.add_tab(EditorState::from_loaded(&path, loaded), window, cx);
            let editor = ws.active_editor().unwrap();
            ws.close_editor(&editor, window, cx);
        });
        previous_line_query(&ws, vcx, "seed", path.to_str().unwrap(), None);
        ws.update(vcx, |ws, _| ws.closed.clear());
        previous_line_query(&ws, vcx, "café", "<previous matching lines>", Some("été"));
        ws.update_in(vcx, |ws, window, cx| ws.apply_replacements(&ApplyReplacements, window, cx));
        settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            encoding::encode_owned("seed été\r\nother café\r\n".into(), codec, bom).unwrap()
        );
    }
}

#[gpui::test]
fn previous_matching_lines_find_ignores_replacement_only_errors_and_cancel_retains_choice(cx: &mut TestAppContext) {
    let dir = temp("orlo-previous-lines-template").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "seed target\nother target\n").unwrap();
    let (ws, vcx) = window(cx);
    previous_line_query(&ws, vcx, "seed", path.to_str().unwrap(), None);
    ws.update_in(vcx, |ws, window, cx| {
        ws.find_in_files(&FindInFiles, window, cx);
        let p = ws.fif_panel_mut().unwrap();
        p.opts.regex = true;
        let (q, r, w) = (p.query.clone(), p.replace.clone(), p.where_.clone());
        q.update(cx, |e, cx| e.set_text("target", cx));
        r.update(cx, |e, cx| e.set_text("$missing", cx));
        w.update(cx, |e, cx| e.set_text("<previous matching lines>", cx));
        ws.validate_fif(cx);
        assert!(ws.fif_panel().unwrap().error.is_some());
        ws.run_find_in_files(&RunFindInFiles, window, cx);
    });
    settle_previous_line_search(&ws, vcx);
    ws.update_in(vcx, |ws, window, cx| {
        assert_eq!(ws.result_set.as_ref().unwrap().counts().matches, 1);
        ws.run_search("target", "<previous matching lines>", None, FindOpts::default(), Default::default(), window, cx)
            .unwrap();
        ws.stop_search(cx);
        assert!(ws.result_set.as_ref().unwrap().cancelled);
    });
    previous_line_query(&ws, vcx, "target", "<previous matching lines>", None);
    ws.read_with(vcx, |ws, _| assert_eq!(ws.result_set.as_ref().unwrap().counts().matches, 1));
}

#[gpui::test]
fn previous_matching_lines_apply_rechecks_readonly_ime_and_stale_preview(cx: &mut TestAppContext) {
    use crate::searching::results::Status;
    use gpui::EntityInputHandler;
    use std::os::unix::fs::PermissionsExt;
    for mode in ["viewer", "follow", "ime", "after-preview", "disk-readonly"] {
        let dir = temp(&format!("orlo-previous-lines-apply-{mode}")).canonicalize().unwrap();
        let path = dir.join("source.txt");
        std::fs::write(&path, "seed target\nother target\n").unwrap();
        let (peer, pcx) = window(cx);
        let buffer = if mode != "disk-readonly" {
            peer.update_in(pcx, |ws, window, cx| {
                ws.open_path(path.clone(), window, cx);
                if mode == "ime" {
                    ws.with_active(cx, |e, cx| {
                        e.edit(cx, |s| {
                            s.set_cursor(0);
                            s.insert("!");
                            s.undo();
                        })
                    });
                    ws.with_active(cx, |e, cx| {
                        let len = e.doc(cx).len();
                        e.sel(cx).set_cursor(len);
                    });
                    let editor = ws.active_editor().unwrap();
                    editor.update(cx, |e, cx| e.replace_and_mark_text_in_range(None, "中", None, window, cx));
                }
                if mode == "follow" {
                    ws.with_active(cx, |e, cx| {
                        e.document.update(cx, |d, _| {
                            d.model.set_following(true);
                        })
                    });
                }
                ws.active_editor().unwrap()
            })
        } else {
            peer.read_with(pcx, |ws, _| ws.active_editor().unwrap())
        };
        let (ws, vcx) = window(pcx);
        previous_line_query(&ws, vcx, "seed", path.to_str().unwrap(), None);
        previous_line_query(&ws, vcx, "target", "<previous matching lines>", Some("X"));
        if mode == "viewer" {
            buffer.update(vcx, |e, _| e.input_enabled = false);
        }
        if mode == "after-preview" {
            buffer.update(vcx, |e, cx| {
                e.edit(cx, |s| {
                    s.set_cursor(0);
                    s.insert("later ");
                })
            });
        }
        if mode == "disk-readonly" {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        let before = buffer.read_with(vcx, |e, cx| {
            (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
                e.marked.clone(),
            )
        });
        ws.update_in(vcx, |ws, window, cx| ws.apply_replacements(&ApplyReplacements, window, cx));
        settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
        assert!(
            buffer.read_with(vcx, |e, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
                e.marked.clone()
            )) == before,
            "{mode}"
        );
        ws.read_with(vcx, |ws, _| {
            let status = ws.result_set.as_ref().unwrap().files[0].status.as_ref().unwrap();
            assert!(matches!(status, Status::Skipped(_) | Status::Stale(_) | Status::Failed(_)), "{mode}: {status:?}");
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "seed target\nother target\n");
        if mode == "disk-readonly" {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
}

#[gpui::test]
fn previous_matching_lines_choice_survives_original_refresh_publication(cx: &mut TestAppContext) {
    let dir = temp("orlo-previous-lines-original-refresh").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "seed target\n").unwrap();
    let (ws, vcx) = window(cx);
    previous_line_query(&ws, vcx, "seed", path.to_str().unwrap(), None);
    ws.update_in(vcx, |ws, window, cx| {
        ws.find_in_files(&FindInFiles, window, cx);
        ws.choose_where(&ChooseWhere, window, cx);
        let p = ws.quick_panel.as_mut().unwrap();
        let at = p.items.iter().position(|item| item.label == "Previous Matching Lines").unwrap();
        p.selected = p.filtered.iter().position(|i| *i == at).unwrap();
        ws.confirm_quick_panel(window, cx);
        std::fs::write(&path, "seed target\nseed another target\n").unwrap();
        // This refresh belongs to the source query, after its lines were explicitly chosen.
        ws.refresh_results(&RefreshResults, window, cx);
    });
    settle_previous_line_search(&ws, vcx);
    previous_line_query(&ws, vcx, "target", "<previous matching lines>", None);
    ws.read_with(vcx, |ws, _| {
        let result = ws.result_set.as_ref().unwrap();
        assert_eq!(result.counts().matches, 0, "source refresh must not silently renew a previously chosen scope");
        assert!(matches!(&result.files[0].body, crate::searching::results::Body::Failed(why) if why.contains("stale")));
    });
}

#[gpui::test]
fn n13_result_pages_revisit_two_queries_through_reachable_page_controls(cx: &mut TestAppContext) {
    let dir = temp("orlo-n13-causal").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "alpha é\nbeta beta\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    previous_line_query(&ws, vcx, "alpha", path.to_str().unwrap(), Some("first"));
    previous_line_query(&ws, vcx, "beta", path.to_str().unwrap(), Some("second"));
    settle(vcx);
    let first = vcx.debug_bounds("result-page-1").expect("the first query must remain reachable after a second query");
    vcx.simulate_click(first.center(), gpui::Modifiers::default());
    ws.read_with(vcx, |ws, _| {
        let set = ws.result_set.as_ref().unwrap();
        assert_eq!(set.query, "alpha");
        assert_eq!(set.counts().matches, 1);
        assert_eq!(set.replacement.as_deref(), Some("first"));
    });
    let second = vcx.debug_bounds("result-page-2").unwrap();
    vcx.simulate_click(second.center(), gpui::Modifiers::default());
    ws.read_with(vcx, |ws, _| {
        let set = ws.result_set.as_ref().unwrap();
        assert_eq!(set.query, "beta");
        assert_eq!(set.counts().matches, 2);
        assert_eq!(set.replacement.as_deref(), Some("second"));
    });
}

#[gpui::test]
fn n13_result_pages_keep_review_options_and_refresh_only_selected_page(cx: &mut TestAppContext) {
    let dir = temp("orlo-n13-review").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "alpha alpha\nbeta beta\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, w, cx| {
        ws.run_search(
            "alpha",
            path.to_str().unwrap(),
            Some("A"),
            FindOpts { case_sensitive: true, ..Default::default() },
            crate::searching::results::SearchFlags { context: false, ..Default::default() },
            w,
            cx,
        )
        .unwrap();
    });
    settle_previous_line_search(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| {
        ws.results_rows(cx);
        ws.results_panel_mut().unwrap().selected = 1;
        ws.toggle_row(&results_panel::ToggleRow, w, cx);
        ws.collapse_all(&results_panel::CollapseAll, w, cx);
        let p = ws.results_panel_mut().unwrap();
        p.note = Some("review alpha".into());
        p.selected = 0;
        let filter = p.filter.clone();
        filter.update(cx, |e, cx| e.set_text("source", cx));
    });
    previous_line_query(&ws, vcx, "beta", path.to_str().unwrap(), Some("B"));
    ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.results_panel().unwrap().filter.read(cx).text(cx), "");
        ws.exclude_all(&results_panel::ExcludeAll, w, cx);
        ws.select_result_page(1, w, cx);
        let set = ws.result_set.as_ref().unwrap();
        assert_eq!(set.counts().selected, 1);
        assert!(set.opts.case_sensitive);
        assert!(!set.flags.context);
        assert!(set.files[0].collapsed);
        let p = ws.results_panel().unwrap();
        assert_eq!(p.filter.read(cx).text(cx), "source");
        assert_eq!(p.note.as_deref(), Some("review alpha"));
        ws.refresh_results(&RefreshResults, w, cx);
    });
    settle_previous_line_search(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.result_pages.active, 1);
        assert!(ws.results_panel().unwrap().note.is_none());
        assert_eq!(ws.result_pages.inactive.len(), 1);
        assert_eq!(ws.result_set.as_ref().unwrap().counts().selected, 2);
        ws.select_result_page(2, w, cx);
        assert_eq!(ws.result_set.as_ref().unwrap().query, "beta");
        assert_eq!(ws.result_set.as_ref().unwrap().counts().selected, 0);
        assert!(!ws.result_set.as_ref().unwrap().opts.case_sensitive);
    });
}

#[gpui::test]
fn n13_result_pages_apply_selected_checks_stale_and_keeps_one_undo_step(cx: &mut TestAppContext) {
    use crate::searching::results::Status;
    let (ws, vcx, path) = window_on(cx, "orlo-n13-apply", "alpha alpha beta\n");
    let source = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    previous_line_query(&ws, vcx, "alpha", path.to_str().unwrap(), Some("é"));
    ws.update_in(vcx, |ws, _, _| ws.result_set.as_mut().unwrap().files[0].entries_mut()[1].included = false);
    previous_line_query(&ws, vcx, "beta", path.to_str().unwrap(), Some("中"));
    ws.update_in(vcx, |ws, w, cx| {
        ws.select_result_page(1, w, cx);
        ws.apply_replacements(&ApplyReplacements, w, cx);
        assert!(ws.applying.is_some());
        ws.select_result_page(2, w, cx);
        ws.close_result_page(1, w, cx);
        assert_eq!(ws.result_pages.active, 1);
        assert_eq!(ws.result_set.as_ref().unwrap().query, "alpha");
        assert!(ws
            .run_search("new", path.to_str().unwrap(), None, FindOpts::default(), Default::default(), w, cx)
            .is_err());
        ws.refresh_results(&RefreshResults, w, cx);
        assert_eq!(ws.result_pages.active, 1);
    });
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
    source.update(vcx, |e, cx| {
        assert_eq!(e.text(cx), "é alpha beta\n");
        e.edit(cx, |s| s.undo());
        assert_eq!(e.text(cx), "alpha alpha beta\n");
    });
    ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.result_set.as_ref().unwrap().files[0].status, Some(Status::ChangedInBuffer));
        ws.select_result_page(2, w, cx);
        ws.apply_replacements(&ApplyReplacements, w, cx);
    });
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
    assert_eq!(source.read_with(vcx, |e, cx| e.text(cx)), "alpha alpha beta\n");
    ws.update_in(vcx, |ws, w, cx| {
        assert!(matches!(ws.result_set.as_ref().unwrap().files[0].status, Some(Status::Stale(_))));
        ws.refresh_results(&RefreshResults, w, cx);
    });
    settle_previous_line_search(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| ws.apply_replacements(&ApplyReplacements, w, cx));
    settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
    source.update(vcx, |e, cx| {
        assert_eq!(e.text(cx), "alpha alpha 中\n");
        e.edit(cx, |s| s.undo());
        assert_eq!(e.text(cx), "alpha alpha beta\n");
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "alpha alpha beta\n");
}

#[gpui::test]
fn n13_result_pages_bound_retention_and_reject_when_no_inactive_page_has_ended(cx: &mut TestAppContext) {
    let dir = temp("orlo-n13-retention").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "query").unwrap();
    let (ws, vcx) = window(cx);
    for n in 1..=8 {
        previous_line_query(&ws, vcx, &format!("query{n}"), path.to_str().unwrap(), Some("X"));
    }
    ws.update_in(vcx, |ws, w, cx| ws.select_result_page(1, w, cx));
    previous_line_query(&ws, vcx, "ninth", path.to_str().unwrap(), Some("X"));
    ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.result_pages.active, 9);
        assert_eq!(ws.result_pages.inactive.len(), 7);
        assert!(ws.result_page(1).is_some(), "the active oldest page cannot be retired");
        assert!(ws.result_page(2).is_none(), "the oldest inactive ended page is retired");
        for p in &mut ws.result_pages.inactive {
            p.set.done = false;
        }
        let request = ws.result_pages.request;
        let error = ws
            .run_search("tenth", path.to_str().unwrap(), None, FindOpts::default(), Default::default(), w, cx)
            .unwrap_err();
        assert!(error.contains("All eight") && error.contains("close a page"));
        assert_eq!(ws.result_pages.active, 9);
        assert_eq!(ws.result_pages.request, request);
        assert_eq!(ws.result_set.as_ref().unwrap().query, "ninth");
        ws.close_result_page(4, w, cx);
        assert_eq!(ws.result_pages.inactive.len(), 6);
        assert_eq!(ws.result_pages.active, 9);
        ws.select_result_page(2, w, cx);
        assert_eq!(ws.result_pages.active, 9, "queued actions cannot revive a retired ID");
    });
    let (other, ocx) = window(vcx);
    previous_line_query(&other, ocx, "query", path.to_str().unwrap(), Some("other"));
    assert_eq!(other.read_with(ocx, |ws, _| ws.result_pages.active), 1);
    assert_eq!(ws.read_with(ocx, |ws, _| ws.result_pages.active), 9);
}

#[gpui::test]
fn n13_result_pages_running_switch_close_and_late_drains_keep_exact_request(cx: &mut TestAppContext) {
    let dir = temp("orlo-n13-running").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "alpha beta beta\n").unwrap();
    let (ws, vcx) = window(cx);
    previous_line_query(&ws, vcx, "alpha", path.to_str().unwrap(), Some("A"));
    let second_request = ws.update_in(vcx, |ws, w, cx| {
        ws.run_search("beta", path.to_str().unwrap(), Some("B"), FindOpts::default(), Default::default(), w, cx)
            .unwrap();
        let request = ws.result_pages.request;
        ws.select_result_page(1, w, cx);
        assert!(ws.search.is_some());
        request
    });
    settle_previous_line_search(&ws, vcx);
    let refresh_request = ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.result_set.as_ref().unwrap().query, "alpha");
        assert_eq!(ws.result_set.as_ref().unwrap().counts().matches, 1);
        assert_eq!(ws.result_page(2).unwrap().counts().matches, 2);
        assert!(ws.result_page(2).unwrap().done);
        ws.refresh_results(&RefreshResults, w, cx);
        let request = ws.result_pages.request;
        assert!(ws.drain_search_for(2, second_request, cx));
        assert!(ws.search.is_some(), "an old timer cannot consume the newer request");
        ws.close_result_page(1, w, cx);
        assert!(ws.search.is_none());
        assert_eq!(ws.result_pages.active, 2);
        assert!(ws.result_page(1).is_none());
        assert!(ws.drain_search_for(1, request, cx));
        request
    });
    settle(vcx);
    ws.update_in(vcx, |ws, w, cx| {
        assert!(ws.drain_search_for(1, refresh_request, cx));
        assert_eq!(ws.result_set.as_ref().unwrap().counts().matches, 2);
        ws.close_result_page(2, w, cx);
        assert!(ws.result_set.is_none());
        assert!(ws.results_panel().is_none());
        ws.close_result_page(2, w, cx);
    });
}

#[gpui::test]
fn n13_result_pages_cancel_prior_run_and_preserve_appended_text_navigation(cx: &mut TestAppContext) {
    let dir = temp("orlo-n13-text").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "alpha é\nbeta\n").unwrap();
    let (ws, vcx) = window(cx);
    let old_request = ws.update_in(vcx, |ws, w, cx| {
        ws.run_search("never", path.to_str().unwrap(), None, FindOpts::default(), Default::default(), w, cx).unwrap();
        let request = ws.result_pages.request;
        ws.run_search("alpha", path.to_str().unwrap(), None, FindOpts::default(), Default::default(), w, cx).unwrap();
        assert!(ws.result_page(1).unwrap().cancelled);
        assert!(ws.result_page(1).unwrap().done);
        assert!(ws.drain_search_for(1, request, cx));
        request
    });
    settle_previous_line_search(&ws, vcx);
    previous_line_query(&ws, vcx, "beta", path.to_str().unwrap(), None);
    let textual = ws.read_with(vcx, |ws, cx| ws.views.tabs().find(|e| e.read(cx).results).unwrap().clone());
    let text = textual.read_with(vcx, |e, cx| e.text(cx));
    assert!(text.contains("alpha") && text.contains("beta") && text.contains("Cancelled!"));
    ws.update_in(vcx, |ws, w, cx| {
        ws.show_results(&ShowResults, w, cx);
        ws.close_result_page(1, w, cx);
        ws.close_result_page(2, w, cx);
        ws.close_result_page(3, w, cx);
        assert!(ws.drain_search_for(1, old_request, cx));
        assert!(ws.result_set.is_none());
        assert_eq!(textual.read(cx).text(cx), text);
        ws.next_result(&NextResult, w, cx);
    });
    settle(vcx);
    ws.read_with(vcx, |ws, cx| {
        let editor = ws.active_editor().unwrap();
        assert_eq!(editor.read(cx).doc(cx).path.as_ref(), Some(&path));
    });
}

#[gpui::test]
fn n13_result_pages_refresh_retains_resolved_file_scope_and_its_own_n12_basis(cx: &mut TestAppContext) {
    let dir = temp("orlo-n13-scope").canonicalize().unwrap();
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    std::fs::write(&a, "seed alpha target\nother target\n").unwrap();
    std::fs::write(&b, "seed beta target\nother target\n").unwrap();
    let (ws, vcx) = window(cx);
    previous_line_query(&ws, vcx, "seed", &format!("{}, {}", a.display(), b.display()), None);
    previous_line_query(&ws, vcx, "alpha", "<previous results>", Some("A"));
    ws.update_in(vcx, |ws, w, cx| ws.refresh_results(&RefreshResults, w, cx));
    settle_previous_line_search(&ws, vcx);
    ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.result_set.as_ref().unwrap().searched, 2, "Refresh keeps files without current hits");
        ws.select_result_page(1, w, cx);
    });
    previous_line_query(&ws, vcx, "target", "<previous matching lines>", Some("first"));
    let basis = ws.read_with(vcx, |ws, _| ws.result_set.as_ref().unwrap().previous_lines.clone().unwrap());
    previous_line_query(&ws, vcx, "other", b.to_str().unwrap(), None);
    ws.update_in(vcx, |ws, _, _| {
        ws.previous_lines = Some(ws.result_set.as_ref().unwrap().line_scope().unwrap());
    });
    let different = ws.read_with(vcx, |ws, _| ws.previous_lines.clone().unwrap());
    ws.update_in(vcx, |ws, w, cx| {
        ws.select_result_page(3, w, cx);
        ws.refresh_results(&RefreshResults, w, cx);
    });
    settle_previous_line_search(&ws, vcx);
    ws.read_with(vcx, |ws, _| {
        let set = ws.result_set.as_ref().unwrap();
        assert_eq!(set.counts().matches, 2);
        assert!(std::sync::Arc::ptr_eq(set.previous_lines.as_ref().unwrap(), &basis));
        assert!(std::sync::Arc::ptr_eq(ws.previous_lines.as_ref().unwrap(), &different));
    });
}

#[gpui::test]
fn n13_result_pages_close_controls_and_reopen_clear_only_the_closed_review(cx: &mut TestAppContext) {
    let dir = temp("orlo-n13-close-controls").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "alpha beta\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    previous_line_query(&ws, vcx, "alpha", path.to_str().unwrap(), Some("A"));
    previous_line_query(&ws, vcx, "beta", path.to_str().unwrap(), Some("B"));
    vcx.run_until_parked();
    let close = vcx.debug_bounds("close-result-page-1").unwrap();
    vcx.simulate_click(close.center(), gpui::Modifiers::default());
    ws.update_in(vcx, |ws, _, cx| {
        assert!(ws.result_page(1).is_none());
        assert_eq!(ws.result_pages.active, 2);
        let p = ws.results_panel_mut().unwrap();
        p.note = Some("closed review".into());
        let filter = p.filter.clone();
        filter.update(cx, |e, cx| e.set_text("hidden", cx));
    });
    vcx.run_until_parked();
    let close = vcx.debug_bounds("close-result-page-2").unwrap();
    vcx.simulate_click(close.center(), gpui::Modifiers::default());
    previous_line_query(&ws, vcx, "alpha", path.to_str().unwrap(), Some("new"));
    ws.read_with(vcx, |ws, cx| {
        assert_eq!(ws.result_pages.active, 3);
        assert_eq!(ws.results_panel().unwrap().filter.read(cx).text(cx), "");
        assert!(ws.results_panel().unwrap().note.is_none());
    });
}

#[gpui::test]
fn n13_result_pages_late_filter_events_cannot_reset_another_pages_review(cx: &mut TestAppContext) {
    let dir = temp("orlo-n13-filter-events").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "alpha alpha beta beta\n").unwrap();
    let (ws, vcx) = window(cx);
    previous_line_query(&ws, vcx, "alpha", path.to_str().unwrap(), Some("A"));
    previous_line_query(&ws, vcx, "beta", path.to_str().unwrap(), Some("B"));
    ws.update_in(vcx, |ws, w, cx| {
        ws.select_result_page(1, w, cx);
        let filter = ws.results_panel().unwrap().filter.clone();
        filter.update(cx, |e, cx| e.set_text("source", cx));
        ws.select_result_page(2, w, cx);
        ws.results_rows(cx);
        let panel = ws.results_panel_mut().unwrap();
        panel.selected = 1;
        panel.note = Some("beta review".into());
    });
    vcx.run_until_parked();
    ws.update_in(vcx, |ws, w, cx| {
        let panel = ws.results_panel().unwrap();
        assert_eq!(panel.filter.read(cx).text(cx), "");
        assert_eq!(panel.selected, 1, "another page's deferred filter edit cannot reset this row");
        assert_eq!(panel.note.as_deref(), Some("beta review"));
        ws.select_result_page(1, w, cx);
        assert_eq!(ws.results_panel().unwrap().filter.read(cx).text(cx), "source");
    });
}

#[gpui::test]
fn n13_result_pages_handoff_native_filter_focus_and_composition_to_selected_page(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let dir = temp("orlo-n13-filter-focus").canonicalize().unwrap();
    let path = dir.join("source.txt");
    std::fs::write(&path, "alpha beta\n").unwrap();
    let (ws, vcx) = window(cx);
    ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    previous_line_query(&ws, vcx, "alpha", path.to_str().unwrap(), Some("A"));
    previous_line_query(&ws, vcx, "beta", path.to_str().unwrap(), Some("B"));
    let composing = ws.update_in(vcx, |ws, w, cx| {
        ws.select_result_page(1, w, cx);
        ws.focus_filter(&results_panel::FocusFilter, w, cx);
        let filter = ws.results_panel().unwrap().filter.clone();
        filter.update(cx, |e, cx| e.replace_and_mark_text_in_range(None, "仮", None, w, cx));
        assert!(!filter.read(cx).marked.is_empty());
        filter
    });
    vcx.run_until_parked();
    let page = vcx.debug_bounds("result-page-2").unwrap();
    vcx.simulate_click(page.center(), gpui::Modifiers::default());
    ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.result_pages.active, 2);
        assert!(ws.results_panel().unwrap().focus.is_focused(w));
        assert!(!composing.read(cx).is_focused(w));
        assert_ne!(ws.results_panel().unwrap().filter, composing);
        ws.focus_filter(&results_panel::FocusFilter, w, cx);
        assert!(ws.results_panel().unwrap().filter.read(cx).is_focused(w));
    });
    vcx.simulate_input("selected");
    ws.update_in(vcx, |ws, w, cx| {
        assert_eq!(ws.results_panel().unwrap().filter.read(cx).text(cx), "selected");
        assert_eq!(composing.read(cx).text(cx), "仮");
        ws.select_result_page(1, w, cx);
        assert_eq!(ws.results_panel().unwrap().filter, composing);
        assert_eq!(ws.results_panel().unwrap().filter.read(cx).text(cx), "仮");
        assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "alpha beta\n");
    });
}

// N14: native action ownership, complete-source behavior and transient presentation state.
#[gpui::test]
fn n14_native_bindings_refuse_local_shared_composition_preserving_full_state_and_redo(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    let actions = [
        ("cmd-alt-1", "editor::NarrowToSelection", serde_json::json!({})),
        ("cmd-alt-2", "editor::NarrowToFold", serde_json::json!({})),
        ("cmd-alt-3", "editor::ShowMatchingLines", serde_json::json!({"pattern":"hit"})),
        ("cmd-alt-4", "editor::ClearFilter", serde_json::json!({})),
    ];
    let bindings: Vec<_> = actions
        .iter()
        .map(|(key, name, args)| serde_json::json!({"keys":[key],"command":name,"args":args,"context":"Editor"}))
        .collect();
    std::fs::write(crate::config::customization::keymap_path(), serde_json::to_string(&bindings).unwrap()).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "before\nhit\nafter\n");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    on_editor(&ws, vcx, |e, w, cx| {
        let end = e.doc(cx).len();
        e.edit(cx, |s| s.replace_ranges(vec![(end..end, "!".into())]));
        e.perform_action(&editor::Undo, w, cx);
    });
    // Mark unchanged source bytes so a genuine Redo is still present.
    views[0].update_in(vcx, |e, w, cx| e.replace_and_mark_text_in_range(Some(7..10), "hit", Some(0..2), w, cx));
    let peer = views[0].read_with(vcx, |e, cx| (e.marked.clone(), e.read_state(cx).selection_state(), e.text(cx)));
    for local in [false, true] {
        if local {
            views[0].update_in(vcx, |e, w, cx| e.unmark_text(w, cx));
            on_editor(&ws, vcx, |e, w, cx| e.replace_and_mark_text_in_range(Some(7..10), "hit", Some(0..2), w, cx));
        }
        for (key, name, args) in &actions {
            let before = on_editor(&ws, vcx, |e, _, cx| {
                e.sel(cx).sel_checkpoint();
                (
                    e.text(cx),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.doc(cx).unsaved(),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation(),
                    e.marked.clone(),
                    e.folds().clone(),
                )
            });
            vcx.simulate_keystrokes(key);
            let after = on_editor(&ws, vcx, |e, _, cx| {
                (
                    e.text(cx),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.doc(cx).unsaved(),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation(),
                    e.marked.clone(),
                    e.folds().clone(),
                )
            });
            assert!(after == before, "native {name} must refuse before touching view state");
            ws.update_in(vcx, |ws, w, cx| {
                assert!(ws
                    .execute_command(
                        &crate::keybindings::Command { command: (*name).into(), args: args.clone() },
                        w,
                        cx
                    )
                    .is_err())
            });
            if !local {
                assert!(
                    views[0].read_with(vcx, |e, cx| (e.marked.clone(), e.read_state(cx).selection_state(), e.text(cx)))
                        == peer
                );
            }
        }
    }
    on_editor(&ws, vcx, |e, w, cx| e.unmark_text(w, cx));
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "before\nhit\nafter\n!");
}

#[gpui::test]
fn n14_native_menu_palette_form_defaults_validation_stale_and_resolved_macro(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    assert_eq!(
        names(&submenu(&ws, vcx, "Edit", "Narrow View")),
        ["Narrow to Selection", "Narrow to Fold", "Show Matching Lines…", "Clear Filter"]
    );
    let palette = crate::overlays::quick_panel::command_items();
    assert!(palette.iter().any(|item| item.label == "View: Show Matching Lines…"));
    type_in(&ws, vcx, "before\nhit\nafter\n");
    set_selections(&ws, vcx, &[(7, 10)]);
    vcx.dispatch_action(ShowMatchingLinesPrompt);
    vcx.simulate_input("hit");
    vcx.simulate_keystrokes("tab");
    vcx.simulate_input("-1");
    vcx.simulate_keystrokes("enter");
    assert!(form_error(&ws, vcx).unwrap().unwrap().contains("nonnegative"));
    assert!(on_editor(&ws, vcx, |e, _, _| e.folds().is_empty()));
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("0");
    vcx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    vcx.simulate_keystrokes("enter");
    settle_until(vcx, |cx| on_editor(&ws, cx, |e, _, _| !e.folds().is_empty()));
    let recorded = vcx.update(|_, cx| {
        let r = cx.default_global::<Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].command, "editor::ShowMatchingLines");
    assert_eq!(recorded[0].args, serde_json::json!({"pattern":"hit","context":0,"ignore_case":false}));
    vcx.dispatch_action(editor::ClearFilter);
    vcx.update(|_, cx| cx.default_global::<Recorder>().last = recorded);
    vcx.dispatch_action(ReplayMacro);
    settle_until(vcx, |cx| on_editor(&ws, cx, |e, _, _| !e.folds().is_empty()));
    vcx.dispatch_action(editor::ClearFilter);
    vcx.dispatch_action(ShowMatchingLinesPrompt);
    set_selections(&ws, vcx, &[(0, 0)]);
    set_selections(&ws, vcx, &[(7, 10)]);
    submit_form(vcx, "hit");
    assert!(form_error(&ws, vcx).unwrap().unwrap().contains("changed"));
    assert_eq!(text(&ws, vcx), "before\nhit\nafter\n");
}

#[gpui::test]
fn n14_native_filters_allow_viewer_follow_readonly_and_ignore_wrong_focus(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    std::fs::write(crate::config::customization::keymap_path(), r#"[{"keys":["cmd-alt-1"],"command":"editor::NarrowToSelection"},{"keys":["cmd-alt-2"],"command":"editor::ClearFilter"}]"#).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "before\nhit\nafter\n");
    for mode in ["ordinary", "viewer", "follow", "readonly"] {
        on_editor(&ws, vcx, |e, w, cx| {
            e.input_enabled = mode != "viewer";
            e.update_doc(cx, |d| {
                d.set_following(mode == "follow");
                d.read_only = mode == "readonly";
            });
            e.sel(cx).set_selection(7, 10);
            e.focus(w);
        });
        let before = on_editor(&ws, vcx, |e, _, cx| {
            (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved())
        });
        vcx.update(|_, cx| {
            let r = cx.default_global::<Recorder>();
            r.commands.clear();
            r.recording = true;
        });
        vcx.simulate_keystrokes("cmd-alt-1 cmd-alt-2");
        let commands = vcx.update(|_, cx| {
            let r = cx.default_global::<Recorder>();
            r.recording = false;
            r.commands.clone()
        });
        assert_eq!(commands.len(), 2, "native capture once: {mode}");
        assert_eq!(
            on_editor(&ws, vcx, |e, _, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.doc(cx).unsaved()
            )),
            before
        );
        assert!(on_editor(&ws, vcx, |e, _, _| e.folds().is_empty()));
    }
    vcx.dispatch_action(Find);
    vcx.simulate_keystrokes("cmd-alt-1");
    assert!(on_editor(&ws, vcx, |e, _, _| e.folds().is_empty()), "a field binding cannot change the source view");
}

#[gpui::test]
fn n14_shared_hidden_edits_recompute_source_and_capture_only_mapped_prior_folds(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "before\nhit\nafter\nmore\n");
    let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    first.update(vcx, |e, cx| {
        e.fold_chars_now([16..20, 17..19], cx);
        e.sel(cx).set_selection(7, 10);
    });
    vcx.dispatch_action(editor::NarrowToSelection);
    new_view(&ws, vcx);
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.replace_ranges(vec![(0..0, "雪".into()), (11..16, "HIT".into())])));
    first.update(vcx, |e, cx| e.catch_up(cx));
    let captured = first.read_with(vcx, |e, cx| e.capture_folds(cx).unwrap().restore(e.doc(cx)));
    assert_eq!(captured.chars(), [15..19, 16..18]);
    first.update_in(vcx, |e, w, cx| e.perform_action(&editor::ClearFilter, w, cx));
    assert_eq!(first.read_with(vcx, |e, _| e.folds().clone()), captured);
    let before = first.read_with(vcx, |e, cx| e.text(cx));
    assert_eq!(before, "雪before\nhit\nHIT\nmore\n");
    first.update_in(vcx, |e, w, cx| {
        e.perform_action(&editor::ShowMatchingLines { pattern: "HIT".into(), context: 0, ignore_case: false }, w, cx)
    });
    settle_until(vcx, |cx| first.read_with(cx, |e, _| e.folds().hiding(2).is_some()));
    assert_eq!(text(&ws, vcx), before);
}

#[gpui::test]
fn n14_delayed_regex_reply_defers_shared_marked_text_and_rejects_newer_source(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "before\nhit\nafter\n");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    // Start work and mark identical bytes in one event, before any reply can publish.
    ws.update_in(vcx, |ws, w, cx| {
        ws.execute_command(
            &crate::keybindings::Command {
                command: "editor::ShowMatchingLines".into(),
                args: serde_json::json!({"pattern":"hit"}),
            },
            w,
            cx,
        )
        .unwrap();
        views[0].update(cx, |e, cx| e.replace_and_mark_text_in_range(Some(7..10), "hit", Some(0..2), w, cx));
    });
    let peer = views[0].read_with(vcx, |e, cx| (e.marked.clone(), e.read_state(cx).selection_state(), e.text(cx)));
    settle_until(vcx, |cx| views[1].read_with(cx, |e, _| !e.filter_pending()));
    assert!(
        views[1].read_with(vcx, |e, _| e.folds().is_empty()),
        "reply must not publish exclusions during shared composition"
    );
    assert!(
        views[0].read_with(vcx, |e, cx| (e.marked.clone(), e.read_state(cx).selection_state(), e.text(cx))) == peer
    );
    views[0].update_in(vcx, |e, w, cx| e.unmark_text(w, cx));
    vcx.dispatch_action(editor::ClearFilter);
    ws.update_in(vcx, |ws, w, cx| {
        ws.execute_command(
            &crate::keybindings::Command {
                command: "editor::ShowMatchingLines".into(),
                args: serde_json::json!({"pattern":"hit"}),
            },
            w,
            cx,
        )
        .unwrap();
        views[0].update(cx, |e, cx| e.edit(cx, |s| s.replace_ranges(vec![(7..10, "gone".into())])));
    });
    settle_until(vcx, |cx| {
        views[1].read_with(cx, |e, cx| e.folds().chars() == std::slice::from_ref(&(0..e.doc(cx).len())))
    });
    assert_eq!(text(&ws, vcx), "before\ngone\nafter\n");
}

#[gpui::test]
fn n14_new_shared_view_starts_without_transient_exclusions_and_filters_cancel_independently(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "before\nhit\nafter\n");
    set_selections(&ws, vcx, &[(7, 10)]);
    vcx.dispatch_action(editor::NarrowToSelection);
    let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    let first_folds = first.read_with(vcx, |e, _| e.folds().clone());
    assert!(!first_folds.is_empty());
    new_view(&ws, vcx);
    assert!(on_editor(&ws, vcx, |e, _, _| e.folds().is_empty()));
    vcx.dispatch_action(editor::ShowMatchingLines { pattern: "after".into(), context: 0, ignore_case: false });
    settle_until(vcx, |cx| on_editor(&ws, cx, |e, _, _| !e.filter_pending()));
    let second_folds = on_editor(&ws, vcx, |e, _, _| e.folds().clone());
    assert_ne!(second_folds, first_folds);
    assert_eq!(first.read_with(vcx, |e, _| e.folds().clone()), first_folds);
    vcx.dispatch_action(editor::ClearFilter);
    assert!(on_editor(&ws, vcx, |e, _, _| e.folds().is_empty()));
    assert_eq!(first.read_with(vcx, |e, _| e.folds().clone()), first_folds);
    assert_eq!(text(&ws, vcx), "before\nhit\nafter\n");
}

#[gpui::test]
fn n15_native_menu_palette_json_macro_and_readonly_viewer_follow(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, vcx) = window(cx);
    let expected = [
        "Move to Next Fold",
        "Move to Previous Fold",
        "Move to Parent Fold",
        "Select to Next Fold",
        "Select to Previous Fold",
        "Select to Parent Fold",
        "Select Fold",
    ];
    assert_eq!(names(&submenu(&ws, vcx, "Edit", "Fold Navigation")), expected);
    let items = crate::overlays::quick_panel::command_items();
    for label in expected {
        assert!(items.iter().any(|item| item.label == format!("Fold Navigation: {label}")));
    }
    std::fs::write(crate::config::customization::keymap_path(), r#"[{"keys":["cmd-alt-1"],"command":"editor::MoveToFold","args":{"motion":"next"}},{"keys":["cmd-alt-2"],"command":"editor::SelectFold"}]"#).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "a\n  雪😀\nb\n  c\n");
    for mode in ["ordinary", "readonly", "viewer", "follow"] {
        on_editor(&ws, vcx, |e, w, cx| {
            e.input_enabled = mode != "viewer";
            e.update_doc(cx, |d| {
                d.read_only = mode == "readonly";
                d.set_following(mode == "follow");
            });
            e.sel(cx).set_cursor(0);
            e.focus(w);
        });
        let before = on_editor(&ws, vcx, |e, _, cx| {
            (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved())
        });
        vcx.update(|_, cx| {
            let r = cx.default_global::<Recorder>();
            r.commands.clear();
            r.recording = true;
        });
        vcx.simulate_keystrokes("cmd-alt-1 cmd-alt-2");
        let commands = vcx.update(|_, cx| {
            let r = cx.default_global::<Recorder>();
            r.recording = false;
            r.commands.clone()
        });
        assert_eq!(commands.len(), 2, "native capture once: {mode}");
        assert_eq!(commands[0].command, "editor::MoveToFold");
        assert_eq!(commands[0].args, serde_json::json!({"motion":"next","select":false}));
        let selected = on_editor(&ws, vcx, |e, _, cx| {
            assert_eq!((e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved()), before);
            e.read_state(cx).selection_state()
        });
        assert_eq!(selected.selections, [crate::editing::text::Selection::new(7, 12)]);
        on_editor(&ws, vcx, |e, _, cx| e.sel(cx).set_cursor(0));
        // Macro/JSON dispatch must also work back-to-back without any render frame.
        ws.update_in(vcx, |ws, w, cx| {
            for command in &commands {
                ws.execute_command(command, w, cx).unwrap();
            }
        });
        assert!(on_editor(&ws, vcx, |e, _, cx| e.read_state(cx).selection_state()) == selected);
    }
    vcx.dispatch_action(Find);
    let before = on_editor(&ws, vcx, |e, _, cx| e.read_state(cx).selection_state());
    vcx.simulate_keystrokes("cmd-alt-1");
    assert!(
        on_editor(&ws, vcx, |e, _, cx| e.read_state(cx).selection_state()) == before,
        "wrong focus must not navigate the source"
    );
}

#[gpui::test]
fn n15_native_and_json_refuse_local_shared_composition_preserving_state_and_redo(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    let actions = [
        ("cmd-alt-1", "editor::MoveToFold", serde_json::json!({"motion":"next","select":true})),
        ("cmd-alt-2", "editor::SelectFold", serde_json::json!({})),
    ];
    let bindings: Vec<_> = actions
        .iter()
        .map(|(key, name, args)| serde_json::json!({"keys":[key],"command":name,"args":args,"context":"Editor"}))
        .collect();
    std::fs::write(crate::config::customization::keymap_path(), serde_json::to_string(&bindings).unwrap()).unwrap();
    ws.update_in(vcx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
    type_in(&ws, vcx, "head\n  body\ntail\n  end\n");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    on_editor(&ws, vcx, |e, w, cx| {
        let end = e.doc(cx).len();
        e.edit(cx, |s| s.replace_ranges(vec![(end..end, "!".into())]));
        e.perform_action(&editor::Undo, w, cx);
    });
    views[0].update_in(vcx, |e, w, cx| e.replace_and_mark_text_in_range(Some(7..11), "body", Some(0..2), w, cx));
    let peer = views[0].read_with(vcx, |e, cx| (e.marked.clone(), e.read_state(cx).selection_state(), e.text(cx)));
    for local in [false, true] {
        if local {
            views[0].update_in(vcx, |e, w, cx| e.unmark_text(w, cx));
            on_editor(&ws, vcx, |e, w, cx| e.replace_and_mark_text_in_range(Some(7..11), "body", Some(0..2), w, cx));
        }
        for (key, name, args) in &actions {
            let before = on_editor(&ws, vcx, |e, _, cx| {
                e.sel(cx).sel_checkpoint();
                (
                    e.text(cx),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.doc(cx).unsaved(),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation(),
                    e.marked.clone(),
                    e.folds().clone(),
                )
            });
            vcx.simulate_keystrokes(key);
            let after = on_editor(&ws, vcx, |e, _, cx| {
                (
                    e.text(cx),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.doc(cx).unsaved(),
                    e.read_state(cx).selection_state(),
                    e.read_state(cx).selection_generation(),
                    e.marked.clone(),
                    e.folds().clone(),
                )
            });
            assert!(after == before, "native {name} must refuse before touching view state");
            ws.update_in(vcx, |ws, w, cx| {
                assert!(ws
                    .execute_command(
                        &crate::keybindings::Command { command: (*name).into(), args: args.clone() },
                        w,
                        cx
                    )
                    .is_err());
            });
            if !local {
                assert!(
                    views[0].read_with(vcx, |e, cx| (e.marked.clone(), e.read_state(cx).selection_state(), e.text(cx)))
                        == peer
                );
            }
        }
    }
    on_editor(&ws, vcx, |e, w, cx| e.unmark_text(w, cx));
    vcx.dispatch_action(editor::Redo);
    assert_eq!(text(&ws, vcx), "head\n  body\ntail\n  end\n!");
}

#[gpui::test]
fn n15_native_shared_edit_refreshes_narrowing_before_resolving_destinations(cx: &mut TestAppContext) {
    use crate::editing::fold;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "head\n  inner\n    body\ntail\n");
    new_view(&ws, vcx);
    let views: Vec<_> = ws.read_with(vcx, |ws, _| ws.views.tabs().cloned().collect());
    let active = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    let peer = views.into_iter().find(|e| e != &active).unwrap();
    let cases: Vec<(Box<dyn gpui::Action>, Selection)> = vec![
        (Box::new(editor::MoveToFold { motion: fold::Motion::Next, select: false }), Selection::caret(5)),
        (Box::new(editor::MoveToFold { motion: fold::Motion::Next, select: true }), Selection::new(0, 5)),
        (Box::new(editor::SelectFold), Selection::new(0, 21)),
    ];
    for (action, expected) in cases {
        active.update_in(vcx, |e, w, cx| {
            e.sel(cx).set_selection(0, 4);
            e.perform_action(&editor::NarrowToSelection, w, cx);
            e.focus(w);
        });
        // The peer's edit and native dispatch share one update: no frame can refresh
        // the narrowed view before the installed workspace action handler runs.
        let before = vcx.update(|w, cx| {
            peer.update(cx, |e, cx| {
                let end = e.doc(cx).len();
                e.edit(cx, |s| s.replace_ranges(vec![(end..end, "雪😀".into())]));
            });
            let before = active.read(cx);
            let identity =
                (before.text(cx), before.doc(cx).version, before.doc(cx).save_point(), before.doc(cx).unsaved());
            w.dispatch_action(action, cx);
            identity
        });
        active.read_with(vcx, |e, cx| {
            assert_eq!(e.state.selections, [expected]);
            assert!(e.folds().is_empty(), "hidden destination must clear narrowing without being clamped");
            assert_eq!((e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.doc(cx).unsaved()), before);
            assert_eq!(e.text(cx), peer.read(cx).text(cx));
        });
    }
}

#[gpui::test]
fn n20_bookmark_picker_lists_two_files_and_deduplicates_shared_views(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    let dir = temp("n20-bookmark-files").canonicalize().unwrap();
    let first = dir.join("café.txt");
    let second = dir.join("other.txt");
    std::fs::write(&first, "first\n雪 café\nlast").unwrap();
    std::fs::write(&second, "other\nneedle").unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(first.clone(), window, cx);
        ws.with_active(cx, |e, cx| {
            e.goto_line(2, cx);
            e.sel(cx).toggle_bookmark();
        });
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        ws.open_path(second, window, cx);
        ws.with_active(cx, |e, cx| {
            e.goto_line(2, cx);
            e.sel(cx).toggle_bookmark();
        });
        ws.execute_command(
            &crate::keybindings::Command { command: "workspace::GotoBookmark".into(), args: serde_json::json!({}) },
            window,
            cx,
        )
        .unwrap();
        let p = ws.quick_panel.as_mut().expect("bookmark picker opens");
        assert_eq!(p.items.len(), 2, "shared document lines appear once");
        p.refilter("café");
        assert_eq!(p.filtered.len(), 1, "filename and Unicode preview are searchable");
        let item = &p.items[p.filtered[0]];
        assert!(item.label.contains("café.txt"));
        assert!(item.label.contains('2'));
        assert!(item.detail.contains("雪 café"));
    });
}

#[gpui::test]
fn n21_explicit_selected_address_opens_existing_unicode_file(cx: &mut TestAppContext) {
    let dir = temp("orlo-n21-causal").canonicalize().unwrap();
    let target = write_in(&dir, "雪 notes!", "first\n雪 target\nlast");
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, &format!("\"{}\":2:3", target.display()));
    ws.update_in(vcx, |ws, w, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).select_all());
        let result = ws.execute_command(
            &crate::keybindings::Command { command: "workspace::OpenSelectedPath".into(), args: serde_json::json!({}) },
            w,
            cx,
        );
        assert!(result.is_ok(), "selected path action must exist: {result:?}");
    });
    settle_until(vcx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref() == Some(&target))
    });
    ws.read_with(vcx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        let e = e.read(cx);
        assert_eq!(e.state.cursor(), 8);
        assert_eq!(e.text(cx), "first\n雪 target\nlast");
    });
}

#[gpui::test]
fn n23_viewer_command_protects_dirty_shared_unicode_input(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "dirty😀中\n");
    let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    new_view(&ws, vcx);
    let peer = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    let before = first.read_with(vcx, |e, cx| (e.text(cx), e.doc(cx).save_point(), e.doc(cx).unsaved()));
    ws.update_in(vcx, |ws, w, cx| {
        let command = crate::config::customization::Command {
            command: "workspace::ToggleViewerMode".into(),
            args: serde_json::json!({}),
        };
        // Absent toggle leaves the real buffer editable on the causal baseline.
        let _ = ws.execute_command(&command, w, cx);
    });
    for editor in [&first, &peer] {
        editor.update_in(vcx, |e, w, cx| {
            e.replace_text_in_range(None, "BAD", w, cx);
            e.paste_entry(&crate::editing::clipboard_history::ClipEntry { text: "BAD".into(), whole_lines: false }, cx);
            e.replace_and_mark_text_in_range(None, "悪", None, w, cx);
            e.edit(cx, |s| {
                s.insert("BAD");
                s.undo();
                s.redo();
            });
            assert_eq!((e.text(cx), e.doc(cx).save_point(), e.doc(cx).unsaved()), before);
            assert!(e.marked.is_empty());
            e.sel(cx).select_all();
            assert_eq!(e.read_state(cx).copy_text().0, before.0);
        });
    }
}

#[gpui::test]
fn n23_toggle_menu_palette_key_json_indicator_and_underlying_input(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, vcx) = window(cx);
    assert!(menu(&ws, vcx, "File").iter().any(|item| matches!(item,
        MenuItem::Action { name, action, .. } if name == "Toggle Viewer Mode" && action.as_any().is::<ToggleViewerMode>())));
    assert!(quick_panel::command_items().iter().any(|item| item.label == "File: Toggle Viewer Mode"
        && matches!(&item.action, ItemAction::Dispatch(action) if action.as_any().is::<ToggleViewerMode>())));
    let keys = vcx.update(|_, cx| {
        crate::keybindings::bindings(
            r#"[{"keys":["cmd-alt-1"],"command":"workspace::ToggleViewerMode","context":"Editor && !single_line"}]"#,
            cx,
        )
        .unwrap()
    });
    vcx.update(|_, cx| cx.bind_keys(keys));
    type_in(&ws, vcx, "a😀");
    vcx.simulate_keystrokes("cmd-alt-1");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("viewer-mode").is_some());
    ws.update(vcx, |ws, cx| ws.status_message("Other status", cx));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("viewer-mode").is_some());
    ws.update(vcx, |ws, cx| {
        ws.show_status_bar = false;
        ws.show_tabs = false;
        assert!(ws.window_title(cx).contains("[VIEWER]"));
    });
    vcx.simulate_keystrokes("x cmd-z cmd-shift-z");
    assert_eq!(text(&ws, vcx), "a😀");
    vcx.dispatch_action(editor::SelectAll);
    vcx.dispatch_action(editor::Copy);
    assert_eq!(vcx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap()), "a😀");
    vcx.dispatch_action(editor::ConvertCase { case: Case::Upper });
    assert_eq!(text(&ws, vcx), "a😀");
    ws.update(vcx, |ws, cx| {
        ws.show_status_bar = true;
        cx.notify();
    });
    vcx.run_until_parked();
    let button = vcx.debug_bounds("viewer-mode").unwrap();
    vcx.simulate_click(button.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    assert!(
        ws.read_with(vcx, |ws, cx| !ws.active_editor().unwrap().read(cx).doc(cx).viewer_mode()),
        "click should unlock; button={button:?}"
    );
    assert!(ws.read_with(vcx, |ws, cx| !ws.window_title(cx).contains("[VIEWER]")));
    let e = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    e.update_in(vcx, |e, w, cx| {
        e.input_enabled = false;
        e.update_doc(cx, |d| d.read_only = true);
        e.replace_text_in_range(None, "BAD", w, cx);
    });
    vcx.dispatch_action(ToggleViewerMode);
    vcx.dispatch_action(ToggleViewerMode);
    e.update_in(vcx, |e, w, cx| {
        assert!(!e.input_enabled && e.doc(cx).read_only && !e.doc(cx).viewer_mode());
        e.replace_text_in_range(None, "BAD", w, cx);
        assert_eq!(e.text(cx), "a😀");
    });
}

#[gpui::test]
fn n23_shared_composition_identity_refusal_and_transient_reopen(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let path = temp("orlo-n23-composition").canonicalize().unwrap().join("a.txt");
    std::fs::write(&path, "source😀").unwrap();
    let (ws, _) = window(cx);
    let (other, _) = window(cx);
    let mut a = gpui::VisualTestContext::from_window(cx.windows()[0], cx);
    let mut b = gpui::VisualTestContext::from_window(cx.windows()[1], cx);
    ws.update_in(&mut a, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    other.update_in(&mut b, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    let first = ws.read_with(&a, |ws, _| ws.active_editor().unwrap());
    let peer = other.read_with(&b, |ws, _| ws.active_editor().unwrap());
    peer.update_in(&mut b, |e, w, cx| e.replace_and_mark_text_in_range(None, "仮", None, w, cx));
    let before = peer.read_with(&b, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
    ws.update_in(&mut a, |ws, w, cx| ws.toggle_viewer_mode(&ToggleViewerMode, w, cx));
    assert!(!first.read_with(&a, |e, cx| e.doc(cx).viewer_mode()));
    assert_eq!(peer.read_with(&b, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)), before);
    assert!(ws.read_with(&a, |ws, _| ws.status.as_ref().unwrap().0.contains("every shared view")));
    peer.update_in(&mut b, |e, w, cx| e.unmark_text(w, cx));
    first.update_in(&mut a, |e, w, cx| e.replace_and_mark_text_in_range(None, "地", None, w, cx));
    let local = first.read_with(&a, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
    ws.update_in(&mut a, |ws, w, cx| {
        ws.execute_command(
            &crate::config::customization::Command {
                command: "workspace::ToggleViewerMode".into(),
                args: serde_json::json!({}),
            },
            w,
            cx,
        )
        .unwrap();
    });
    assert_eq!(first.read_with(&a, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)), local);
    assert!(!first.read_with(&a, |e, cx| e.doc(cx).viewer_mode()));
    first.update_in(&mut a, |e, w, cx| e.unmark_text(w, cx));
    ws.update_in(&mut a, |ws, w, cx| ws.toggle_viewer_mode(&ToggleViewerMode, w, cx));
    assert!(peer.read_with(&b, |e, cx| e.doc(cx).viewer_mode()));
    // A supported same-path collision is a distinct document, not a shared view.
    other.update_in(&mut b, |ws, w, cx| {
        ws.new_file(&NewFile, w, cx);
        let distinct = ws.active_editor().unwrap();
        distinct.update(cx, |e, cx| {
            e.update_doc(cx, |d| d.path = Some(path.clone()));
            e.replace_and_mark_text_in_range(None, "別", None, w, cx);
        });
    });
    ws.update_in(&mut a, |ws, w, cx| ws.toggle_viewer_mode(&ToggleViewerMode, w, cx));
    assert!(!first.read_with(&a, |e, cx| e.doc(cx).viewer_mode()));
    ws.update_in(&mut a, |ws, w, cx| {
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
        ws.close_editor(&first, w, cx);
        ws.open_path(path.with_file_name("fresh.txt"), w, cx);
        assert!(!ws.active_editor().unwrap().read(cx).doc(cx).viewer_mode());
    });
}

#[gpui::test]
fn n23_viewer_flag_does_not_survive_last_close_or_session_restore(cx: &mut TestAppContext) {
    let path = temp("orlo-n23-transient").canonicalize().unwrap().join("a.txt");
    std::fs::write(&path, "source😀\n").unwrap();
    let (ws, cx) = window(cx);
    let saved = ws.update_in(cx, |ws, w, cx| {
        ws.open_path(path.clone(), w, cx);
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
        let old = ws.active_editor().unwrap();
        assert!(old.read(cx).doc(cx).viewer_mode());
        let saved = ws.capture(w, false, &mut Texts::new(), cx);
        let json = serde_json::to_string(&saved).unwrap();
        assert!(!json.contains("viewer_mode"));
        ws.close_editor(&old, w, cx);
        ws.reopen_closed_file(&ReopenClosedFile, w, cx);
        let fresh = ws.active_editor().unwrap();
        assert!(!fresh.read(cx).same_document(old.read(cx)));
        assert!(!fresh.read(cx).doc(cx).viewer_mode());
        ws.close_editor(&fresh, w, cx);
        serde_json::from_str::<WindowState>(&json).unwrap()
    });
    ws.update_in(cx, |ws, w, cx| ws.restore(saved, w, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
    ws.read_with(cx, |ws, cx| {
        let all: Vec<_> = ws.views.tabs().collect();
        assert!(!all.is_empty());
        assert!(all.iter().all(|e| !e.read(cx).doc(cx).viewer_mode()));
        assert!(all.iter().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&path)));
    });
}

#[gpui::test]
fn n25_ui_convert_endings_is_a_metadata_only_undo_step(cx: &mut TestAppContext) {
    let (ws, vcx, _) = window_on(cx, "orlo-n25-ui-convert", "a\r\nb\nc");
    settle_until(vcx, |cx| {
        ws.read_with(cx, |ws, cx| ws.active_editor().is_some_and(|e| e.read(cx).doc(cx).path.is_some()))
    });
    ws.update_in(vcx, |ws, window, cx| ws.set_line_ending(&SetLineEnding { ending: "\r" }, window, cx));
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.undo()));
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).line_ending), "\n");
    assert!(!on_editor(&ws, vcx, |e, _, cx| e.doc(cx).dirty));
}

#[gpui::test]
fn n25_inspector_navigation_native_conversion_and_protected_routes(cx: &mut TestAppContext) {
    let (ws, vcx, _) = window_on(cx, "orlo-n25-native-inspect", "a\r\nb\nc\rd");
    assert!(quick_panel::command_items().iter().any(|i| i.label == "Document: Convert Line Endings to CRLF"));
    assert!(menu(&ws, vcx, "View").iter().any(|i| matches!(i, MenuItem::Submenu(m) if m.name == "Line Endings")));
    let keys = vcx.update(|_, cx| crate::keybindings::bindings(r#"[{"keys":["cmd-alt-1"],"command":"workspace::SetLineEnding","args":{"ending":"\r\n"},"context":"Editor && !single_line"}]"#, cx).unwrap());
    vcx.update(|_, cx| cx.bind_keys(keys));
    vcx.simulate_keystrokes("cmd-alt-1");
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).bytes_as_saved(None).unwrap()), b"a\r\nb\r\nc\r\nd");
    on_editor(&ws, vcx, |e, _, cx| e.edit(cx, |s| s.undo()));
    ws.update_in(vcx, |ws, w, cx| ws.toggle_viewer_mode(&ToggleViewerMode, w, cx));
    let before = on_editor(&ws, vcx, |e, _, cx| (e.doc(cx).save_point(), e.doc(cx).version));
    vcx.simulate_keystrokes("cmd-alt-1");
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| (e.doc(cx).save_point(), e.doc(cx).version)), before);
    ws.update_in(vcx, |ws, w, cx| ws.inspect_line_endings(&InspectLineEndings, w, cx));
    ws.read_with(vcx, |ws, _| {
        let p = ws.quick_panel.as_ref().unwrap();
        assert!(p.title.as_ref().unwrap().contains("final newline: no"));
        assert_eq!(p.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["LF: 1", "CRLF: 1", "CR: 1"]);
    });
    ws.update_in(vcx, |ws, w, cx| ws.next_line_ending(&NextLineEnding { ending: "\r" }, w, cx));
    assert_eq!(on_editor(&ws, vcx, |e, _, _| e.state.cursor()), 5);
    ws.update_in(vcx, |ws, w, cx| {
        ws.cancel_quick_panel(cx);
        ws.focus_editor(w, cx);
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
    });
    on_editor(&ws, vcx, |e, _, cx| {
        e.input_enabled = false;
        e.update_doc(cx, |d| d.read_only = true);
    });
    vcx.dispatch_action(SetLineEnding { ending: "\r" });
    assert_eq!(on_editor(&ws, vcx, |e, _, cx| e.doc(cx).bytes_as_saved(None).unwrap()), b"a\r\nb\nc\rd");
}

#[gpui::test]
fn n25_native_conversion_refuses_composing_shared_document_before_borrow(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let path = temp("orlo-n25-ime").join("source.txt");
    std::fs::write(&path, "a\r\nb\nc").unwrap();
    let (ws, other, mut a, mut b) = two_windows(cx);
    ws.update_in(&mut a, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    other.update_in(&mut b, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    let first = ws.read_with(&a, |ws, _| ws.active_editor().unwrap());
    let peer = other.read_with(&b, |ws, _| ws.active_editor().unwrap());
    let keys = a.update(|_, cx| crate::keybindings::bindings(r#"[{"keys":["cmd-alt-1"],"command":"workspace::SetLineEnding","args":{"ending":"\r"},"context":"Editor && !single_line"}]"#, cx).unwrap());
    a.update(|_, cx| cx.bind_keys(keys));
    peer.update_in(&mut b, |e, w, cx| e.replace_and_mark_text_in_range(None, "仮", None, w, cx));
    let prior = peer.read_with(&b, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).save_point(), e.doc(cx).version));
    a.simulate_keystrokes("cmd-alt-1");
    assert_eq!(
        peer.read_with(&b, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).save_point(), e.doc(cx).version)),
        prior
    );
    peer.update_in(&mut b, |e, w, cx| e.unmark_text(w, cx));
    a.simulate_keystrokes("cmd-alt-1");
    assert_eq!(first.read_with(&a, |e, cx| e.doc(cx).ending_counts()), [0, 0, 2]);
}

#[gpui::test]
fn n25_new_untitled_keeps_existing_kinds_when_preference_changes(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a\nb");
    on_editor(&ws, cx, |e, _, cx| e.update_doc(cx, |d| d.set_line_ending("\r\n")));
    type_in(&ws, cx, "\nc");
    let original = on_editor(&ws, cx, |e, _, cx| e.doc(cx).bytes_as_saved(None).unwrap());
    assert_eq!(original, b"a\nb\r\nc");
    assert_eq!(on_editor(&ws, cx, |e, _, cx| e.doc(cx).ending_counts()), [1, 1, 0]);
    ws.update_in(cx, |ws, w, cx| ws.set_line_ending(&SetLineEnding { ending: "\r" }, w, cx));
    assert_eq!(on_editor(&ws, cx, |e, _, cx| e.doc(cx).bytes_as_saved(None).unwrap()), b"a\rb\rc");
    on_editor(&ws, cx, |e, _, cx| e.edit(cx, |s| s.undo()));
    assert_eq!(on_editor(&ws, cx, |e, _, cx| e.doc(cx).bytes_as_saved(None).unwrap()), original);
}

#[gpui::test]
fn n32_date_time_menu_palette_form_and_recordable_action_share_one_snapshot(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    vcx.update(|_, cx| editor::fixed_clock(cx, calls.clone()));
    type_in(&ws, vcx, "a\nb");
    set_selections(&ws, vcx, &[(1, 1), (3, 3)]);
    assert!(quick_panel::command_items().iter().any(|i| i.label == "Edit: Insert Date/Time…" && i.hint.is_empty()));
    let action = menu(&ws, vcx, "Edit")
        .into_iter()
        .find_map(|i| match i {
            MenuItem::Action { name, action, .. } if name.as_ref() == "Insert Date/Time…" => Some(action),
            _ => None,
        })
        .unwrap();
    vcx.update(|w, cx| w.dispatch_action(action, cx));
    vcx.run_until_parked();
    assert_eq!(calls.get(), 0);
    vcx.update(|_, cx| cx.default_global::<crate::keybindings::Recorder>().recording = true);
    submit_form(vcx, "");
    let local = "a2024-02-29T19:07:25-05:00\nb2024-02-29T19:07:25-05:00";
    assert_eq!(text(&ws, vcx), local);
    assert_eq!(calls.get(), 1);
    let recorded = vcx.update(|_, cx| {
        let r = cx.default_global::<crate::keybindings::Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].command, "editor::InsertDateTime");
    assert_eq!(recorded[0].args, serde_json::json!({"format":"%Y-%m-%dT%H:%M:%S%:z","utc":false}));
    undo_redo(&ws, vcx, "a\nb", local);
    vcx.dispatch_action(editor::Undo);
    vcx.dispatch_action(InsertDateTimePrompt);
    click_field(vcx, "zone");
    submit_form(vcx, "界 %% %z %:z");
    assert_eq!(text(&ws, vcx), "a界 % +0000 +00:00\nb界 % +0000 +00:00");
    assert_eq!(calls.get(), 2);
    vcx.dispatch_action(editor::Undo);
    ws.update_in(vcx, |ws, w, cx| {
        ws.execute_command(
            &crate::keybindings::Command {
                command: "editor::InsertDateTime".into(),
                args: serde_json::json!({"utc":true}),
            },
            w,
            cx,
        )
        .unwrap()
    });
    assert_eq!(text(&ws, vcx), "a2024-03-01T00:07:25+00:00\nb2024-03-01T00:07:25+00:00");
    assert_eq!(calls.get(), 3);
}

#[gpui::test]
fn n32_date_time_form_rejects_invalid_stale_protected_and_composing_targets(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    vcx.update(|_, cx| editor::fixed_clock(cx, calls.clone()));
    type_in(&ws, vcx, "abc");
    vcx.dispatch_action(InsertDateTimePrompt);
    submit_form(vcx, "%Q");
    assert!(form_error(&ws, vcx).flatten().unwrap().contains("Unsupported"));
    assert_eq!(text(&ws, vcx), "abc");
    assert_eq!(calls.get(), 0);
    vcx.simulate_keystrokes("escape");
    vcx.dispatch_action(InsertDateTimePrompt);
    on_editor(&ws, vcx, |e, _, cx| {
        e.sel(cx).set_cursor(0);
        e.sel(cx).set_cursor(3);
    });
    submit_form(vcx, "");
    assert!(form_error(&ws, vcx).flatten().unwrap().contains("changed"));
    assert_eq!(text(&ws, vcx), "abc");
    vcx.simulate_keystrokes("escape");
    vcx.dispatch_action(InsertDateTimePrompt);
    on_editor(&ws, vcx, |e, _, cx| {
        e.document.update(cx, |d, cx| d.set_viewer_mode(true, cx));
    });
    submit_form(vcx, "");
    assert!(form_error(&ws, vcx).flatten().is_some());
    assert_eq!(text(&ws, vcx), "abc");
    assert_eq!(calls.get(), 0);
    vcx.simulate_keystrokes("escape");
    on_editor(&ws, vcx, |e, _, cx| {
        e.document.update(cx, |d, cx| d.set_viewer_mode(false, cx));
    });
    ws.update_in(vcx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
    let views = ws.read_with(vcx, |ws, cx| ws.open_editors(cx));
    let active = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    let peer = views.iter().find(|e| **e != active).unwrap().clone();
    peer.update(vcx, |e, _| e.marked.push(0..1));
    ws.update_in(vcx, |ws, w, cx| {
        assert!(ws
            .execute_command(
                &crate::keybindings::Command { command: "editor::InsertDateTime".into(), args: serde_json::json!({}) },
                w,
                cx
            )
            .unwrap_err()
            .contains("composition"));
    });
    assert_eq!(text(&ws, vcx), "abc");
    assert_eq!(calls.get(), 0);
}

#[gpui::test]
fn n32_workspace_variables_follow_roots_immediately_with_indexing_disabled(cx: &mut TestAppContext) {
    use crate::config::project::Root;
    let (ws, vcx, path) = window_on(cx, "orlo-n32-roots", "text");
    vcx.update(|_, cx| {
        let mut s = cx.global::<Settings>().clone();
        s.index_files = false;
        cx.set_global(s);
    });
    let nested = path.parent().unwrap().to_path_buf();
    let mut root = Root::new(nested.clone());
    root.name = Some("Custom root".into());
    ws.update(cx, |ws, cx| {
        ws.set_roots(vec![Root::new(nested.parent().unwrap().into()), root], cx);
        let e = ws.active_editor().unwrap();
        let vars = e.read(cx).snippet_variables(cx);
        assert_eq!(vars.resolve("WORKSPACE_NAME").as_deref(), Some("Custom root"));
        assert_eq!(vars.resolve("WORKSPACE_FOLDER"), Some(nested.to_string_lossy().into_owned()));
        assert_eq!(vars.resolve("RELATIVE_FILEPATH").as_deref(), Some("f.txt"));
        ws.set_roots(Vec::new(), cx);
        let vars = e.read(cx).snippet_variables(cx);
        assert_eq!(vars.resolve("WORKSPACE_NAME").as_deref(), Some(""));
        assert_eq!(vars.resolve("WORKSPACE_FOLDER").as_deref(), Some(""));
        assert_eq!(vars.resolve("RELATIVE_FILEPATH").as_deref(), Some("f.txt"));
    });
}

#[gpui::test]
fn u01_menu_palette_unicode_form_validates_and_inserts_one_shared_undo(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "ab\ncd");
    set_selections(&ws, cx, &[(1, 1), (4, 4)]);
    for label in ["Insert Unicode Character…", "Insert Literal Next Key", "Character Viewer"] {
        assert!(names(&menu(&ws, cx, "Edit")).iter().any(|name| name == label));
    }
    for label in ["Edit: Insert Unicode Character", "Edit: Insert Literal Next Key", "Edit: Character Viewer"] {
        assert!(quick_panel::command_items().iter().any(|item| item.label == label));
    }
    let action = menu(&ws, cx, "Edit")
        .into_iter()
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name.as_ref() == "Insert Unicode Character…" => Some(action),
            _ => None,
        })
        .unwrap();
    cx.update(|w, cx| w.dispatch_action(action, cx));
    cx.run_until_parked();
    submit_form(cx, "U+D800");
    assert!(form_error(&ws, cx).flatten().unwrap().contains("scalar"));
    assert_eq!(text(&ws, cx), "ab\ncd");
    cx.simulate_keystrokes("cmd-a");
    submit_form(cx, "U+1F600");
    assert_eq!(form_error(&ws, cx), None);
    assert_eq!(text(&ws, cx), "a😀b\nc😀d");
    undo_redo(&ws, cx, "ab\ncd", "a😀b\nc😀d");
}

#[gpui::test]
fn u01_unicode_form_rejects_stale_selections_and_protected_target(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "abc");
    cx.dispatch_action(InsertUnicodeCharacterPrompt);
    on_editor(&ws, cx, |e, _, cx| {
        e.sel(cx).set_cursor(0);
        e.sel(cx).set_cursor(3);
    });
    submit_form(cx, "301");
    assert!(form_error(&ws, cx).flatten().unwrap().contains("changed"));
    assert_eq!(text(&ws, cx), "abc");
    cx.simulate_keystrokes("escape");
    cx.dispatch_action(InsertUnicodeCharacterPrompt);
    on_editor(&ws, cx, |e, _, cx| e.document.update(cx, |d, cx| d.set_viewer_mode(true, cx)));
    submit_form(cx, "1F600");
    assert!(form_error(&ws, cx).flatten().is_some());
    assert_eq!(text(&ws, cx), "abc");
}

#[gpui::test]
fn u01_installed_bindings_menu_and_json_refuse_shared_native_composition(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "ab");
    let first = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    new_view(&ws, cx);
    let peer = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(cx, |ws, w, cx| {
        ws.views.select(0, 0);
        ws.focus_editor(w, cx);
    });
    let keys = cx.update(|_, cx| crate::keybindings::bindings(r#"[
        {"keys":["cmd-alt-1"],"command":"editor::InsertUnicodeCharacter","args":{"value":"1F600"},"context":"Editor && !single_line"},
        {"keys":["cmd-alt-2"],"command":"editor::InsertLiteralNextKey","context":"Editor && !single_line"},
        {"keys":["cmd-alt-3"],"command":"editor::InsertLiteralText","args":{"text":"("},"context":"Editor && !single_line"},
        {"keys":["x"],"command":"editor::SelectAll","context":"Editor && !single_line"}
    ]"#, cx).unwrap());
    cx.update(|_, cx| cx.bind_keys(keys));
    cx.simulate_keystrokes("cmd-alt-1");
    assert_eq!(text(&ws, cx), "ab😀");
    cx.simulate_keystrokes("cmd-z cmd-alt-2 x");
    assert_eq!(text(&ws, cx), "abx", "installed arming bypasses an installed text binding");
    cx.simulate_keystrokes("cmd-z");
    cx.simulate_keystrokes("cmd-alt-3");
    assert_eq!(text(&ws, cx), "ab(", "resolved literal binding bypasses auto-pairs");
    cx.simulate_keystrokes("cmd-z");
    peer.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(None, "仮", Some(0..1), w, cx));
    let before = peer.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version, e.doc(cx).save_point()));
    cx.simulate_keystrokes("cmd-alt-1 cmd-alt-2 cmd-alt-3 x");
    cx.dispatch_action(InsertUnicodeCharacterPrompt);
    assert_eq!(form_error(&ws, cx), None, "shared composition prevents opening scalar form");
    cx.dispatch_action(editor::InsertLiteralNextKey);
    ws.update_in(cx, |ws, w, cx| {
        for command in ["editor::InsertUnicodeCharacter", "editor::InsertLiteralNextKey", "editor::InsertLiteralText"] {
            let args = if command.ends_with("Character") {
                serde_json::json!({"value":"301"})
            } else if command.ends_with("LiteralText") {
                serde_json::json!({"text":"("})
            } else {
                serde_json::json!({})
            };
            assert!(ws
                .execute_command(&crate::keybindings::Command { command: command.into(), args }, w, cx)
                .unwrap_err()
                .contains("composition"));
        }
    });
    assert_eq!(
        peer.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version, e.doc(cx).save_point())),
        before
    );
    assert_eq!(first.read_with(cx, |e, cx| e.text(cx)), before.0);
}

#[gpui::test]
fn u01_review_canceled_literal_macro_replays_ordinary_text(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    for cancel in ["escape", "left"] {
        let (ws, cx) = window(cx);
        cx.update(|_, cx| cx.set_global(Recorder { recording: true, ..Default::default() }));
        cx.dispatch_action(editor::InsertLiteralNextKey);
        cx.simulate_keystrokes(cancel);
        cx.simulate_keystrokes("(");
        assert_eq!(text(&ws, cx), "()", "cancellation must restore ordinary auto-pairs");
        cx.update(|_, cx| {
            let r = cx.default_global::<Recorder>();
            r.recording = false;
            r.last = r.commands.clone();
            assert!(r.last.iter().all(|c| c.command != "editor::InsertLiteralNextKey"));
            assert_eq!(r.last.len(), if cancel == "escape" { 1 } else { 2 });
        });
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&ws, cx), "");
        cx.dispatch_action(ReplayMacro);
        assert_eq!(text(&ws, cx), "()", "canceled literal sequence: {cancel}");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&ws, cx), "");
    }
}

#[gpui::test]
fn u01_literal_multicaret_native_text_and_recorded_playback_keep_literal_undo(cx: &mut TestAppContext) {
    use crate::keybindings::Recorder;
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a\nb");
    set_selections(&ws, cx, &[(1, 1), (3, 3)]);
    cx.update(|_, cx| cx.default_global::<Recorder>().recording = true);
    cx.dispatch_action(editor::InsertLiteralNextKey);
    cx.simulate_keystrokes("(");
    assert_eq!(text(&ws, cx), "a(\nb(");
    let recorded = cx.update(|_, cx| {
        let r = cx.default_global::<Recorder>();
        r.recording = false;
        r.commands.clone()
    });
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].command, "editor::InsertLiteralText");
    assert_eq!(recorded[0].args["text"], "(");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, cx), "a\nb");
    ws.update_in(cx, |ws, w, cx| {
        for command in recorded {
            ws.execute_command(&command, w, cx).unwrap();
        }
    });
    assert_eq!(text(&ws, cx), "a(\nb(");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&ws, cx), "a\nb");
}

#[gpui::test]
fn u01_late_identical_peer_ime_preserves_marked_range_without_revision_change(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    for marked in [false, true] {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "ab");
        let first = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        new_view(&ws, cx);
        let peer = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(cx, |ws, w, cx| {
            ws.views.select(0, 0);
            ws.focus_editor(w, cx);
        });
        let version = first.read_with(cx, |e, cx| e.doc(cx).version);
        if marked {
            ws.update_in(cx, |ws, w, cx| {
                ws.execute_command(
                    &crate::keybindings::Command {
                        command: "editor::InsertLiteralNextKey".into(),
                        args: serde_json::json!({}),
                    },
                    w,
                    cx,
                )
                .unwrap()
            });
        } else {
            cx.dispatch_action(editor::InsertLiteralNextKey);
        }
        peer.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(Some(0..1), "a", Some(0..1), w, cx));
        assert_eq!(first.read_with(cx, |e, cx| e.doc(cx).version), version, "identical native preview keeps revision");
        let before = peer
            .read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.state.selections.clone(), e.doc(cx).save_point()));
        first.update_in(cx, |e, w, cx| {
            if marked {
                e.replace_and_mark_text_in_range(None, "仮", None, w, cx);
            } else {
                e.replace_text_in_range(None, "😀", w, cx);
            }
        });
        assert_eq!(
            peer.read_with(cx, |e, cx| (
                e.text(cx),
                e.marked.clone(),
                e.state.selections.clone(),
                e.doc(cx).save_point()
            )),
            before
        );
        assert_eq!(first.read_with(cx, |e, cx| e.doc(cx).version), version);
        peer.update_in(cx, |e, w, cx| e.replace_text_in_range(None, "愛", w, cx));
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&ws, cx), "ab", "only the peer commit entered Undo");
    }
}
