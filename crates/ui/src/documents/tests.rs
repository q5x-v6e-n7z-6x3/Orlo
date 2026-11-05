//! One document in several views: two groups and two windows, each view's own state, undo from
//! either view, views closed and opened again, Save As, a reload and a session coming back.

use crate::editing::encoding::Encoding;
use crate::editing::text::Selection;
use crate::workspace::ui_tests::{settle_until, temp, text, window};
use crate::workspace::*;
use gpui::{TestAppContext, VisualTestContext};

/// A second window, beside the first `window` made, in its own test context.
fn second_window(cx: &mut TestAppContext) -> (Entity<Workspace>, VisualTestContext) {
    let (ws, _) = cx.add_window_view(Workspace::new);
    let w = *cx.windows().last().unwrap();
    (ws, VisualTestContext::from_window(w, cx))
}

fn active(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<Editor> {
    ws.read_with(cx, |ws, _| ws.active_editor().unwrap())
}

/// `path` opened in `ws`, and the view it shows.
fn open(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &Path) -> Entity<Editor> {
    ws.update_in(cx, |ws, window, cx| ws.open_path(path.to_path_buf(), window, cx));
    active(ws, cx)
}

fn new_file(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<Editor> {
    ws.update_in(cx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    active(ws, cx)
}

/// `text` typed in `e` at `at`.
fn insert_at(e: &Entity<Editor>, cx: &mut VisualTestContext, at: usize, text: &str) {
    e.update(cx, |e, cx| {
        e.edit(cx, |s| {
            s.set_cursor(at);
            s.insert(text)
        })
    });
}

fn doc_id(e: &Entity<Editor>, cx: &mut VisualTestContext) -> u64 {
    e.read_with(cx, |e, cx| e.doc(cx).id())
}

fn cursor(e: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    e.read_with(cx, |e, _| e.state.cursor())
}

fn file(name: &str, text: &str) -> PathBuf {
    let dir = temp(name).canonicalize().unwrap();
    let p = dir.join("doc.txt");
    std::fs::write(&p, text).unwrap();
    p
}

/// `ws` as a hot exit saves it, with its unsaved texts as the next launch reads them from the
/// session folder.
fn hot_exit(ws: &Entity<Workspace>, cx: &mut VisualTestContext, texts: &mut Texts) -> WindowState {
    let mut st = ws.update_in(cx, |ws, window, cx| ws.capture(window, true, texts, cx));
    for v in st.groups.iter_mut().flat_map(|g| &mut g.views) {
        if let Some(u) = &mut v.unsaved {
            let name = u.file.as_ref().unwrap();
            u.text = texts.iter().find(|(n, _)| n == name).unwrap().1.to_string();
        }
    }
    st
}

fn tabs(ws: &Entity<Workspace>, cx: &VisualTestContext) -> Vec<Entity<Editor>> {
    ws.read_with(cx, |ws, _| ws.views.tabs().cloned().collect())
}

/// `path` open in two groups of `ws`: its view in the first group and another in the second.
fn two_groups(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &Path) -> (Entity<Editor>, Entity<Editor>) {
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(path.to_path_buf(), window, cx);
        let a = ws.active_editor().unwrap();
        ws.set_layout(&SetLayout { preset: crate::chrome::layout::Preset::Columns(2) }, window, cx);
        ws.views.active = 1;
        ws.open_path_here(path.to_path_buf(), window, cx);
        let b = ws.active_editor().unwrap();
        assert_ne!(a, b, "a view in each group");
        (a, b)
    })
}

#[gpui::test]
fn two_groups_and_two_windows_show_one_document(cx: &mut TestAppContext) {
    let path = file("orlo-doc-two-windows", "one\ntwo\n");
    let (ws, vcx) = window(cx);
    let (a, b) = two_groups(&ws, vcx, &path);
    let (ws2, mut vcx2) = second_window(vcx);
    let c = open(&ws2, &mut vcx2, &path);
    // one document, three views
    let id = doc_id(&a, vcx);
    assert_eq!((doc_id(&b, vcx), doc_id(&c, &mut vcx2)), (id, id));
    // typing in the other window shows in both groups here, unsaved there too
    insert_at(
        &c, &mut vcx2, 0, "zero
",
    );
    vcx2.run_until_parked();
    for v in [&a, &b] {
        v.read_with(vcx, |e, cx| {
            assert_eq!(e.doc(cx).text(), "zero\none\ntwo\n");
            assert!(e.doc(cx).dirty);
        });
    }
    // an encoding chosen in one group is the document's, in every window
    b.update(vcx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Utf16Le, true)));
    vcx.run_until_parked();
    c.read_with(&vcx2, |e, cx| {
        assert_eq!((e.doc(cx).encoding, e.doc(cx).bom), (Encoding::Utf16Le, true));
        assert_eq!(e.doc(cx).path.as_deref(), Some(path.as_path()));
    });
    // the tab titles and dots of both windows come from the same state
    ws2.read_with(&vcx2, |ws, cx| {
        let tab = ws.active_editor().unwrap();
        assert!(tab.read(cx).doc(cx).unsaved());
    });
}

