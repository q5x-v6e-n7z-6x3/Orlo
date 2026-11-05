use crate::buffer::{DocRef, Document, Editing, SelState, Selection, ViewRef};
use std::ops::Range;
use std::path::PathBuf;

pub struct FilterSnapshot {
    buffer: u64,
    view: u64,
    version: u64,
    path: Option<PathBuf>,
    selection_generation: u64,
    state: SelState,
    pub selections: Vec<Selection>,
    /// The legacy single-input API range.
    pub range: Range<usize>,
    pub ranges: Vec<Range<usize>>,
}

impl FilterSnapshot {
    /// Legacy single-input commands, including extension API replies.
    pub fn new<D: DocRef, V: ViewRef>(s: &Editing<D, V>) -> Result<Self, String> {
        if s.view.selections.len() != 1 {
            return Err("Use one selection, or a caret to filter the document".into());
        }
        Self::for_selections(s, true)
    }

    /// Independent physical ranges; only the legacy filter permits one real caret's fallback.
    pub fn for_selections<D: DocRef, V: ViewRef>(s: &Editing<D, V>, whole_document: bool) -> Result<Self, String> {
        if s.view.selections.len() > 4096 {
            return Err("Shell filters exceed the 4096-selection limit".into());
        }
        if s.view.selections.iter().any(Selection::has_virtual) {
            return Err("Shell filters require physical selections without virtual space".into());
        }
        let mut ranges: Vec<_> = s.view.selections.iter().filter(|sel| !sel.is_empty()).map(Selection::range).collect();
        ranges.sort_by_key(|range| range.start);
        if ranges.windows(2).any(|pair| pair[0].end > pair[1].start) {
            return Err("Shell filter selections must not overlap".into());
        }
        if ranges.is_empty() {
            if whole_document && s.view.selections.len() == 1 {
                ranges.push(0..s.len());
            } else {
                return Err("Select physical nonempty text for each shell input".into());
            }
        }
        Ok(Self {
            buffer: s.buffer_id(),
            view: s.view_id(),
            version: s.doc.version,
            path: s.doc.path.clone(),
            selection_generation: s.selection_generation(),
            state: s.selection_state(),
            selections: s.view.selections.clone(),
            range: ranges[0].clone(),
            ranges,
        })
    }

    pub fn current<D: DocRef, V: ViewRef>(&self, s: &Editing<D, V>) -> bool {
        self.buffer == s.buffer_id()
            && self.view == s.view_id()
            && self.version == s.doc.version
            && self.path == s.doc.path
            && self.selection_generation == s.selection_generation()
            && self.state == s.selection_state()
    }

    pub fn input(&self, s: &Document) -> String {
        s.rope.slice(self.range.clone()).to_string()
    }

    pub fn inputs(&self, s: &Document) -> Vec<(Range<usize>, String)> {
        self.ranges.iter().map(|range| (range.clone(), s.rope.slice(range.clone()).to_string())).collect()
    }

    pub fn retained(&self, keep: &[bool]) -> SelState {
        let mut state = self.state.clone();
        let primary = state.selections[state.primary];
        state.selections.retain(|sel| {
            !sel.is_empty()
                && self.ranges.binary_search_by_key(&sel.start(), |range| range.start).ok().is_some_and(|i| keep[i])
        });
        if state.selections.is_empty() {
            state.selections.push(Selection::caret(primary.head));
        }
        state.primary = state.selections.iter().position(|sel| *sel == primary).unwrap_or(state.selections.len() - 1);
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orlo_config::automation::{diagnostic, result_output};
    use std::path::{Path, PathBuf};

    #[test]
    fn filtering_requires_the_original_buffer_version_and_selections() {
        let mut state = crate::buffer::EditorState::from_text("aéz");
        state.view.selections = vec![crate::buffer::Selection::new(1, 2)];
        let snapshot = FilterSnapshot::new(&state).unwrap();
        assert_eq!(snapshot.input(&state.doc), "é");
        assert!(snapshot.current(&state));
        state.set_cursor(0);
        assert!(!snapshot.current(&state));
        state.view.selections = snapshot.selections.clone();
        state.replace_ranges(vec![(0..1, "b".into())]);
        assert!(!snapshot.current(&state));
        assert!(!snapshot.current(&crate::buffer::EditorState::from_text("aéz")));
    }

    #[test]
    fn n38_snapshot_refuses_virtual_document_fallback_and_selection_away_back() {
        let mut state = crate::buffer::EditorState::from_text("aéz");
        state.set_virtual_space(crate::buffer::VirtualSpace::All);
        state.set_selections(vec![crate::buffer::Selection::from_points((3, 2), (3, 2))]);
        assert!(FilterSnapshot::new(&state).is_err(), "a virtual caret must never filter the document");
        state.set_selections(vec![crate::buffer::Selection::new(1, 2)]);
        state.sel_checkpoint();
        let snapshot = FilterSnapshot::new(&state).unwrap();
        state.set_selections(vec![crate::buffer::Selection::caret(0)]);
        state.set_selections(snapshot.selections.clone());
        assert!(!snapshot.current(&state), "away and back invalidates pending filters");
    }

    #[test]
    fn n38_capture_rejects_ambiguous_inputs_and_preserves_legacy_api() {
        let mut state = crate::buffer::EditorState::from_text("é / 界 / fin");
        assert_eq!(FilterSnapshot::new(&state).unwrap().input(&state.doc), "é / 界 / fin");
        assert!(FilterSnapshot::for_selections(&state, false).is_err());
        state.set_selections(vec![Selection::caret(0), Selection::caret(4)]);
        assert!(FilterSnapshot::for_selections(&state, true).is_err());
        state.view.selections = vec![Selection::new(4, 5), Selection::caret(9), Selection::new(1, 0)];
        let snapshot = FilterSnapshot::for_selections(&state, false).unwrap();
        assert_eq!(snapshot.inputs(&state.doc), vec![(0..1, "é".into()), (4..5, "界".into())]);
        assert!(FilterSnapshot::new(&state).is_err(), "extension API stays single-input");
        state.view.selections = vec![Selection::new(0, 5), Selection::new(4, 7)];
        assert!(FilterSnapshot::for_selections(&state, true).is_err(), "overlap must not cause double replacement");
        state.view.selections = vec![Selection::new(0, 1); 4097];
        assert!(FilterSnapshot::for_selections(&state, true).is_err());
    }

    #[test]
    fn diagnostics_become_navigable_results_with_unicode_columns() {
        let (out, ranges) = result_output("src/a file.rs:12:3: failure\nplain log\n", Path::new("/work"));
        let state = crate::buffer::EditorState::from_text(&out);
        let line = state.point(ranges[0].start).0;
        assert_eq!(
            crate::search::search::result_target(&state.doc, line),
            Some((PathBuf::from("/work/src/a file.rs"), 11))
        );
        assert_eq!(crate::search::search::result_match(&state.doc, &ranges, line, ranges[0].start), Some((2, None)));
        assert!(out.contains("plain log"));
        assert!(diagnostic("x:0:2: bad").is_none());
        assert!(diagnostic("x:1:0: bad").is_none());
    }
}
