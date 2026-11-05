//! A Find in Files run as structured results, for the results browser, Replace in Files,
//! F4 in the browser, extraction and `<previous results>`. Pure: no gpui.
//!
//! Matches carry their full locations (`search::Hit`), so a cut excerpt never cuts what is
//! replaced or opened. Inclusion (what Apply changes) and the display filter (what the browser
//! lists) are kept apart: filtering never includes or excludes anything.

use crate::buffer::FindOpts;
use crate::search::search::{self, Hit};
use orlo_project::watch::Stamp;
use std::path::PathBuf;
use std::{ops::Range, sync::Arc};

/// Immutable physical hit lines from one exact source, independent of browser visibility.
#[derive(Clone, Debug)]
pub struct MatchingLines {
    pub path: PathBuf,
    /// The name originally read, so rebinding a symlink cannot retarget the scope.
    pub route: PathBuf,
    pub source: Source,
    pub ranges: Vec<Range<usize>>,
    pub buffer: Option<ropey::Rope>,
    pub file: Option<(u64, u64)>,
}

/// Whole physical lines touched by hits, including their terminators; adjacent lines merge.
pub fn matching_lines(text: &str, hits: &[Hit]) -> Vec<Range<usize>> {
    let mut starts = vec![0];
    let mut len = 0;
    for c in text.chars() {
        len += 1;
        if c == '\n' {
            starts.push(len);
        }
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for hit in hits {
        let lines = hit.lines();
        let range = starts[*lines.start()]..starts.get(*lines.end() + 1).copied().unwrap_or(len);
        match ranges.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => ranges.push(range),
        }
    }
    ranges
}

/// The Find in Files toggles beside the matching ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SearchFlags {
    /// Context lines around matches in the results: presentation only.
    pub context: bool,
    /// Open buffers, in every window, are searched as they are in the editor, unsaved edits
    /// included; off, every file is read from disk.
    pub use_buffer: bool,
    /// `.gitignore` files apply to the folders walked; explicit filters always apply.
    pub gitignore: bool,
    /// Replacements take the case of what they replace: replacement only.
    pub preserve_case: bool,
}

impl Default for SearchFlags {
    fn default() -> Self {
        Self { context: true, use_buffer: true, gitignore: true, preserve_case: false }
    }
}

/// The text a file's results were found in, to tell later whether it is still that text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// An open buffer's identity and version, shared by its views in every window.
    Buffer { id: u64, version: u64 },
    /// The file as read from disk: its stamp, a hash of its bytes, and how it is written back.
    Disk {
        stamp: Option<Stamp>,
        hash: u64,
        line_ending: &'static str,
        bom: bool,
        encoding: orlo_project::encoding::Encoding,
        /// Malformed text: shown, never replaced.
        lossy: bool,
        binary: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub hit: Hit,
    pub included: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Matches(Vec<Entry>),
    Binary,
    TooLarge,
    Failed(String),
}

/// What happened to a file when replacements were applied, or why its results are out of date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Written on disk.
    Changed,
    /// Changed in its open buffer, left unsaved for review.
    ChangedInBuffer,
    /// Every included replacement was the text already there.
    Unchanged,
    /// Nothing of it was included.
    Excluded,
    /// Its text changed after the preview: nothing applied; Refresh to search again.
    Stale(String),
    /// Left alone on purpose (an open buffer with unsaved changes, not UTF-8, binary).
    Skipped(String),
    Failed(String),
    /// Apply was cancelled before it was reached.
    NotReached,
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Status::Changed => "changed".into(),
            Status::ChangedInBuffer => "changed in its buffer (unsaved)".into(),
            Status::Unchanged => "unchanged".into(),
            Status::Excluded => "excluded".into(),
            Status::Stale(why) => format!("stale: {why}"),
            Status::Skipped(why) => format!("skipped: {why}"),
            Status::Failed(why) => format!("failed: {why}"),
            Status::NotReached => "not reached (cancelled)".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileResult {
    /// The file's real path: what opens and what is written.
    pub path: PathBuf,
    /// As Where named it (a folder reached through a link keeps the link's name).
    pub shown: PathBuf,
    pub source: Source,
    pub body: Body,
    /// The lines shown with their numbers (0-based): matching lines and their context.
    pub lines: Vec<(usize, String)>,
    pub included: bool,
    pub collapsed: bool,
    pub status: Option<Status>,
}