#[gpui::test]
fn change_markers_found_in_one_window_reach_every_view_of_the_document(cx: &mut TestAppContext) {
    let path = file("orlo-doc-markers", "one\ntwo\n");
    let (ws, vcx) = window(cx);
    let (a, b) = two_groups(&ws, vcx, &path);
    let (ws2, mut vcx2) = second_window(vcx);
    let c = open(&ws2, &mut vcx2, &path);
    // the focused window edits and finds its markers, as its disk poll does; the other window
    // does not poll while it is in the background
    insert_at(&a, vcx, 0, "zero\n");
    ws.update(vcx, |ws, cx| ws.refresh_git(cx));
    vcx.run_until_parked();
    vcx2.run_until_parked();
    // the markers a view draws: those for the text it shows
    let shown = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
        e.read_with(cx, |e, cx| (e.changes_version == e.doc(cx).version).then(|| e.changes.clone()))
    };
    let found = shown(&a, vcx).expect("the window that looked");
    assert_eq!(found.len(), 1, "the added line");
    assert_eq!(shown(&b, vcx), Some(found.clone()), "the other group");
    assert_eq!(shown(&c, &mut vcx2), Some(found), "the other window");
}

#[gpui::test]
fn each_view_keeps_its_own_selections_bookmarks_folds_and_scroll(cx: &mut TestAppContext) {
    let path = file("orlo-doc-view-state", "a\n  b\n  c\nd\ne\nf\n");
    let (ws, vcx) = window(cx);
    let (a, b) = two_groups(&ws, vcx, &path);
    a.update(vcx, |e, cx| {
        e.sel(cx).set_cursor(2);
        e.sel(cx).toggle_bookmark();
        e.fold_lines(0, 2, cx);
        e.set_top((8, 0.));
    });
    b.update(vcx, |e, cx| {
        e.sel(cx).set_selection(10, 11);
    });
    // an edit above both: each maps its own once
    insert_at(&a, vcx, 0, ">>");
    vcx.run_until_parked();
    b.read_with(vcx, |e, cx| {
        assert_eq!(e.state.selections, vec![Selection::new(12, 13)], "mapped once");
        assert!(e.state.bookmarks.is_empty(), "the bookmark is a's");
        assert!(e.hidden_lines(cx).is_empty(), "the fold is a's");
        assert_eq!(e.state.revision(), e.doc(cx).version);
    });
    a.read_with(vcx, |e, cx| {
        assert_eq!(e.state.bookmarks, vec![4], "still on its line");
        assert_eq!(e.hidden_lines(cx), vec![1..3]);
    });
    assert_eq!(cursor(&a, vcx), 2);
}

#[gpui::test]
fn undo_and_redo_from_either_view_keep_the_other_views_carets(cx: &mut TestAppContext) {
    let path = file("orlo-doc-undo", "hello world\n");
    let (ws, vcx) = window(cx);
    let (a, b) = two_groups(&ws, vcx, &path);
    b.update(vcx, |e, cx| e.sel(cx).set_cursor(11));
    insert_at(&a, vcx, 5, ",");
    vcx.run_until_parked();
    assert_eq!(cursor(&b, vcx), 12, "b's caret follows a's edit");
    // undo from b: b keeps its own caret, mapped; a's moves only with the text
    b.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
    vcx.run_until_parked();
    assert_eq!(text(&ws, vcx), "hello world\n");
    assert_eq!((cursor(&b, vcx), cursor(&a, vcx)), (11, 5));
    // redo from a: b's undo recorded the step, so a maps its caret and b keeps its own
    a.update(vcx, |e, cx| e.edit(cx, |s| s.redo()));
    vcx.run_until_parked();
    assert_eq!(b.read_with(vcx, |e, cx| e.doc(cx).text()), "hello, world\n");
    assert_eq!((cursor(&a, vcx), cursor(&b, vcx)), (5, 12));
    assert!(a.read_with(vcx, |e, cx| e.doc(cx).dirty));
    a.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
    vcx.run_until_parked();
    assert!(!b.read_with(vcx, |e, cx| e.doc(cx).dirty), "back at the file's text");
}

