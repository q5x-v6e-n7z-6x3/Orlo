use crate::buffer::*;

/// The find panel's toggles; wrap-around is on by default like Sublime's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindOpts {
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub wrap: bool,
}

impl Default for FindOpts {
    fn default() -> Self {
        Self { regex: false, case_sensitive: false, whole_word: false, wrap: true }
    }
}

impl FindOpts {
    /// The query as the regex engine reads it, whole-word edges included.
    pub fn hir(&self, query: &str) -> Result<Hir, String> {
        let mut hir =
            syntax::parse_with(query, &syntax::Config::new().case_insensitive(!self.case_sensitive).multi_line(true))
                .map_err(|e| e.to_string())?;
        if self.whole_word {
            // Apply boundaries inside the engine so greedy matches can choose a shorter end.
            let edge =
                || Hir::alternation(vec![Hir::look(Look::WordStartHalfUnicode), Hir::look(Look::WordEndHalfUnicode)]);
            hir = Hir::concat(vec![edge(), hir, edge()]);
        }
        Ok(hir)
    }

    /// The engine every pattern is built with.
    pub fn engine() -> meta::Builder {
        let mut b = meta::Regex::builder();
        b.configure(meta::Regex::config().nfa_size_limit(Some(10 << 20)));
        b
    }

    /// The linear engine alone (tests compare it with `Pattern`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn pattern(&self, query: &str) -> Result<meta::Regex, String> {
        let hir = self.hir(query)?;
        Self::engine().build_from_hir(&hir).map_err(|e| e.to_string())
    }
}
/// Non-overlapping matches of `query` in `hay` with the Find panel's case and whole-word rules,
/// shared with Find in Files so both agree.
#[cfg(any(test, feature = "test-support"))]
pub fn find_in(hay: &[char], query: &[char], o: &FindOpts) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + query.len() <= hay.len() {
        let hit = hay[i..i + query.len()].iter().zip(query).all(|(a, b)| same_char(*a, *b, o))
            && (!o.whole_word || (word_edge(hay, i) && word_edge(hay, i + query.len())));
        if hit {
            out.push(i..i + query.len());
            i += query.len();
        } else {
            i += 1;
        }
    }
    out
}

/// `find_in` on a string, with the same matches as char ranges, but no `Vec<char>` (four
/// bytes a char) of the haystack: Find in Files reads a 99 MB one-line file without one.
pub fn find_in_str(hay: &str, query: &str, o: &FindOpts) -> Vec<Range<usize>> {
    find_in_str_until(hay, query, o, &|| false).unwrap_or_default()
}

/// `find_in_str`, given up (`Cancelled`) once `stop` says so; it is asked every megabyte, the
/// window searched set once a window, not once a match. An invalid pattern finds nothing (the
/// panels say why); a runaway one is an error.
pub fn find_in_str_until(
    hay: &str,
    query: &str,
    o: &FindOpts,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<Range<usize>>, SearchError> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    if o.regex {
        return match Pattern::new(query, o) {
            Ok(re) => regex_matches(hay, &re, stop),
            Err(_) => Ok(Vec::new()),
        };
    }
    literal_matches_until(hay, query, o, stop, &|b| word_edge_in_str(hay, b))
}