impl FileResult {
    pub fn entries(&self) -> &[Entry] {
        match &self.body {
            Body::Matches(e) => e,
            _ => &[],
        }
    }

    pub fn entries_mut(&mut self) -> &mut [Entry] {
        match &mut self.body {
            Body::Matches(e) => e,
            _ => &mut [],
        }
    }

    /// The included matches, if the file is.
    pub fn selected(&self) -> impl Iterator<Item = &Entry> {
        (self.entries().iter()).filter(move |e| self.included && e.included)
    }

    /// The text of line `l` as shown, if it is.
    pub fn line(&self, l: usize) -> Option<&str> {
        let i = self.lines.binary_search_by_key(&l, |(n, _)| *n).ok()?;
        Some(&self.lines[i].1)
    }
}

/// A row of the results browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    File(usize),
    /// File, match.
    Match(usize, usize),
    /// File, line number (0-based) of a context line.
    Context(usize, usize),
}

/// What Find in Files was asked, and what it found.
#[derive(Clone, Debug)]
pub struct ResultSet {
    pub query: String,
    /// Replace in Files: the replacement each match shows and Apply writes.
    pub replacement: Option<String>,
    pub opts: FindOpts,
    pub flags: SearchFlags,
    pub where_: String,
    /// Resolved inputs of this page; Refresh retains the chosen roots/files and filters.
    pub scope: Option<crate::search::files::Scope>,
    /// Files searched.
    pub searched: usize,
    pub files: Vec<FileResult>,
    pub done: bool,
    pub cancelled: bool,
    /// Why the run ended early, if it did on its own.
    pub stopped: Option<String>,
    /// Source hit lines collected by this run, never its context or filtered display rows.
    pub matching_lines: Vec<MatchingLines>,
    /// The chosen previous run's bounds; Refresh of this query never recaptures its results.
    pub previous_lines: Option<Arc<Vec<MatchingLines>>>,
}

/// (matches, files with matches, included matches, files with included matches)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub matches: usize,
    pub files: usize,
    pub selected: usize,
    pub selected_files: usize,
}

impl ResultSet {
    pub fn new(query: &str, replacement: Option<&str>, opts: FindOpts, flags: SearchFlags, where_: &str) -> Self {
        Self {
            query: query.to_string(),
            replacement: replacement.map(str::to_string),
            opts,
            flags,
            where_: where_.to_string(),
            scope: None,
            searched: 0,
            files: Vec::new(),
            done: false,
            cancelled: false,
            stopped: None,
            matching_lines: Vec::new(),
            previous_lines: None,
        }
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for f in &self.files {
            let (n, s) = (f.entries().len(), f.selected().count());
            c.matches += n;
            c.files += usize::from(n > 0);
            c.selected += s;
            c.selected_files += usize::from(s > 0);
        }
        c
    }

    /// "12 of 30 matches in 4 of 7 files selected", or for a plain search "30 matches in 7
    /// files".
    pub fn summary(&self) -> String {
        let c = self.counts();
        let n = |k: usize, one: &str, many: &str| match k {
            1 => format!("1 {one}"),
            k => format!("{k} {many}"),
        };
        let found = match self.replacement {
            None => format!("{} in {}", n(c.matches, "match", "matches"), n(c.files, "file", "files")),
            Some(_) => format!(
                "{} of {} in {} of {} selected",
                c.selected,
                n(c.matches, "match", "matches"),
                c.selected_files,
                n(c.files, "file", "files"),
            ),
        };
        let state = match (self.done, self.cancelled, &self.stopped) {
            (_, true, _) => " (cancelled)",
            (_, _, Some(_)) => " (stopped)",
            (false, ..) => " (searching…)",
            _ => "",
        };
        format!("{found}{state}")
    }

    pub fn set_all(&mut self, included: bool) {
        for f in &mut self.files {
            f.included = included;
            f.entries_mut().iter_mut().for_each(|e| e.included = included);
        }
    }