#[gpui::test]
fn typing_in_turn_in_two_views_makes_one_undo_step_a_turn(cx: &mut TestAppContext) {
    let path = file("orlo-doc-typing", "");
    let (ws, vcx) = window(cx);
    let (a, b) = two_groups(&ws, vcx, &path);
    for (view, keys) in [(&a, "ab"), (&b, "cd"), (&a, "ef")] {
        view.update(vcx, |e, cx| {
            let end = e.doc(cx).len();
            e.sel(cx).set_cursor(end);
        });
        for k in keys.chars() {
            view.update(vcx, |e, cx| e.edit(cx, |s| s.type_text(&k.to_string())));
            vcx.run_until_parked();
        }
    }
    let mut texts = Vec::new();
    for _ in 0..3 {
        a.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
        vcx.run_until_parked();
        texts.push(b.read_with(vcx, |e, cx| e.doc(cx).text()));
    }
    assert_eq!(texts, ["abcd", "ab", ""]);
}

#[gpui::test]
fn opening_another_view_adds_view_state_and_a_subscription_not_a_document(cx: &mut TestAppContext) {
    let path = file("orlo-doc-another-view", "text\n");
    let (ws, vcx) = window(cx);
    let a = open(&ws, vcx, &path);
    let doc = a.read_with(vcx, |e, _| e.document.clone());
    let views = |vcx: &mut VisualTestContext| doc.read_with(vcx, |d, _| d.model.view_count());
    let before = views(vcx);
    ws.update_in(vcx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
    let b = active(&ws, vcx);
    assert_ne!(a, b);
    assert!(b.read_with(vcx, |e, _| e.document == doc), "the same document entity");
    assert_eq!(views(vcx), before + 1, "one more view state");
    assert_ne!(a.read_with(vcx, |e, _| e.state.view_id()), b.read_with(vcx, |e, _| e.state.view_id()));
    // the window follows the document once, however many of its tabs show it
    ws.read_with(vcx, |ws, _| assert_eq!(ws.doc_subs.len(), 1));
    assert!(vcx.update(|_, cx| Documents::for_path(&path, cx)).is_some_and(|d| d == doc));
}

#[gpui::test]
fn untitled_documents_save_as_and_a_reopened_file_each_get_their_own_identity(cx: &mut TestAppContext) {
    let path = file("orlo-doc-identity", "on disk\n");
    let (ws, vcx) = window(cx);
    // two untitled tabs: two documents
    let first = new_file(&ws, vcx);
    let second = new_file(&ws, vcx);
    assert_ne!(doc_id(&first, vcx), doc_id(&second, vcx));
    // typed in, so opening a file does not take its place
    second.update(vcx, |e, cx| e.edit(cx, |s| s.insert("new")));
    // a file closed and opened again is a new document
    let opened = open(&ws, vcx, &path);
    let id = doc_id(&opened, vcx);
    opened.update(vcx, |e, cx| e.edit(cx, |s| s.insert("unsaved ")));
    ws.update_in(vcx, |ws, window, cx| ws.close_editor(&opened, window, cx));
    assert!(vcx.update(|_, cx| Documents::for_path(&path, cx)).is_none());
    let again = open(&ws, vcx, &path);
    assert_ne!(doc_id(&again, vcx), id);
    assert_eq!(text(&ws, vcx), "on disk\n", "the closed one's edits are gone");
    // an untitled document saved as a file is found by that path from then on
    let named = path.with_file_name("named.txt");
    second.update(vcx, |e, cx| e.update_doc(cx, |d| d.save_to(named.clone())).unwrap());
    let found = vcx.update(|_, cx| Documents::for_path(&named, cx));
    assert!(found.is_some_and(|d| d.read_with(vcx, |d, _| d.id()) == doc_id(&second, vcx)));
}

#[gpui::test]
fn save_as_and_a_reload_reach_every_view(cx: &mut TestAppContext) {
    let path = file("orlo-doc-save-as", "first\n");
    let (ws, vcx) = window(cx);
    let (a, b) = two_groups(&ws, vcx, &path);
    // Save As from one view: the other shows the new name and is clean
    let to = path.with_file_name("renamed.rs");
    a.update(vcx, |e, cx| {
        e.edit(cx, |s| s.insert("x"));
        e.update_doc(cx, |d| d.save_to(to.clone())).unwrap();
    });
    vcx.run_until_parked();
    b.read_with(vcx, |e, cx| {
        assert_eq!(e.doc(cx).path.as_deref(), Some(to.as_path()));
        assert!(!e.doc(cx).dirty);
        assert_eq!(e.title(cx), "renamed.rs");
        assert_eq!(e.highlighter.syntax_name(), "Rust", "picked again for the new name");
    });
    // the file changed on disk and reloaded: both views have it, clean, carets on their text
    b.update(vcx, |e, cx| e.sel(cx).set_cursor(4));
    std::fs::write(&to, "new top\nxfirst\n").unwrap();
    ws.update_in(vcx, |ws, window, cx| ws.revert(&Revert, window, cx));
    settle_until(vcx, |vcx| b.read_with(vcx, |e, cx| e.doc(cx).text() == "new top\nxfirst\n"));
    a.read_with(vcx, |e, cx| assert!(!e.doc(cx).dirty));
    assert_eq!(cursor(&b, vcx), 12, "still in 'first'");
}

#[gpui::test]
fn a_closed_view_and_a_hidden_tab_stay_consistent(cx: &mut TestAppContext) {
    let path = file("orlo-doc-hidden", "abc\n");
    let (ws, vcx) = window(cx);
    let (a, b) = two_groups(&ws, vcx, &path);
    // b's group shows another tab: b is hidden, and still follows
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    insert_at(&a, vcx, 3, "d");
    vcx.run_until_parked();
    b.read_with(vcx, |e, cx| {
        assert_eq!(e.doc(cx).text(), "abcd\n");
        assert_eq!(e.state.revision(), e.doc(cx).version);
    });
    // closing a leaves b with the document, its edits and its undo history
    ws.update_in(vcx, |ws, window, cx| ws.close_editor(&a, window, cx));
    drop(a);
    vcx.run_until_parked();
    b.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
    assert_eq!(b.read_with(vcx, |e, cx| e.doc(cx).text()), "abc\n");
    assert!(vcx.update(|_, cx| Documents::for_path(&path, cx)).is_some());
}

#[gpui::test]
fn a_session_brings_back_one_document_with_each_views_own_state(cx: &mut TestAppContext) {
    let path = file("orlo-doc-session", "one\ntwo\nthree\n");
    let (ws, vcx) = window(cx);
    let (a, b) = two_groups(&ws, vcx, &path);
    insert_at(&a, vcx, 0, "zero\n");
    vcx.run_until_parked();
    a.update(vcx, |e, cx| e.sel(cx).set_cursor(2));
    b.update(vcx, |e, cx| e.sel(cx).set_selection(9, 12));
    let st = hot_exit(&ws, vcx, &mut Vec::new());
    let (ws2, mut vcx2) = second_window(vcx);
    ws2.update_in(&mut vcx2, |ws, window, cx| ws.restore(st, window, cx));
    let tabs = tabs(&ws2, &vcx2);
    assert_eq!(tabs.len(), 2);
    let ids: Vec<_> = tabs.iter().map(|t| doc_id(t, &mut vcx2)).collect();
    assert_eq!(ids[0], ids[1], "the two views are of one document again");
    tabs[0].read_with(&vcx2, |e, cx| {
        assert_eq!(e.doc(cx).text(), "zero\none\ntwo\nthree\n");
        assert!(e.doc(cx).dirty);
        assert_eq!(e.state.cursor(), 2);
    });
    tabs[1].read_with(&vcx2, |e, _| {
        assert_eq!(e.state.selections, vec![Selection::new(9, 12)]);
    });
}

#[gpui::test]
fn a_session_brings_back_one_document_shown_in_two_windows(cx: &mut TestAppContext) {
    let path = file("orlo-doc-session-windows", "one\n");
    let (ws, vcx) = window(cx);
    let a = open(&ws, vcx, &path);
    let (ws2, mut vcx2) = second_window(vcx);
    open(&ws2, &mut vcx2, &path);
    a.update(vcx, |e, cx| e.edit(cx, |s| s.insert("zero ")));
    vcx.run_until_parked();
    // hot exit: both windows go into one session
    let mut texts = Vec::new();
    let st = hot_exit(&ws, vcx, &mut texts);
    let st2 = hot_exit(&ws2, &mut vcx2, &mut texts);
    // the next launch opens a window for each
    let (ws3, mut vcx3) = second_window(vcx);
    ws3.update_in(&mut vcx3, |ws, window, cx| ws.restore(st, window, cx));
    let (ws4, mut vcx4) = second_window(vcx);
    ws4.update_in(&mut vcx4, |ws, window, cx| ws.restore(st2, window, cx));
    let (c, d) = (active(&ws3, &mut vcx3), active(&ws4, &mut vcx4));
    assert_eq!(doc_id(&c, &mut vcx3), doc_id(&d, &mut vcx4), "one document in both windows again");
    d.read_with(&vcx4, |e, cx| {
        assert_eq!(e.doc(cx).text(), "zero one\n");
        assert!(e.doc(cx).dirty);
    });
}

#[gpui::test]
fn restoring_a_window_keeps_the_file_state_of_a_document_open_in_another(cx: &mut TestAppContext) {
    let small = file("orlo-doc-restore-open", "a\n\tb\n");
    // read in the background when a window is restored
    let big = small.with_file_name("big.txt");
    std::fs::write(&big, "\tx\n".repeat(1_500_000)).unwrap();
    let (ws, vcx) = window(cx);
    let mut open = Vec::new();
    for p in [&small, &big] {
        ws.update_in(vcx, |ws, window, cx| ws.open_path(p.clone(), window, cx));
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.loading.is_empty()));
        // View > Indentation > Tab Width: 2, chosen in this window
        ws.update_in(vcx, |ws, window, cx| ws.set_tab_size(&SetTabSize { size: 2 }, window, cx));
        open.push(active(&ws, vcx));
    }
    let facts = |e: &Entity<Editor>, vcx: &mut VisualTestContext| {
        e.read_with(vcx, |e, cx| {
            let d = e.doc(cx);
            (
                d.tab_size,
                d.indent_size,
                d.translate_tabs,
                d.indent_override,
                d.line_ending,
                d.line_ending_override,
                d.unsaved(),
            )
        })
    };
    let before: Vec<_> = open.iter().map(|e| facts(e, vcx)).collect();
    // another window comes back with both files, indented and ended otherwise (a project's
    // workspace)
    let view = |p: &PathBuf| ViewState {
        path: p.clone(),
        tab_size: Some(4),
        translate_tabs: Some(true),
        line_ending_override: Some("\r\n".into()),
        ..Default::default()
    };
    let st = WindowState {
        groups: vec![GroupState { views: vec![view(&small), view(&big)], active: 0 }],
        ..Default::default()
    };
    let (ws2, mut vcx2) = second_window(vcx);
    ws2.update_in(&mut vcx2, |ws, window, cx| ws.restore(st, window, cx));
    settle_until(&mut vcx2, |vcx| ws2.read_with(vcx, |ws, _| ws.loading.is_empty()));
    let tabs = tabs(&ws2, &vcx2);
    let mut ids: Vec<_> = tabs.iter().map(|t| doc_id(t, &mut vcx2)).collect();
    let mut was: Vec<_> = open.iter().map(|e| doc_id(e, vcx)).collect();
    ids.sort();
    was.sort();
    assert_eq!(ids, was, "the documents open already");
    let after: Vec<_> = open.iter().map(|e| facts(e, vcx)).collect();
    assert_eq!(after, before, "the first window's choices stand");
}