/// Literal matches test their boundary before consuming a candidate. Scoped searches
/// supply boundaries from the full document, including context outside the scope.
fn literal_matches_until(
    hay: &str,
    query: &str,
    o: &FindOpts,
    stop: &dyn Fn() -> bool,
    edge: &dyn Fn(usize) -> bool,
) -> Result<Vec<Range<usize>>, SearchError> {
    const SEG: usize = 1 << 20;
    #[cfg(any(test, feature = "test-support"))]
    SCANNED.set(SCANNED.get() + hay.len());
    let (hb, qb) = (hay.as_bytes(), query.as_bytes());
    let qc: Vec<char> = query.chars().collect();
    // Unicode letters can lowercase to ASCII too (for example K to k). The byte
    // matcher is sufficient only when both the query and haystack are ASCII.
    let fold = !o.case_sensitive && (!query.is_ascii() || !hay.is_ascii());
    // the bytes a case-insensitive match can start with: the first char's either case, and a
    // char's lead byte when Unicode folding is needed; the rest is passed over byte by byte.
    // An ASCII first char is only matched by ASCII and by KELVIN SIGN (E2 84 AA, lowercase k),
    // so Chinese or Russian text is not tried at every char for a query such as "key"
    let mut starts = [false; 256];
    let k = qc[0].eq_ignore_ascii_case(&'k');
    for (b, s) in (0..=255u8).zip(&mut starts) {
        *s = match b.is_ascii() {
            true => EditorState::eq_ignore_case(b as char, qc[0]),
            false => fold && b >= 0xC0 && (!qc[0].is_ascii() || k && b == 0xE2),
        };
    }
    // the end of a case-insensitive match at byte `b`, if there is one
    let at = |b: usize| -> Option<usize> {
        if !fold {
            let w = hb.get(b..b + qb.len())?;
            return w.eq_ignore_ascii_case(qb).then_some(b + qb.len());
        }
        let mut e = b;
        let mut cs = hay[b..].chars();
        for q in &qc {
            let c = cs.next().filter(|c| EditorState::eq_ignore_case(*c, *q))?;
            e += c.len_utf8();
        }
        Some(e)
    };
    // the next match's (start, end) bytes starting in `from..to`; a case-sensitive one is looked
    // for in `from..end`, and one starting past `to` is the next window's first, taken now
    let next = |mut from: usize, to: usize, end: usize| -> Option<(usize, usize)> {
        if o.case_sensitive {
            let b = from + hay[from..end].find(query)?;
            return Some((b, b + qb.len()));
        }
        while let Some(i) = hb[from..to].iter().position(|&c| starts[c as usize]) {
            let b = from + i;
            if let Some(e) = at(b) {
                return Some((b, e));
            }
            from = b + 1;
        }
        None
    };
    let n = qc.len();
    let mut out = Vec::new();
    let (mut from, mut chars, mut counted) = (0, 0, 0);
    let (mut to, mut end) = (0, 0);
    while from < hay.len() {
        if from >= to {
            if stop() {
                return Err(SearchError::Cancelled);
            }
            to = hay.ceil_char_boundary((from + SEG).min(hay.len()));
            end = hay.ceil_char_boundary((to + qb.len()).min(hay.len()));
        }
        let Some((b, e)) = next(from, to, end) else {
            from = to;
            continue;
        };
        if !o.whole_word || (edge(b) && edge(e)) {
            chars += hay[counted..b].chars().count();
            out.push(chars..chars + n);
            (chars, counted, from) = (chars + n, e, e);
        } else {
            from = b + hay[b..].chars().next().map_or(1, char::len_utf8);
        }
    }
    Ok(out)
}

fn word_edge_in_str(hay: &str, b: usize) -> bool {
    let prev = hay[..b].chars().next_back();
    let next = hay[b..].chars().next();
    !(prev.is_some_and(word_continuation) && next.is_some_and(word_continuation))
}

/// Regex byte offsets become the character offsets used by every editor command.
pub fn regex_matches(hay: &str, re: &Pattern, stop: &dyn Fn() -> bool) -> Result<Vec<Range<usize>>, SearchError> {
    regex_matches_in(hay, re, std::slice::from_ref(&(0..hay.len())), stop)
}

pub(crate) fn regex_matches_in(
    hay: &str,
    re: &Pattern,
    scopes: &[Range<usize>],
    stop: &dyn Fn() -> bool,
) -> Result<Vec<Range<usize>>, SearchError> {
    let found = re.find(hay, scopes, stop)?;
    Ok(byte_to_char_ranges(hay, &found))
}