    /// A file's checkbox: on when any of its matches is to be applied; clicking it includes
    /// all of them, or excludes the file.
    pub fn toggle_file(&mut self, i: usize) {
        let Some(f) = self.files.get_mut(i) else {
            return;
        };
        let on = f.included && f.entries().iter().any(|e| e.included);
        f.included = !on;
        if !on {
            f.entries_mut().iter_mut().for_each(|e| e.included = true);
        }
    }

    pub fn toggle_match(&mut self, i: usize, j: usize) {
        let Some(f) = self.files.get_mut(i) else {
            return;
        };
        if f.included {
            if let Some(e) = f.entries_mut().get_mut(j) {
                e.included ^= true;
            }
            return;
        }
        // including a match of an excluded file includes the file with only that match
        f.included = true;
        for (k, e) in f.entries_mut().iter_mut().enumerate() {
            e.included = k == j;
        }
    }

    /// The browser's rows: each file, then (unless collapsed) its matches with their context
    /// lines when Show Context is on, in file order; `filter` hides rows only. A file or match
    /// shows under `filter` when a case-insensitive piece of its path or of a matching line is it.
    pub fn rows(&self, filter: &str) -> Vec<Row> {
        let filter = filter.to_lowercase();
        let mut rows = Vec::new();
        for (i, f) in self.files.iter().enumerate() {
            // what the filter hits, worked out once a file (not once a match, each render)
            let hit = |s: &str| s.to_lowercase().contains(&filter);
            let all = filter.is_empty() || hit(&search::tilde(&f.shown));
            let kept: Vec<bool> = if all { Vec::new() } else { f.lines.iter().map(|(_, l)| hit(l)).collect() };
            let line_hit = |l: usize| f.lines.binary_search_by_key(&l, |(n, _)| *n).is_ok_and(|k| kept[k]);
            let shown = |e: &Entry| all || e.hit.lines().any(line_hit);
            if !all && !f.entries().iter().any(shown) {
                continue;
            }
            rows.push(Row::File(i));
            if f.collapsed {
                continue;
            }
            let mut last_line: Option<usize> = None;
            for (j, e) in f.entries().iter().enumerate() {
                if !shown(e) {
                    continue;
                }
                let first = *e.hit.lines().start();
                if self.flags.context {
                    let from = first.saturating_sub(search::CONTEXT);
                    for l in from..first {
                        if last_line.is_none_or(|p| p < l) && f.line(l).is_some() {
                            rows.push(Row::Context(i, l));
                            last_line = Some(l);
                        }
                    }
                }
                rows.push(Row::Match(i, j));
                let end = *e.hit.lines().end();
                last_line = Some(last_line.map_or(end, |p| p.max(end)));
                if self.flags.context {
                    // the context after a match comes before the next match's own lines
                    let next = f.entries().get(j + 1).map(|n| *n.hit.lines().start());
                    for l in end + 1..=end + search::CONTEXT {
                        if next.is_some_and(|n| l >= n) {
                            break;
                        }
                        if last_line.is_none_or(|p| p < l) && f.line(l).is_some() {
                            rows.push(Row::Context(i, l));
                            last_line = Some(l);
                        }
                    }
                }
            }
        }
        rows
    }

    /// `<previous results>`: every file that had a match (the binary ones too), in order.
    pub fn source_files(&self) -> Vec<PathBuf> {
        (self.files.iter())
            .filter(|f| !f.entries().is_empty() || f.body == Body::Binary)
            .map(|f| f.path.clone())
            .collect()
    }

    pub fn line_scope(&self) -> Result<Arc<Vec<MatchingLines>>, String> {
        if !self.done {
            return Err(
                "Wait for the source search to finish, or cancel it, before choosing Previous Matching Lines".into()
            );
        }
        if self.matching_lines.is_empty() {
            return Err("Previous Matching Lines: no source hit lines; refresh the source search first".into());
        }
        Ok(Arc::new(self.matching_lines.clone()))
    }