#[gpui::test]
fn a_document_saved_as_an_open_file_and_closed_leaves_that_file_found(cx: &mut TestAppContext) {
    let path = file("orlo-doc-save-as-open", "x\n");
    let (ws, vcx) = window(cx);
    let x = open(&ws, vcx, &path);
    // an untitled document saved as the file x shows, through the Save As panel
    let y = new_file(&ws, vcx);
    y.update(vcx, |e, cx| e.edit(cx, |s| s.insert("y\n")));
    ws.update_in(vcx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
    vcx.simulate_new_path_selection(|_| Some(path.clone()));
    settle_until(vcx, |vcx| y.read_with(vcx, |e, cx| e.doc(cx).path.is_some() && !e.doc(cx).unsaved()));
    // its tab closes: x still shows the file, and is what opening it finds
    ws.update_in(vcx, |ws, window, cx| ws.close_editor(&y, window, cx));
    drop(y);
    vcx.run_until_parked();
    let found = vcx.update(|_, cx| Documents::for_path(&path, cx));
    assert_eq!(
        found.map(|d| d.read_with(vcx, |d, _| d.id())),
        Some(doc_id(&x, vcx)),
        "the document still open for the file"
    );
    let (ws2, mut vcx2) = second_window(vcx);
    let c = open(&ws2, &mut vcx2, &path);
    assert_eq!(doc_id(&c, &mut vcx2), doc_id(&x, vcx), "not a third document");
}
