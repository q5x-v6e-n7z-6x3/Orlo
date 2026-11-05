//! Find in Files: the where field, a .gitignore-aware walk, literal/regex search per line and
//! Sublime's Find Results text. Pure, so it runs on a background thread.

use crate::buffer::{Document, FindOpts};
use crate::search::pattern::{group_digits, Deadline, Matcher, Replacer, SearchError};

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Submitted queries (or replacements, or Where values) in this window only, newest first, at
/// most `HISTORY` of them, each once; search text is never written to disk.
#[derive(Default)]
pub struct QueryHistory {
    entries: Vec<String>,
    at: Option<usize>,
    draft: String,
}

impl QueryHistory {
    /// Newest first.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.at = None;
    }

    pub fn remember(&mut self, query: &str) {
        if query.is_empty() {
            return;
        }
        self.entries.retain(|q| q != query);
        self.entries.insert(0, query.to_owned());
        self.entries.truncate(HISTORY);
        self.at = None;
    }

    pub fn step(&mut self, current: &str, older: bool) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        if self.at.is_some_and(|i| self.entries[i] != current) {
            self.at = None;
        }
        let at = match self.at {
            None if older => {
                self.draft = current.to_owned();
                0
            }
            Some(i) if older => (i + 1).min(self.entries.len() - 1),
            Some(0) => {
                self.at = None;
                return Some(self.draft.clone());
            }
            Some(i) => i - 1,
            None => return None,
        };
        self.at = Some(at);
        Some(self.entries[at].clone())
    }
}

/// How many entries each search history keeps.
pub const HISTORY: usize = 32;

/// Sublime's `binary_file_patterns` and `folder_exclude_patterns`: skipped and not counted.
const SKIPPED: [&str; 19] = [
    "*.jpg",
    "*.jpeg",
    "*.png",
    "*.gif",
    "*.ttf",
    "*.tga",
    "*.dds",
    "*.ico",
    "*.eot",
    "*.pdf",
    "*.swf",
    "*.jar",
    "*.zip",
    ".svn/",
    ".git/",
    ".hg/",
    "CVS/",
    ".Trash/",
    ".Trash-*/",
];

/// Larger files are listed as "File too large, skipping".
pub use orlo_project::io::{read_bytes, MAX_SIZE};

#[cfg(test)]
use orlo_project::io::BEFORE_READ;

/// Results lines longer than this many chars are cut around their first match.
const MAX_LINE: usize = 1000;

/// The Where field, parsed.
#[derive(Debug, PartialEq, Default)]
pub struct Where {
    pub paths: Vec<PathBuf>,
    pub open_folders: bool,
    pub open_files: bool,
    /// `<previous results>`: the files of the window's last Find in Files results.
    pub previous: bool,
    pub previous_lines: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// Where's tokens other than paths and globs.
pub const OPEN_FOLDERS: &str = "<open folders>";
pub const OPEN_FILES: &str = "<open files>";
pub const PREVIOUS_RESULTS: &str = "<previous results>";
pub const PREVIOUS_MATCHING_LINES: &str = "<previous matching lines>";

/// Comma-separated: absolute or `~` paths, `<open folders>`, `<open files>`,
/// `<previous results>`, `-` exclude globs, and include globs; an empty field means open
/// folders and open files.
pub fn parse_where(text: &str) -> Where {
    let mut w = Where::default();
    for t in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match t {
            OPEN_FOLDERS => w.open_folders = true,
            OPEN_FILES => w.open_files = true,
            PREVIOUS_RESULTS => w.previous = true,
            PREVIOUS_MATCHING_LINES => w.previous_lines = true,
            _ if t.starts_with('-') => w.exclude.extend(Some(t[1..].trim().to_string()).filter(|g| !g.is_empty())),
            _ if t.starts_with('/') || t.starts_with('~') => {
                let p = t.trim_end_matches('/');
                w.paths.push(untilde(if p.is_empty() { "/" } else { p }));
            }
            _ => w.include.push(t.to_string()),
        }
    }
    if w == Where::default() {
        (w.open_folders, w.open_files) = (true, true);
    }
    w
}

/// `field` (a Where value) with `token` added after a comma, unless it is there already.
pub fn append_token(field: &str, token: &str) -> String {
    let mut have: Vec<&str> = (field.split(',').map(str::trim)).filter(|t| !t.is_empty()).collect();
    if !have.contains(&token) {
        have.push(token);
    }
    have.join(", ")
}

/// What a search walks: folders with their globs, and single files.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub roots: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// `.gitignore` and git's excludes apply to the folders walked (Use gitignore).
    pub gitignore: bool,
    /// The globs filter `files` too, as they do the folders' files (Find in Files); a batch of
    /// files chosen one by one takes them all (pipelines).
    pub filter_files: bool,
    /// The project roots' exclude patterns: (root, folder patterns, file patterns),
    /// hiding what they match from a walk of that root or of a folder inside it.
    pub root_excludes: Vec<(PathBuf, Vec<String>, Vec<String>)>,
}

impl Scope {
    /// Where's include and exclude globs that do not parse (`[a`, say): left out of the
    /// search, and named in Find Results so a typo is not a silent no-op.
    pub fn bad_globs(&self) -> Vec<String> {
        (self.include.iter().chain(&self.exclude))
            .filter(|g| OverrideBuilder::new("/").add(g).is_err())
            .cloned()
            .collect()
    }
}

/// `w` for a window with the roots `folders` open, the buffers `open` (their paths) and the
/// files of its last results `previous`. Globs apply to every file however it was named.
pub fn resolve(w: &Where, folders: &[PathBuf], open: &[PathBuf], previous: &[PathBuf]) -> Scope {
    let mut roots = w.paths.clone();
    // globs alone apply inside the open folders
    let globs_only = roots.is_empty()
        && !w.open_files
        && !w.previous
        && !w.previous_lines
        && !(w.include.is_empty() && w.exclude.is_empty());
    if w.open_folders || globs_only {
        roots.extend(folders.iter().cloned());
    }
    let mut files = Vec::new();
    if w.open_files {
        files.extend_from_slice(open);
    }
    if w.previous || w.previous_lines {
        files.extend_from_slice(previous);
    }
    Scope {
        roots,
        files,
        include: w.include.clone(),
        exclude: w.exclude.clone(),
        gitignore: true,
        filter_files: true,
        root_excludes: Vec::new(),
    }
}

/// Why `scope` cannot be searched: a glob that does not parse (which would otherwise widen the
/// search), or nothing to search at all.
pub fn scope_problem(w: &Where, scope: &Scope, previous: bool) -> Option<String> {
    if w.previous_lines && (w.previous || w.open_files || w.open_folders || !w.paths.is_empty()) {
        return Some("Previous Matching Lines must be used without other source paths or scope tokens".into());
    }
    let bad = scope.bad_globs();
    if !bad.is_empty() {
        return Some(format!("Not a valid pattern in Where: {}", bad.join(", ")));
    }
    if w.previous && !previous {
        return Some(format!("{PREVIOUS_RESULTS}: there are no results in this window yet"));
    }
    if w.previous_lines && !previous {
        return Some("Previous Matching Lines: there are no results in this window yet".into());
    }
    (scope.roots.is_empty() && scope.files.is_empty())
        .then(|| "Nothing to search: open a folder or files, or name some in Where".to_string())
}

/// Whether a file named on its own (an open file, a path in Where, a previous result) passes
/// the globs: no exclude matches it or a folder above it, and with includes, one of them does.
/// Globs with a `/` are read from the first root holding the file, else from its folder.
fn passes(scope: &Scope, roots: &[PathBuf], path: &Path) -> bool {
    if !scope.filter_files || (scope.include.is_empty() && scope.exclude.is_empty()) {
        return true;
    }
    let base =
        (roots.iter().find(|r| path.starts_with(r))).map(PathBuf::as_path).or(path.parent()).unwrap_or(Path::new("/"));
    let build = |globs: &[String], negate: bool| {
        let mut b = OverrideBuilder::new(base);
        for g in globs {
            let _ = b.add(&if negate { format!("!{g}") } else { g.clone() });
        }
        b.build().ok()
    };
    let chain = || {
        std::iter::once((path, false))
            .chain((path.ancestors().skip(1)).take_while(|d| *d != base && d.starts_with(base)).map(|d| (d, true)))
    };
    if let Some(ex) = build(&scope.exclude, true) {
        if chain().any(|(p, dir)| ex.matched(p, dir).is_ignore()) {
            return false;
        }
    }
    match build(&scope.include, false) {
        Some(inc) if !scope.include.is_empty() => chain().any(|(p, dir)| inc.matched(p, dir).is_whitelist()),
        _ => true,
    }
}