    /// The report Apply writes: a heading, then each kind of outcome with its files.
    pub fn report(&self, cancelled: bool) -> String {
        let mut out = format!(
            "Replace in Files: \"{}\" with \"{}\"\n",
            escape(&self.query),
            escape(self.replacement.as_deref().unwrap_or(""))
        );
        if cancelled {
            out += "Cancelled: files after those changed were not touched.\n";
        }
        type Is = fn(&Status) -> bool;
        let kinds: [(&str, Is); 8] = [
            ("Changed on disk", |s| *s == Status::Changed),
            ("Changed in open buffer, not saved", |s| *s == Status::ChangedInBuffer),
            ("Unchanged", |s| *s == Status::Unchanged),
            ("Excluded", |s| *s == Status::Excluded),
            ("Stale (refresh and apply again)", |s| matches!(s, Status::Stale(_))),
            ("Skipped", |s| matches!(s, Status::Skipped(_))),
            ("Failed", |s| matches!(s, Status::Failed(_))),
            ("Not reached", |s| *s == Status::NotReached),
        ];
        for (heading, is) in kinds {
            let rows: Vec<String> = (self.files.iter())
                .filter_map(|f| {
                    let s = f.status.as_ref().filter(|s| is(s))?;
                    let why = match s {
                        Status::Stale(w) | Status::Skipped(w) | Status::Failed(w) => {
                            format!(": {w}")
                        }
                        _ => String::new(),
                    };
                    let n = f.selected().count();
                    let what = match s {
                        Status::Changed | Status::ChangedInBuffer => {
                            format!(" ({n} replacement{})", if n == 1 { "" } else { "s" })
                        }
                        _ => String::new(),
                    };
                    Some(format!("    {}{what}{why}", search::tilde(&f.shown)))
                })
                .collect();
            if !rows.is_empty() {
                out += &format!("\n{heading} ({}):\n", rows.len());
                out += &rows.join("\n");
                out.push('\n');
            }
        }
        out += "\nUndo: each changed open buffer has one undo step. Files written on disk have \
                no undo in Orlo; keep them under version control or back them up.\n";
        out
    }
}

