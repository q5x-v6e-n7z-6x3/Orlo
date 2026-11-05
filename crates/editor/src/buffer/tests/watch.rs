use super::fresh_dir;
use crate::buffer::EditorState;
use orlo_project::watch::*;

#[test]
fn a_deleted_buffer_saved_before_the_result_lands_is_left_alone() {
    let p = fresh_dir("orlo-watch-saved").join("f");
    std::fs::write(&p, "a").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    std::fs::remove_file(&p).unwrap();
    s.doc.deleted = true;
    let then = (s.doc.disk, s.doc.deleted);
    // the stat lands in the middle of Orlo's Save As, then the user types on
    std::fs::write(&p, "half written").unwrap();
    let polled = check(&p, s.doc.disk, s.doc.deleted);
    s.save_to(p.clone()).unwrap();
    s.insert("x");
    let now = (s.doc.disk, s.doc.deleted);
    assert_eq!(act(polled, then, now, s.doc.dirty, s.doc.watching), Act::Nothing);
    assert_eq!(check(&p, s.doc.disk, s.doc.deleted), Disk::Same);
}

#[test]
fn an_outside_change_overwritten_by_an_own_save_is_not_asked_about() {
    let p = fresh_dir("orlo-watch-own").join("f");
    std::fs::write(&p, "a").unwrap();
    let mut s = EditorState::load(&p).unwrap();
    std::fs::write(&p, "from another app").unwrap();
    let then = (s.doc.disk, s.doc.deleted);
    let polled = check(&p, s.doc.disk, s.doc.deleted);
    assert!(matches!(polled, Disk::Changed(_)));
    // before the result lands: an edit, cmd-s over the outside change, more typing
    s.insert("mine ");
    s.save_to(p.clone()).unwrap();
    s.insert("z");
    let now = (s.doc.disk, s.doc.deleted);
    assert_eq!(act(polled, then, now, s.doc.dirty, s.doc.watching), Act::Nothing);
    assert_eq!(check(&p, s.doc.disk, s.doc.deleted), Disk::Same);
}
