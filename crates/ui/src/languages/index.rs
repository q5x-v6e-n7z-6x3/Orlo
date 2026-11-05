//! The project index: one per window, over its folders' files and its open buffers, with
//! each one's definitions (syntect's `entity.name` scopes, as Goto Symbol finds them), their doc
//! comments, and its identifier counts. Goto Definition, completion, hover, the outline and
//! related files read it; nothing else keeps a symbol index.

use crate::editor::{Cancel, Editor};
use gpui::{App, AppContext as _, AsyncApp, Context, Entity, EntityId, Subscription, Task, WeakEntity};
use orlo_config::settings::Settings;
use orlo_editor::searching::search;
use orlo_project::project::Root;
use orlo_project::watch::{self, Stamp};
use orlo_syntax::highlight::{self, Checkpoints, Highlighter, SymbolParse};
use ropey::Rope;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

/// Larger files are left out of the walk.
const MAX_BYTES: u64 = 4 << 20;
/// An edited buffer is parsed again this long after its last edit.
const EDIT_DELAY: Duration = Duration::from_millis(300);
/// A rescan comes this long after the last one, or ten of its walks if longer.
const RESCAN_EVERY: Duration = Duration::from_secs(10);
/// A file's most frequent identifiers kept.
const TOP_IDENTS: usize = 2_000;
/// The window hears of files read at most this often while more are coming.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

pub use orlo_syntax::index::{signature, Docs, IndexedSymbol, Line, MAX_FILES};

/// What an entry was made from: the work that made it carries the same key, and a result whose
/// key is no longer the one wanted is dropped.
#[derive(Clone, Debug, PartialEq)]
pub enum EntryKey {
    /// A folder's file as the walk found it.
    File(PathBuf, Stamp),
    /// An open buffer (by one of its views, the one an untitled buffer is found by) at a version.
    Buffer(EntityId, u64),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub key: EntryKey,
    /// The file, or the buffer's file; `None` for an untitled buffer.
    pub path: Option<PathBuf>,
    /// The syntax it was parsed as, as View > Syntax names it (`Highlighter::syntax_name`).
    pub syntax: String,
    /// In the order they appear.
    pub symbols: Arc<Vec<IndexedSymbol>>,
    /// Words of 3+ chars (completion's words) and how often each occurs: the 2,000 most frequent.
    /// Each word is the one copy every entry shares (`WordTable`).
    pub idents: Arc<HashMap<Arc<str>, u32>>,
    /// Lexical `receiver.member` counts, capped like identifiers.
    pub members: Arc<HashMap<String, u32>>,
}

/// Shared entry metadata crosses into background navigation and completion jobs.
pub type SymbolEntry = Entry;

enum SymbolUpdate {
    File(PathBuf, Option<SymbolEntry>),
    Buffer(u64, Option<SymbolEntry>),
    ClearFiles,
}

struct SymbolEntries {
    updates: mpsc::Receiver<SymbolUpdate>,
    files: HashMap<PathBuf, SymbolEntry>,
    buffers: HashMap<u64, SymbolEntry>,
}

/// Queries drain index updates on the background pool; the UI only queues individual changes.
#[derive(Clone)]
pub struct SymbolSnapshot(Arc<Mutex<SymbolEntries>>);

impl SymbolSnapshot {
    fn drain(&self) -> std::sync::MutexGuard<'_, SymbolEntries> {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        while let Ok(update) = entries.updates.try_recv() {
            match update {
                SymbolUpdate::File(path, Some(entry)) => {
                    entries.files.insert(path, entry);
                }
                SymbolUpdate::File(path, None) => {
                    entries.files.remove(&path);
                }
                SymbolUpdate::Buffer(id, Some(entry)) => {
                    entries.buffers.insert(id, entry);
                }
                SymbolUpdate::Buffer(id, None) => {
                    entries.buffers.remove(&id);
                }
                SymbolUpdate::ClearFiles => entries.files.clear(),
            }
        }
        entries
    }

    fn flush_later(&self, cx: &App) {
        let snapshot = self.clone();
        cx.background_executor()
            .spawn(async move {
                drop(snapshot.drain());
            })
            .detach();
    }

    pub fn entries(&self) -> Vec<SymbolEntry> {
        let entries = self.drain();
        let open: HashSet<_> = entries.buffers.values().filter_map(|e| e.path.as_ref()).collect();
        entries
            .buffers
            .values()
            .chain(entries.files.iter().filter(|(p, _)| !open.contains(p)).map(|(_, e)| e))
            .cloned()
            .collect()
    }

    pub fn definitions(&self, name: &str) -> Vec<(SymbolEntry, IndexedSymbol)> {
        self.entries()
            .into_iter()
            .flat_map(|entry| {
                entry.symbols.iter().filter(|s| s.name == name).map(|s| (entry.clone(), s.clone())).collect::<Vec<_>>()
            })
            .collect()
    }
}

/// Identifier counts: completion's words of 3+ chars anywhere in `rope`, the 2,000 most
/// frequent (ties to the alphabetically first). None once `stop` is set.
pub fn idents(rope: &Rope, stop: &AtomicBool) -> HashMap<Arc<str>, u32> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    let mut budget = usize::MAX;
    for line in rope.lines() {
        if stop.load(Ordering::Relaxed) {
            return HashMap::new();
        }
        orlo_syntax::complete::line_words(line, &mut budget, |w, cols| {
            if cols.len() < 3 {
                return;
            }
            match counts.get_mut(w) {
                Some(n) => *n += 1,
                None => drop(counts.insert(w.to_string(), 1)),
            }
        });
    }
    static WORDS: OnceLock<Mutex<WordTable>> = OnceLock::new();
    let table = WORDS.get_or_init(|| Mutex::new(WordTable::new(1 << 16)));
    (table.lock().unwrap_or_else(|e| e.into_inner())).share(top(counts))
}

/// The `TOP_IDENTS` most frequent of `counts`, ties to the first in order.
fn top<K: Ord>(counts: HashMap<K, u32>) -> Vec<(K, u32)> {
    let mut top: Vec<_> = counts.into_iter().collect();
    top.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top.truncate(TOP_IDENTS);
    top
}

/// Each distinct identifier once, shared by the counts of every entry in every window: about
/// half the index's memory is identifiers, most of them in many files.
struct WordTable {
    words: HashSet<Arc<str>>,
    /// Its size after the last sweep, and the size below which none is needed.
    swept: usize,
    floor: usize,
}

impl WordTable {
    fn new(floor: usize) -> Self {
        Self { words: HashSet::new(), swept: 0, floor }
    }

    /// `top`'s words as the table's copies. Once the table has doubled since the last sweep,
    /// the words no entry holds any more (a deleted file's, a typed word's prefixes) go.
    fn share(&mut self, top: Vec<(String, u32)>) -> HashMap<Arc<str>, u32> {
        let counts = (top.into_iter())
            .map(|(w, n)| match self.words.get(w.as_str()) {
                Some(shared) => (shared.clone(), n),
                None => {
                    let shared: Arc<str> = w.into();
                    self.words.insert(shared.clone());
                    (shared, n)
                }
            })
            .collect();
        if self.words.len() > 2 * self.swept.max(self.floor) {
            // only the table holds one: no entry can take it again but from the table
            self.words.retain(|w| Arc::strong_count(w) > 1);
            self.swept = self.words.len();
        }
        counts
    }
}