/// Sorted char ranges of `hay` as byte ranges, counted once along the text.
pub fn char_to_byte_ranges(hay: &str, found: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut it = hay.char_indices().map(|(b, _)| b).chain(std::iter::once(hay.len()));
    let (mut at, mut byte) = (0, it.next().unwrap_or(0));
    let mut to = |c: usize| {
        while at < c {
            byte = it.next().unwrap_or(hay.len());
            at += 1;
        }
        byte
    };
    found.iter().map(|r| to(r.start)..to(r.end)).collect()
}

/// Sorted byte ranges of `hay` as char ranges, counted once along the text.
pub fn byte_to_char_ranges(hay: &str, found: &[Range<usize>]) -> Vec<Range<usize>> {
    let (mut counted, mut chars) = (0, 0);
    let mut out = Vec::with_capacity(found.len());
    for m in found {
        chars += hay[counted..m.start].chars().count();
        let end = chars + hay[m.clone()].chars().count();
        out.push(chars..end);
        (counted, chars) = (m.end, end);
    }
    out
}

/// Search each selected span from its own boundary, retaining the surrounding anchor context.
pub fn find_in_rope_until(
    rope: &Rope,
    query: &str,
    o: &FindOpts,
    scope: Option<&[Range<usize>]>,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<Range<usize>>, SearchError> {
    let hay = rope.to_string();
    let Some(scope) = scope else {
        return find_in_str_until(&hay, query, o, stop);
    };
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let mut scopes = scope.to_vec();
    scopes.sort_by_key(|r| r.start);
    scopes.dedup_by(|next, prev| {
        let overlap = next.start < prev.end;
        if overlap {
            prev.end = prev.end.max(next.end);
        }
        overlap
    });
    let scope = scopes.as_slice();
    let byte = |c: usize| rope.char_to_byte(c.min(rope.len_chars()));
    if o.regex {
        let Ok(re) = Pattern::new(query, o) else {
            return Ok(Vec::new());
        };
        let bytes: Vec<_> = scope.iter().map(|r| byte(r.start)..byte(r.end)).collect();
        let mut all = regex_matches_in(&hay, &re, &bytes, stop)?;
        all.dedup();
        return Ok(all);
    }
    let mut all = Vec::new();
    for r in scope {
        let (start, b0) = (r.start.min(rope.len_chars()), byte(r.start));
        let text = &hay[b0..byte(r.end)];
        all.extend(
            literal_matches_until(text, query, o, stop, &|b| word_edge_in_str(&hay, b0 + b))?
                .into_iter()
                .map(|m| start + m.start..start + m.end),
        );
    }
    all.dedup();
    Ok(all)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Kept matches moved through edits on this thread (`catch_up_found`).
    pub static MAPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Bytes `find_in_str` has searched on this thread: how much work a find did.
    pub static SCANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether two matches of `q` could overlap ("aa" in "aaa"): then matches far from an edit
/// may shift, so `catch_up_found` searches again from the start.
pub(crate) fn overlaps_itself(q: &str, o: &FindOpts) -> bool {
    let c: Vec<char> = q.chars().collect();
    (1..c.len()).any(|k| c[k..].iter().zip(&c).all(|(a, b)| same_char(*a, *b, o)))
}

/// Whether `a` and `b` match, as the case toggle says.
fn same_char(a: char, b: char, o: &FindOpts) -> bool {
    match o.case_sensitive {
        true => a == b,
        false => EditorState::eq_ignore_case(a, b),
    }
}

/// Whether a word cannot continue across `i` (`find_in`'s whole-word test).
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn word_edge(hay: &[char], i: usize) -> bool {
    i == 0 || i >= hay.len() || !word_continuation(hay[i - 1]) || !word_continuation(hay[i])
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    // ---- search ----

    pub(crate) fn eq_ignore_case(a: char, b: char) -> bool {
        match (a.is_ascii(), b.is_ascii()) {
            (true, true) => a.eq_ignore_ascii_case(&b),
            _ => a == b || a.to_lowercase().eq(b.to_lowercase()),
        }
    }

    /// Non-overlapping matches, honouring the case and whole-word toggles.
    /// Every match, kept for this text, query and options: find-as-you-type, Find Next and
    /// the outlines ask several times a keystroke, and a search copies the buffer once.
    pub fn find_all(&self, query: &str, o: &FindOpts) -> Vec<Range<usize>> {
        self.with_matches(query, o, <[_]>::to_vec)
    }

    pub fn find_scoped(&self, query: &str, o: &FindOpts, scope: &[Range<usize>]) -> Vec<Range<usize>> {
        self.with_scoped_matches(query, o, Some(scope), <[_]>::to_vec)
    }

    /// `f` on `find_all`'s matches, without copying them.
    pub(crate) fn with_matches<R>(&self, query: &str, o: &FindOpts, f: impl FnOnce(&[Range<usize>]) -> R) -> R {
        self.with_scoped_matches(query, o, None, f)
    }

    pub(crate) fn with_scoped_matches<R>(
        &self,
        query: &str,
        o: &FindOpts,
        scope: Option<&[Range<usize>]>,
        f: impl FnOnce(&[Range<usize>]) -> R,
    ) -> R {
        if query.is_empty() {
            return f(&[]);
        }
        self.found_up_to_date();
        let kept = (self.view.found.borrow().as_ref()).is_some_and(|f| self.kept(f, query, o, scope));
        if !kept {
            // one copy of the text a search; scan the rope's chunks if it shows on
            // huge files
            let d = Deadline::new(crate::search::pattern::UI_SECONDS);
            match find_in_rope_until(&self.doc.rope, query, o, scope, &|| d.passed()) {
                Ok(all) => self.remember_scoped_found(query, o, scope, all),
                Err(e) => self.remember_search_error(query, o, scope, d.explain(e)),
            }
        }
        match &*self.view.found.borrow() {
            Some((_, _, _, all, ..)) => f(all),
            None => f(&[]),
        }
    }

    /// Why the last search for these stopped without its matches, if it did and the text is
    /// still the one it searched.
    pub fn search_error(&self, query: &str, o: &FindOpts, scope: Option<&[Range<usize>]>) -> Option<SearchError> {
        let found = self.view.found.borrow();
        let f = found.as_ref().filter(|f| self.kept(f, query, o, scope))?;
        f.7.clone()
    }

    /// Whether `f` holds the matches of `query`, `o` and `scope` in the text as it is.
    fn kept(&self, f: &Found, query: &str, o: &FindOpts, scope: Option<&[Range<usize>]>) -> bool {
        let key = (self.doc.version, self.doc.rope.len_chars());
        f.0 == key && f.1 == query && f.2 == *o && f.6.as_deref() == scope
    }

    /// A search for these that gave up: no matches, and `e` for `search_error`.
    pub fn remember_search_error(&self, query: &str, o: &FindOpts, scope: Option<&[Range<usize>]>, e: SearchError) {
        self.remember_scoped_found(query, o, scope, Vec::new());
        if let Some(f) = &mut *self.view.found.borrow_mut() {
            f.7 = Some(e);
        }
    }

    /// Whether `r` is one of the cached matches, without copying them.
    pub fn is_match(&self, query: &str, o: &FindOpts, r: &Range<usize>) -> bool {
        self.with_matches(query, o, |all| all.binary_search_by_key(&r.start, |m| m.start).is_ok_and(|i| all[i] == *r))
    }

    /// `find_all`'s matches if they are kept for this text: after an edit, brought up to it
    /// around the edit alone.
    pub fn found(&self, query: &str, o: &FindOpts) -> Option<Vec<Range<usize>>> {
        self.found_scoped(query, o, None)
    }

    pub fn found_scoped(&self, query: &str, o: &FindOpts, scope: Option<&[Range<usize>]>) -> Option<Vec<Range<usize>>> {
        self.found_up_to_date();
        let found = self.view.found.borrow();
        let f = found.as_ref()?;
        self.kept(f, query, o, scope).then(|| f.3.clone())
    }

    /// Whether `found` has them, without copying them (10M matches are 160 MB).
    pub fn has_found(&self, query: &str, o: &FindOpts) -> bool {
        self.found_up_to_date();
        let found = self.view.found.borrow();
        found.as_ref().is_some_and(|f| self.kept(f, query, o, None))
    }

    /// The kept matches brought through the edits since they were taken: those noted while
    /// `changes` was taken (`defer_found`), then those `changes` still holds.
    pub(crate) fn found_up_to_date(&self) {
        let pending = {
            let mut found = self.view.found.borrow_mut();
            let Some(f) = &mut *found else { return };
            let mut all = std::mem::take(&mut f.5);
            all.extend_from_slice(self.view.changes.get(f.4..).unwrap_or(&[]));
            f.4 = self.view.changes.len();
            all
        };
        self.catch_up_found(&pending);
    }

    pub fn remember_scoped_found(
        &self,
        query: &str,
        o: &FindOpts,
        scope: Option<&[Range<usize>]>,
        all: Vec<Range<usize>>,
    ) {
        let key = (self.doc.version, self.doc.rope.len_chars());
        *self.view.found.borrow_mut() = Some((
            key,
            query.to_string(),
            *o,
            all,
            self.view.changes.len(),
            Vec::new(),
            scope.map(<[_]>::to_vec),
            None,
        ));
    }

    /// Bring the kept matches through `changes` (made to the rope since they were found):
    /// a query that cannot overlap itself keeps its matches away from the edits, and only the
    /// text around each edit is searched again. Otherwise, or after more than 64 changes (a
    /// Replace All), they are dropped for a whole search.
    pub(crate) fn catch_up_found(&self, changes: &[Change]) {
        let mut f = self.view.found.borrow_mut();
        let Some((key, q, o, all, _, _, scope, error)) = &mut *f else {
            return;
        };
        let now = (self.doc.version, self.len());
        if *key == now {
            return;
        }
        if scope.is_some()
            || error.is_some()
            || o.regex
            || changes.is_empty()
            || changes.len() > 64
            || overlaps_itself(q, o)
        {
            *f = None;
            return;
        }
        let n = q.chars().count();
        // what each change left, where it is after the later ones, widened by a match and a
        // char each side (a whole word's edges)
        let mut dirty: Vec<Range<usize>> = (changes.iter().enumerate())
            .map(|(i, c)| {
                let later = &changes[i + 1..];
                let (a, b) = (c.start, c.start + c.inserted);
                let (a, b) = (map_offset(a, later), map_offset(b, later).max(a));
                a.saturating_sub(n + 1)..(b + n + 1).min(now.1)
            })
            .collect();
        dirty.sort_by_key(|r| r.start);
        let mut windows: Vec<Range<usize>> = Vec::new();
        for r in dirty {
            match windows.last_mut() {
                Some(w) if r.start <= w.end => w.end = w.end.max(r.end),
                _ => windows.push(r),
            }
        }
        #[cfg(any(test, feature = "test-support"))]
        MAPPED.set(MAPPED.get() + all.len());
        let mut starts: Vec<usize> = all.iter().map(|r| r.start).collect();
        map_sorted(&mut starts, changes);
        let inside = |s: usize| {
            let k = windows.partition_point(|w| w.end < s + n);
            k < windows.len() && windows[k].start <= s
        };
        let mut out: Vec<Range<usize>> = starts.into_iter().filter(|&s| !inside(s)).map(|s| s..s + n).collect();
        for w in &windows {
            // a char of context each side, for a whole word's edges
            let from = w.start.saturating_sub(1);
            let hay = self.doc.rope.slice(from..(w.end + 1).min(now.1));
            out.extend(
                (find_in_str(&hay.to_string(), q, o).into_iter())
                    .map(|r| r.start + from..r.end + from)
                    .filter(|r| w.start <= r.start && r.end <= w.end),
            );
        }
        out.sort_by_key(|r| r.start);
        *all = out;
        *key = now;
    }

    pub fn find_next(&self, query: &str, from: usize, o: &FindOpts) -> Option<Range<usize>> {
        self.with_matches(query, o, |all| {
            let i = all.partition_point(|r| r.start < from);
            all.get(i).or(all.first().filter(|_| o.wrap)).cloned()
        })
    }

    pub fn find_prev(&self, query: &str, from: usize, o: &FindOpts) -> Option<Range<usize>> {
        self.with_matches(query, o, |all| {
            let i = all.partition_point(|r| r.start < from);
            let before = i.checked_sub(1).map(|i| &all[i]);
            before.or(all.last().filter(|_| o.wrap)).cloned()
        })
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn replace_all(&mut self, query: &str, replacement: &str, o: &FindOpts) -> usize {
        let all = self.find_all(query, o);
        self.replace_matches(query, replacement, o, &all)
    }

    /// Replace known matches, preserving the full buffer as the regex capture context. An
    /// invalid pattern or replacement changes nothing.
    #[cfg(any(test, feature = "test-support"))]
    pub fn replace_matches(&mut self, query: &str, replacement: &str, o: &FindOpts, all: &[Range<usize>]) -> usize {
        match Replacer::new(query, replacement, o, false) {
            Ok(r) => self.replace_with(&r, all),
            Err(_) => 0,
        }
    }

    /// `all` (char ranges, sorted) replaced as `r` says, one undo step; a regex match that no
    /// longer matches exactly there is left alone.
    pub fn replace_with(&mut self, r: &Replacer, all: &[Range<usize>]) -> usize {
        if self.doc.edits_blocked() {
            return 0;
        }
        if all.is_empty() {
            return 0;
        }
        let hay = (r.fixed().is_none()).then(|| (self.doc.rope.clone(), self.text()));
        self.set_typing(false);
        self.record();
        let n = self.view.changes.len();
        let aside = self.set_marks_aside();
        let mut done = 0;
        for m in all.iter().rev() {
            let text = match (&hay, r.fixed()) {
                (_, Some(t)) => t.to_string(),
                (Some((rope, hay)), None) => {
                    let bytes = rope.char_to_byte(m.start)..rope.char_to_byte(m.end);
                    match r.replacement(hay, bytes) {
                        Some(t) => t,
                        None => continue,
                    }
                }
                (None, None) => continue,
            };
            self.splice(m.clone(), &text);
            done += 1;
        }
        self.restore_marks(aside);
        if self.view.changes.len() > n {
            let changes = self.view.changes[n..].to_vec();
            self.map_selections(&changes, false);
            self.touch(self.doc.rope.char_to_line(all[0].start.min(self.len())));
        }
        self.normalize();
        done
    }
}

/// Current-buffer Find eligibility, composed after the ordinary engine and selection scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyntaxMask {
    #[default]
    Any,
    Comments,
    Strings,
    CommentsOrStrings,
}

impl SyntaxMask {
    pub fn next(self) -> Self {
        match self {
            Self::Any => Self::Comments,
            Self::Comments => Self::Strings,
            Self::Strings => Self::CommentsOrStrings,
            Self::CommentsOrStrings => Self::Any,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "Any syntax",
            Self::Comments => "Comments",
            Self::Strings => "Strings",
            Self::CommentsOrStrings => "Comments or Strings",
        }
    }

    /// Classify only START, including zero-width and cross-scope/multiline matches.
    /// Missing classification refuses the complete operation, including an exclusion.
    pub fn filter(
        self,
        exclude: bool,
        matches: &[Range<usize>],
        classify: impl Fn(usize) -> Option<(bool, bool)>,
    ) -> Option<Vec<Range<usize>>> {
        if self == Self::Any {
            return Some(matches.to_vec());
        }
        let mut eligible = Vec::new();
        for range in matches {
            let (comment, string) = classify(range.start)?;
            let included = match self {
                Self::Any => true,
                Self::Comments => comment,
                Self::Strings => string,
                Self::CommentsOrStrings => comment || string,
            };
            if included != exclude {
                eligible.push(range.clone());
            }
        }
        Some(eligible)
    }
}