/// A query or replacement on one line, its line breaks and tabs written as escapes.
pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(line: usize) -> Hit {
        Hit {
            chars: line * 10..line * 10 + 3,
            bytes: line * 10..line * 10 + 3,
            start: (line, 0),
            end: (line, 3),
            replacement: Some("new".into()),
        }
    }

    fn file(name: &str, lines: &[usize]) -> FileResult {
        FileResult {
            path: PathBuf::from(format!("/p/{name}")),
            shown: PathBuf::from(format!("/p/{name}")),
            source: Source::Buffer { id: 1, version: 1 },
            body: Body::Matches((lines.iter()).map(|&l| Entry { hit: hit(l), included: true }).collect()),
            lines: (0..20).map(|l| (l, format!("line {l} foo"))).collect(),
            included: true,
            collapsed: false,
            status: None,
        }
    }

    fn set() -> ResultSet {
        let mut r = ResultSet::new("foo", Some("new"), FindOpts::default(), SearchFlags::default(), "");
        r.files = vec![file("a.rs", &[1, 2, 10]), file("b.rs", &[5])];
        r
    }

    #[test]
    fn counts_follow_file_and_match_inclusion() {
        let mut r = set();
        assert_eq!(r.summary(), "4 of 4 matches in 2 of 2 files selected (searching…)");
        r.toggle_match(0, 1);
        r.toggle_file(1);
        assert_eq!(r.counts(), Counts { matches: 4, files: 2, selected: 2, selected_files: 1 });
        // including a match of an excluded file includes that match alone
        r.toggle_match(1, 0);
        assert_eq!(r.counts().selected, 3);
        r.toggle_file(0);
        assert_eq!(r.counts().selected, 1, "a file with included matches goes out whole");
        r.toggle_file(0);
        assert_eq!(r.counts().selected, 4, "and comes back with all of them");
        r.set_all(false);
        assert_eq!(r.counts().selected, 0);
    }

    #[test]
    fn filtering_a_long_line_with_many_matches_lowercases_it_once() {
        let mut f = file("a.rs", &[0; 2_000]);
        f.lines[0].1 = "a".repeat(200_000) + "Needle";
        let mut r = set();
        r.files = vec![f];
        let t = std::time::Instant::now();
        assert!(r.rows("zzz").is_empty());
        assert_eq!(r.rows("NEEDLE"), r.rows(""), "every match is on the line it hits");
        assert!(t.elapsed() < std::time::Duration::from_millis(500), "{:?}", t.elapsed());
    }

    #[test]
    fn filtering_hides_rows_and_never_changes_inclusion() {
        let mut r = set();
        r.flags.context = false;
        r.files[0].lines[10].1 = "line 10 special".into();
        let before = r.counts();
        let rows = r.rows("SPECIAL");
        assert_eq!(rows, vec![Row::File(0), Row::Match(0, 2)]);
        assert_eq!(r.counts(), before, "filtering includes or excludes nothing");
        assert_eq!(r.rows("b.rs"), vec![Row::File(1), Row::Match(1, 0)]);
        r.files[0].collapsed = true;
        assert_eq!(r.rows("").len(), 3, "a collapsed file shows its row only");
    }

    #[test]
    fn context_rows_surround_matches_once() {
        let mut r = set();
        r.files.truncate(1);
        let rows = r.rows("");
        use Row::*;
        assert_eq!(
            rows,
            vec![
                File(0),
                Context(0, 0),
                Match(0, 0),
                Match(0, 1),
                Context(0, 3),
                Context(0, 4),
                Context(0, 8),
                Context(0, 9),
                Match(0, 2),
                Context(0, 11),
                Context(0, 12),
            ]
        );
        r.flags.context = false;
        assert_eq!(r.rows("").len(), 4, "Show Context changes presentation only");
        assert_eq!(r.counts().matches, 3);
    }

    #[test]
    fn a_multi_line_first_match_shows_none_of_its_lines_as_context() {
        let mut r = set();
        let mut f = file("a.rs", &[0, 3]);
        let Body::Matches(entries) = &mut f.body else { unreachable!() };
        entries[0].hit.end = (2, 3);
        r.files = vec![f];
        use Row::*;
        assert_eq!(r.rows(""), vec![File(0), Match(0, 0), Match(0, 1), Context(0, 4), Context(0, 5)]);
    }

    #[test]
    fn previous_results_are_the_files_with_matches() {
        let mut r = set();
        let mut empty = file("c.rs", &[]);
        empty.body = Body::Matches(Vec::new());
        r.files.push(empty);
        let mut bin = file("d.bin", &[]);
        bin.body = Body::Binary;
        r.files.push(bin);
        let names: Vec<_> = r.source_files().into_iter().map(|p| p.display().to_string()).collect();
        assert_eq!(names, ["/p/a.rs", "/p/b.rs", "/p/d.bin"]);
    }

    #[test]
    fn the_report_lists_every_outcome() {
        let mut r = set();
        r.files[0].status = Some(Status::ChangedInBuffer);
        r.files[1].status = Some(Status::Stale("edited since the preview".into()));
        let text = r.report(false);
        assert!(text.contains("Changed in open buffer, not saved (1):\n    /p/a.rs (3 replacements)"), "{text}");
        assert!(text.contains("Stale (refresh and apply again) (1):\n    /p/b.rs: edited since the preview"), "{text}");
        assert!(text.contains("no undo in Orlo"));
        assert_eq!(escape("a\nb\\"), "a\\nb\\\\");
    }
}

#[cfg(test)]
mod previous_line_tests {
    use super::*;
    use crate::search::pattern::Matcher;

    #[test]
    fn previous_matching_lines_merge_physical_lines_and_preserve_gaps() {
        let text = "seed é\nseed 中\ngap\nseed 🦀\n";
        let matcher = Matcher::new("seed", &FindOpts::default()).unwrap();
        let search::Found::Lines { matches, .. } =
            search::search_text(text, &matcher, 2, None, &|| false).unwrap().unwrap()
        else {
            panic!()
        };
        assert_eq!(matching_lines(text, &matches), [0..14, 18..25]);
        let matcher = Matcher::new("é\nseed 中\n", &FindOpts::default()).unwrap();
        let search::Found::Lines { matches, .. } =
            search::search_text(text, &matcher, 0, None, &|| false).unwrap().unwrap()
        else {
            panic!()
        };
        assert_eq!(
            matching_lines(text, &matches),
            std::iter::once(0..14).collect::<Vec<_>>(),
            "ending after LF does not include the next line"
        );
    }

    #[test]
    fn previous_matching_lines_require_completed_nonempty_source_results() {
        let mut set = ResultSet::new("seed", None, FindOpts::default(), SearchFlags::default(), "");
        assert!(set.line_scope().unwrap_err().contains("finish"));
        set.done = true;
        assert!(set.line_scope().unwrap_err().contains("no source hit lines"));
    }
}
