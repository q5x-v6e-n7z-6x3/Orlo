use super::*;

mod changes;
mod columns;
/// Drag and drop in the editing model: dragged text moves or is copied to a new place in one step.
mod drag_tests;
mod editing;
#[allow(clippy::single_range_in_vec_init)]
mod editor_feature_tests;
mod endings;
mod files;
mod follow;
mod lines;
/// Bookmarks from matches and highlight styles in the editing model.
#[allow(clippy::single_range_in_vec_init)]
mod marking_tests;
mod motion;
mod multi_caret;
mod overtype;
mod positions;
mod preferred_x;
mod saving;
mod search;
/// Lookarounds, multiline patterns and case-preserving replacement in the editing model: the
/// shared engine through find, scoped find and replace.
#[allow(clippy::single_range_in_vec_init)]
mod search_workflow_tests;
mod selections;
#[allow(clippy::single_range_in_vec_init)]
mod shared;
mod snippet_tests;
mod transforms;
mod virtual_space;

// ---- one document in several views ----

/// One document in two views, as two groups show it. `a()` and `b()` are each view at work: it
/// publishes the other's edits and applies them first, as the editor does at each command.
pub(crate) struct Views {
    pub(crate) doc: Document,
    pub(crate) a: ViewModel,
    pub(crate) b: ViewModel,
}

pub(crate) fn views(text: &str) -> Views {
    two_views(EditorState::from_text(text))
}

/// `s`'s document with its view, and a second view where that one is.
pub(crate) fn two_views(s: EditorState) -> Views {
    let (doc, a) = s.into_parts();
    let b = a.duplicate(&doc);
    Views { doc, a, b }
}

impl Views {
    pub(crate) fn a(&mut self) -> ViewEdit<'_> {
        Self::switch(&mut self.doc, &mut self.a, &mut self.b)
    }

    pub(crate) fn b(&mut self) -> ViewEdit<'_> {
        Self::switch(&mut self.doc, &mut self.b, &mut self.a)
    }

    fn switch<'a>(doc: &'a mut Document, this: &'a mut ViewModel, other: &mut ViewModel) -> ViewEdit<'a> {
        Editing { doc: &mut *doc, view: other }.take_change();
        let mut e = Editing { doc, view: this };
        e.catch_up();
        e
    }
}

/// Where an outline's start belongs: at its first original char that survives `changes`,
/// past any text inserted before it (the end of the text when none survives).
pub(crate) fn map_start(x: usize, changes: &[Change]) -> usize {
    // each char's original index, `None` for inserted ones
    let mut text: Vec<Option<usize>> = (0..200).map(Some).collect();
    for c in changes {
        text.splice(c.start..c.start + c.removed, (0..c.inserted).map(|_| None));
    }
    (text.iter()).position(|i| i.is_some_and(|i| i >= x)).unwrap_or(text.len())
}

/// The bookmarked lines' texts, checking each bookmark sits on its line's start.
pub(crate) fn marked<D: DocMut, V: ViewMut>(s: &Editing<D, V>) -> Vec<String> {
    s.view
        .bookmarks
        .iter()
        .map(|&b| {
            let l = s.point(b).0;
            assert_eq!(b, s.line_start(l), "bookmark {b} is not on a line start");
            s.line(l)
        })
        .collect()
}

pub(crate) fn loaded(text: &str) -> Loaded {
    Loaded {
        endings: None,
        rope: Rope::from_str(text),
        encoding: Default::default(),
        line_ending: "\n",
        read_only: false,
        disk: None,
        bom: false,
    }
}

pub(crate) fn reload<D: DocMut, V: ViewMut>(s: &mut Editing<D, V>, l: Loaded) {
    let changes = l.diff(&s.doc.rope);
    s.reload_from(l, &changes);
}

pub(crate) fn fresh_dir(name: &str) -> PathBuf {
    use std::os::unix::fs::DirBuilderExt;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let dir = std::env::temp_dir().join(format!(
            "{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return dir,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("Cannot create test directory {}: {e}", dir.display()),
        }
    }
}

#[test]
fn fresh_test_directories_never_reuse_or_remove_an_existing_fixture() {
    let first = fresh_dir("orlo-fixture-isolation");
    let keep = first.join("keep.txt");
    std::fs::write(&keep, "first fixture").unwrap();
    let second = fresh_dir("orlo-fixture-isolation");
    assert_ne!(first, second);
    assert_eq!(std::fs::read_to_string(keep).unwrap(), "first fixture");
    std::fs::remove_dir_all(first).unwrap();
    std::fs::remove_dir_all(second).unwrap();
}