/// The files to index under `scope`'s folders with their stamps: Find in Files' walk, less
/// files over 4 MB, each once however the folders nest. At most `limit`: true when more were
/// left out. Nothing once `stop` is set; `found` counts them as they come.
pub fn walk(
    scope: &search::Scope,
    limit: usize,
    stop: &AtomicBool,
    found: &AtomicUsize,
) -> (Vec<(PathBuf, Stamp)>, bool) {
    let (mut out, mut seen) = (Vec::new(), HashSet::new());
    for root in &scope.roots {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        for file in search::walk(scope, &root) {
            if stop.load(Ordering::Relaxed) {
                return (Vec::new(), false);
            }
            let Some(path) = file else { continue };
            let Some(stamp) = watch::stamp(&path).filter(|s| s.len <= MAX_BYTES) else {
                continue;
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            if out.len() == limit {
                return (out, true);
            }
            out.push((path, stamp));
            found.fetch_add(1, Ordering::Relaxed);
        }
    }
    (out, false)
}

/// What the walk of `roots` covers: Find in Files' walk with the roots' exclude patterns, the
/// file ones joined by `index_exclude_patterns`.
fn scope(roots: &[Root], excludes: &[String]) -> search::Scope {
    let root_excludes = (roots.iter())
        .map(|r| {
            let files = (r.file_exclude_patterns.iter()).chain(excludes).cloned();
            (r.path.clone(), r.folder_exclude_patterns.clone(), files.collect())
        })
        .collect();
    search::Scope {
        roots: orlo_project::project::present(roots),
        files: Vec::new(),
        include: Vec::new(),
        exclude: Vec::new(),
        gitignore: true,
        filter_files: false,
        root_excludes,
    }
}

/// How many folders apart `a` and `b` are: up from one to where they meet, then down.
fn distance(a: &Path, b: &Path) -> usize {
    let (a, b): (Vec<_>, Vec<_>) = (a.components().collect(), b.components().collect());
    let shared = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    a.len() + b.len() - 2 * shared
}

/// What a walk found, against what the walk before it found: worked out with the walk, off
/// the UI thread, which only applies it.
struct Walked {
    /// Every file found and its stamp, for the next walk to compare with.
    files: HashMap<PathBuf, Stamp>,
    /// Files the walk before did not find.
    new: HashMap<PathBuf, File>,
    /// Files it found with another stamp.
    changed: Vec<(PathBuf, Stamp)>,
    /// Files it found that this walk did not.
    gone: Vec<PathBuf>,
    /// The new and changed files, nearest the active file's folder first: read in this order.
    order: Vec<PathBuf>,
    limited: bool,
}

/// `found` against `before`, the walk before's files.
fn compare(
    before: HashMap<PathBuf, Stamp>,
    found: Vec<(PathBuf, Stamp)>,
    limited: bool,
    near: Option<&Path>,
) -> Walked {
    let mut files = HashMap::with_capacity(found.len());
    let (mut new, mut changed, mut order) = (HashMap::new(), Vec::new(), Vec::new());
    for (path, stamp) in found {
        match before.get(&path) {
            Some(s) if *s == stamp => {}
            Some(_) => {
                order.push(path.clone());
                changed.push((path.clone(), stamp));
            }
            None => {
                order.push(path.clone());
                new.insert(path.clone(), File { stamp, entry: None });
            }
        }
        files.insert(path, stamp);
    }
    let gone = (before.into_keys()).filter(|p| !files.contains_key(p)).collect();
    Walked { files, new, changed, gone, order: nearest_first(order, near), limited }
}

/// `files` with those whose folder is nearest `near` first, else by path.
fn nearest_first(mut files: Vec<PathBuf>, near: Option<&Path>) -> Vec<PathBuf> {
    files.sort_by_cached_key(|f| {
        let dir = f.parent().unwrap_or(f);
        (near.map_or(0, |n| distance(n, dir)), f.clone())
    });
    files
}

/// Whether a rescan is due: the last walk ended at `end` and took `took`.
fn rescan_due(end: Instant, took: Duration, now: Instant) -> bool {
    now.saturating_duration_since(end) >= RESCAN_EVERY.max(took * 10)
}

/// The parse for `h`'s syntax's symbols; Plain Text has none.
fn parser(h: &Highlighter) -> Option<SymbolParse> {
    (h.syntax_name() != "Plain Text").then(|| h.symbol_parse())
}

/// `path` as the index reads it: its text, its syntax and the parse for its symbols. `None` for
/// a file Find in Files would call binary, or one unreadable or too large now.
fn load(path: &Path) -> Option<(Rope, String, Option<SymbolParse>)> {
    let file = search::read_file(path).ok()?;
    if file.binary || file.text.len() as u64 > MAX_BYTES {
        return None;
    }
    let h = Highlighter::detect(Some(path), file.text.lines().next().unwrap_or(""));
    let syntax = h.syntax_name().to_string();
    Some((Rope::from_str(&file.text), syntax, parser(&h)))
}

/// `rope` parsed on the highlighting thread for its symbols (`None` without a parse, or when it
/// panicked), and its identifiers, counted on another thread meanwhile. `stop` set gives up
/// the parse.
async fn read(
    rope: Rope,
    parse: Option<SymbolParse>,
    stop: Arc<AtomicBool>,
    cx: &AsyncApp,
) -> (Option<SymbolParse>, Arc<HashMap<Arc<str>, u32>>, Arc<HashMap<String, u32>>) {
    // a newer parse drops this one: its count stops too, rather than pile up behind it
    let words = {
        let (rope, stop) = (rope.clone(), stop.clone());
        cx.background_executor().spawn(async move {
            let idents = Arc::new(idents(&rope, &stop));
            let counts = orlo_syntax::complete::member_counts(&rope, 0..rope.len_lines(), usize::MAX, || {
                stop.load(Ordering::Relaxed)
            });
            (idents, Arc::new(top(counts).into_iter().collect()))
        })
    };
    let parse = match parse {
        Some(p) => highlight::parse_on_worker(p, rope, move || stop.load(Ordering::Relaxed)).await,
        None => None,
    };
    let (idents, members) = words.await;
    (parse, idents, members)
}

/// A folder's file: the stamp the last walk found, and its entry once read.
struct File {
    stamp: Stamp,
    entry: Option<Entry>,
}

impl File {
    /// Its entry is of the file as the walk found it.
    fn current(&self, path: &Path) -> bool {
        let made = |e: &Entry| matches!(&e.key, EntryKey::File(p, s) if p == path && *s == self.stamp);
        self.entry.as_ref().is_some_and(made)
    }
}

/// An open buffer of this window.
#[derive(Default)]
struct Buffer {
    /// Its views here, each watched for changes; the first one's syntax is the entry's.
    views: Vec<(WeakEntity<Editor>, Subscription)>,
    /// Retained when a view replaces its buffer before the observer runs.
    path: Option<PathBuf>,
    entry: Option<Entry>,
    /// What the parse that made the entry leaves for the next: an edit is parsed again from
    /// the checkpoint above it.
    checkpoints: Option<Arc<Checkpoints>>,
    /// The wait after an edit (for that version and syntax), before the parse.
    wait: Option<((u64, String), Task<()>)>,
    /// The parse under way, for that version and syntax.
    parse: Option<((u64, String), Task<()>)>,
}

impl Buffer {
    /// Where `editor` is among its views.
    fn view_of(&self, editor: &Entity<Editor>) -> Option<usize> {
        (self.views.iter()).position(|(v, _)| v.entity_id() == editor.entity_id())
    }
}

/// The window's index (a gpui entity the workspace owns).
pub struct ProjectIndex {
    /// Bumped when the roots change or the index is turned on or off: the folders' work begun
    /// before is cancelled, and its results dropped. Buffers' work does not depend on it.
    pub generation: u64,
    roots: Vec<Root>,
    /// `index_files` and `index_exclude_patterns` as the walk uses them.
    enabled: bool,
    excludes: Vec<String>,
    /// Where the window's active file is: the files nearest it are read first.
    near: Option<PathBuf>,
    files: HashMap<PathBuf, File>,
    /// What the last walk found, for the next one to compare with (it has it while it runs).
    seen: HashMap<PathBuf, Stamp>,
    /// The folders' files still to read, nearest first; one is read at a time.
    queue: VecDeque<PathBuf>,
    reading: Option<Task<()>>,
    /// The walk under way, and another asked for meanwhile.
    walking: Option<Task<()>>,
    walk_again: bool,
    /// When the last walk ended and how long it took.
    last_walk: Option<(Instant, Duration)>,
    /// The first build since the roots were set is under way; while its walk runs, what it found.
    building: bool,
    found: Option<Arc<AtomicUsize>>,
    /// The last walk stopped at `limit` files.
    limited: bool,
    limit: usize,
    buffers: HashMap<u64, Buffer>,
    /// `ensure_fresh`'s callers, woken when a buffer's parse ends.
    waiters: Vec<Waker>,
    notified: Instant,
    symbol_updates: mpsc::Sender<SymbolUpdate>,
    symbols: SymbolSnapshot,
    _settings: Subscription,
}

impl ProjectIndex {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let s = cx.global::<Settings>();
        let (enabled, excludes) = (s.index_files, s.index_exclude_patterns.clone());
        let (symbol_updates, updates) = mpsc::channel();
        let symbols = SymbolSnapshot(Arc::new(Mutex::new(SymbolEntries {
            updates,
            files: HashMap::new(),
            buffers: HashMap::new(),
        })));
        Self {
            generation: 0,
            roots: Vec::new(),
            enabled,
            excludes,
            near: None,
            files: HashMap::new(),
            seen: HashMap::new(),
            queue: VecDeque::new(),
            reading: None,
            walking: None,
            walk_again: false,
            last_walk: None,
            building: false,
            found: None,
            limited: false,
            limit: MAX_FILES,
            buffers: HashMap::new(),
            waiters: Vec::new(),
            notified: Instant::now(),
            symbol_updates,
            symbols,
            _settings: cx.observe_global::<Settings>(Self::settings_changed),
        }
    }

    /// Every entry: the open buffers', then the folders' files no open buffer stands for.
    #[cfg(test)]
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        let open: HashSet<&Path> = (self.buffers.values()).filter_map(|b| b.entry.as_ref()?.path.as_deref()).collect();
        let files =
            (self.files.iter()).filter(move |(p, _)| !open.contains(p.as_path())).filter_map(|(_, f)| f.entry.as_ref());
        (self.buffers.values()).filter_map(|b| b.entry.as_ref()).chain(files)
    }

    /// Every definition, with the entry it is in.
    #[cfg(test)]
    pub fn symbols(&self) -> impl Iterator<Item = (&Entry, &IndexedSymbol)> {
        self.entries().flat_map(|e| e.symbols.iter().map(move |s| (e, s)))
    }

    /// The definitions named `name` exactly.
    #[cfg(test)]
    pub fn definitions(&self, name: &str) -> Vec<(&Entry, &IndexedSymbol)> {
        self.symbols().filter(|(_, s)| s.name == name).collect()
    }

    pub fn snapshot(&self) -> SymbolSnapshot {
        self.symbols.clone()
    }

    fn publish(&self, update: SymbolUpdate, cx: &App) {
        let _ = self.symbol_updates.send(update);
        self.symbols.flush_later(cx);
    }

    fn publish_file(&self, path: &Path, cx: &App) {
        let entry = self.files.get(path).and_then(|f| f.entry.clone());
        self.publish(SymbolUpdate::File(path.to_path_buf(), entry), cx);
    }

    fn publish_buffer(&self, id: u64, cx: &App) {
        let entry = self.buffers.get(&id).and_then(|b| b.entry.clone());
        self.publish(SymbolUpdate::Buffer(id, entry), cx);
    }

    pub fn disk_entry(&self, path: &Path) -> Option<&Entry> {
        self.files.get(path)?.entry.as_ref()
    }

    /// The entry of `editor`'s buffer, as last parsed (`ensure_fresh` brings it up to date).
    pub fn entry_for(&self, editor: &Entity<Editor>, cx: &App) -> Option<&Entry> {
        let id = editor.read(cx).doc(cx).id();
        self.buffer_entry(id)
    }

    pub fn buffer_entry(&self, id: u64) -> Option<&Entry> {
        self.buffers.get(&id)?.entry.as_ref()
    }

    /// `(read, found)` while the first build runs; the walk is still counting while read is 0.
    pub fn progress(&self) -> Option<(usize, usize)> {
        if !self.building {
            return None;
        }
        Some(match &self.found {
            Some(found) => (0, found.load(Ordering::Relaxed)),
            None => {
                let left = self.queue.len() + usize::from(self.reading.is_some());
                (self.files.len().saturating_sub(left), self.files.len())
            }
        })
    }

    /// The file limit, while the last walk stopped at it.
    pub fn limited(&self) -> Option<usize> {
        self.limited.then_some(self.limit)
    }

    /// Tests: a walk stops at `limit` files.
    #[cfg(test)]
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    /// Current roots are installed synchronously, even while indexing is disabled or warming.
    pub(crate) fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// The window's roots: a change cancels the work under way and builds the index again;
    /// the same roots are rescanned. `near` is the active file's folder.
    pub fn set_roots(&mut self, roots: Vec<Root>, near: Option<PathBuf>, cx: &mut Context<Self>) {
        let key = |r: &Root| {
            let (d, f) = (&r.folder_exclude_patterns, &r.file_exclude_patterns);
            (r.path.clone(), d.clone(), f.clone())
        };
        let same = roots.iter().map(key).eq(self.roots.iter().map(key));
        self.roots = roots;
        match same {
            true => self.rescan(near, cx),
            false => self.rebuild(near, cx),
        }
    }

    /// The folders' work stops and their entries go; a build starts if the index is on.
    fn rebuild(&mut self, near: Option<PathBuf>, cx: &mut Context<Self>) {
        self.generation += 1;
        (self.walking, self.reading, self.walk_again) = (None, None, false);
        self.queue.clear();
        self.files.clear();
        self.publish(SymbolUpdate::ClearFiles, cx);
        self.seen.clear();
        (self.limited, self.last_walk, self.found) = (false, None, None);
        self.building = self.enabled && !self.roots.is_empty();
        self.rescan(near, cx);
        cx.notify();
    }

    /// Walk the folders again: new files are read, changed ones read again, missing ones
    /// dropped (a rename is both). One walk at a time.
    pub fn rescan(&mut self, near: Option<PathBuf>, cx: &mut Context<Self>) {
        self.near = near;
        if !self.enabled || self.roots.is_empty() {
            return;
        }
        if self.walking.is_some() {
            self.walk_again = true;
            return;
        }
        let (generation, limit) = (self.generation, self.limit);
        let scope = scope(&self.roots, &self.excludes);
        let (before, near) = (std::mem::take(&mut self.seen), self.near.clone());
        let found = Arc::new(AtomicUsize::new(0));
        if self.building && self.last_walk.is_none() {
            self.found = Some(found.clone());
        }
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let stop = cancel.0.clone();
        // the executor's clock, which tests can move
        let start = cx.background_executor().now();
        let walk = cx.background_spawn(async move {
            let (files, limited) = walk(&scope, limit, &stop, &found);
            (!stop.load(Ordering::Relaxed)).then(|| compare(before, files, limited, near.as_deref()))
        });
        self.walking = Some(cx.spawn(async move |this, cx| {
            let _cancel = cancel;
            let walked = walk.await;
            let end = cx.background_executor().now();
            this.update(cx, |i, cx| {
                i.walking = None;
                i.last_walk = Some((end, end.saturating_duration_since(start)));
                if let Some(w) = walked.filter(|_| i.generation == generation) {
                    i.walked(w, cx);
                }
                if std::mem::take(&mut i.walk_again) {
                    i.rescan(i.near.clone(), cx);
                }
            })
            .ok();
        }));
    }

    /// Whether the periodic rescan is due (every 10 s, or ten walks' time on a big tree); not
    /// while a walk runs, or the polls during it would ask for another at its end.
    pub fn rescan_due(&self, cx: &App) -> bool {
        let now = cx.background_executor().now();
        self.walking.is_none() && (self.last_walk).is_some_and(|(end, took)| rescan_due(end, took, now))
    }

    /// The walk's result: missing files go, new and changed ones are read, nearest first and
    /// ahead of any still waiting.
    fn walked(&mut self, w: Walked, cx: &mut Context<Self>) {
        (self.found, self.limited, self.seen) = (None, w.limited, w.files);
        for path in &w.gone {
            self.files.remove(path);
            self.publish_file(path, cx);
        }
        for (path, stamp) in w.changed {
            // one a save left out for its size may be back
            (self.files.entry(path).or_insert(File { stamp, entry: None })).stamp = stamp;
        }
        match self.files.is_empty() {
            true => self.files = w.new,
            false => self.files.extend(w.new),
        }
        match self.queue.is_empty() {
            true => self.queue = w.order.into(),
            false => w.order.into_iter().rev().for_each(|p| self.queue.push_front(p)),
        }
        self.read_next(cx);
        cx.notify();
    }

    /// Read the next file queued, unless one is being read; the first build ends with the last.
    fn read_next(&mut self, cx: &mut Context<Self>) {
        if self.reading.is_some() {
            return;
        }
        while let Some(path) = self.queue.pop_front() {
            let Some(f) = self.files.get(&path).filter(|f| !f.current(&path)) else {
                continue;
            };
            // keyed by the walk's stamp: a write since then is read again at the next rescan
            let (generation, stamp) = (self.generation, f.stamp);
            let cancel = Cancel(Arc::new(AtomicBool::new(false)));
            let stop = cancel.0.clone();
            let p = path.clone();
            let load = cx.background_spawn(async move { load(&p) });
            self.reading = Some(cx.spawn(async move |this, cx| {
                let _cancel = cancel;
                let entry = match load.await {
                    Some((rope, syntax, parse)) => {
                        let (parse, idents, members) = read(rope, parse, stop, cx).await;
                        Some(Entry {
                            key: EntryKey::File(path.clone(), stamp),
                            path: Some(path.clone()),
                            syntax,
                            // a parse that panicked gives none, rather than being tried at
                            // every rescan
                            symbols: Arc::new(parse.map(|p| p.found).unwrap_or_default()),
                            idents,
                            members,
                        })
                    }
                    None => None,
                };
                this.update(cx, |i, cx| {
                    i.reading = None;
                    i.file_read(generation, path, stamp, entry, cx);
                    i.read_next(cx);
                })
                .ok();
            }));
            return;
        }
        if self.building && self.found.is_none() {
            self.building = false;
            cx.notify();
        }
    }

    /// A file's read landed: `entry` (`None` when it could not be read) is taken if it is for
    /// the roots and the stamp still wanted, so older work never overwrites newer.
    fn file_read(
        &mut self,
        generation: u64,
        path: PathBuf,
        stamp: Stamp,
        entry: Option<Entry>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        let Some(f) = self.files.get_mut(&path).filter(|f| f.stamp == stamp) else {
            return;
        };
        // one that cannot be read keeps an empty entry: it is not read again until it changes
        f.entry = Some(entry.unwrap_or_else(|| Entry {
            key: EntryKey::File(path.clone(), stamp),
            path: Some(path.clone()),
            syntax: String::new(),
            symbols: Arc::default(),
            idents: Arc::default(),
            members: Arc::default(),
        }));
        self.publish_file(&path, cx);
        // the window hears of it, often enough for the status bar's count
        let idle = self.queue.is_empty() && self.reading.is_none();
        if idle || self.notified.elapsed() >= PROGRESS_EVERY {
            self.notified = Instant::now();
            cx.notify();
        }
    }

    /// Read folder file `path` (as it is at `stamp`) before the others.
    fn read_first(&mut self, path: &Path, stamp: Stamp, cx: &mut Context<Self>) {
        let Some(f) = self.files.get_mut(path) else {
            return;
        };
        f.stamp = stamp;
        self.queue.retain(|p| p != path);
        self.queue.push_front(path.to_path_buf());
        self.read_next(cx);
    }

    /// A view opened in this window: a buffer new to the index is parsed at once.
    pub fn open_view(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let id = editor.read(cx).doc(cx).id();
        let watch = cx.observe(editor, |i, editor, cx| i.view_changed(&editor, cx));
        let b = self.buffers.entry(id).or_default();
        if b.view_of(editor).is_some() {
            return;
        }
        b.views.push((editor.downgrade(), watch));
        b.path = editor.read(cx).doc(cx).path.clone();
        if b.views.len() == 1 {
            self.parse_buffer(id, cx);
        }
    }

    /// A view left this window. With its buffer's last view the buffer's entry goes, and the
    /// file's own shows again: read again first if it changed while the edits stood for it.
    pub fn close_view(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let s = editor.read(cx).read_state(cx);
        let Some(id) = (self.buffers.iter()).find_map(|(&id, b)| b.view_of(editor).map(|_| id)) else {
            return;
        };
        let dirty = s.doc.dirty || id != s.buffer_id();
        let Some(b) = self.buffers.get_mut(&id) else {
            return;
        };
        b.views.retain(|(v, _)| v.entity_id() != editor.entity_id());
        if let Some(first) = b.views.iter().find_map(|(v, _)| v.upgrade()) {
            // an untitled buffer is found by a view still open
            if let Some(EntryKey::Buffer(view, _)) = b.entry.as_mut().map(|e| &mut e.key) {
                *view = first.entity_id();
            }
            self.publish_buffer(id, cx);
            self.ensure_fresh(&first, cx).detach();
            cx.notify();
            return;
        }
        let path = self.buffers.remove(&id).and_then(|b| b.path);
        self.publish_buffer(id, cx);
        self.wake();
        if let (true, Some(path)) = (dirty, path) {
            match watch::stamp(&path).filter(|s| s.len <= MAX_BYTES) {
                Some(stamp) => self.read_first(&path, stamp, cx),
                None => {
                    self.files.remove(&path);
                    self.publish_file(&path, cx);
                }
            }
        }
        cx.notify();
    }

    /// Orlo saved `editor`'s file: its entry is the saved text's at once.
    pub fn saved(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let s = editor.read(cx).read_state(cx);
        let (id, path, stamp) = (s.buffer_id(), s.doc.path.clone(), s.doc.disk);
        let (Some(path), Some(stamp)) = (path, stamp) else {
            return;
        };
        if !self.files.contains_key(&path) {
            // a new file (Save As) in a folder: the walk says whether the index takes it
            let roots: Vec<PathBuf> = (self.roots.iter()).map(|r| orlo_project::io::resolve(&r.path)).collect();
            if roots.iter().any(|r| path.starts_with(r)) {
                self.rescan(self.near.clone(), cx);
            }
            return;
        }
        if stamp.len > MAX_BYTES {
            self.files.remove(&path);
            self.publish_file(&path, cx);
            return cx.notify();
        }
        // the buffer's entry, when it is of the text just written, is the file's: not once
        // the buffer has moved on from it (typing during the save)
        let written = !s.doc.dirty;
        let fresh = written && (self.wanted(id, cx)).is_some_and(|w| self.fresh(id, &w));
        let entry = fresh.then(|| self.buffers.get(&id)?.entry.clone()).flatten();
        match (entry, self.files.get_mut(&path)) {
            (Some(mut entry), Some(f)) => {
                // a Save As lands here before the view says its path changed
                (entry.key, entry.path) = (EntryKey::File(path.clone(), stamp), Some(path.clone()));
                (f.stamp, f.entry) = (stamp, Some(entry));
                self.publish_file(&path, cx);
                cx.notify();
            }
            _ => self.read_first(&path, stamp, cx),
        }
    }

    /// The grammars were reloaded (Preferences: Reload Customizations): the open buffers are
    /// parsed again, from the top, and stand for nothing until they are.
    pub fn reparse_buffers(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.buffers.keys().copied().collect();
        for id in ids {
            if let Some(b) = self.buffers.get_mut(&id) {
                let previous = (b.entry.take(), b.checkpoints.take());
                cx.background_spawn(async move { drop(previous) }).detach();
            }
            self.publish_buffer(id, cx);
            self.parse_buffer(id, cx);
        }
        cx.notify();
    }

    /// Parse `editor`'s buffer now unless its entry is up to date; the task ends when it is (or
    /// when an edit or close overtook the parse).
    pub fn ensure_fresh(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) -> Task<()> {
        self.view_changed(editor, cx);
        let id = editor.read(cx).doc(cx).id();
        // Syntax is a view choice; navigation makes the requesting view speak for its buffer.
        if let Some(b) = self.buffers.get_mut(&id) {
            if let Some(at) = b.view_of(editor) {
                b.views.rotate_left(at);
            }
        }
        let Some(want) = self.wanted(id, cx) else {
            return Task::ready(());
        };
        if self.fresh(id, &want) {
            return Task::ready(());
        }
        let parse = self.buffers.get(&id).and_then(|b| b.parse.as_ref());
        if parse.is_none_or(|(k, _)| *k != want) {
            self.parse_buffer(id, cx);
        }
        cx.spawn(async move |this, cx| {
            std::future::poll_fn(|task| {
                let busy = this.update(cx, |i, _| {
                    let busy = i.buffers.get(&id).is_some_and(|b| b.parse.is_some());
                    if busy {
                        i.waiters.push(task.waker().clone());
                    }
                    busy
                });
                match busy {
                    Ok(true) => Poll::Pending,
                    _ => Poll::Ready(()),
                }
            })
            .await
        })
    }

    /// The first live view of buffer `id` here, which speaks for it.
    fn view(&self, id: u64) -> Option<Entity<Editor>> {
        (self.buffers.get(&id)?.views.iter()).find_map(|(v, _)| v.upgrade())
    }

    /// What buffer `id`'s entry is to be made from now: its version and syntax.
    fn wanted(&self, id: u64, cx: &App) -> Option<(u64, String)> {
        let view = self.view(id)?;
        let e = view.read(cx);
        (e.doc(cx).id() == id).then(|| (e.doc(cx).version, e.highlighter.syntax_name().to_string()))
    }

    /// Buffer `id`'s entry is of `want`.
    fn fresh(&self, id: u64, want: &(u64, String)) -> bool {
        let entry = self.buffers.get(&id).and_then(|b| b.entry.as_ref());
        entry.is_some_and(|e| matches!(e.key, EntryKey::Buffer(_, v) if v == want.0) && e.syntax == want.1)
    }

    /// A view of buffer `id` changed (its text, file or syntax, or only its caret): an edit is
    /// parsed again once the edits pause.
    fn view_changed(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let id = editor.read(cx).doc(cx).id();
        let registered = (self.buffers.get(&id)).is_some_and(|b| b.view_of(editor).is_some());
        if !registered {
            self.close_view(editor, cx);
            self.open_view(editor, cx);
            return;
        }
        let (Some(view), Some(want)) = (self.view(id), self.wanted(id, cx)) else {
            return;
        };
        let path = view.read(cx).doc(cx).path.clone();
        let fresh = self.fresh(id, &want);
        let Some(b) = self.buffers.get_mut(&id) else {
            return;
        };
        b.path = path.clone();
        // Save As or a rename: the entry stands for the new file
        if let Some(e) = b.entry.as_mut().filter(|e| e.path != path) {
            e.path = path;
            let _ = (self.symbol_updates).send(SymbolUpdate::Buffer(id, Some(e.clone())));
            self.symbols.flush_later(cx);
            cx.notify();
        }
        let pending = |job: &Option<((u64, String), Task<()>)>| job.as_ref().is_some_and(|(k, _)| *k == want);
        if fresh || pending(&b.parse) || pending(&b.wait) {
            return;
        }
        // an older text's parse is no use now
        if b.parse.take().is_some() {
            self.wake();
        }
        let wait = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(EDIT_DELAY).await;
            this.update(cx, |i, cx| i.parse_buffer(id, cx)).ok();
        });
        if let Some(b) = self.buffers.get_mut(&id) {
            b.wait = Some((want, wait));
        }
    }

    /// Parse buffer `id` as its first view has it now, in place of any wait or parse under way:
    /// from the checkpoint above the edit since the last parse.
    fn parse_buffer(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(view) = self.view(id) else {
            return;
        };
        let e = view.read(cx);
        if e.doc(cx).id() != id {
            return;
        }
        let syntax = e.highlighter.syntax_name().to_string();
        let want = (e.doc(cx).version, syntax.clone());
        let key = EntryKey::Buffer(view.entity_id(), e.doc(cx).version);
        let (rope, path) = (e.doc(cx).rope.clone(), e.doc(cx).path.clone());
        let parse = parser(&e.highlighter);
        let last = self.buffers.get(&id).and_then(|b| b.checkpoints.clone());
        let previous =
            self.buffers.get(&id).and_then(|b| b.entry.as_ref()).map(|e| e.symbols.clone()).unwrap_or_default();
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let stop = cancel.0.clone();
        let task = cx.spawn(async move |this, cx| {
            let _cancel = cancel;
            // where the edit is, found off this thread
            let parse = match parse {
                Some(p) => {
                    let text = rope.clone();
                    let resume = async move { p.resume(last.as_deref(), &text) };
                    Some(cx.background_spawn(resume).await)
                }
                None => None,
            };
            let (parse, idents, members) = read(rope.clone(), parse, stop, cx).await;
            // Splicing and dropping a huge symbol list must not block the window either.
            let (symbols, checkpoints) = cx
                .background_spawn(async move {
                    let mut symbols = Vec::new();
                    let checkpoints = parse.and_then(|p| {
                        symbols = Arc::unwrap_or_clone(previous);
                        p.finish(&mut symbols, rope).map(Arc::new)
                    });
                    (Arc::new(symbols), checkpoints)
                })
                .await;
            let entry = Entry { key, path, syntax, symbols, idents, members };
            this.update(cx, |i, cx| i.buffer_read(id, entry, checkpoints, cx)).ok();
        });
        if let Some(b) = self.buffers.get_mut(&id) {
            (b.wait, b.parse) = (None, Some((want, task)));
        }
    }

    /// Buffer `id`'s parse landed: taken if the buffer is still at its version and syntax, so
    /// an older text's result never overwrites a newer one's. The parse (of the text
    /// with it) went from above the edit to where it found the state it had before: the
    /// entry's definitions before and after that stand.
    fn buffer_read(
        &mut self,
        id: u64,
        mut entry: Entry,
        checkpoints: Option<Arc<Checkpoints>>,
        cx: &mut Context<Self>,
    ) {
        let EntryKey::Buffer(_, version) = entry.key else {
            return;
        };
        let Some(view) = self.view(id) else { return };
        let current = self.wanted(id, cx) == Some((version, entry.syntax.clone()));
        let Some(b) = self.buffers.get_mut(&id) else {
            return;
        };
        if b.parse.as_ref().is_some_and(|(k, _)| k == &(version, entry.syntax.clone())) {
            b.parse = None;
        }
        if current {
            entry.key = EntryKey::Buffer(view.entity_id(), version);
            entry.path = view.read(cx).doc(cx).path.clone();
            let previous = (b.entry.replace(entry), std::mem::replace(&mut b.checkpoints, checkpoints));
            cx.background_spawn(async move { drop(previous) }).detach();
            self.publish_buffer(id, cx);
            cx.notify();
        }
        self.wake();
    }

    /// `ensure_fresh`'s callers look again.
    fn wake(&mut self) {
        for w in self.waiters.drain(..) {
            w.wake();
        }
    }

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let s = cx.global::<Settings>();
        if (s.index_files, &s.index_exclude_patterns) == (self.enabled, &self.excludes) {
            return;
        }
        (self.enabled, self.excludes) = (s.index_files, s.index_exclude_patterns.clone());
        self.rebuild(self.near.clone(), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use orlo_editor::text::EditorState;
    use std::future::Future as _;

    /// A test's own folder, emptied each run (canonical: the walk names files under real paths).
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orlo-index-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// `text`'s definitions as the index's parse finds them, parsed here.
    fn parse(path: &str, text: &str) -> Vec<IndexedSymbol> {
        let mut p = Highlighter::new(Some(Path::new(path))).symbol_parse();
        assert!(p.step(&Rope::from_str(text), || false));
        p.found
    }

    fn doc(path: &str, text: &str, name: &str) -> Option<String> {
        let found = parse(path, text);
        let s = found.into_iter().find(|s| s.name == name);
        s.unwrap_or_else(|| panic!("no {name}")).doc
    }

    #[test]
    fn doc_comments_lose_their_markers_and_skip_attributes() {
        let rust = "/// Doc line one.\n///\n/// Two.\n#[derive(Debug)]\n#[cfg(test)]\npub struct Foo;\n";
        assert_eq!(doc("x.rs", rust, "Foo").as_deref(), Some("Doc line one.\n\nTwo."));
        let c = "/** Adds.\n * Two numbers.\n */\nint add(int a, int b) { return a + b; }\n";
        assert_eq!(doc("x.c", c, "add").as_deref(), Some("Adds.\nTwo numbers."));
        let py = "# Comment doc.\n@decorator\n@other.thing(1)\ndef f(a):\n    return 1\n";
        assert_eq!(doc("x.py", py, "f").as_deref(), Some("Comment doc."));
        let py = "def g(a):\n    \"\"\"Doc string.\n\n    More.\n    \"\"\"\n    return 1\n\
                  class C:\n    '''One.'''\n    def m(self): pass\n";
        assert_eq!(doc("x.py", py, "g").as_deref(), Some("Doc string.\n\nMore."));
        assert_eq!(doc("x.py", py, "C").as_deref(), Some("One."));
        assert_eq!(doc("x.py", py, "m"), None, "the class's docstring is not its method's");
        // a header over several lines; the docstring wins over a comment above
        let py = "# note\ndef h(\n    a,\n):\n    \"\"\"Docstring.\"\"\"\n";
        assert_eq!(doc("x.py", py, "h").as_deref(), Some("Docstring."));
        assert_eq!(doc("x.js", "/** JS doc */\nfunction f() {}\n", "f").as_deref(), Some("JS doc"));
    }

    #[gpui::test]
    fn symbol_snapshot_matches_live_entries_after_edits_and_close(cx: &mut TestAppContext) {
        let (idx, e) = opened(cx, "fn first() {}\n");
        let snapshot = idx.read_with(cx, |i, _| i.snapshot());
        assert_eq!(snapshot.definitions("first").len(), 1);
        e.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert("fn second() {}\n");
            })
        });
        idx.update(cx, |i, cx| i.ensure_fresh(&e, cx)).detach();
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        assert!(snapshot.definitions("first").is_empty());
        assert_eq!(snapshot.definitions("second").len(), 1);
        idx.update(cx, |i, cx| i.close_view(&e, cx));
        assert!(snapshot.entries().is_empty());
    }

    #[test]
    fn prefixed_python_docstrings_strip_their_quotes() {
        for prefix in ["r", "u", "R", "U"] {
            let text = format!("def raw():\n    {prefix}\"\"\"Raw documentation.\"\"\"\n");
            assert_eq!(doc("x.py", &text, "raw").as_deref(), Some("Raw documentation."));
        }
    }

    #[test]
    fn without_a_comment_right_above_there_is_no_doc() {
        assert_eq!(doc("x.rs", "fn first() {}\n", "first"), None);
        assert_eq!(doc("x.rs", "// note\n\nfn after_blank() {}\n", "after_blank"), None);
        let code = "// note\nconst X: u8 = 1;\nfn after_code() {}\n";
        assert_eq!(doc("x.rs", code, "after_code"), None);
        assert_eq!(doc("x.py", "x = 1\ndef f():\n    return 1\n", "f"), None);
    }

    #[test]
    fn a_doc_keeps_its_first_30_lines() {
        let rust: String = (0..40).map(|i| format!("/// line {i}\n")).collect();
        let doc = parse("x.rs", &(rust + "fn capped() {}\n"))[0].doc.clone().unwrap();
        assert_eq!(doc.lines().count(), 30);
        assert!(doc.starts_with("line 0\nline 1\n") && doc.ends_with("\nline 29"), "{doc}");
        // the last line kept is not the closing one: its quote stays
        let body: String = (1..40).map(|i| format!("    d{i} \"q\"\n")).collect();
        let py = format!("def long():\n    \"\"\"d0\n{body}    \"\"\"\n    return 1\n");
        let doc = parse("x.py", &py)[0].doc.clone().unwrap();
        assert_eq!(doc.lines().count(), 30);
        assert!(doc.starts_with("d0\nd1 \"q\"\n") && doc.ends_with("\nd29 \"q\""), "{doc}");
    }

    #[test]
    fn a_symbol_has_its_column_and_signature_line() {
        let s = parse("x.rs", "    pub fn spaced(a: u8) -> u8 { a }\n/* éé */ fn named() {}\n");
        assert_eq!((s[0].name.as_str(), s[0].line, s[0].col), ("spaced", 0, 11));
        assert_eq!(s[0].signature, "pub fn spaced(a: u8) -> u8 { a }");
        assert_eq!((s[1].name.as_str(), s[1].line, s[1].col), ("named", 1, 12), "in chars");
        let long = format!("fn long() {{ {} }}\n", "x + ".repeat(100));
        let s = parse("x.rs", &long);
        assert_eq!(s[0].signature.chars().count(), 200);
        assert!(long.starts_with(&s[0].signature));
    }

    #[test]
    fn a_60000_line_file_has_symbols_past_line_20000() {
        let text = format!("fn first() {{}}\n{}fn last() {{}}\n", "\n".repeat(59_998));
        let h = Highlighter::by_name("Rust");
        let mut rope = Rope::from_str(&text);
        let (mut found, mut last) = (Vec::new(), None);
        reparse(&h, &rope, &mut found, &mut last);
        let at: Vec<_> = found.iter().map(|s| (s.name.as_str(), s.line)).collect();
        assert_eq!(at, [("first", 0), ("last", 59_999)]);
        let reused = found[1].name.as_ptr();
        rope.insert(rope.line_to_char(25_000), "fn middle() {}\n");
        assert!(reparse(&h, &rope, &mut found, &mut last));
        assert!(last.as_ref().unwrap().parsed <= 1_025);
        assert_eq!(found[2].line, 60_000);
        assert_eq!(found[2].name.as_ptr(), reused, "the tail symbol is reused");
        assert_eq!(found, parse("x.rs", &rope.to_string()));
    }

    /// Rust, 48 lines a block: a doc comment, a module holding an impl, its method's `if`
    /// body mostly blank lines (cheap to parse, and where checkpoints fall: four blocks deep).
    fn nested(blocks: usize) -> String {
        let body = "\n".repeat(42);
        (0..blocks)
            .map(|i| {
                format!(
                    "/// Module {i}.\npub mod m{i} {{ impl S{i} {{\n    \
                     /// Its method.\n    pub fn f{i}(&self) -> u8 {{ if true {{\n{body}        \
                     {i} }} else {{ 0 }} }}\n}} }}\n"
                )
            })
            .collect()
    }

    /// An open buffer's parse of `rope` as the index runs it, by `h`: from the checkpoints
    /// `last` left, into `symbols`. Whether it stopped at a checkpoint past the edit.
    fn reparse(h: &Highlighter, rope: &Rope, symbols: &mut Vec<IndexedSymbol>, last: &mut Option<Checkpoints>) -> bool {
        let mut p = h.symbol_parse().resume(last.as_ref(), rope);
        assert!(p.step(rope, || false));
        let early = p.stopped_early();
        *last = p.finish(symbols, rope.clone());
        early
    }

    #[test]
    fn incremental_checkpoints_are_kept_in_a_file_with_a_definition_on_every_line() {
        let h = Highlighter::by_name("Rust");
        let text: String = (0..4_096).map(|i| format!("fn f{i}() {{}}\n")).collect();
        let mut rope = Rope::from_str(&text);
        let (mut symbols, mut last) = (Vec::new(), None);
        reparse(&h, &rope, &mut symbols, &mut last);
        assert_eq!(last.as_ref().unwrap().lines(), [0, 1_024, 2_048, 3_072, 4_096]);
        let reused = symbols[3_000].name.as_ptr();
        rope.insert(rope.line_to_char(1_500), "fn added() {}\n");
        assert!(reparse(&h, &rope, &mut symbols, &mut last));
        assert!(last.as_ref().unwrap().parsed <= 1_025);
        assert_eq!(symbols[3_001].name.as_ptr(), reused);
        assert_eq!(symbols, parse("x.rs", &rope.to_string()));
    }

    #[test]
    fn incremental_checkpoints_inside_python_docstrings_keep_the_definition_and_its_doc() {
        let h = Highlighter::by_name("Python");
        let text =
            format!("def first():\n    \"\"\"doc\n{}    \"\"\"\n\ndef last(): pass\n", "    more\n".repeat(3_100));
        let mut rope = Rope::from_str(&text);
        let (mut symbols, mut last) = (Vec::new(), None);
        reparse(&h, &rope, &mut symbols, &mut last);
        assert_eq!(last.as_ref().unwrap().lines(), [0, 1_024, 2_048, 3_072]);
        for line in [0, 1_500, 2_500] {
            rope.insert(rope.line_to_char(line) + 4, "x");
            assert!(reparse(&h, &rope, &mut symbols, &mut last));
            assert!(last.as_ref().unwrap().parsed <= 1_024);
            assert_eq!(symbols, parse("x.py", &rope.to_string()));
        }
    }

    #[test]
    fn incremental_checkpoints_keep_an_edited_doc_just_before_the_checkpoint() {
        for (syntax, path, comment, definition) in
            [("Rust", "x.rs", "///", "fn documented() {}\n"), ("Python", "x.py", "#", "def documented():\n    pass\n")]
        {
            let text = format!("{}{comment} old\n{definition}after = 0;\n{}", "\n".repeat(1_022), "\n".repeat(2_100));
            let h = Highlighter::by_name(syntax);
            let mut rope = Rope::from_str(&text);
            let (mut symbols, mut last) = (Vec::new(), None);
            reparse(&h, &rope, &mut symbols, &mut last);
            let at = text.find("old").unwrap();
            rope.remove(at..at + 3);
            rope.insert(at, "edited");
            assert!(reparse(&h, &rope, &mut symbols, &mut last));
            assert_eq!(symbols, parse(path, &rope.to_string()), "{syntax}");
            assert_eq!(symbols[0].doc.as_deref(), Some("edited"));
        }
    }

    #[test]
    fn an_incremental_parse_equals_a_full_parse_after_random_edits() {
        // one grammar throughout, whatever other tests reload meanwhile
        let h = Highlighter::by_name("Rust");
        let full = |rope: &Rope| {
            let mut p = h.symbol_parse();
            assert!(p.step(rope, || false));
            p.found
        };
        // 21 blocks, then a doc comment over line 1,024, where the first checkpoint falls
        let docs: String = (0..30).map(|i| format!("/// Doc line {i}.\n")).collect();
        let text = format!("{}{docs}pub fn documented() {{}}\n{}", nested(21), nested(45));
        let mut rope = Rope::from_str(&text);
        assert!(rope.len_lines() > 3_000);
        let (mut symbols, mut last) = (Vec::new(), None);
        reparse(&h, &rope, &mut symbols, &mut last);
        assert_eq!(symbols, full(&rope));
        assert_eq!(last.as_ref().unwrap().lines()[..2], [0, 1_024]);
        // xorshift: the crate has no random number generator
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 33) as usize % n.max(1)
        };
        let mut early = 0;
        for step in 0..50 {
            let lines = rope.len_lines() - 1;
            let at = next(lines);
            let line_start = rope.line_to_char(at);
            let line_text = rope.line(at).to_string();
            let what = match step {
                // inside the doc comment, on the line right above the checkpoint: the
                // definition under it takes the new doc
                0 => {
                    assert!(rope.line(1_023).to_string().starts_with("/// Doc"));
                    rope.insert(rope.line_to_char(1_023) + 4, "edited ");
                    "a doc comment line above a checkpoint"
                }
                // a block comment opened here and closed ten edits later: what follows is
                // comment meanwhile
                10 => {
                    rope.insert(line_start, "/* ");
                    "open a block comment"
                }
                20 => {
                    let open = rope.to_string().find("/* ").expect("the comment's start");
                    let open = rope.byte_to_char(open);
                    let below = (rope.char_to_line(open) + 300).min(lines);
                    rope.insert(rope.line_to_char(below), "*/\n");
                    "close it 300 lines down"
                }
                // never into the comment's own markers
                _ if line_text.contains("/*") || line_text.contains("*/") => {
                    rope.insert(line_start, "fn spared() {}\n");
                    "add a line"
                }
                _ => match next(6) {
                    0 => {
                        rope.insert(line_start, &format!("fn added{step}() {{}}\n"));
                        "add a line"
                    }
                    1 => {
                        rope.remove(line_start..rope.line_to_char(at + 1));
                        "remove a line"
                    }
                    2 => {
                        let col = next(line_text.chars().count());
                        rope.insert(line_start + col, "é");
                        "type a char"
                    }
                    3 => {
                        let col = next(line_text.chars().count().saturating_sub(1));
                        rope.remove(line_start + col..line_start + col + 1);
                        "delete a char"
                    }
                    4 => {
                        rope.insert(line_start, "mod inner {\n    fn g() {}\n}\n");
                        "add a nested block"
                    }
                    _ => {
                        let n = 1 + next(2);
                        rope.insert(line_start, &nested(n));
                        "paste blocks"
                    }
                },
            };
            early += usize::from(reparse(&h, &rope, &mut symbols, &mut last));
            assert!(last.as_ref().unwrap().parsed <= rope.len_lines());
            assert_eq!(symbols, full(&rope), "edit {step}: {what} at line {at}");
            if step == 0 {
                let documented = symbols.iter().find(|s| s.name == "documented").unwrap();
                let doc = documented.doc.as_deref().unwrap();
                assert!(doc.contains("Doc line 14.\nedited Doc line 15."), "{doc}");
            }
        }
        assert!(early >= 30, "only {early} of 50 parses stopped before the end");
    }

    #[test]
    fn identifiers_are_completions_words_of_3_chars_or_more_the_top_2000() {
        let go = AtomicBool::new(false);
        let ids = idents(&Rope::from_str("ab abc abc abcd _x9 9abc\nabc // abcd in a comment\n"), &go);
        let want: HashMap<Arc<str>, u32> =
            [("abc", 3), ("abcd", 2), ("_x9", 1), ("comment", 1)].map(|(w, n)| (w.into(), n)).into();
        assert_eq!(ids, want);
        // 2,500 words once and one twice: that one, then the first 1,999 alphabetically
        let words: String = (0..2_500).map(|i| format!("w{i:04} ")).collect();
        let ids = idents(&Rope::from_str(&(words + "zzz zzz")), &go);
        assert_eq!(ids.len(), 2_000);
        assert_eq!(ids["zzz"], 2);
        assert!(ids.contains_key("w0000") && ids.contains_key("w1998"));
        assert!(!ids.contains_key("w1999"));
        // a newer parse sets the stop flag: the count gives up at the next line
        let lines = Rope::from_str(&"word\n".repeat(1_000));
        assert!(idents(&lines, &AtomicBool::new(true)).is_empty());
        assert_eq!(idents(&lines, &go)["word"], 1_000);
    }

    #[test]
    fn every_entrys_words_are_one_shared_copy() {
        let go = AtomicBool::new(false);
        let a = idents(&Rope::from_str("shared alpha"), &go);
        let b = idents(&Rope::from_str("beta shared"), &go);
        let (x, _) = a.get_key_value("shared").unwrap();
        let (y, _) = b.get_key_value("shared").unwrap();
        assert!(Arc::ptr_eq(x, y));
        // past twice its size at the last sweep, the table keeps only words an entry holds
        let mut t = WordTable::new(2);
        let one = |w: &str| vec![(w.to_string(), 1)];
        let kept = t.share(one("kept"));
        drop(t.share(vec![("gone".into(), 1), ("also".into(), 1)]));
        let held = t.share(one("kept"));
        assert!(Arc::ptr_eq(kept.keys().next().unwrap(), held.keys().next().unwrap()));
        assert_eq!(t.words.len(), 3, "no sweep at 3 of 4");
        let more = t.share(vec![("more".into(), 1), ("again".into(), 1)]);
        let mut left: Vec<&str> = t.words.iter().map(|w| &**w).collect();
        left.sort();
        assert_eq!(left, ["again", "kept", "more"]);
        assert_eq!((t.swept, more.len()), (3, 2));
    }

    fn walked(scope: &search::Scope, limit: usize) -> (Vec<String>, bool) {
        let (files, limited) = walk(scope, limit, &AtomicBool::new(false), &AtomicUsize::new(0));
        let root = &scope.roots[0];
        let mut names: Vec<String> = (files.iter())
            .map(|(p, s)| {
                assert_eq!(watch::stamp(p), Some(*s));
                p.strip_prefix(root).unwrap().display().to_string()
            })
            .collect();
        names.sort();
        (names, limited)
    }

    #[test]
    fn the_walk_skips_ignored_excluded_large_and_binary_typed_files() {
        let d = temp("walk");
        write(&d.join(".gitignore"), "ignored.rs\n");
        for f in ["a.rs", "ignored.rs", "debug.log", "image.png", "gen/out.rs", "deep/er/b.rs"] {
            write(&d.join(f), "fn x() {}\n");
        }
        // 4 MB is the most a file may have
        write(&d.join("edge.rs"), &"x".repeat(4 << 20));
        write(&d.join("big.rs"), &"x".repeat((4 << 20) + 1));
        let root = Root { folder_exclude_patterns: vec!["gen".into()], ..Root::new(d.clone()) };
        let one = scope(std::slice::from_ref(&root), &["*.log".to_string()]);
        let all = [".gitignore", "a.rs", "deep/er/b.rs", "edge.rs"].map(String::from);
        assert_eq!(walked(&one, MAX_FILES), (all.to_vec(), false));
        assert_eq!(walked(&one, 4), (all.to_vec(), false), "as many as the limit");
        assert_eq!(walked(&one, 3).0.len(), 3);
        assert!(walked(&one, 3).1, "one more: limited");
        // nested folders give each file once; a stop gives nothing
        let both = scope(&[root, Root::new(d.join("deep"))], &["*.log".to_string()]);
        let found = AtomicUsize::new(0);
        let (files, _) = walk(&both, MAX_FILES, &AtomicBool::new(false), &found);
        assert_eq!(files.len(), 4);
        assert_eq!(found.load(Ordering::Relaxed), 4);
        assert_eq!(walk(&both, MAX_FILES, &AtomicBool::new(true), &found).0, []);
    }

    #[test]
    fn files_nearest_the_active_folder_are_read_first() {
        let p = Path::new;
        assert_eq!(distance(p("/p/a/b"), p("/p/a/b")), 0);
        assert_eq!(distance(p("/p/a/b"), p("/p/a")), 1);
        assert_eq!(distance(p("/p/a/b"), p("/p/c")), 3);
        let files = ["/p/c/x.rs", "/p/a/b/y.rs", "/p/a/z.rs", "/p/a/b/a.rs"].map(PathBuf::from);
        let order = nearest_first(files.to_vec(), Some(p("/p/a/b")));
        let want = ["/p/a/b/a.rs", "/p/a/b/y.rs", "/p/a/z.rs", "/p/c/x.rs"].map(PathBuf::from);
        assert_eq!(order, want);
        let sorted = ["/p/a/b/a.rs", "/p/a/b/y.rs", "/p/a/z.rs", "/p/c/x.rs"].map(PathBuf::from);
        assert_eq!(nearest_first(files.to_vec(), None), sorted, "no active file: by path");
    }

    #[test]
    fn a_walk_is_compared_with_the_one_before_it_nearest_first() {
        let s = |n: u64| Stamp { mtime: (n as i64, 0), len: n, ctime: (0, 0), ino: 1 };
        let p = PathBuf::from;
        let before: HashMap<PathBuf, Stamp> =
            [("/p/a/same.rs", 1), ("/p/a/changed.rs", 1), ("/p/gone.rs", 1)].map(|(f, n)| (p(f), s(n))).into();
        let found = vec![
            (p("/p/c/new.rs"), s(1)),
            (p("/p/a/same.rs"), s(1)),
            (p("/p/a/changed.rs"), s(2)),
            (p("/p/a/b/new.rs"), s(1)),
        ];
        let w = compare(before, found.clone(), true, Some(Path::new("/p/a/b")));
        let order = ["/p/a/b/new.rs", "/p/a/changed.rs", "/p/c/new.rs"].map(p);
        assert_eq!(w.order, order, "nearest the active folder first");
        assert_eq!(w.changed, [(p("/p/a/changed.rs"), s(2))]);
        assert_eq!(w.gone, [p("/p/gone.rs")]);
        let mut new: Vec<PathBuf> = w.new.keys().cloned().collect();
        new.sort();
        assert_eq!(new, ["/p/a/b/new.rs", "/p/c/new.rs"].map(p));
        assert_eq!(w.files, found.iter().cloned().collect());
        assert!(w.limited);
        // a first walk: all new, by path without an active file
        let w = compare(HashMap::new(), found, false, None);
        let all = ["/p/a/b/new.rs", "/p/a/changed.rs", "/p/a/same.rs", "/p/c/new.rs"].map(p);
        assert_eq!(w.order, all);
        assert_eq!((w.new.len(), w.changed.len(), w.gone.len()), (4, 0, 0));
    }

    #[test]
    fn rescans_come_every_10_s_or_ten_walks_time() {
        let t = Instant::now();
        let quick = Duration::from_millis(50);
        assert!(!rescan_due(t, quick, t + Duration::from_secs(9)));
        assert!(rescan_due(t, quick, t + Duration::from_secs(10)));
        let slow = Duration::from_secs(2);
        assert!(!rescan_due(t, slow, t + Duration::from_secs(19)));
        assert!(rescan_due(t, slow, t + Duration::from_secs(20)));
    }

    fn index(cx: &mut TestAppContext) -> Entity<ProjectIndex> {
        cx.update(|cx| cx.set_global(Settings::default()));
        cx.new(ProjectIndex::new)
    }

    /// Run the app until `done`, letting the highlighting thread (a real one, which the test
    /// executor does not wait for) do its part; twenty seconds, then fail.
    #[track_caller]
    fn settle(cx: &mut TestAppContext, idx: &Entity<ProjectIndex>, done: impl Fn(&ProjectIndex, &App) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            cx.run_until_parked();
            if idx.read_with(cx, |i, cx| done(i, cx)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("settle: still not done after 20 s");
    }

    /// Every definition's name, sorted.
    fn names(i: &ProjectIndex) -> Vec<String> {
        let mut v: Vec<String> = i.symbols().map(|(_, s)| s.name.clone()).collect();
        v.sort();
        v
    }

    fn built(i: &ProjectIndex, _: &App) -> bool {
        i.progress().is_none() && i.reading.is_none() && i.walking.is_none()
    }

    fn set_roots(cx: &mut TestAppContext, idx: &Entity<ProjectIndex>, roots: &[&Path]) {
        let roots = roots.iter().map(|r| Root::new(r.to_path_buf())).collect();
        idx.update(cx, |i, cx| i.set_roots(roots, None, cx));
    }

    #[gpui::test]
    fn a_build_reads_a_pony_files_definitions_by_its_tags_scm(cx: &mut TestAppContext) {
        let d = temp("pony");
        write(&d.join("a.pony"), "actor Main\n  be go() => None\n");
        write(&d.join("b.rs"), "fn rusty() {}\n");
        let idx = index(cx);
        set_roots(cx, &idx, &[&d]);
        settle(cx, &idx, built);
        idx.read_with(cx, |i, _| {
            assert_eq!(names(i), ["Main", "go", "rusty"]);
            let (entry, go) = i.definitions("go")[0];
            assert_eq!(entry.syntax, "Pony");
            assert_eq!((go.line, go.col, go.kind.as_str(), go.signature.as_str()), (1, 5, "method", "be go() => None"));
        });
    }

    #[gpui::test]
    fn a_build_reads_the_folders_files(cx: &mut TestAppContext) {
        let d = temp("build");
        write(&d.join(".gitignore"), "ignored.rs\n");
        write(&d.join("a.rs"), "/// Says hi.\nfn hello() {}\n");
        write(&d.join("ignored.rs"), "fn ignored() {}\n");
        write(&d.join("notes.txt"), "plain words here\n");
        write(&d.join("data.bin"), "fn binary() {}\0\n");
        write(&d.join("big.rs"), &format!("fn huge() {{}}\n{}", "x".repeat(4 << 20)));
        write(&d.join("long.rs"), &format!("{}fn late() {{}}\n", "\n".repeat(30_000)));
        let idx = index(cx);
        set_roots(cx, &idx, &[&d]);
        assert_eq!(idx.read_with(cx, |i, _| i.progress()), Some((0, 0)), "walking");
        settle(cx, &idx, built);
        idx.read_with(cx, |i, _| {
            assert_eq!(names(i), ["hello", "late"]);
            let (entry, hello) = i.definitions("hello")[0];
            assert_eq!(entry.key, EntryKey::File(d.join("a.rs"), watch::stamp(&d.join("a.rs")).unwrap()));
            assert_eq!(entry.path.as_deref(), Some(d.join("a.rs").as_path()));
            assert_eq!(entry.syntax, "Rust");
            assert_eq!((hello.line, hello.col, hello.doc.as_deref()), (1, 3, Some("Says hi.")));
            assert_eq!(i.definitions("late")[0].1.line, 30_000);
            // plain text gives words, no symbols
            let notes = i.entries().find(|e| e.path == Some(d.join("notes.txt"))).unwrap();
            assert!(notes.symbols.is_empty() && notes.idents.contains_key("words"));
            assert!(i.entries().all(|e| e.path != Some(d.join("data.bin")) || e.idents.is_empty()));
            assert_eq!(i.limited(), None);
        });
    }

    #[gpui::test]
    fn the_first_build_counts_what_it_read(cx: &mut TestAppContext) {
        let d = temp("progress");
        for n in 0..3 {
            write(&d.join(format!("f{n}.rs")), &format!("fn f{n}() {{}}\n"));
        }
        let idx = index(cx);
        let busy = Busy::new();
        set_roots(cx, &idx, &[&d]);
        cx.run_until_parked();
        assert_eq!(idx.read_with(cx, |i, _| i.progress()), Some((0, 3)));
        drop(busy);
        settle(cx, &idx, built);
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["f0", "f1", "f2"]);
        // a rescan is not a build
        write(&d.join("f3.rs"), "fn f3() {}\n");
        idx.update(cx, |i, cx| i.rescan(None, cx));
        assert_eq!(idx.read_with(cx, |i, _| i.progress()), None);
        settle(cx, &idx, |i, _| names(i).len() == 4);
    }

    #[gpui::test]
    fn a_walk_past_the_limit_leaves_the_index_limited(cx: &mut TestAppContext) {
        let d = temp("limit");
        for n in 0..3 {
            write(&d.join(format!("f{n}.rs")), &format!("fn f{n}() {{}}\n"));
        }
        let idx = index(cx);
        idx.update(cx, |i, _| i.limit = 2);
        set_roots(cx, &idx, &[&d]);
        settle(cx, &idx, built);
        idx.read_with(cx, |i, _| {
            assert_eq!(i.limited(), Some(2));
            assert_eq!(names(i).len(), 2);
        });
        std::fs::remove_file(d.join("f2.rs")).unwrap();
        idx.update(cx, |i, cx| i.rescan(None, cx));
        settle(cx, &idx, |i, _| i.limited().is_none());
    }

    #[gpui::test]
    fn a_rescan_is_not_due_again_while_its_walk_runs(cx: &mut TestAppContext) {
        let d = temp("due");
        write(&d.join("a.rs"), "fn a() {}\n");
        let idx = index(cx);
        set_roots(cx, &idx, &[&d]);
        settle(cx, &idx, built);
        assert!(!idx.read_with(cx, |i, cx| i.rescan_due(cx)));
        cx.executor().advance_clock(RESCAN_EVERY);
        assert!(idx.read_with(cx, |i, cx| i.rescan_due(cx)));
        idx.update(cx, |i, cx| i.rescan(None, cx));
        // the polls while the walk runs (a big tree's take seconds) ask for no second walk
        idx.update(cx, |i, cx| {
            assert!(i.walking.is_some());
            assert!(!i.rescan_due(cx));
        });
        cx.run_until_parked();
        idx.read_with(cx, |i, cx| {
            assert!(i.walking.is_none() && !i.walk_again);
            assert!(!i.rescan_due(cx), "due again 10 s after this walk");
        });
    }

    #[gpui::test]
    fn a_rescan_finds_files_created_changed_deleted_and_renamed(cx: &mut TestAppContext) {
        let d = temp("rescan");
        let deep = d.join("never/shown/here");
        write(&deep.join("keep.rs"), "fn keep() {}\n");
        write(&deep.join("change.rs"), "fn before() {}\n");
        write(&deep.join("gone.rs"), "fn gone() {}\n");
        write(&deep.join("old_name.rs"), "fn renamed() {}\n");
        let idx = index(cx);
        set_roots(cx, &idx, &[&d]);
        settle(cx, &idx, built);
        let want = ["before", "gone", "keep", "renamed"];
        assert_eq!(idx.read_with(cx, |i, _| names(i)), want);
        write(&deep.join("new.rs"), "fn created() {}\n");
        write(&deep.join("change.rs"), "fn after_the_change() {}\n");
        std::fs::remove_file(deep.join("gone.rs")).unwrap();
        std::fs::rename(deep.join("old_name.rs"), deep.join("new_name.rs")).unwrap();
        idx.update(cx, |i, cx| i.rescan(None, cx));
        settle(cx, &idx, |i, cx| built(i, cx) && names(i).len() == 4 && names(i)[0] != "before");
        idx.read_with(cx, |i, _| {
            assert_eq!(names(i), ["after_the_change", "created", "keep", "renamed"]);
            let (entry, _) = i.definitions("renamed")[0];
            assert_eq!(entry.path, Some(deep.join("new_name.rs")));
        });
    }

    /// Keeps the highlighting thread busy until dropped: parses queued behind it wait.
    struct Busy(Arc<AtomicBool>);

    impl Busy {
        fn new() -> Self {
            let (open, running) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
            let (o, r) = (open.clone(), running.clone());
            drop(highlight::on_worker(move || {
                r.store(true, Ordering::Relaxed);
                while !o.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }));
            while !running.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Self(open)
        }
    }

    impl Drop for Busy {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[gpui::test]
    fn new_roots_cancel_the_build_under_way_and_start_another(cx: &mut TestAppContext) {
        let (a, b) = (temp("roots-a"), temp("roots-b"));
        write(&a.join("a.rs"), "fn in_a() {}\n");
        write(&b.join("b.rs"), "fn in_b() {}\n");
        let idx = index(cx);
        let busy = Busy::new();
        set_roots(cx, &idx, &[&a]);
        cx.run_until_parked();
        // a.rs is read and waits for the highlighting thread
        assert!(idx.read_with(cx, |i, _| i.reading.is_some()));
        let generation = idx.read_with(cx, |i, _| i.generation);
        set_roots(cx, &idx, &[&b]);
        assert_eq!(idx.read_with(cx, |i, _| i.generation), generation + 1);
        drop(busy);
        settle(cx, &idx, |i, cx| built(i, cx) && !names(i).is_empty());
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["in_b"]);
        // the same roots again: no rebuild, a rescan
        set_roots(cx, &idx, &[&b]);
        assert_eq!(idx.read_with(cx, |i, _| i.generation), generation + 1);
    }

    #[gpui::test]
    fn a_slow_result_landing_after_a_newer_one_is_dropped(cx: &mut TestAppContext) {
        let d = temp("stale");
        let x = d.join("x.rs");
        write(&x, "fn old() {}\n");
        let idx = index(cx);
        set_roots(cx, &idx, &[&d]);
        settle(cx, &idx, built);
        let first = watch::stamp(&x).unwrap();
        let old = idx.read_with(cx, |i, _| i.definitions("old")[0].0.clone());
        write(&x, "fn newer_text() {}\n");
        idx.update(cx, |i, cx| i.rescan(None, cx));
        settle(cx, &idx, |i, cx| built(i, cx) && names(i) == ["newer_text"]);
        idx.update(cx, |i, cx| {
            // the first read's result comes in late, keyed by the stamp it read
            i.file_read(i.generation, x.clone(), first, Some(old.clone()), cx);
            // and one from before the roots changed, though its stamp is current
            let now = watch::stamp(&x).unwrap();
            let stale = Entry { key: EntryKey::File(x.clone(), now), ..old.clone() };
            i.file_read(i.generation - 1, x.clone(), now, Some(stale), cx);
        });
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["newer_text"]);
    }

    fn editor(cx: &mut TestAppContext, state: EditorState) -> Entity<Editor> {
        cx.new(|cx| Editor::new(state, false, cx))
    }

    /// An untitled buffer set to Rust, as View > Syntax does (untitled text is Plain Text).
    fn untitled(cx: &mut TestAppContext, text: &str) -> Entity<Editor> {
        let e = editor(cx, EditorState::from_text(text));
        e.update(cx, |e, _| e.highlighter = Highlighter::by_name("Rust"));
        e
    }

    /// An index with an untitled Rust buffer of `text` open in it, parsed.
    fn opened(cx: &mut TestAppContext, text: &str) -> (Entity<ProjectIndex>, Entity<Editor>) {
        let idx = index(cx);
        let e = untitled(cx, text);
        idx.update(cx, |i, cx| i.open_view(&e, cx));
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        (idx, e)
    }

    /// The buffer's entry is for its text as it is now.
    fn fresh(i: &ProjectIndex, cx: &App, e: &Entity<Editor>) -> bool {
        let version = e.read(cx).doc(cx).version;
        (i.entry_for(e, cx)).is_some_and(|en| matches!(en.key, EntryKey::Buffer(_, v) if v == version))
    }

    #[gpui::test]
    fn unsaved_edits_stand_for_the_file_until_the_buffer_closes(cx: &mut TestAppContext) {
        let d = temp("unsaved");
        let x = d.join("x.rs");
        let tail = "\n".repeat(4_000);
        write(&x, &format!("fn on_disk() {{}}\n{tail}"));
        let idx = index(cx);
        set_roots(cx, &idx, &[&d]);
        settle(cx, &idx, built);
        let e = editor(cx, EditorState::load(&x).unwrap());
        idx.update(cx, |i, cx| i.open_view(&e, cx));
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["on_disk"], "once: the buffer stands for the file");
        // an edit is parsed again once the edits pause
        e.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert(&format!("fn unsaved() {{}}\n{tail}"));
            })
        });
        cx.run_until_parked();
        assert!(!idx.read_with(cx, |i, cx| fresh(i, cx, &e)));
        cx.executor().advance_clock(EDIT_DELAY);
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["unsaved"]);
        let (entry, s) = idx.read_with(cx, |i, cx| {
            let id = e.read(cx).doc(cx).id();
            assert!(i.buffers[&id].checkpoints.as_ref().unwrap().parsed <= 1_024);
            let (en, s) = i.definitions("unsaved")[0];
            (en.clone(), s.clone())
        });
        assert_eq!(entry.key, EntryKey::Buffer(e.entity_id(), e.read_with(cx, |e, cx| e.doc(cx).version)));
        assert_eq!((entry.path, s.line), (Some(x.clone()), 0));
        // closed without saving: the file's own symbols are back
        idx.update(cx, |i, cx| i.close_view(&e, cx));
        settle(cx, &idx, |i, cx| built(i, cx) && names(i) == ["on_disk"]);
    }

    #[gpui::test]
    fn a_save_typed_over_while_it_was_written_reads_the_file_not_the_buffer(cx: &mut TestAppContext) {
        let d = temp("saved-typed-over");
        let x = d.join("x.rs");
        write(&x, "fn on_disk() {}\n");
        let idx = index(cx);
        set_roots(cx, &idx, &[&d]);
        settle(cx, &idx, built);
        let e = editor(cx, EditorState::load(&x).unwrap());
        idx.update(cx, |i, cx| i.open_view(&e, cx));
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        let retype = |t: &'static str, cx: &mut TestAppContext| {
            e.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert(t)
                })
            })
        };
        retype("fn written() {}\n", cx);
        let req = e.update(cx, |e, cx| e.update_doc(cx, |d| d.capture_save(x.clone(), None)));
        let req = req.unwrap();
        let written = req.write().unwrap();
        // typed while the write was under way, and parsed before it lands
        retype("fn typed() {}\n", cx);
        idx.update(cx, |i, cx| drop(i.ensure_fresh(&e, cx)));
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        e.update(cx, |e, cx| e.update_doc(cx, |d| d.finish_save(&req, &written)));
        idx.update(cx, |i, cx| i.saved(&e, cx));
        settle(cx, &idx, built);
        // closed: the file's symbols are what was written, not the buffer's
        idx.update(cx, |i, cx| i.close_view(&e, cx));
        settle(cx, &idx, built);
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["written"]);
    }

    #[gpui::test]
    fn ensure_fresh_parses_at_once_and_ends_when_the_entry_is_current(cx: &mut TestAppContext) {
        let (idx, e) = opened(cx, "fn scratch() {}\n");
        // an untitled buffer is found by its view
        idx.read_with(cx, |i, _| {
            let (entry, _) = i.definitions("scratch")[0];
            assert_eq!((entry.key.clone(), entry.path.clone()), (EntryKey::Buffer(e.entity_id(), 0), None));
        });
        e.update(cx, |e, cx| e.edit(cx, |s| s.insert("fn more() {}\n")));
        let done = Arc::new(AtomicBool::new(false));
        let task = idx.update(cx, |i, cx| i.ensure_fresh(&e, cx));
        let d = done.clone();
        cx.spawn(async move |_| {
            task.await;
            d.store(true, Ordering::Relaxed);
        })
        .detach();
        // no wait for the edits to pause
        settle(cx, &idx, |_, _| done.load(Ordering::Relaxed));
        assert!(idx.read_with(cx, |i, cx| fresh(i, cx, &e)));
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["more", "scratch"]);
        // already current: at once
        let task = idx.update(cx, |i, cx| i.ensure_fresh(&e, cx));
        let mut waker = std::task::Context::from_waker(Waker::noop());
        assert!(std::pin::pin!(task).poll(&mut waker).is_ready());
    }

    #[gpui::test]
    fn a_buffers_parse_of_an_older_version_is_dropped(cx: &mut TestAppContext) {
        let (idx, e) = opened(cx, "fn one() {}\n");
        let old = idx.read_with(cx, |i, cx| i.entry_for(&e, cx).cloned().unwrap());
        e.update(cx, |e, cx| e.edit(cx, |s| s.insert("fn two() {}\n")));
        let task = idx.update(cx, |i, cx| i.ensure_fresh(&e, cx));
        drop(task);
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        let id = e.read_with(cx, |e, cx| e.doc(cx).id());
        idx.update(cx, |i, cx| i.buffer_read(id, old, None, cx));
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["one", "two"]);
    }

    #[gpui::test]
    fn incremental_checkpoints_follow_the_remaining_view_when_the_first_closes(cx: &mut TestAppContext) {
        let idx = index(cx);
        let first = untitled(cx, &nested(70));
        let other = first.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
        idx.update(cx, |i, cx| {
            i.open_view(&first, cx);
            i.open_view(&other, cx);
        });
        settle(cx, &idx, |i, cx| fresh(i, cx, &other));
        let busy = Busy::new();
        other.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(s.line_start(1_500));
                s.insert("fn added() {}\n");
            })
        });
        // the first view applies the other's edit as the document's change is delivered
        cx.run_until_parked();
        idx.update(cx, |i, cx| i.ensure_fresh(&first, cx)).detach();
        cx.run_until_parked();
        idx.update(cx, |i, cx| i.close_view(&first, cx));
        drop(busy);
        settle(cx, &idx, |i, cx| fresh(i, cx, &other));
        idx.read_with(cx, |i, cx| {
            let entry = i.entry_for(&other, cx).unwrap();
            assert_eq!(entry.key, EntryKey::Buffer(other.entity_id(), other.read(cx).doc(cx).version));
            assert_eq!(*entry.symbols, parse("x.rs", &other.read(cx).doc(cx).text()));
            let checkpoints = i.buffers[&other.read(cx).doc(cx).id()].checkpoints.as_ref().unwrap();
            assert!(checkpoints.parsed <= 1_025);
        });
    }

    #[gpui::test]
    fn edits_wait_300_ms_after_the_last_edit_then_reuse_checkpoints(cx: &mut TestAppContext) {
        let (idx, e) = opened(cx, &nested(70));
        for name in ["first", "second"] {
            e.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.set_cursor(s.line_start(1_500));
                    s.insert(&format!("fn {name}() {{}}\n"));
                })
            });
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(150));
        }
        cx.run_until_parked();
        idx.read_with(cx, |i, cx| {
            let b = &i.buffers[&e.read(cx).doc(cx).id()];
            assert!(b.wait.is_some() && b.parse.is_none());
            assert!(!fresh(i, cx, &e));
        });
        cx.executor().advance_clock(Duration::from_millis(150));
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        idx.read_with(cx, |i, cx| {
            let b = &i.buffers[&e.read(cx).doc(cx).id()];
            assert!(b.checkpoints.as_ref().unwrap().parsed <= 1_026);
            assert_eq!(*i.entry_for(&e, cx).unwrap().symbols, parse("x.rs", &e.read(cx).doc(cx).text()));
        });
    }

    #[gpui::test]
    fn final_automatic_completion_preserves_same_syntax_edit_debounce(cx: &mut TestAppContext) {
        let idx = index(cx);
        let (editor, vcx) = cx.add_window_view(|_, cx| {
            let mut state = EditorState::from_text("fn target() {}\ntar");
            state.set_cursor(state.len());
            let mut editor = Editor::new(state, false, cx);
            editor.highlighter = Highlighter::by_name("Rust");
            editor
        });
        editor.update_in(vcx, |e, window, _| {
            e.completion_index = Some(idx.downgrade());
            window.activate_window();
            e.focus(window);
        });
        idx.update(vcx, |i, cx| i.open_view(&editor, cx));
        settle(&mut vcx.cx, &idx, |i, cx| fresh(i, cx, &editor));
        let held = Busy::new();
        vcx.simulate_input("g");
        vcx.run_until_parked();
        assert!(editor.read_with(vcx, |e, _| e.completion_waiting().is_some()), "typing must open completion");
        idx.read_with(vcx, |i, cx| {
            let b = &i.buffers[&editor.read(cx).doc(cx).id()];
            assert!(b.wait.is_some() && b.parse.is_none(), "completion bypassed the edit debounce");
            assert!(!fresh(i, cx, &editor));
        });
        drop(held);
        vcx.executor().advance_clock(EDIT_DELAY);
        settle(&mut vcx.cx, &idx, |i, cx| fresh(i, cx, &editor));
    }

    #[gpui::test]
    fn closing_the_indexed_view_refreshes_the_remaining_views_syntax(cx: &mut TestAppContext) {
        let idx = index(cx);
        let rust = untitled(cx, "fn kept() {}\n");
        let plain = rust.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
        plain.update(cx, |e, _| e.highlighter = Highlighter::by_name("Plain Text"));
        idx.update(cx, |i, cx| {
            i.open_view(&rust, cx);
            i.open_view(&plain, cx);
        });
        settle(cx, &idx, |i, cx| fresh(i, cx, &rust));
        idx.update(cx, |i, cx| i.ensure_fresh(&plain, cx)).detach();
        settle(cx, &idx, |i, cx| i.entry_for(&plain, cx).is_some_and(|e| e.syntax == "Plain Text"));
        idx.update(cx, |i, cx| {
            i.close_view(&plain, cx);
            assert!(i.buffers[&rust.read(cx).doc(cx).id()].parse.is_some());
        });
        settle(cx, &idx, |i, cx| i.entry_for(&rust, cx).is_some_and(|e| e.syntax == "Rust"));
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["kept"]);
    }

    #[gpui::test]
    fn index_files_off_leaves_out_the_folders_but_not_the_open_buffers(cx: &mut TestAppContext) {
        let d = temp("off");
        write(&d.join("x.rs"), "fn on_disk() {}\n");
        cx.update(|cx| cx.set_global(Settings { index_files: false, ..Settings::default() }));
        let idx = cx.new(ProjectIndex::new);
        set_roots(cx, &idx, &[&d]);
        let e = untitled(cx, "fn in_buffer() {}\n");
        idx.update(cx, |i, cx| i.open_view(&e, cx));
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        idx.read_with(cx, |i, _| {
            assert_eq!(names(i), ["in_buffer"]);
            assert_eq!(i.progress(), None);
        });
        // turned on: a build
        cx.update(|cx| cx.set_global(Settings::default()));
        settle(cx, &idx, |i, cx| built(i, cx) && names(i).len() == 2);
        // a pattern that leaves the file out: built again without it
        cx.update(|cx| cx.set_global(Settings { index_exclude_patterns: vec!["*.rs".into()], ..Settings::default() }));
        settle(cx, &idx, |i, cx| built(i, cx) && names(i) == ["in_buffer"]);
        cx.update(|cx| cx.set_global(Settings::default()));
        settle(cx, &idx, |i, cx| built(i, cx) && names(i).len() == 2);
        // off again: the folders' entries go at once
        cx.update(|cx| cx.set_global(Settings { index_files: false, ..Settings::default() }));
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["in_buffer"]);
    }

    #[gpui::test]
    fn navigation_replacing_a_buffer_rebinds_its_index_and_rejects_the_old_job(cx: &mut TestAppContext) {
        let (idx, e) = opened(cx, "fn previous() {}\n");
        let old_id = e.read_with(cx, |e, cx| e.doc(cx).id());
        let old = idx.read_with(cx, |i, cx| i.entry_for(&e, cx).unwrap().clone());
        e.update(cx, |e, cx| e.set_text("fn replacement() {}\n", cx));
        cx.run_until_parked();
        // new text in the same document: its entry is for the new version, not the old one
        let (new_id, version) = e.read_with(cx, |e, cx| (e.doc(cx).id(), e.doc(cx).version));
        assert_eq!(new_id, old_id);
        idx.read_with(cx, |i, _| assert!(i.buffers.contains_key(&new_id)));
        assert_ne!(old.key, EntryKey::Buffer(e.entity_id(), version));
        idx.update(cx, |i, cx| i.buffer_read(old_id, old, None, cx));
        // an edit is parsed once the edits pause
        cx.executor().advance_clock(EDIT_DELAY);
        settle(cx, &idx, |i, cx| fresh(i, cx, &e));
        assert_eq!(idx.read_with(cx, |i, _| names(i)), ["replacement"]);
    }

    #[gpui::test]
    fn navigation_a_stale_syntax_parse_cannot_cancel_the_new_syntax_job(cx: &mut TestAppContext) {
        let (idx, e) = opened(cx, "fn previous() {}\n");
        let old = idx.read_with(cx, |i, cx| i.entry_for(&e, cx).unwrap().clone());
        e.update(cx, |e, cx| {
            e.highlighter = Highlighter::by_name("Plain Text");
            cx.notify();
        });
        let id = e.read_with(cx, |e, cx| e.doc(cx).id());
        idx.update(cx, |i, cx| {
            i.ensure_fresh(&e, cx).detach();
            i.buffer_read(id, old, None, cx);
            assert!(i.buffers[&id].parse.as_ref().is_some_and(|(key, _)| key.1 == "Plain Text"));
        });
    }

    #[gpui::test]
    fn navigation_an_in_flight_parse_uses_the_buffers_current_path(cx: &mut TestAppContext) {
        let (idx, e) = opened(cx, "fn previous() {}\n");
        let old = idx.read_with(cx, |i, cx| i.entry_for(&e, cx).unwrap().clone());
        let id = e.read_with(cx, |e, cx| e.doc(cx).id());
        e.update(cx, |e, cx| {
            e.update_doc(cx, |d| d.path = Some("/tmp/renamed.rs".into()));
            cx.notify();
        });
        cx.run_until_parked();
        idx.update(cx, |i, cx| i.buffer_read(id, old, None, cx));
        assert_eq!(
            idx.read_with(cx, |i, cx| i.entry_for(&e, cx).unwrap().path.clone()),
            Some("/tmp/renamed.rs".into())
        );
    }
}