/// Every file the scope names, sorted and once each: .gitignore (nested ones too) applies with
/// or without a .git folder, symlinks are not followed, unreadable folders are skipped. Nothing
/// when `cancel` is set; `listed` counts the files found so far.
pub fn collect(scope: &Scope, cancel: &AtomicBool, listed: &AtomicUsize) -> Vec<PathBuf> {
    // each file once, so a cancelled run's header counts files, not finds: the open files
    // first, then whatever the walks add
    let mut out: BTreeSet<PathBuf> = BTreeSet::new();
    let mut add = |p: PathBuf| {
        if out.insert(p) {
            listed.fetch_add(1, Ordering::Relaxed);
        }
    };
    let canonical: Vec<PathBuf> =
        (scope.roots.iter()).map(|r| r.canonicalize().unwrap_or_else(|_| r.clone())).collect();
    (scope.files.iter()).filter(|f| passes(scope, &canonical, f)).cloned().for_each(&mut add);
    // open buffers have canonical paths: walk under the same names, so their text is found
    for root in &canonical {
        for file in walk(scope, root) {
            if cancel.load(Ordering::Relaxed) {
                return Vec::new();
            }
            if let Some(f) = file {
                add(f);
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Vec::new();
    }
    out.into_iter().collect()
}

/// Each entry of `collect`'s walk of `root` (a canonical folder), `Some` for a file it takes in:
/// shared with the project index, which walks the same way.
pub fn walk(scope: &Scope, root: &Path) -> impl Iterator<Item = Option<PathBuf>> {
    // overrides beat .gitignore, so only the excludes go into the walk; a malformed glob is
    // left out rather than failing the search
    let mut skip = OverrideBuilder::new(root);
    for g in scope.exclude.iter().map(String::as_str).chain(SKIPPED) {
        let _ = skip.add(&format!("!{g}"));
    }
    let mut keep = OverrideBuilder::new(root);
    for g in &scope.include {
        let _ = keep.add(g);
    }
    let keep = keep.build().ok();
    // ignore files above the folder apply up to its repo's root, and outside a repo not at
    // all: with require_git off, the ignore crate reads every parent's up to `/`. Only
    // .gitignore, as Sublime: ripgrep's .ignore files apply from every parent.
    // An ignore file that never ends (a link to /dev/zero, a FIFO) would be read forever: a
    // folder holding one is not walked, and the root's or its repository's turns them all off.
    let readable = |dir: &Path| orlo_utils::paths::readable_ignore_file(&dir.join(".gitignore"));
    let repo = root.ancestors().find(|d| d.join(".git").exists());
    let ignores = scope.gitignore
        && match repo {
            Some(repo) => {
                (root.ancestors().take_while(|d| *d != repo).chain([repo])).all(readable)
                    && orlo_utils::paths::readable_ignore_file(&repo.join(".git/info/exclude"))
            }
            None => readable(root),
        };
    let in_repo = ignores && repo.is_some();
    let mut walk = WalkBuilder::new(root);
    walk.hidden(false)
        .git_ignore(ignores)
        .git_exclude(ignores)
        .git_global(ignores)
        .ignore(false)
        .parents(in_repo)
        .require_git(in_repo)
        .follow_links(false);
    if let Ok(skip) = skip.build() {
        walk.overrides(skip);
    }
    // the project's patterns of every root this folder is in
    let hidden: Vec<orlo_project::project::Excludes> = (scope.root_excludes.iter())
        .filter_map(|(r, folders, files)| {
            let r = r.canonicalize().unwrap_or_else(|_| r.clone());
            root.starts_with(&r).then(|| orlo_project::project::Excludes::new(&r, folders, files))
        })
        .collect();
    // a save's temp file a crash left is not the user's
    walk.filter_entry(move |e| {
        let dir = e.file_type().is_some_and(|t| t.is_dir());
        let temp = (e.file_name().to_str()).is_some_and(|n| crate::buffer::temp_pid(n).is_some());
        let endless = ignores && dir && !readable(e.path());
        !temp && !endless && !hidden.iter().any(|x| x.hides(e.path(), dir))
    });
    let root = root.to_path_buf();
    walk.build().map(move |entry| {
        // permission denied, vanished files: skipped
        let e = entry.ok()?;
        // an include naming a folder (`src/`, `src`) takes in the files under it
        let included = keep.as_ref().is_none_or(|k| {
            !k.matched(e.path(), false).is_ignore()
                || (e.path().ancestors().skip(1)).take_while(|d| *d != root).any(|d| k.matched(d, true).is_whitelist())
        });
        // a linked file is searched (a linked folder is not walked: it could loop)
        let file = e.file_type().is_some_and(|t| t.is_file() || t.is_symlink() && e.path().is_file());
        (included && file).then(|| e.into_path())
    })
}

/// Each root's canonical path with the path as typed, for `shown`.
pub fn typed_roots(scope: &Scope) -> Vec<(PathBuf, PathBuf)> {
    (scope.roots.iter()).filter_map(|r| Some((r.canonicalize().ok()?, r.clone()))).collect()
}

/// A walked file named under its root as typed: Where's `/tmp/x` lists `/tmp/x/a.rs`, as
/// Sublime does, though the walk and the open buffers use `/private/tmp/x`.
pub fn shown(p: &Path, roots: &[(PathBuf, PathBuf)]) -> PathBuf {
    (roots.iter())
        .find_map(|(canonical, typed)| {
            let rest = p.strip_prefix(canonical).ok()?;
            // a file root: joining "" would add a slash, and `a.rs/` does not open
            Some(match rest.as_os_str().is_empty() {
                true => typed.clone(),
                false => typed.join(rest),
            })
        })
        .unwrap_or_else(|| p.to_path_buf())
}

/// Context lines each side of a match in Find Results, as Sublime shows them (Show Context).
pub const CONTEXT: usize = 2;

/// A match over more lines than this shows its first ones in Find Results; its location and
/// its replacement still cover all of it.
pub const MATCH_LINES_SHOWN: usize = 10;

/// One match in a file, in its text as the editor holds it (`\n` line breaks, no byte order
/// mark): char and byte offsets, its first position and where it ends as (line, char column),
/// both 0-based, and for Replace in Files the text that would replace it (`None` once the
/// pattern no longer matches exactly there).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub chars: Range<usize>,
    pub bytes: Range<usize>,
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub replacement: Option<String>,
}

impl Hit {
    /// The lines it touches: a match ending just after a line break does not touch the next.
    pub fn lines(&self) -> std::ops::RangeInclusive<usize> {
        let last = match self.end.1 == 0 && self.end.0 > self.start.0 {
            true => self.end.0 - 1,
            false => self.end.0,
        };
        self.start.0..=last
    }
}

/// One file's result.
#[derive(Debug, PartialEq)]
pub enum Found {
    /// The lines shown, (line number, text) of each match and its context; each shown match
    /// line's char ranges; and the matches themselves.
    Lines {
        lines: Vec<(usize, String)>,
        hits: Vec<(usize, Vec<Range<usize>>)>,
        matches: Vec<Hit>,
    },
    /// It has a NUL byte and matches.
    Binary,
    TooLarge,
    /// The search gave up on it (a runaway pattern, its time).
    Failed(String),
}

impl Found {
    /// What the footer counts: each match, one for a binary file.
    pub fn matches(&self) -> usize {
        match self {
            Found::Lines { matches, .. } => matches.len(),
            Found::Binary => 1,
            Found::TooLarge | Found::Failed(_) => 0,
        }
    }
}

/// A file read for searching, as Orlo would open it.
pub struct FileText {
    pub text: String,
    /// It has a NUL byte: listed as `<binary>`, never replaced.
    pub binary: bool,
    /// Malformed text, decoded lossily: disk search must report it, never match or replace it.
    pub lossy: bool,
    pub line_ending: &'static str,
    pub endings: crate::buffer::LineEndings,
    pub bom: bool,
    pub encoding: orlo_project::encoding::Encoding,
    /// The bytes as read: Replace in Files checks the file still has them before writing.
    pub hash: u64,
}

impl FileText {
    /// Disk search must not publish matches or source coordinates from substituted text.
    pub fn search(
        &self,
        matcher: &Matcher,
        context: usize,
        replacer: Option<&Replacer>,
        stop: &dyn Fn() -> bool,
    ) -> Option<Found> {
        if self.lossy {
            return (!stop()).then(|| Found::Failed("malformed text; reopen with the correct encoding".into()));
        }
        search_source(&self.text, self.binary, matcher, context, replacer, stop)
    }
}

pub fn hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// `path` read and decoded for searching; `Err(None)` when it is too large, `Err(Some(why))`
/// when it cannot be read.
pub fn read_file(path: &Path) -> Result<FileText, Option<String>> {
    read_file_with_hint(path, None)
}

/// A known disk codec applies only to BOM-less bytes. Unopened files are never guessed.
pub fn read_file_with_hint(
    path: &Path,
    hint: Option<orlo_project::encoding::Encoding>,
) -> Result<FileText, Option<String>> {
    let bytes = read_bytes(path)?;
    let hash = hash(&bytes);
    let encoding = orlo_project::encoding::bom_encoding(&bytes).or(hint);
    let d = orlo_project::encoding::decode_owned(bytes, encoding);
    let binary = d.text.contains('\0');
    let (rope, line_ending, endings) = crate::buffer::decoded_file(&d.text);
    Ok(FileText {
        text: rope.to_string(),
        binary,
        lossy: d.lossy,
        line_ending,
        endings,
        bom: d.bom,
        encoding: d.encoding,
        hash,
    })
}

/// Byte offsets met in order (each at or after the last) as (line, char column).
struct Locator<'a> {
    text: &'a str,
    line: usize,
    at: usize,
    col: usize,
}

