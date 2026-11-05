use super::*;

/// `text` with the (anchor, head) pairs selected.
fn selected(text: &str, sels: &[(usize, usize)]) -> EditorState {
    let mut s = EditorState::from_text(text);
    s.set_selections(super::sels(sels));
    s
}

#[test]
fn move_text_moves_a_selection_forward_and_backward() {
    let mut s = selected("hello world", &[(0, 5)]);
    // to the end (a drop past it clamps), then back to the start
    assert!(s.move_text(99, false));
    assert_eq!(s.text(), " worldhello");
    assert_eq!(s.view.selections, vec![Selection::new(6, 11)]);
    assert!(s.move_text(0, false));
    assert_eq!(s.text(), "hello world");
    assert_eq!(s.view.selections, vec![Selection::new(0, 5)]);
    // into the middle: forward lands after the cut, backward before it
    let mut s = selected("hello world", &[(0, 5)]);
    assert!(s.move_text(6, false));
    assert_eq!((s.text().as_str(), s.view.selections[0]), (" helloworld", Selection::new(1, 6)));
    let mut s = selected("one two three", &[(8, 13)]);
    assert!(s.move_text(4, false));
    assert_eq!(s.text(), "one threetwo ");
    assert_eq!(s.view.selections, vec![Selection::new(4, 9)]);
    // a selection dragged backwards is moved all the same
    let mut s = selected("hello world", &[(5, 0)]);
    assert!(s.move_text(11, false));
    assert_eq!(s.text(), " worldhello");
}

#[test]
fn move_text_with_copy_inserts_a_copy_and_leaves_the_sources() {
    let mut s = selected("hello world", &[(0, 5)]);
    assert!(s.move_text(11, true));
    assert_eq!(s.text(), "hello worldhello");
    assert_eq!(s.view.selections, vec![Selection::new(11, 16)], "the copy is selected");
    let mut s = selected("hello world", &[(6, 11)]);
    assert!(s.move_text(0, true));
    assert_eq!(s.text(), "worldhello world");
    assert_eq!(s.view.selections, vec![Selection::new(0, 5)]);
}

#[test]
fn move_text_joins_several_selections_in_document_order() {
    // "ab" and "ef", given in the wrong order: after both, between them, and after the cut
    for (drop, want, sel) in
        [(11, " cd  ghab\nef", (7, 12)), (5, " cdab\nef  gh", (3, 8)), (9, " cd  ab\nefgh", (5, 10))]
    {
        let mut s = selected("ab cd ef gh", &[(6, 8), (0, 2)]);
        assert!(s.move_text(drop, false), "{drop}");
        assert_eq!(s.text(), want, "{drop}");
        assert_eq!(s.view.selections, vec![Selection::new(sel.0, sel.1)], "{drop}");
    }
    // copies keep the sources
    let mut s = selected("ab cd ef gh", &[(0, 2), (6, 8)]);
    assert!(s.move_text(11, true));
    assert_eq!(s.text(), "ab cd ef ghab\nef");
    assert_eq!(s.view.selections, vec![Selection::new(11, 16)]);
}

#[test]
fn move_text_leaves_the_carets_on_their_text() {
    let mut s = selected("ab cd ef", &[(0, 2), (4, 4)]);
    assert!(s.move_text(8, false));
    assert_eq!(s.text(), " cd efab");
    assert_eq!(s.view.selections, sels(&[(2, 2), (6, 8)]));
    assert_eq!(s.cursor(), 8, "the moved text is the primary selection");
}

#[test]
fn move_text_into_or_against_a_source_changes_nothing() {
    let mut s = selected("hello world", &[(2, 6), (8, 10)]);
    let (version, sels) = (s.doc.version, s.view.selections.clone());
    for drop in [2, 4, 6, 8, 9, 10] {
        assert!(!s.move_text(drop, false), "{drop}");
        assert!(!s.move_text(drop, true), "{drop} with copy");
    }
    assert_eq!((s.text().as_str(), s.doc.version), ("hello world", version));
    assert_eq!(s.view.selections, sels);
    assert!(!s.doc.dirty, "no undo step was started");
    // carets only: nothing to drag
    let mut s = selected("abc", &[(1, 1)]);
    assert!(!s.move_text(0, false));
    assert_eq!(s.text(), "abc");
}

#[test]
fn move_text_is_one_undo_step_that_brings_back_the_selections() {
    let mut s = selected("ab cd ef gh", &[(0, 2), (6, 8)]);
    assert!(s.move_text(11, false));
    let moved = (s.text(), s.view.selections.clone());
    s.undo();
    assert_eq!(s.text(), "ab cd ef gh", "the cut and the insert are one step");
    assert_eq!(s.view.selections, sels(&[(0, 2), (6, 8)]));
    s.redo();
    assert_eq!((s.text(), s.view.selections.clone()), moved);
    // and a copy
    s.undo();
    assert!(s.move_text(3, true));
    let copied = (s.text(), s.view.selections.clone());
    s.undo();
    assert_eq!(s.text(), "ab cd ef gh");
    assert_eq!(s.view.selections, sels(&[(0, 2), (6, 8)]));
    s.redo();
    assert_eq!((s.text(), s.view.selections.clone()), copied);
}

#[test]
fn move_text_moves_the_text_in_the_other_views_and_maps_their_selections() {
    let mut v = views("hello world");
    v.a().set_selections(vec![Selection::new(0, 5)]);
    // a caret in "world" and a selection in the moved text
    v.b().set_selections(sels(&[(1, 3), (8, 8)]));
    assert!(v.a().move_text(11, false));
    assert_eq!(v.b().text(), " worldhello");
    assert_eq!(v.b().view.selections, cs(&[0, 3]));
    // the document's one history undoes it for both
    v.a().undo();
    assert_eq!((v.a().text(), v.b().text()), ("hello world".into(), "hello world".into()));
}
