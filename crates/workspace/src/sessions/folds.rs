//! Bounded, versioned persistence for active per-view folds.
use orlo_editor::{fold::Folds, text::Document};
use ropey::Rope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const MAX_FOLDS: usize = 4096;
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CLOSED_VIEWS: usize = 32;

type Fingerprint = [u8; 32];

#[cfg(test)]
thread_local! {
    static HASHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn fingerprint(rope: &Rope) -> Fingerprint {
    #[cfg(test)]
    HASHED.set(HASHED.get() + 1);
    let mut hash = Sha256::new();
    for chunk in rope.chunks() {
        hash.update(chunk.as_bytes());
    }
    hash.finalize().into()
}

/// One view's immutable text fingerprint. Unchanged captures never scan its text, regardless
/// of how many other files are open. At most one bounded rope snapshot is retained per view.
#[derive(Default)]
pub struct FoldCache(Option<(Rope, Fingerprint)>);

impl FoldCache {
    pub fn capture(&mut self, doc: &Document, folds: &Folds) -> Option<FoldState> {
        if folds.is_empty() || doc.rope.len_bytes() > MAX_TEXT_BYTES {
            self.0 = None;
            return None;
        }
        if folds.len() > MAX_FOLDS {
            self.0 = None;
            return None;
        }
        let ranges = folds.chars();
        if self.0.as_ref().is_none_or(|(r, _)| !r.is_instance(&doc.rope)) {
            self.0 = Some((doc.rope.clone(), fingerprint(&doc.rope)));
        }
        Some(FoldState {
            version: 1,
            bytes: doc.rope.len_bytes(),
            sha256: self.0.as_ref().unwrap().1,
            ranges: ranges.into_iter().map(|r| (r.start, r.end)).collect(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FoldState {
    version: u8,
    bytes: usize,
    sha256: Fingerprint,
    ranges: Vec<(usize, usize)>,
}

impl FoldState {
    pub fn capture(doc: &Document, folds: &Folds) -> Option<Self> {
        FoldCache::default().capture(doc, folds)
    }

    /// Invalid records are discarded whole, never clamped or merged into different folds.
    pub fn restore(&self, doc: &Document) -> Folds {
        let mut folds = Folds::default();
        if self.version != 1
            || self.bytes > MAX_TEXT_BYTES
            || self.bytes != doc.rope.len_bytes()
            || self.ranges.is_empty()
            || self.ranges.len() > MAX_FOLDS
        {
            return folds;
        }
        let mut ends = Vec::new();
        let mut previous = None;
        for &(start, end) in &self.ranges {
            let key = (start, std::cmp::Reverse(end));
            if start >= end || end > doc.len() || previous.is_some_and(|p| p >= key) {
                return folds;
            }
            while ends.last().is_some_and(|&e| e <= start) {
                ends.pop();
            }
            if ends.last().is_some_and(|&e| end > e) {
                return folds;
            }
            ends.push(end);
            previous = Some(key);
        }
        if self.sha256 == fingerprint(&doc.rope) {
            folds.fold_many(doc, self.ranges.iter().map(|&(start, end)| start..end));
        }
        folds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::{Session, ViewState};
    use orlo_editor::text::EditorState;

    fn one(range: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
        std::iter::once(range).collect()
    }

    fn snapshot(text: &str, ranges: Vec<std::ops::Range<usize>>) -> (EditorState, FoldState) {
        let state = EditorState::from_text(text);
        let mut folds = Folds::default();
        folds.fold_many(&state.doc, ranges);
        let saved = FoldState::capture(&state.doc, &folds).unwrap();
        (state, saved)
    }

    #[test]
    fn fold_state_roundtrip_preserves_unicode_nested_inline_and_node_ends() {
        let (mut state, folds) = snapshot("é🦀 fn {\ninner();\n} suffix\n", vec![6..18, 8..13, 19..22]);
        let text = state.text();
        let view = ViewState { folds: Some(folds), ..Default::default() };
        let json = serde_json::to_string(&view).unwrap();
        let view: ViewState = serde_json::from_str(&json).unwrap();
        state.doc.read_only = true;
        let actual = view.folds.unwrap().restore(&state.doc);
        assert_eq!(actual.chars(), [6..18, 8..13, 19..22]);
        assert_eq!(state.text(), text);
        assert!(!state.doc.dirty);
        state.doc.read_only = false;
        state.insert("x");
        state.undo();
        assert_eq!(state.text(), text);
        state.redo();
        assert_eq!(state.text(), format!("x{text}"));
    }

    #[test]
    fn fold_state_rejects_stale_same_size_text_and_every_invalid_range_shape() {
        let (mut state, saved) = snapshot("abcdef\nghijk\nlmnop", vec![1..6, 2..4, 8..11]);
        for ranges in [
            vec![(2, 2)],
            vec![(4, 2)],
            vec![(0, usize::MAX)],
            vec![(1, 6), (4, 8)],
            vec![(1, 6), (1, 6)],
            vec![(8, 11), (1, 6)],
            vec![(1, 4), (1, 6)],
            vec![],
            vec![(1, 2); MAX_FOLDS + 1],
        ] {
            let mut invalid = saved.clone();
            invalid.ranges = ranges;
            assert!(invalid.restore(&state.doc).is_empty());
        }
        let mut invalid = saved.clone();
        invalid.version = 2;
        assert!(invalid.restore(&state.doc).is_empty());
        invalid = saved.clone();
        invalid.bytes += 1;
        assert!(invalid.restore(&state.doc).is_empty());
        // Same byte length and identical folded text: a changed suffix still invalidates it.
        state.doc.rope.remove(16..17);
        state.doc.rope.insert(16, "q");
        assert!(saved.restore(&state.doc).is_empty());
        state.doc.rope.insert(0, "🦀");
        assert!(saved.restore(&state.doc).is_empty());
    }

    #[test]
    fn fold_state_bounds_allow_large_files_and_cache_unchanged_captures() {
        let text = "abcde\n".repeat(100_000);
        let (state, saved) = snapshot(&text, (0..1000).map(|i| i * 60..i * 60 + 3).collect());
        assert_eq!(saved.restore(&state.doc).chars().len(), 1000);
        let mut folds = Folds::default();
        folds.fold_many(&state.doc, (0..=MAX_FOLDS).map(|i| i * 6..i * 6 + 1));
        assert!(FoldState::capture(&state.doc, &folds).is_none());
        let (mut limit, saved) = snapshot(&"a".repeat(MAX_TEXT_BYTES), one(1..2));
        assert_eq!(saved.restore(&limit.doc).chars(), one(1..2));
        limit.doc.rope.insert(0, "a");
        assert!(FoldState::capture(&limit.doc, &folds).is_none());
        assert!(saved.restore(&limit.doc).is_empty());
        let mut cache = FoldCache::default();
        let mut folds = Folds::default();
        folds.fold(&state.doc, 1..3);
        HASHED.set(0);
        let first = cache.capture(&state.doc, &folds).unwrap();
        let held = cache.0.as_ref().unwrap().0.clone();
        folds.fold(&state.doc, 4..5);
        let second = cache.capture(&state.doc, &folds).unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(HASHED.get(), 1, "fold-only changes reuse the immutable text fingerprint");
        assert!(cache.0.as_ref().unwrap().0.is_instance(&held));
        cache.capture(&state.doc, &Folds::default());
        assert!(cache.0.is_none());
    }

    #[test]
    fn fold_state_old_and_malformed_schemas_leave_the_view_and_session_usable() {
        for extra in ["", r#", "folds": 9"#, r#", "folds": {"version":1,"ranges":"wrong"}"#] {
            let json =
                format!(r#"{{"windows":[{{"groups":[{{"views":[{{"path":"/a","selections":[[2,2]]{extra}}}]}}]}}]}}"#);
            let session: Session = serde_json::from_str(&json).unwrap();
            let view = &session.windows[0].groups[0].views[0];
            assert_eq!(view.path.to_str(), Some("/a"));
            assert_eq!(view.selections, [(2, 2)]);
            assert!(view.folds.is_none());
        }
    }
}