pub(crate) fn set_mode(p: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))
}

/// The permission bits of `p`.
pub(crate) fn mode_of(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

/// `cmd args path`'s output; a command that fails fails the test, with what it said.
pub(crate) fn sh(cmd: &str, args: &[&str], path: &Path) -> String {
    let out = std::process::Command::new(cmd).args(args).arg(path).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{cmd} {args:?} {path:?}: {err}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub(crate) fn st(text: &str, caret: usize) -> EditorState {
    carets(text, &[caret])
}

pub(crate) fn wrapped(text: &str, caret: usize) -> (EditorState, crate::viewport::WrapMap) {
    let s = st(text, caret);
    let map = crate::viewport::WrapMap::new(&s.doc, 40, 4, &crate::fold::Folds::NONE);
    (s, map)
}

pub(crate) fn walk(s: &mut EditorState, map: &dyn Rows, motion: Motion, n: usize) -> Vec<usize> {
    (0..n)
        .map(|_| {
            s.move_cursors_in(motion, false, map, &Cells);
            s.cursor()
        })
        .collect()
}

/// `undo_selection`'s stack depth, for tests.
pub(crate) fn sel_depth<D, V: ViewRef>(s: &Editing<D, V>) -> (usize, usize) {
    (s.view.sel_hist.undo.len(), s.view.sel_hist.redo.len())
}

pub(crate) fn sel_texts(s: &EditorState) -> Vec<String> {
    s.view.selections.iter().map(|x| s.doc.rope.slice(x.range()).to_string()).collect()
}

pub(crate) fn carets(text: &str, at: &[usize]) -> EditorState {
    let mut s = EditorState::from_text(text);
    s.view.selections = cs(at);
    s
}

/// `text` with the selections `pairs` (anchor, head).
pub(crate) fn selecting(text: &str, pairs: &[(usize, usize)]) -> EditorState {
    let mut s = EditorState::from_text(text);
    s.view.selections = sels(pairs);
    s
}

// ---- Edit > Columns ----

/// A column command at once, as the editor does for a small document.
pub(crate) fn column_command<D: DocMut, V: ViewMut>(
    s: &mut Editing<D, V>,
    op: ColumnOp,
) -> Result<Option<String>, String> {
    let plan = s.column_job(op).run()?;
    Ok(s.apply_column_plan(plan))
}

/// Carets at `at`.
pub(crate) fn cs(at: &[usize]) -> Vec<Selection> {
    at.iter().map(|&o| Selection::caret(o)).collect()
}

/// Selections from (anchor, head) pairs.
pub(crate) fn sels(pairs: &[(usize, usize)]) -> Vec<Selection> {
    pairs.iter().map(|&(a, h)| Selection::new(a, h)).collect()
}

pub(crate) fn change(start: usize, removed: usize, inserted: usize) -> Change {
    Change { start, removed, inserted, ..Change::default() }
}

/// A linear congruential generator: `rnd(n)` is below `n`.
pub(crate) fn lcg(mut seed: u64) -> impl FnMut(usize) -> usize {
    move |n| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize % n
    }
}

pub(crate) fn opts_for(case_sensitive: bool, whole_word: bool) -> FindOpts {
    FindOpts { case_sensitive, whole_word, ..FindOpts::default() }
}

pub(crate) fn sort(case_sensitive: bool) -> Permute {
    Permute::Sort { case_sensitive }
}

pub(crate) fn carets_of(s: &EditorState) -> Vec<usize> {
    s.view.selections.iter().map(|x| x.head).collect()
}

/// Each selection's (anchor, head).
pub(crate) fn ends_of(s: &EditorState) -> Vec<(usize, usize)> {
    s.view.selections.iter().map(|x| (x.anchor, x.head)).collect()
}

pub(crate) fn strings(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

/// Undo puts back the text and the selections, redo the edit and its selections.
pub(crate) fn assert_undo_redo(s: &mut EditorState, before: (&str, Vec<Selection>), after: (&str, Vec<Selection>)) {
    s.undo();
    assert_eq!((s.text().as_str(), s.view.selections.clone()), before, "undo");
    s.redo();
    assert_eq!((s.text().as_str(), s.view.selections.clone()), after, "redo");
}

mod watch;