impl Locator<'_> {
    fn at(&mut self, b: usize) -> (usize, usize) {
        for c in self.text[self.at..b].chars() {
            if c == '\n' {
                (self.line, self.col) = (self.line + 1, 0);
            } else {
                self.col += 1;
            }
        }
        self.at = b;
        (self.line, self.col)
    }
}

/// `text` searched as a whole, so a match may run over lines; `context` lines each side of the
/// matching lines are kept for Find Results, and `replacer` gives each match its replacement.
pub fn search_text(
    text: &str,
    m: &Matcher,
    context: usize,
    replacer: Option<&Replacer>,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Found>, SearchError> {
    let found = m.find(text, stop)?;
    Ok(found_text(text, found, context, replacer))
}

fn found_text(
    text: &str,
    found: Vec<(Range<usize>, Range<usize>)>,
    context: usize,
    replacer: Option<&Replacer>,
) -> Option<Found> {
    if found.is_empty() {
        return None;
    }
    let mut loc = Locator { text, line: 0, at: 0, col: 0 };
    let matches: Vec<Hit> = (found.into_iter())
        .map(|(bytes, chars)| {
            let start = loc.at(bytes.start);
            let end = loc.at(bytes.end);
            let replacement = replacer.and_then(|r| r.replacement(text, bytes.clone()));
            Hit { chars, bytes, start, end, replacement }
        })
        .collect();
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line = |l: usize| {
        let end = starts.get(l + 1).map_or(text.len(), |e| e - 1);
        &text[starts[l]..end]
    };
    let mut per_line: Vec<(usize, Vec<Range<usize>>)> = Vec::new();
    for h in &matches {
        for l in h.lines().take(MATCH_LINES_SHOWN) {
            let from = if l == h.start.0 { h.start.1 } else { 0 };
            let to = match l == h.end.0 {
                true => h.end.1,
                false => line(l).chars().count(),
            };
            match per_line.last_mut() {
                Some((k, r)) if *k == l => r.push(from..to.max(from)),
                _ => per_line.push((l, std::iter::once(from..to.max(from)).collect())),
            }
        }
    }
    let mut lines: Vec<(usize, String)> = Vec::new();
    for &(l, _) in &per_line {
        let last = (l + context).min(starts.len() - 1);
        for k in l.saturating_sub(context)..=last {
            if lines.last().is_none_or(|(p, _)| *p < k) {
                lines.push((k, line(k).to_string()));
            }
        }
    }
    Some(Found::Lines { lines, hits: per_line, matches })
}

/// `path` searched for `query`; `text` is an open buffer's current contents, searched instead
/// of the disk (Use Buffer). `None` without a match or when it cannot be read.
#[cfg(any(test, feature = "test-support"))]
pub fn search_file(path: &Path, text: Option<&str>, query: &str, o: &FindOpts) -> Option<Found> {
    let m = Matcher::new(query, o).ok()?;
    search_path(path, text, &m, CONTEXT, None, &|| false)
}

/// `search_file` with the query compiled once for the run.
#[cfg(any(test, feature = "test-support"))]
pub fn search_path(
    path: &Path,
    text: Option<&str>,
    m: &Matcher,
    context: usize,
    replacer: Option<&Replacer>,
    stop: &dyn Fn() -> bool,
) -> Option<Found> {
    let (text, binary) = match text {
        Some(t) => (t, false),
        None => match read_file(path) {
            Ok(f) => {
                return f.search(m, context, replacer, stop);
            }
            Err(None) => return Some(Found::TooLarge),
            Err(Some(_)) => return None,
        },
    };
    search_source(text, binary, m, context, replacer, stop)
}

/// One text's result within its time: `None` with no match or when stopped by `stop`.
pub fn search_source(
    text: &str,
    binary: bool,
    m: &Matcher,
    context: usize,
    replacer: Option<&Replacer>,
    stop: &dyn Fn() -> bool,
) -> Option<Found> {
    let d = Deadline::new(crate::search::pattern::BACKGROUND_SECONDS);
    match search_text(text, m, context, replacer, &|| stop() || d.passed()) {
        Ok(Some(_)) if binary => Some(Found::Binary),
        Ok(found) => found,
        Err(SearchError::Cancelled) if !d.passed() => None,
        Err(e) => Some(Found::Failed(d.explain(e).message())),
    }
}

/// Previous hit-line scope; bounds constrain complete matches without clipping regex context.
pub fn search_source_scoped(
    text: &str,
    m: &Matcher,
    ranges: &[Range<usize>],
    context: usize,
    replacer: Option<&Replacer>,
    stop: &dyn Fn() -> bool,
) -> Option<Found> {
    let d = Deadline::new(crate::search::pattern::BACKGROUND_SECONDS);
    match m.find_scoped(text, ranges, &|| stop() || d.passed()) {
        Ok(found) => found_text(text, found, context, replacer),
        Err(SearchError::Cancelled) if !d.passed() => None,
        Err(e) => Some(Found::Failed(d.explain(e).message())),
    }
}

/// `line` cut to `MAX_LINE` chars around the first of `ranges` (a context line: from its start),
/// with "…" where it was cut; the ranges moved along, those cut off dropped.
pub fn cut(line: &str, ranges: &[Range<usize>]) -> (String, Vec<Range<usize>>) {
    let n = line.chars().count();
    if n <= MAX_LINE {
        return (line.to_string(), ranges.to_vec());
    }
    let start = ranges.first().map_or(0, |r| {
        let room = MAX_LINE.saturating_sub(r.len());
        r.start.saturating_sub(room / 2).min(n - MAX_LINE)
    });
    let end = (start + MAX_LINE).max(ranges.first().map_or(0, |r| r.end));
    let dots = usize::from(start > 0);
    let mut out = "…".repeat(dots);
    out.extend(line.chars().skip(start).take(end - start));
    if end < n {
        out.push('…');
    }
    let kept = ranges
        .iter()
        .filter(|r| r.start >= start && r.end <= end)
        .map(|r| r.start - start + dots..r.end - start + dots)
        .collect();
    (out, kept)
}

/// `Searching 5 files for "foo" (case sensitive, whole word)` and a blank line.
pub fn header(files: usize, query: &str, o: &FindOpts) -> String {
    let flags: Vec<&str> = [(o.regex, "regex"), (o.case_sensitive, "case sensitive"), (o.whole_word, "whole word")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    let flags = match flags.is_empty() {
        true => String::new(),
        false => format!(" ({})", flags.join(", ")),
    };
    let s = if files == 1 { "" } else { "s" };
    // a query of several lines stays on the header's line
    let query = query.replace('\n', "\\n");
    format!("Searching {files} file{s} for \"{query}\"{flags}\n\n")
}

/// One file's block, as Sublime writes it: the path, matching lines with two lines of context
/// each side (dots for a gap), a blank line. The ranges are the matches' chars in the block: a
/// match over several lines is one range from its start to its end on the last line shown, so
/// F4 stops on it once.
pub fn file_block(path: &Path, found: &Found) -> (String, Vec<Range<usize>>) {
    let mut out = format!("{}:\n", tilde(path));
    let mut ranges: Vec<Range<usize>> = Vec::new();
    match found {
        Found::Binary => out.push_str("    <binary>\n"),
        Found::TooLarge => out.push_str("    File too large, skipping\n"),
        Found::Failed(why) => out.push_str(&format!("    {why}\n")),
        Found::Lines { lines, hits, matches } => {
            // lines that begin with the rest of a match from the line above
            let continued: std::collections::HashSet<usize> =
                (matches.iter()).flat_map(|h| h.lines().take(MATCH_LINES_SHOWN).skip(1)).collect();
            let mut at = out.chars().count();
            let mut prev: Option<usize> = None;
            for (l, text) in lines {
                let l = *l;
                // a gap: a dot per digit of the number above it, as Sublime writes it
                if let Some(p) = prev.filter(|p| p + 1 < l) {
                    let gap = format!("{:>5}\n", ".".repeat((p + 1).to_string().len()));
                    at += gap.chars().count();
                    out.push_str(&gap);
                }
                prev = Some(l);
                let line = match hits.binary_search_by_key(&l, |(i, _)| *i) {
                    Ok(h) => {
                        let prefix = format!("{:>5}: ", l + 1);
                        let base = at + prefix.chars().count();
                        let (text, kept) = cut(text, &hits[h].1);
                        let mut kept = kept.into_iter().map(|r| base + r.start..base + r.end);
                        if continued.contains(&l) {
                            if let (Some(first), Some(last)) = (kept.next(), ranges.last_mut()) {
                                last.end = first.end;
                            }
                        }
                        ranges.extend(kept);
                        prefix + &text
                    }
                    Err(_) => format!("{:>5}  {}", l + 1, cut(text, &[]).0),
                };
                at += line.chars().count() + 1;
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out.push('\n');
    (out, ranges)
}

/// `7 matches across 4 files`, `1 match in 1 file`, `0 matches`; a cancelled run says so first.
pub fn footer(matches: usize, files: usize, cancelled: bool) -> String {
    let counts = match (matches, files) {
        (0, _) => "0 matches".to_string(),
        (1, _) => "1 match in 1 file".to_string(),
        (n, 1) => format!("{n} matches in 1 file"),
        (n, m) => format!("{n} matches across {m} files"),
    };
    let cancelled = if cancelled { "Cancelled!\n" } else { "" };
    format!("{cancelled}{counts}\n")
}

/// A file header line (`/path/to/file:` or `~/file:`; the paths are always absolute), so a
/// match line like `100000: else:` is not one.
fn header_path(text: &str) -> Option<PathBuf> {
    let path = text.strip_suffix(':')?;
    path.starts_with(['/', '~']).then(|| untilde(path))
}

/// The file and 0-based line a Find Results line points at: a header means the first line, a
/// match or context line its own.
pub fn result_target(s: &Document, line: usize) -> Option<(PathBuf, usize)> {
    let text = s.line(line);
    if let Some(path) = header_path(&text) {
        return Some((path, 0));
    }
    // "   10: foo();" or "    9  line nine", under its file's header
    let t = text.trim_start_matches(' ');
    let digits = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let rest = &t[digits..];
    // line numbers past 99999 fill the five columns, leaving no leading space
    if digits == 0 || !(rest.starts_with(':') || rest.starts_with("  ")) {
        return None;
    }
    let n: usize = t[..digits].parse().ok()?;
    (0..line)
        .rev()
        .map(|l| s.line(l))
        .take_while(|l| !l.is_empty())
        .find_map(|l| header_path(&l))
        .map(|p| (p, n.saturating_sub(1)))
}

/// A double click on Find Results line `line`: the match it opens at (the one clicked, else the
/// first on the line) as a char column in the line's shown text, and that text when it starts
/// with a possible truncation marker, for `file_column`. `None` on a header, context or gap line.
pub fn result_match(
    s: &Document,
    outlines: &[Range<usize>],
    line: usize,
    click: usize,
) -> Option<(usize, Option<String>)> {
    let t = s.line(line);
    // "   12: text": the text starts after the number, its colon and a space (ASCII, so bytes
    // are chars)
    let lead = t.find(|c: char| c != ' ')?;
    let digits = t[lead..].find(|c: char| !c.is_ascii_digit())?;
    if digits == 0 || !t[lead + digits..].starts_with(": ") {
        return None;
    }
    let (at, end) = (s.line_start(line) + lead + digits + 2, s.line_end(line));
    // a match over lines starts on this one and ends further down
    let mine: Vec<&Range<usize>> = (outlines.iter()).filter(|r| r.start >= at && r.start <= end).collect();
    let m = (mine.iter().find(|r| r.contains(&click)))
        .or_else(|| mine.iter().find(|r| r.start >= click))
        .or(mine.first())?;
    let shown = s.rope.slice(at..end).to_string();
    // A leading ellipsis can be literal source text. Keep the displayed column and
    // text intact until file_column can distinguish it from a truncation marker.
    let cut = shown.starts_with('…').then_some(shown);
    Some((m.start - at, cut))
}

/// The char column in `file_line` of a match `col` chars into a results line's shown text;
/// `cut` is the displayed text when it starts with an ellipsis, found again to distinguish
/// literal source text from truncation markers and learn where it starts.
pub fn file_column(file_line: &str, col: usize, cut: Option<&str>) -> usize {
    let Some(shown) = cut else {
        return col;
    };
    // Prefer the exact displayed text: short source lines may start with a literal
    // ellipsis, and a long source line may only have been clipped at its end.
    if file_line.starts_with(shown) || shown.strip_suffix('…').is_some_and(|s| file_line.starts_with(s)) {
        return col;
    }
    // Remove only the generated leading marker, preserving literal ellipses in
    // the source. A trailing ellipsis may be literal or generated: choose the earliest
    // candidate, rather than letting a later literal ellipsis steal the first match.
    let core = shown.strip_prefix('…').unwrap_or(shown);
    let start = file_line.find(core).into_iter().chain(core.strip_suffix('…').and_then(|c| file_line.find(c))).min();
    start.map_or(col, |b| file_line[..b].chars().count() + col.saturating_sub(1))
}

/// The status bar while a search runs (an Orlo addition: Sublime shows no progress); `total`
/// is `None` while the walk is still listing files.
pub fn progress(done: usize, total: Option<usize>) -> String {
    let Some(total) = total else {
        return "Find in Files: listing files…".into();
    };
    let s = if total == 1 { "" } else { "s" };
    format!("Find in Files: {} of {} file{s}", group_digits(done), group_digits(total))
}

/// Paths in results and menus read like Sublime's: the home folder as `~`.
pub fn tilde(p: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(h) if p == h => "~".into(),
        Some(h) if p.starts_with(&h) => {
            format!("~/{}", p.strip_prefix(&h).unwrap_or(p).to_string_lossy())
        }
        _ => p.to_string_lossy().into_owned(),
    }
}

pub use orlo_utils::paths::untilde;

/// An empty folder `name` in the temp folder.
#[cfg(test)]
fn fresh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every file `scope` names, never cancelled.
#[cfg(test)]
fn all(scope: &Scope) -> Vec<PathBuf> {
    collect(scope, &AtomicBool::new(false), &AtomicUsize::new(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_in_str_finds_what_find_in_finds() {
        let cases = [
            ("aaa foo Foo fooo foo", "foo"),
            ("ééa Éé é", "é"),
            ("x aa aaa aa", "aa"),
            ("日本 本日本", "本"),
            ("", "x"),
        ];
        for (hay, q) in cases {
            for (case_sensitive, whole_word) in [(false, false), (true, false), (false, true), (true, true)] {
                let o = FindOpts { case_sensitive, whole_word, ..FindOpts::default() };
                let chars: Vec<char> = hay.chars().collect();
                let qc: Vec<char> = q.chars().collect();
                assert_eq!(
                    crate::buffer::find_in_str(hay, q, &o),
                    crate::buffer::find_in(&chars, &qc, &o),
                    "{hay:?} {q:?} case {case_sensitive} word {whole_word}"
                );
            }
        }
    }

    #[test]
    fn find_in_files_reads_cr_files_follows_linked_files_and_names_bad_globs() {
        let dir = fresh("orlo-fif-26");
        std::fs::write(dir.join("mac.txt"), "one\rtwo needle\rthree").unwrap();
        let o = FindOpts::default();
        match search_file(&dir.join("mac.txt"), None, "needle", &o) {
            Some(Found::Lines { hits, .. }) => {
                assert_eq!((hits.len(), hits[0].0, hits[0].1.first()), (1, 1, Some(&(4..10))))
            }
            other => panic!("{other:?}"),
        }
        std::fs::write(dir.join("real.txt"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("real.txt"), dir.join("link.txt")).unwrap();
        let scope = Scope {
            roots: vec![dir.clone()],
            files: vec![],
            include: vec!["[a".into()],
            exclude: vec![],
            gitignore: true,
            filter_files: true,
            root_excludes: Vec::new(),
        };
        assert_eq!(scope.bad_globs(), ["[a"]);
        let scope = Scope { include: vec![], ..scope };
        let names: Vec<_> =
            (all(&scope).into_iter()).map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"link.txt".to_string()), "{names:?}");
        let w = parse_where("[a");
        let e = scope_problem(&w, &resolve(&w, std::slice::from_ref(&dir), &[], &[]), false).unwrap();
        assert!(e.contains("Not a valid pattern in Where: [a"), "{e}");
    }
    use crate::buffer::{EditorState, FindOpts};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    /// The folder Sublime's Find in Files was run on: a.rs (matches on lines 3 and 10 of 12),
    /// b.txt, sub/c.rs, a file the .gitignore lists, an image and a file with a NUL byte.
    fn fixture(name: &str) -> PathBuf {
        let dir = fresh(name);
        std::fs::create_dir(dir.join("sub")).unwrap();
        let a = [
            "line one",
            "line two",
            "let foo = 1;",
            "line four",
            "line five",
            "line six",
            "line seven",
            "line eight",
            "line nine",
            "foo();",
            "line eleven",
        ];
        std::fs::write(dir.join("a.rs"), a.join("\n") + "\n").unwrap();
        std::fs::write(dir.join("b.txt"), "Foo\nfoobar\nfoo\n").unwrap();
        std::fs::write(dir.join("sub/c.rs"), "foo\n").unwrap();
        std::fs::write(dir.join("ignored.rs"), "foo\n").unwrap();
        std::fs::write(dir.join("img.png"), "foo").unwrap();
        std::fs::write(dir.join("nul.dat"), b"foo\0bar\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "ignored.rs\n").unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn search_skips_private_copy_staging_directories() {
        let dir = fixture("orlo-fif-copy-stage");
        let before = files(&dir, "<open folders>");
        let stage = dir.join(".orlo-123-456.tmp");
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("partial.txt"), "incomplete").unwrap();
        assert_eq!(files(&dir, "<open folders>"), before);
    }

    fn opts(case_sensitive: bool, whole_word: bool) -> FindOpts {
        FindOpts { case_sensitive, whole_word, ..FindOpts::default() }
    }

    fn files(dir: &Path, where_: &str) -> Vec<PathBuf> {
        all(&resolve(&parse_where(where_), &[dir.to_path_buf()], &[], &[]))
    }

    /// The files `where_` names in `dir`, relative to it.
    fn names(dir: &Path, where_: &str) -> Vec<String> {
        (files(dir, where_).iter()).map(|p| p.strip_prefix(dir).unwrap().to_string_lossy().into_owned()).collect()
    }

    /// `text` searched as the file `/x`, and its Find Results block with the matches' ranges.
    fn x_block(text: &str, q: &str, o: &FindOpts) -> (Found, String, Vec<Range<usize>>) {
        let found = search_file(Path::new("/x"), Some(text), q, o).unwrap();
        let (block, ranges) = file_block(Path::new("/x"), &found);
        (found, block, ranges)
    }

    /// The chars `r` of `text`.
    fn chars_at(text: &str, r: Range<usize>) -> String {
        text.chars().skip(r.start).take(r.len()).collect()
    }

    /// A Find in Files run over `dir`, in one go: the Find Results text and the match count.
    fn run(dir: &Path, query: &str, o: &FindOpts) -> (String, usize) {
        let files = files(dir, "<open folders>");
        let mut text = header(files.len(), query, o);
        let (mut matches, mut hit_files) = (0, 0);
        for f in &files {
            if let Some(found) = search_file(f, None, query, o) {
                matches += found.matches();
                hit_files += usize::from(found.matches() > 0);
                text += &file_block(f, &found).0;
            }
        }
        text += &footer(matches, hit_files, false);
        (text, matches)
    }

    #[test]
    fn results_match_sublime_verbatim() {
        let dir = fixture("orlo-fif-verbatim");
        let p = |f: &str| tilde(&dir.join(f));
        let expected = format!(
            "Searching 5 files for \"foo\"\n\n\
             {}:\n    1  line one\n    2  line two\n    3: let foo = 1;\n    4  line four\n    5  line five\n    .\n    8  line eight\n    9  line nine\n   10: foo();\n   11  line eleven\n   12  \n\n\
             {}:\n    1: Foo\n    2: foobar\n    3: foo\n    4  \n\n\
             {}:\n    <binary>\n\n\
             {}:\n    1: foo\n    2  \n\n\
             7 matches across 4 files\n",
            p("a.rs"),
            p("b.txt"),
            p("nul.dat"),
            p("sub/c.rs")
        );
        assert_eq!(run(&dir, "foo", &FindOpts::default()), (expected, 7));
    }

    #[test]
    fn case_and_whole_word_change_counts_and_header() {
        let dir = fixture("orlo-fif-opts");
        let (text, n) = run(&dir, "foo", &opts(true, false));
        assert_eq!(n, 6);
        assert!(text.starts_with("Searching 5 files for \"foo\" (case sensitive)\n\n"));
        assert_eq!(run(&dir, "foo", &opts(false, true)).1, 6);
        let (text, n) = run(&dir, "foo", &opts(true, true));
        assert_eq!(n, 5);
        assert!(text.starts_with("Searching 5 files for \"foo\" (case sensitive, whole word)"));
        assert!(header(1, "x", &opts(false, true)).starts_with("Searching 1 file for \"x\" (whole word)"));
    }

    #[test]
    fn a_gitignore_that_never_ends_is_not_read() {
        let dir = fresh("orlo-files-endless-ignore").canonicalize().unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join("ok")).unwrap();
        std::fs::write(dir.join("ok/a.rs"), "").unwrap();
        std::fs::write(dir.join("sub/b.rs"), "").unwrap();
        // a folder's: the rest is walked, that folder is not
        std::os::unix::fs::symlink("/dev/zero", dir.join("sub/.gitignore")).unwrap();
        assert_eq!(names(&dir, "<open folders>"), ["ok/a.rs"]);
        // the root's: ignore files are left alone, and every file is walked
        std::os::unix::fs::symlink("/dev/zero", dir.join(".gitignore")).unwrap();
        std::fs::remove_file(dir.join("sub/.gitignore")).unwrap();
        assert_eq!(names(&dir, "<open folders>"), ["ok/a.rs", "sub/b.rs"]);
    }

    #[test]
    fn gitignore_applies_without_a_git_folder() {
        let dir = fixture("orlo-fif-ignore");
        assert_eq!(names(&dir, "<open folders>"), [".gitignore", "a.rs", "b.txt", "nul.dat", "sub/c.rs"]);
    }

    #[test]
    fn where_includes_and_excludes() {
        let dir = fixture("orlo-fif-where");
        let root = dir.display();
        let names = |w: &str| names(&dir, w);
        assert_eq!(names(&format!("{root}, *.rs")), ["a.rs", "sub/c.rs"]);
        assert_eq!(names(&format!("{root}, -*.txt")), [".gitignore", "a.rs", "nul.dat", "sub/c.rs"]);
        assert_eq!(names(&format!("{root}/, -sub/")), [".gitignore", "a.rs", "b.txt", "nul.dat"]);
        assert_eq!(names("*.rs"), ["a.rs", "sub/c.rs"], "globs alone search the open folders");
        assert_eq!(names(&format!("{root}, sub/")), ["sub/c.rs"], "a folder");
        assert_eq!(names(&format!("{root}, sub")), ["sub/c.rs"], "a folder's name");
    }

    #[test]
    fn parse_where_tokens() {
        for (w, open_folders, open_files) in
            [("", true, true), ("<open folders>", true, false), (" <open files> ", false, true)]
        {
            let want = Where { open_folders, open_files, ..Where::default() };
            assert_eq!(parse_where(w), want, "{w:?}");
        }
        assert_eq!(
            parse_where("~/x/, *.rs, -target/"),
            Where {
                paths: vec![untilde("~/x")],
                include: vec!["*.rs".into()],
                exclude: vec!["target/".into()],
                ..Where::default()
            }
        );
    }

    #[test]
    fn open_files_are_searched_with_the_folders() {
        let dir = fixture("orlo-fif-open");
        let open = [PathBuf::from("/elsewhere/x.rs"), dir.join("a.rs")];
        let found = all(&resolve(&parse_where(""), std::slice::from_ref(&dir), &open, &[]));
        assert_eq!(found.len(), 6, "the folder's five plus /elsewhere/x.rs, a.rs once");
        assert!(found.contains(&PathBuf::from("/elsewhere/x.rs")));
    }

    #[test]
    fn footer_wording() {
        assert_eq!(footer(1, 1, false), "1 match in 1 file\n");
        assert_eq!(footer(3, 1, false), "3 matches in 1 file\n");
        assert_eq!(footer(7, 4, false), "7 matches across 4 files\n");
        assert_eq!(footer(0, 0, false), "0 matches\n");
        assert_eq!(footer(0, 0, true), "Cancelled!\n0 matches\n");
    }

    #[test]
    fn context_gap_and_merge() {
        // lines `l1` to `l{n}`, those in `hits` reading `hit`
        let text = |n: usize, hits: &[usize]| -> String {
            (1..=n)
                .map(|i| match hits.contains(&i) {
                    true => "hit\n".to_string(),
                    false => format!("l{i}\n"),
                })
                .collect()
        };
        let hits = |n, at: &[usize]| x_block(&text(n, at), "hit", &opts(true, false));
        // lines 3 and 7: their context meets at line 5
        assert_eq!(
            hits(12, &[3, 7]).1,
            "/x:\n    1  l1\n    2  l2\n    3: hit\n    4  l4\n    5  l5\n    6  l6\n    7: hit\n    8  l8\n    9  l9\n\n"
        );
        assert_eq!(
            x_block(&text(12, &[]), "l1", &opts(true, true)).1,
            "/x:\n    1: l1\n    2  l2\n    3  l3\n\n",
            "whole word: l10, l11, l12 do not match"
        );
        assert!(hits(12, &[3, 10]).1.contains("    5  l5\n    .\n    8  l8\n"));
        // Sublime 4215's gap has a dot per digit of the line above it
        let (_, block, ranges) = hits(120, &[12, 30, 110]);
        assert!(block.contains("   14  l14\n   ..\n   28  l28\n"), "{block}");
        assert!(block.contains("   32  l32\n   ..\n  108  l108\n"), "{block}");
        for r in ranges {
            assert_eq!(chars_at(&block, r), "hit");
        }
    }

    #[test]
    fn block_ranges_outline_the_matches() {
        let (found, text, ranges) = x_block("a foo b foo\n", "foo", &opts(false, false));
        let hits: Vec<String> = ranges.into_iter().map(|r| chars_at(&text, r)).collect();
        assert_eq!(hits, ["foo", "foo"]);
        assert_eq!(found.matches(), 2);
    }

    #[test]
    fn buffer_text_overrides_the_disk() {
        let dir = fixture("orlo-fif-buffer");
        let p = dir.join("b.txt");
        std::fs::write(&p, "bar\n").unwrap();
        assert!(search_file(&p, None, "foo", &FindOpts::default()).is_none());
        assert!(search_file(&p, Some("foo"), "foo", &FindOpts::default()).is_some());
    }

    #[test]
    fn files_over_100_mib_are_not_read() {
        let dir = fixture("orlo-fif-large");
        let p = dir.join("huge.log");
        std::fs::File::create(&p).unwrap().set_len(101 << 20).unwrap();
        let found = search_file(&p, None, "foo", &FindOpts::default()).unwrap();
        assert_eq!(found, Found::TooLarge);
        assert_eq!(found.matches(), 0);
        assert!(file_block(&p, &found).0.ends_with(":\n    File too large, skipping\n\n"));
    }

    #[test]
    fn a_file_that_grows_after_metadata_is_still_size_limited() {
        let dir = fixture("orlo-fif-growing");
        let path = dir.join("growing.txt");
        std::fs::write(&path, "small").unwrap();
        BEFORE_READ.set(Some(|path| {
            std::fs::OpenOptions::new().write(true).open(path).unwrap().set_len(MAX_SIZE + 1).unwrap();
        }));
        assert!(matches!(read_file(&path), Err(None)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_replaced_by_a_pipe_is_refused_without_waiting_for_input() {
        use std::io::Write;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;
        let dir = fixture("orlo-fif-pipe");
        let path = dir.join("input.pipe");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a live NUL-terminated path and the pipe belongs to this test.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let done = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    if let Ok(mut file) =
                        std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(&path)
                    {
                        let _ = file.write_all(b"needle");
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            });
            let result = read_file(&path);
            done.store(true, Ordering::Relaxed);
            writer.join().unwrap();
            result
        });
        assert!(matches!(result, Err(Some(_))), "a pipe is not a searchable regular file");
    }

    #[test]
    fn result_target_reads_headers_matches_and_context() {
        let home = std::env::var("HOME").unwrap();
        let text = "Searching 2 files for \"foo\"\n\n/p/a.rs:\n    9  line nine\n   10: foo();\n    .\n   12  \n\n~/x/b.rs:\n    1: foo\n\n2 matches across 2 files\n";
        let s = EditorState::from_text(text);
        let a = PathBuf::from("/p/a.rs");
        assert_eq!(result_target(&s.doc, 0), None);
        assert_eq!(result_target(&s.doc, 1), None);
        assert_eq!(result_target(&s.doc, 2), Some((a.clone(), 0)));
        assert_eq!(result_target(&s.doc, 3), Some((a.clone(), 8)));
        assert_eq!(result_target(&s.doc, 4), Some((a.clone(), 9)));
        assert_eq!(result_target(&s.doc, 5), None, "a gap");
        assert_eq!(result_target(&s.doc, 6), Some((a, 11)));
        assert_eq!(result_target(&s.doc, 7), None);
        let b = PathBuf::from(home).join("x/b.rs");
        assert_eq!(result_target(&s.doc, 8), Some((b.clone(), 0)));
        assert_eq!(result_target(&s.doc, 9), Some((b, 0)));
        assert_eq!(result_target(&s.doc, 11), None, "the footer");
    }

    #[test]
    fn result_lines_past_99999_open_their_line() {
        let text = "Searching 1 file for \"x\"\n\n/p/log.txt:\n 99999  before\n100000: else:\n100001  after:\n\n1 match in 1 file\n";
        let s = EditorState::from_text(text);
        let log = PathBuf::from("/p/log.txt");
        assert_eq!(result_target(&s.doc, 3), Some((log.clone(), 99998)));
        assert_eq!(result_target(&s.doc, 4), Some((log.clone(), 99999)), "not a file header");
        assert_eq!(result_target(&s.doc, 5), Some((log, 100000)));
    }

    #[test]
    fn cancel_stops_collect() {
        let dir = fixture("orlo-fif-cancel");
        let scope = resolve(&parse_where(""), std::slice::from_ref(&dir), &[dir.join("a.rs")], &[]);
        let listed = AtomicUsize::new(0);
        assert!(collect(&scope, &AtomicBool::new(true), &listed).is_empty());
        assert_eq!(listed.into_inner(), 1, "the open file is listed at once");
        // what a cancelled run's header counts: the files listed so far, each once, open files
        // outside the roots and files under two roots included
        let elsewhere = fixture("orlo-fif-cancel-elsewhere").join("b.txt");
        let w = format!("{0}, {0}/sub, <open files>", dir.display());
        let scope = resolve(&parse_where(&w), std::slice::from_ref(&dir), std::slice::from_ref(&elsewhere), &[]);
        let listed = AtomicUsize::new(0);
        let all = collect(&scope, &AtomicBool::new(false), &listed);
        assert!(all.contains(&elsewhere) && all.contains(&dir.join("sub/c.rs")));
        assert_eq!(listed.into_inner(), all.len());
    }

    #[test]
    fn ignore_files_above_the_folder_apply_only_inside_its_repo() {
        let top = fresh("orlo-fif-parents");
        for d in ["plain/inner", "repo/.git", "repo/inner"] {
            std::fs::create_dir_all(top.join(d)).unwrap();
        }
        let top = top.canonicalize().unwrap();
        std::fs::write(top.join(".gitignore"), "*\n").unwrap();
        std::fs::write(top.join("plain/inner/a.rs"), "foo\n").unwrap();
        assert_eq!(
            files(&top.join("plain/inner"), ""),
            [top.join("plain/inner/a.rs")],
            "outside a repo the parent's * does not apply"
        );
        std::fs::write(top.join("repo/.gitignore"), "skip.rs\n").unwrap();
        std::fs::write(top.join("repo/inner/a.rs"), "foo\n").unwrap();
        std::fs::write(top.join("repo/inner/skip.rs"), "foo\n").unwrap();
        assert_eq!(
            files(&top.join("repo/inner"), ""),
            [top.join("repo/inner/a.rs")],
            "inside one, its root's .gitignore applies and the * above it does not"
        );
    }

    #[test]
    fn open_buffers_are_found_through_a_symlinked_folder() {
        let dir = fixture("orlo-fif-symlinked");
        let link = std::env::temp_dir().join("orlo-fif-symlinked-link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        // open buffers carry canonical paths
        let open = [dir.join("a.rs")];
        let scope = resolve(&parse_where(""), std::slice::from_ref(&link), &open, &[]);
        let found = all(&scope);
        let a: Vec<_> = found.iter().filter(|p| p.ends_with("a.rs")).collect();
        assert_eq!(a, [&dir.join("a.rs")], "once, under the buffer's name");
    }

    #[test]
    fn results_name_files_under_the_folder_as_typed() {
        let dir = fixture("orlo-fif-typed");
        let link = std::env::temp_dir().join("orlo-fif-typed-link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        let scope = resolve(&parse_where(&link.display().to_string()), &[], &[], &[]);
        let roots = typed_roots(&scope);
        let found = all(&scope);
        let a = found.iter().find(|p| p.ends_with("a.rs")).unwrap();
        assert_eq!(a, &dir.join("a.rs"), "walked under the canonical path");
        let listed = shown(a, &roots);
        assert_eq!(listed.as_os_str(), link.join("a.rs").as_os_str(), "listed as typed");
        let open = Path::new("/elsewhere/x.rs");
        assert_eq!(shown(open, &roots), open, "an open file outside the roots");
    }

    #[test]
    fn a_file_root_is_listed_and_opened_as_typed() {
        let dir = fixture("orlo-fif-typed-file");
        let link = std::env::temp_dir().join("orlo-fif-typed-file-link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        let typed = link.join("a.rs");
        let scope = resolve(&parse_where(&typed.display().to_string()), &[], &[], &[]);
        let roots = typed_roots(&scope);
        let found = all(&scope);
        assert_eq!(found, [dir.join("a.rs")]);
        // Path equality ignores a trailing slash; the header and the file open do not
        let shown = shown(&found[0], &roots);
        assert_eq!(shown.as_os_str(), typed.as_os_str(), "no trailing slash");
        let hit = search_file(&found[0], None, "foo", &FindOpts::default()).unwrap();
        let block = file_block(&shown, &hit).0;
        assert!(block.starts_with(&format!("{}:\n", tilde(&typed))), "{block}");
        let s = EditorState::from_text(&block);
        for line in [0, 3] {
            let (path, _) = result_target(&s.doc, line).unwrap();
            assert_eq!(path.as_os_str(), typed.as_os_str());
            assert!(std::fs::read(&path).is_ok(), "it opens");
        }
    }

    #[test]
    fn generated_trailing_ellipsis_does_not_select_a_later_literal_match() {
        let text = format!("{}foo{}{}foo{}…", "x".repeat(2000), "x".repeat(2000), "x".repeat(498), "x".repeat(499),);
        let (_, block, outlines) = x_block(&text, "foo", &opts(true, false));
        let state = EditorState::from_text(&block);
        let (col, cut) = result_match(&state.doc, &outlines, 1, outlines[0].start).unwrap();
        assert_eq!(file_column(&text, col, cut.as_deref()), 2000);
    }

    #[test]
    fn literal_ellipses_in_results_keep_their_source_columns() {
        for (text, query, expected) in [
            ("…foo".to_string(), "…", 0),
            ("……foo…".to_string(), "foo", 2),
            (format!("…foo{}", "x".repeat(2000)), "foo", 1),
            (format!("{}……foo", "x".repeat(2000)), "foo", 2002),
            (format!("{}{}foo", "x".repeat(2000), "…".repeat(1000)), "foo", 3000),
        ] {
            let (_, block, outlines) = x_block(&text, query, &opts(true, false));
            let state = EditorState::from_text(&block);
            let (col, cut) = result_match(&state.doc, &outlines, 1, outlines[0].start).unwrap();
            assert_eq!(file_column(&text, col, cut.as_deref()), expected, "{query}");
        }
    }

    #[test]
    fn a_double_click_opens_at_the_match_column_cut_lines_too() {
        let long = format!("{}foo{}foo", "a".repeat(5000), "b".repeat(20));
        let file = format!("ctx\nx = foo + foo;\n{long}\n");
        let (_, block, outlines) = x_block(&file, "foo", &opts(false, false));
        let s = EditorState::from_text(&block);
        let lines: Vec<&str> = file.lines().collect();
        // the second match on "    2: x = foo + foo;", clicked
        let second = outlines[1].clone();
        let (col, cut) = result_match(&s.doc, &outlines, 2, second.start + 1).unwrap();
        assert_eq!((col, cut.as_deref()), (10, None));
        assert_eq!(file_column(lines[1], col, None), 10);
        // a click elsewhere on the line: its first match
        let (col, _) = result_match(&s.doc, &outlines, 2, s.line_start(2)).unwrap();
        assert_eq!(file_column(lines[1], col, None), 4);
        // the cut line ("…aaafoobbbfoo"): both matches land on their file columns
        for (k, want) in [(2, 5000), (3, 5023)] {
            let (col, cut) = result_match(&s.doc, &outlines, 3, outlines[k].start).unwrap();
            assert!(cut.is_some(), "cut at its start");
            assert_eq!(file_column(lines[2], col, cut.as_deref()), want);
        }
        assert_eq!(result_match(&s.doc, &outlines, 0, 0), None, "the header");
        assert_eq!(result_match(&s.doc, &outlines, 1, s.line_start(1)), None, "context");
    }

    #[test]
    fn find_results_outlines_follow_edits_between_runs() {
        let dir = fixture("orlo-fif-edit-results");
        // as Editor::append: the text, then its outlines in the coordinates it left
        let run_into = |s: &mut EditorState| {
            if !s.is_empty() {
                s.append("\n\n");
            }
            let mut text = header(1, "foo", &FindOpts::default());
            let mut outlines = Vec::new();
            for f in files(&dir, "<open folders>") {
                if let Some(found) = search_file(&f, None, "foo", &FindOpts::default()) {
                    let (block, ranges) = file_block(&f, &found);
                    let at = s.len() + text.chars().count();
                    outlines.extend(ranges.into_iter().map(|r| at + r.start..at + r.end));
                    text += &block;
                }
            }
            s.append(&text);
            s.take_changes();
            s.doc.outlines.extend(outlines);
        };
        let mut s = EditorState::from_text("");
        s.doc.scratch = true;
        run_into(&mut s);
        // delete the first file's block, as a user might
        let first = s.text().find(":\n").unwrap();
        let start = s.text()[..first].rfind('\n').unwrap() + 1;
        let end = start + s.text()[start..].find("\n\n").unwrap() + 2;
        let (start, end) = (s.text()[..start].chars().count(), s.text()[..end].chars().count());
        s.set_selection(start, end);
        s.insert("");
        s.take_changes();
        run_into(&mut s);
        let outlines = s.doc.outlines.clone();
        for r in &outlines {
            let hit = chars_at(&s.text(), r.clone());
            assert_eq!(hit.to_lowercase(), "foo", "outline {r:?}");
        }
        assert_eq!(outlines.len(), 4 + 6, "the first run less a.rs's two, and the second");
    }

    #[test]
    fn long_result_lines_are_cut_around_the_match() {
        let long = format!("{}foo{}", "a".repeat(5000), "b".repeat(5000));
        let text = format!("ctx {}\n{long}\n{}", "c".repeat(2000), "z\n".repeat(500));
        let (found, block, ranges) = x_block(&text, "foo", &opts(false, false));
        let Found::Lines { lines, .. } = &found else { panic!("{found:?}") };
        assert_eq!(lines.len(), 4, "the match and its context, not the whole file");
        let rows: Vec<&str> = block.lines().collect();
        assert_eq!(rows[1], format!("    1  ctx {}…", "c".repeat(996)));
        assert!(rows[2].starts_with("    2: …aaa") && rows[2].ends_with("bbb…"));
        assert_eq!(rows[2].chars().count(), 7 + 1 + 1000 + 1);
        assert_eq!(chars_at(&block, ranges[0].clone()), "foo");
        let s = EditorState::from_text(&block);
        assert_eq!(result_target(&s.doc, 2), Some((PathBuf::from("/x"), 1)));
        assert_eq!(found.matches(), 1);
    }

    #[test]
    fn walk_survives_symlink_loops_and_unreadable_folders() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fixture("orlo-fif-hostile");
        std::os::unix::fs::symlink(&dir, dir.join("sub/loop")).unwrap();
        std::fs::create_dir(dir.join("locked")).unwrap();
        std::fs::write(dir.join("locked/secret.rs"), "foo").unwrap();
        std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let found = files(&dir, "<open folders>");
        std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(found.len(), 5);
    }

    #[test]
    fn progress_reads_like_the_status_bar() {
        let p = |done, total| progress(done, Some(total));
        assert_eq!(p(1234, 14451), "Find in Files: 1,234 of 14,451 files");
        assert_eq!(p(0, 1), "Find in Files: 0 of 1 file");
        assert_eq!(p(999, 1_000_000), "Find in Files: 999 of 1,000,000 files");
        assert_eq!(progress(0, None), "Find in Files: listing files…", "the walk is still going");
    }

    #[test]
    fn tilde_round_trips() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        assert_eq!(tilde(&home.join("a/b")), "~/a/b");
        assert_eq!(untilde("~/a/b"), home.join("a/b"));
        assert_eq!(tilde(Path::new("/tmp/x")), "/tmp/x");
        assert_eq!(untilde("/tmp/x"), PathBuf::from("/tmp/x"));
    }
}

#[cfg(test)]
mod history_features {
    use super::*;

    #[test]
    fn history_recalls_queries_and_returns_to_the_current_draft() {
        let mut h = QueryHistory::default();
        h.remember("first");
        h.remember("second");
        assert_eq!(h.step("draft", true).as_deref(), Some("second"));
        assert_eq!(h.step("second", true).as_deref(), Some("first"));
        assert_eq!(h.step("first", false).as_deref(), Some("second"));
        assert_eq!(h.step("second", false).as_deref(), Some("draft"));
        assert!(h.step("draft", false).is_none());
    }

    #[test]
    fn history_deduplicates_bounds_entries_and_keeps_edits_to_a_recalled_query() {
        let mut h = QueryHistory::default();
        for i in 0..40 {
            h.remember(&i.to_string());
        }
        h.remember("39");
        h.remember("");
        assert_eq!(h.entries.len(), 32);
        assert_eq!(h.step("", true).as_deref(), Some("39"));
        assert_eq!(h.step("39", true).as_deref(), Some("38"));
        assert_eq!(h.step("edited", true).as_deref(), Some("39"));
        assert_eq!(h.step("39", false).as_deref(), Some("edited"));
    }

    #[test]
    fn expanded_f01_disk_search_rejects_unknown_and_malformed_unicode_without_substitution() {
        let path = std::env::temp_dir().join(format!("orlo-f01-malformed-find-{}", std::process::id()));
        for bytes in [&b"foo caf\xe9"[..], &b"\xff\xfe\x00\xd8"[..], &b"\x00\x00\xfe\xff\x00\x11\x00\x00"[..]] {
            std::fs::write(&path, bytes).unwrap();
            let matcher = Matcher::new("foo", &FindOpts::default()).unwrap();
            assert!(
                search_path(&path, None, &matcher, 0, None, &|| true).is_none(),
                "cancelled reads publish no error"
            );
            for query in ["foo", "�", "absent"] {
                let found = search_file(&path, None, query, &FindOpts::default()).unwrap();
                assert!(
                    matches!(&found, Found::Failed(why) if why == "malformed text; reopen with the correct encoding")
                );
                assert_eq!(found.matches(), 0);
                assert!(!file_block(&path, &found).0.contains('�'));
            }
        }
        let found = search_file(&path, Some("explicit � preview"), "�", &FindOpts::default()).unwrap();
        assert_eq!(found.matches(), 1, "Use Buffer remains an explicit source-text search");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn file_search_regex_uses_character_offsets_and_marks_its_header() {
        let o = FindOpts { regex: true, ..FindOpts::default() };
        let found = search_file(Path::new("unused"), Some("é😀 id12\nID٣"), r"id\d+", &o).unwrap();
        let Found::Lines { hits, .. } = found else { panic!("expected text matches") };
        assert_eq!(
            hits,
            [(0, std::iter::once(3..7).collect::<Vec<_>>()), (1, std::iter::once(0..3).collect::<Vec<_>>())]
        );
        assert!(header(1, r"id\d+", &o).contains("(regex)"));
    }
}
use ignore::{overrides::OverrideBuilder, WalkBuilder};

/// Multiline search in files: whole-text search with the editor's line breaks and structured locations.
#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod cross_line_tests {
    use super::*;

    fn rx() -> FindOpts {
        FindOpts { regex: true, case_sensitive: true, ..FindOpts::default() }
    }

    fn matches(found: &Found) -> &[Hit] {
        match found {
            Found::Lines { matches, .. } => matches,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_match_over_lines_is_found_in_lf_crlf_and_cr_files() {
        let dir = fresh("orlo-fif-cross");
        for (name, text) in
            [("lf.txt", "one\nfoo\nbar\n"), ("crlf.txt", "one\r\nfoo\r\nbar\r\n"), ("cr.txt", "one\rfoo\rbar\r")]
        {
            let p = dir.join(name);
            std::fs::write(&p, text).unwrap();
            for (q, o) in [("foo\nbar", FindOpts::default()), (r"foo\nbar", rx())] {
                let found = search_file(&p, None, q, &o).unwrap();
                let h = &matches(&found)[0];
                assert_eq!((h.chars.clone(), h.start, h.end), (4..11, (1, 0), (2, 3)), "{name} {q}");
                let (block, outlines) = file_block(&p, &found);
                assert!(block.contains("    2: foo\n    3: bar\n"), "{block}");
                assert_eq!(outlines.len(), 1, "one outline for the match, over both lines");
                let chars: Vec<char> = block.chars().collect();
                let shown: String = chars[outlines[0].clone()].iter().collect();
                assert_eq!(shown, "foo\n    3: bar");
                let st = crate::buffer::EditorState::from_text(&block);
                let (col, _) = result_match(&st.doc, &outlines, 2, outlines[0].start).unwrap();
                assert_eq!(col, 0, "it opens at the match's start");
            }
            // `\r` is never a line break of a normalised file
            assert!(search_file(&p, None, r"\r", &rx()).is_none(), "{name}");
        }
    }

    #[test]
    fn locations_count_chars_and_a_match_ending_after_a_break_stops_at_its_line() {
        let text = "é😀 x\ny\nz";
        let m = Matcher::new(r"x\n", &rx()).unwrap();
        let found = search_text(text, &m, CONTEXT, None, &|| false).unwrap().unwrap();
        let h = &matches(&found)[0];
        assert_eq!((h.chars.clone(), h.bytes.clone()), (3..5, 7..9));
        assert_eq!((h.start, h.end), ((0, 3), (1, 0)));
        assert_eq!(h.lines(), 0..=0);
    }

    #[test]
    fn long_matches_show_their_first_lines_but_keep_their_whole_range() {
        let text: String = (0..40).map(|i| format!("l{i}\n")).collect();
        let m = Matcher::new(r"(?s)l0.*l39", &rx()).unwrap();
        let found = search_text(&text, &m, 0, None, &|| false).unwrap().unwrap();
        let Found::Lines { lines, hits, matches } = &found else { unreachable!() };
        assert_eq!(matches[0].end, (39, 3));
        assert_eq!(hits.len(), MATCH_LINES_SHOWN);
        assert_eq!(lines.len(), MATCH_LINES_SHOWN, "no context with Show Context off");
    }

    #[test]
    fn a_runaway_pattern_marks_the_file_failed() {
        let p = fresh("orlo-fif-runaway").join("a.txt");
        std::fs::write(&p, format!("{}c", "a".repeat(40))).unwrap();
        let found = search_file(&p, None, r"^(a|a)*\1b", &rx()).unwrap();
        assert!(matches!(&found, Found::Failed(why) if why.contains("line 1")), "{found:?}");
        assert!(file_block(&p, &found).0.contains("Search stopped"));
        assert_eq!(found.matches(), 0);
    }

    #[test]
    fn replacements_are_previewed_with_each_match() {
        let text = "Foo foo\nFOO";
        let o = FindOpts::default();
        let m = Matcher::new("foo", &o).unwrap();
        let r = Replacer::new("foo", "bar", &o, true).unwrap();
        let found = search_text(text, &m, CONTEXT, Some(&r), &|| false).unwrap().unwrap();
        let reps: Vec<_> = matches(&found).iter().map(|h| h.replacement.clone().unwrap()).collect();
        assert_eq!(reps, ["Bar", "bar", "BAR"]);
    }
}

#[cfg(test)]
mod root_exclude_tests {
    use super::*;

    #[test]
    fn a_roots_exclude_patterns_hide_files_and_folders_from_its_walk() {
        let d = fresh(&format!("orlo-search-root-x-{}", std::process::id()));
        std::fs::create_dir_all(d.join("a/gen")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        for f in ["a/hit.rs", "a/hit.log", "a/gen/x.rs", "b/hit.log"] {
            std::fs::write(d.join(f), "needle\n").unwrap();
        }
        let mut scope = resolve(&parse_where(""), &[d.join("a"), d.join("b")], &[], &[]);
        scope.root_excludes = vec![(d.join("a"), vec!["gen".into()], vec!["*.log".into()])];
        let real = d.canonicalize().unwrap();
        let mut found: Vec<_> =
            (all(&scope).into_iter()).map(|p| p.strip_prefix(&real).unwrap().to_path_buf()).collect();
        found.sort();
        assert_eq!(found, [PathBuf::from("a/hit.rs"), PathBuf::from("b/hit.log")]);
    }
}

#[cfg(test)]
mod previous_line_tests {
    use super::*;

    #[test]
    fn previous_matching_lines_bound_complete_matches_with_full_regex_context() {
        let text = "outside\né seed\n中 seed\ngap\n🦀 seed\ntail";
        let first = Matcher::new("seed", &FindOpts::default()).unwrap();
        let Found::Lines { matches, .. } = search_text(text, &first, 2, None, &|| false).unwrap().unwrap() else {
            panic!()
        };
        let scopes = crate::search::results::matching_lines(text, &matches);
        let ranges = |query: &str, regex| {
            let matcher = Matcher::new(query, &FindOpts { regex, case_sensitive: true, ..Default::default() }).unwrap();
            match search_source_scoped(text, &matcher, &scopes, 0, None, &|| false) {
                Some(Found::Lines { matches, .. }) => matches.into_iter().map(|h| h.chars).collect::<Vec<_>>(),
                None => Vec::new(),
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(ranges("é seed\n中 seed", false), std::iter::once(8..21).collect::<Vec<_>>());
        assert!(ranges("中 seed\ngap\n🦀 seed", false).is_empty());
        assert!(ranges("seed\\n(?:gap\\n)?🦀", true).is_empty(), "a match starting on a hit line cannot escape it");
        assert_eq!(ranges(r"(?<=outside\n)é seed(?=\n中)", true), std::iter::once(8..14).collect::<Vec<_>>());
        assert!(ranges(r"\Aé", true).is_empty(), "the interval is not a new document");
        assert_eq!(
            ranges(r"(?=gap)", true),
            std::iter::once(22..22).collect::<Vec<_>>(),
            "existing scoped-search interval-end zero-width policy"
        );
        let matcher = Matcher::new("seed", &FindOpts::default()).unwrap();
        assert!(search_source_scoped(text, &matcher, &scopes, 0, None, &|| true).is_none());
    }

    #[test]
    fn previous_matching_lines_where_is_distinct_and_cannot_broaden() {
        let w = parse_where(PREVIOUS_MATCHING_LINES);
        assert!(w.previous_lines && !w.previous && !w.open_files && !w.open_folders);
        let paths = vec![PathBuf::from("/source.txt")];
        let scope = resolve(&w, &[PathBuf::from("/project")], &[], &paths);
        assert_eq!(scope.files, paths);
        assert!(scope.roots.is_empty());
        assert!(scope_problem(&w, &scope, true).is_none());
        for value in [
            "<previous matching lines>, <previous results>",
            "<previous matching lines>, <open files>",
            "<previous matching lines>, /other",
        ] {
            let w = parse_where(value);
            assert!(scope_problem(&w, &resolve(&w, &[], &[], &paths), true).unwrap().contains("without other"));
        }
    }
}
