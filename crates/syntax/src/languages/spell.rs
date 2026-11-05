//! Scope-aware visible and complete spelling, sharing filtering, checking and cache keys.

#[path = "spell/platform.rs"]
pub mod platform;

use crate::highlight::{self, receive, Highlighter, Slot};
use ropey::{Rope, RopeSlice};
use std::collections::{HashSet, VecDeque};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

pub use orlo_config::settings::DEFAULT_SELECTOR;
pub static DICTIONARY_GENERATION: AtomicU64 = AtomicU64::new(0);
pub const CHUNK_CHARS: usize = 2048;
const CONTEXT_CHARS: usize = 256;
const MAX_REQUESTS: usize = 32;
const MAX_CACHE: usize = 512;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static FORBID_PREPARATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn without_preparation<T>(f: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FORBID_PREPARATION.set(self.0);
        }
    }
    let _reset = Reset(FORBID_PREPARATION.replace(true));
    f()
}

/// Native misspelling plus its containing native source word, in UTF-8 bytes.
#[derive(Debug)]
pub struct Misspelling {
    pub range: Range<usize>,
    pub source: Range<usize>,
}

pub trait Checker {
    fn misspelled(&self, text: &str, stop: &AtomicBool) -> Vec<Misspelling>;
    /// Native UTF-8 word range at a caret, including a word's end.
    fn word_at(&self, _text: &str, _at: usize, _stop: &AtomicBool) -> Option<Range<usize>> {
        None
    }
    fn guesses(&self, _word: &str) -> Vec<String> {
        Vec::new()
    }
    fn has_learned_word(&self, _word: &str) -> bool {
        false
    }
    fn ignore_word(&self, _word: &str) {}
    fn learn_word(&self, _word: &str) {}
    fn unlearn_word(&self, _word: &str) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub line: usize,
    /// Character offsets within the line, excluding its break.
    pub chars: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub version: u64,
    pub syntax: String,
    pub selector: String,
    pub generation: u64,
}

#[derive(Clone)]
struct Key {
    // Rope slices share their text: even a 99 MB line is never copied or compared on the UI.
    text: Rope,
    segments: Vec<Range<usize>>,
    generation: u64,
}

pub struct Prepared {
    region: Region,
    key: Key,
    context: Range<usize>,
    text: String,
    eligible: Vec<Range<usize>>,
}

#[derive(Clone)]
pub struct Checked {
    region: Region,
    key: Key,
    ranges: Vec<Range<usize>>,
}

#[derive(Default)]
pub struct Cache {
    stamp: Option<Stamp>,
    entries: VecDeque<Checked>,
}

impl Cache {
    pub fn clear(&mut self) {
        self.stamp = None;
        self.entries.clear();
    }

    pub fn sync(&mut self, stamp: Stamp) -> bool {
        if self.stamp.as_ref() != Some(&stamp) {
            self.entries.clear();
            self.stamp = Some(stamp);
            return true;
        }
        false
    }

    pub fn adopt(&mut self, stamp: &Stamp, entries: Vec<Checked>) {
        if self.stamp.as_ref() != Some(stamp) {
            return;
        }
        for entry in entries {
            if entry.key.generation != stamp.generation {
                continue;
            }
            self.entries.retain(|e| e.region != entry.region);
            self.entries.push_back(entry);
            if self.entries.len() > MAX_CACHE {
                self.entries.pop_front();
            }
        }
    }

    pub fn ranges(&self, line: usize) -> impl Iterator<Item = &Range<usize>> {
        self.entries.iter().filter(move |e| e.region.line == line).flat_map(|e| &e.ranges)
    }

    pub fn missing(&self, visible: &[Region]) -> Vec<Region> {
        let mut out = Vec::new();
        for region in visible {
            let mut from = region.chars.start;
            while from < region.chars.end && out.len() < MAX_REQUESTS {
                let chars = from..(from + CHUNK_CHARS).min(region.chars.end);
                let covered = self.entries.iter().any(|e| {
                    e.region.line == region.line
                        && e.region.chars.start <= chars.start
                        && e.region.chars.end >= chars.end
                });
                let request = Region { line: region.line, chars: chars.clone() };
                if !covered && !out.contains(&request) {
                    out.push(request);
                }
                from = chars.end;
            }
            if out.len() == MAX_REQUESTS {
                break;
            }
        }
        out
    }
}

/// Called only on the highlighting worker: scope parsing never migrates to spelling's thread.
pub fn prepare(
    rope: &Rope,
    highlighter: Highlighter,
    regions: Vec<Region>,
    selector: &str,
    generation: u64,
    stop: &AtomicBool,
) -> Vec<Prepared> {
    prepare_batch(rope, highlighter, regions, selector, generation, false, stop).1
}

fn token_break(c: char) -> bool {
    c.is_whitespace() || "[](){}<>\"`".contains(c)
}

fn remaining_start(region: &Region, prepared: &[Prepared]) -> usize {
    let mut from = region.chars.start;
    while let Some(end) = prepared
        .iter()
        .filter(|p| p.region.line == region.line && p.region.chars.start <= from && p.region.chars.end > from)
        .map(|p| p.region.chars.end)
        .max()
    {
        from = end;
    }
    from
}

fn covered(region: &Region, prepared: &[Prepared]) -> bool {
    if region.chars.is_empty() {
        prepared.iter().any(|p| {
            p.region.line == region.line
                && p.region.chars.start <= region.chars.start
                && p.region.chars.end >= region.chars.end
        })
    } else {
        remaining_start(region, prepared) >= region.chars.end
    }
}

fn complete_region(line: RopeSlice<'_>, mut region: Region, stop: &AtomicBool) -> Option<Region> {
    if line.get_char(region.chars.start).is_some_and(|c| !token_break(c)) {
        region.chars.start -= token_len(line.chars_at(region.chars.start).reversed(), stop)?;
    }
    if region.chars.end > 0 && !token_break(line.char(region.chars.end - 1)) {
        region.chars.end += token_len(line.chars_at(region.chars.end), stop)?;
    }
    Some(region)
}

/// How many of `chars` come before a token break; `None` once `stop` is set.
fn token_len(chars: impl Iterator<Item = char>, stop: &AtomicBool) -> Option<usize> {
    let mut len = 0;
    for c in chars {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        if token_break(c) {
            break;
        }
        len += 1;
    }
    Some(len)
}

// Eligibility runs on the highlighting worker, before native word discovery.
fn eligible_ranges(text: &str, stop: &AtomicBool) -> Option<Vec<Range<usize>>> {
    #[cfg(any(test, feature = "test-support"))]
    eligibility_stage();
    let mut eligible = Vec::new();
    let mut offset = 0;
    for chunk in text.split_inclusive(|c| token_break(c) || stop.load(Ordering::Relaxed)) {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        let base = offset;
        offset += chunk.len();
        let token = chunk.trim_end_matches(token_break);
        let mut address = token;
        while let Some(c) = address.chars().next_back() {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if !['.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"', '`'].contains(&c) {
                break;
            }
            address = &address[..address.len() - c.len_utf8()];
        }
        if token.bytes().any(|b| stop.load(Ordering::Relaxed) || matches!(b, b'@' | b'/' | b'\\'))
            || address.bytes().any(|b| stop.load(Ordering::Relaxed) || b == b'.')
        {
            continue;
        }
        // Natural-language boundaries belong to the native checker. These
        // cancellable intervals only exclude lexical addresses and paths.
        if token.is_empty() {
            continue;
        }
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        eligible.push(base..base + token.len());
    }
    Some(eligible)
}

/// Visible requests keep the UAX #29 word prefilter, over at most a chunk and its context on
/// each side. Complete requests never call this iterator, which cannot be interrupted.
fn visible_eligible(
    text: &str,
    tokens: Vec<Range<usize>>,
    context: &Range<usize>,
    line_chars: usize,
    stop: &AtomicBool,
) -> Option<Vec<Range<usize>>> {
    use unicode_segmentation::UnicodeSegmentation;
    debug_assert!(text.len() <= (CHUNK_CHARS + 2 * CONTEXT_CHARS) * 4);
    let mut eligible = Vec::new();
    for token in tokens {
        for (byte, word) in text[token.clone()].unicode_word_indices() {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let range = token.start + byte..token.start + byte + word.len();
            if source_eligible(text, &range, stop)
                && !(range.start == 0 && context.start > 0 || range.end == text.len() && context.end < line_chars)
            {
                eligible.push(range);
            }
        }
    }
    Some(eligible)
}

impl Prepared {
    /// Whether native word `source`, holding `word` (bytes of `text`, which starts at line byte
    /// `base`), may be reported: eligible, and in the scopes the selector picks.
    fn admits(&self, source: &Range<usize>, word: &Range<usize>, base: usize, stop: &AtomicBool) -> bool {
        // Visible checks omit only native words cut by the fixed context margin.
        let clipped = source.start == 0 && self.context.start > 0
            || source.end == self.text.len() && self.context.end < self.key.text.len_chars();
        !clipped
            && source_eligible(&self.text, source, stop)
            && (self.eligible.iter()).any(|e| e.start <= word.start && e.end >= word.end)
            && (self.key.segments.iter()).any(|s| s.start <= base + word.start && s.end >= base + word.end)
    }
}

/// Source identifiers can contain a smaller native misspelling (e.g. a possessive
/// stem). Filter its native envelope so those subranges cannot bypass exclusions.
pub fn source_eligible(text: &str, source: &Range<usize>, stop: &AtomicBool) -> bool {
    let Some(word) = text.get(source.clone()).filter(|w| !w.is_empty()) else {
        return false;
    };
    // The platform splits underscores; protect either adjacent identifier piece.
    if text[..source.start].ends_with('_') || text[source.end..].starts_with('_') {
        return false;
    }
    let (mut lower, mut inner_upper, mut alphanumeric) = (false, false, false);
    for (i, c) in word.chars().enumerate() {
        if stop.load(Ordering::Relaxed) || c.is_numeric() || c == '_' {
            return false;
        }
        lower |= c.is_lowercase();
        inner_upper |= i > 0 && c.is_uppercase();
        alphanumeric |= c.is_alphanumeric();
    }
    alphanumeric && !(lower && inner_upper)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static ELIGIBILITY_STAGE: std::cell::RefCell<Option<std::sync::mpsc::Sender<()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
fn eligibility_stage() {
    ELIGIBILITY_STAGE.with_borrow_mut(|stage| {
        if let Some(stage) = stage.take() {
            stage.send(()).unwrap();
        }
    });
}

fn prepare_batch(
    rope: &Rope,
    highlighter: Highlighter,
    regions: Vec<Region>,
    selector: &str,
    generation: u64,
    complete: bool,
    stop: &AtomicBool,
) -> (Highlighter, Vec<Prepared>) {
    #[cfg(any(test, feature = "test-support"))]
    assert!(!FORBID_PREPARATION.get(), "spelling preparation ran synchronously on the UI path");
    if stop.load(Ordering::Relaxed) {
        return (highlighter, Vec::new());
    }
    let lines: Vec<_> = regions.iter().map(|r| r.line).collect();
    let highlighter = highlight::run_job(highlighter, rope, &lines);
    let mut out = Vec::new();
    for mut region in regions.into_iter().take(MAX_REQUESTS) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let line = rope.line(region.line);
        if complete {
            if covered(&region, &out) {
                continue;
            }
            region.chars.start = remaining_start(&region, &out);
            let Some(expanded) = complete_region(line, region, stop) else {
                break;
            };
            region = expanded;
        } else {
            // Keep the library word walk and native rendering input bounded even
            // if a caller supplies a region larger than Cache::missing produces.
            region.chars.end = region.chars.end.min(region.chars.start.saturating_add(CHUNK_CHARS));
        }
        let Some(runs) = highlighter.cached_scope_runs(rope, region.line) else {
            continue;
        };
        let mut byte = 0;
        let mut segments: Vec<Range<usize>> = Vec::new();
        for &(len, id) in runs {
            if highlight::matches(id, selector) {
                if let Some(last) = segments.last_mut().filter(|s| s.end == byte) {
                    last.end += len;
                } else {
                    segments.push(byte..byte + len);
                }
            }
            byte += len;
        }
        let margin = if complete { 1 } else { CONTEXT_CHARS };
        let context = region.chars.start.saturating_sub(margin)..(region.chars.end + margin).min(line.len_chars());
        let slice = line.slice(context.clone());
        let mut text = String::with_capacity(slice.len_bytes());
        for chunk in slice.chunks() {
            if stop.load(Ordering::Relaxed) {
                return (highlighter, out);
            }
            text.push_str(chunk);
        }
        let Some(mut eligible) = eligible_ranges(&text, stop) else {
            return (highlighter, out);
        };
        if !complete {
            let Some(visible) = visible_eligible(&text, eligible, &context, line.len_chars(), stop) else {
                return (highlighter, out);
            };
            eligible = visible;
        }
        out.push(Prepared {
            region,
            key: Key { text: Rope::from(line), segments, generation },
            context,
            text,
            eligible,
        });
    }
    (highlighter, out)
}

#[cfg(any(test, feature = "test-support"))]
pub fn check(prepared: Vec<Prepared>, checker: &dyn Checker, stop: &AtomicBool) -> Vec<Checked> {
    check_ignoring(&prepared, checker, stop, &HashSet::new())
}

fn check_ignoring(
    prepared: &[Prepared],
    checker: &dyn Checker,
    stop: &AtomicBool,
    ignored: &HashSet<String>,
) -> Vec<Checked> {
    let mut out = Vec::new();
    for p in prepared {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let mut ranges = Vec::new();
        if !p.eligible.is_empty() && !p.key.segments.is_empty() {
            let base = p.key.text.char_to_byte(p.context.start);
            for error in checker.misspelled(&p.text, stop) {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let r = error.range;
                if error.source.start > r.start
                    || error.source.end < r.end
                    || r.start >= r.end
                    || !p.text.is_char_boundary(r.start)
                    || !p.text.is_char_boundary(r.end)
                    || ignored.contains(&p.text[r.clone()])
                    || !p.admits(&error.source, &r, base, stop)
                {
                    continue;
                }
                let start = p.key.text.byte_to_char(base + r.start);
                let end = p.key.text.byte_to_char(base + r.end);
                if start < p.region.chars.end && end > p.region.chars.start {
                    ranges.push(start..end);
                }
            }
        }
        out.push(Checked { region: p.region.clone(), key: p.key.clone(), ranges });
    }
    out
}

/// Complete token boundaries, retaining prepared inputs while retrying yielded scopes.
pub async fn prepare_complete(
    rope: Rope,
    mut highlighter: Highlighter,
    regions: Vec<Region>,
    selector: String,
    generation: u64,
    stop: Arc<AtomicBool>,
) -> Option<(Highlighter, Vec<Prepared>)> {
    let mut ready = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) || DICTIONARY_GENERATION.load(Ordering::Relaxed) != generation {
            return None;
        }
        let requested: Vec<_> = regions
            .iter()
            .filter(|r| !covered(r, &ready))
            .map(|r| Region { line: r.line, chars: remaining_start(r, &ready)..r.chars.end })
            .collect();
        if requested.is_empty() {
            return Some((highlighter, ready));
        }
        let text = rope.clone();
        let select = selector.clone();
        let flag = stop.clone();
        let (job, prepared) = highlight::on_worker(move || {
            prepare_batch(&text, highlighter, requested, &select, generation, true, &flag)
        })
        .await?;
        highlighter = job;
        ready.extend(prepared);
    }
}

/// The background spelling service could not complete a scan.
#[derive(Debug)]
pub struct SpellCheckUnavailable;

impl std::fmt::Display for SpellCheckUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("System spell checking is unavailable")
    }
}

impl std::error::Error for SpellCheckUnavailable {}

/// Scan scopes in ascending batches so the highlighting state advances even for Previous.
/// Complete tokens may exceed visible-input bounds; each 500-line chunk is covered without truncation.
pub async fn scan(
    rope: Rope,
    highlighter: Highlighter,
    anchor: usize,
    forward: bool,
    selector: String,
    stop: Arc<AtomicBool>,
) -> Result<Option<Range<usize>>, SpellCheckUnavailable> {
    scan_inner(rope, highlighter, anchor, forward, selector, stop, None).await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn scan_with_checker(
    rope: Rope,
    highlighter: Highlighter,
    anchor: usize,
    forward: bool,
    selector: String,
    stop: Arc<AtomicBool>,
    checker: Option<Arc<dyn Checker + Send + Sync>>,
) -> Result<Option<Range<usize>>, SpellCheckUnavailable> {
    scan_inner(rope, highlighter, anchor, forward, selector, stop, checker).await
}

async fn scan_inner(
    rope: Rope,
    mut highlighter: Highlighter,
    anchor: usize,
    forward: bool,
    selector: String,
    stop: Arc<AtomicBool>,
    checker: Option<Arc<dyn Checker + Send + Sync>>,
) -> Result<Option<Range<usize>>, SpellCheckUnavailable> {
    let generation = DICTIONARY_GENERATION.load(Ordering::Relaxed);
    highlighter = highlighter.job(0);
    highlighter.cancel_on(stop.clone());
    let at = rope.char_to_line(anchor.min(rope.len_chars()));
    let passes = if forward { [at..rope.len_lines(), 0..at + 1] } else { [0..at + 1, at..rope.len_lines()] };
    for (pass, lines) in passes.into_iter().enumerate() {
        let mut best: Option<Range<usize>> = None;
        for chunk in (lines.start..lines.end).step_by(500) {
            let end = (chunk + 500).min(lines.end);
            let (mut line, mut from) = (chunk, 0);
            while line < end {
                if stop.load(Ordering::Relaxed) || DICTIONARY_GENERATION.load(Ordering::Relaxed) != generation {
                    return Ok(None);
                }
                let mut regions = Vec::new();
                while line < end && regions.len() < MAX_REQUESTS {
                    let len = rope.line(line).len_chars();
                    if from < len {
                        let to = (from + CHUNK_CHARS).min(len);
                        regions.push(Region { line, chars: from..to });
                        from = to;
                    } else {
                        line += 1;
                        from = 0;
                    }
                }
                let (job, prepared) =
                    prepare_complete(rope.clone(), highlighter, regions, selector.clone(), generation, stop.clone())
                        .await
                        .ok_or(SpellCheckUnavailable)?;
                highlighter = job;
                for p in &prepared {
                    if p.region.line == line {
                        from = from.max(p.region.chars.end);
                    }
                }
                if stop.load(Ordering::Relaxed) || DICTIONARY_GENERATION.load(Ordering::Relaxed) != generation {
                    return Ok(None);
                }
                let checked =
                    on_worker_inner(prepared, stop.clone(), checker.clone()).await.ok_or(SpellCheckUnavailable)?;
                for entry in checked {
                    let base = rope.line_to_char(entry.region.line);
                    for r in entry.ranges {
                        let r = base + r.start..base + r.end;
                        let first_pass = if forward { r.start >= anchor } else { r.end <= anchor };
                        if first_pass != (pass == 0) {
                            continue;
                        }
                        if forward {
                            return Ok(Some(r));
                        }
                        if best.as_ref().is_none_or(|old| r.start > old.start) {
                            best = Some(r);
                        }
                    }
                }
            }
        }
        if best.is_some() {
            return Ok(best);
        }
    }
    Ok(None)
}

#[derive(Clone)]
pub enum WordAction {
    Replace(String),
    Ignore,
    Learn,
    Unlearn,
}

pub struct Suggestions {
    pub range: Range<usize>,
    pub word: String,
    pub guesses: Vec<String>,
    pub learned: bool,
}

pub enum WordWork {
    Inspect { prepared: Vec<Prepared>, line_base: usize, at: usize },
    Change { word: String, action: WordAction },
}

pub enum WordAnswer {
    Suggestions(Option<Suggestions>),
    Changed,
}

fn word_work(work: WordWork, checker: &dyn Checker, stop: &AtomicBool, ignored: &mut HashSet<String>) -> WordAnswer {
    match work {
        WordWork::Change { word, action } => {
            match action {
                WordAction::Ignore => {
                    checker.ignore_word(&word);
                    ignored.insert(word);
                }
                WordAction::Learn => checker.learn_word(&word),
                WordAction::Unlearn => checker.unlearn_word(&word),
                WordAction::Replace(_) => unreachable!("replacements belong to the editor"),
            }
            DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
            WordAnswer::Changed
        }
        WordWork::Inspect { prepared, line_base, at } => {
            WordAnswer::Suggestions(inspect(&prepared, line_base, at, checker, stop, ignored))
        }
    }
}

/// The misspelling at `at`, else the learned word there; `None` as soon as `stop` is set.
fn inspect(
    prepared: &[Prepared],
    line_base: usize,
    at: usize,
    checker: &dyn Checker,
    stop: &AtomicBool,
    ignored: &HashSet<String>,
) -> Option<Suggestions> {
    let go = || (!stop.load(Ordering::Relaxed)).then_some(());
    go()?;
    for entry in check_ignoring(prepared, checker, stop, ignored) {
        for range in entry.ranges {
            go()?;
            if range.start <= at && at <= range.end {
                let word = entry.key.text.slice(range.clone()).to_string();
                go()?;
                let guesses = checker.guesses(&word);
                go()?;
                let learned = checker.has_learned_word(&word);
                go()?;
                return Some(Suggestions {
                    range: line_base + range.start..line_base + range.end,
                    guesses,
                    learned,
                    word,
                });
            }
        }
    }
    for p in prepared {
        go()?;
        if at < p.context.start || at > p.context.end {
            continue;
        }
        let base = p.key.text.char_to_byte(p.context.start);
        let byte = p.key.text.char_to_byte(at) - base;
        let Some(bytes) = checker.word_at(&p.text, byte, stop) else {
            continue;
        };
        go()?;
        if !p.admits(&bytes, &bytes, base, stop) {
            continue;
        }
        let word = &p.text[bytes.clone()];
        go()?;
        let learned = checker.has_learned_word(word);
        go()?;
        if learned {
            let guesses = checker.guesses(word);
            go()?;
            let range = p.key.text.byte_to_char(base + bytes.start)..p.key.text.byte_to_char(base + bytes.end);
            return Some(Suggestions {
                range: line_base + range.start..line_base + range.end,
                word: word.into(),
                guesses,
                learned: true,
            });
        }
    }
    None
}

/// Word inspection and dictionary changes share the exact owner and queue used by checks.
pub fn word_on_worker(work: WordWork, stop: Arc<AtomicBool>) -> impl std::future::Future<Output = Option<WordAnswer>> {
    word_on_worker_inner(work, stop, None)
}

#[cfg(any(test, feature = "test-support"))]
pub fn word_on_worker_with_checker(
    work: WordWork,
    stop: Arc<AtomicBool>,
    checker: Option<Arc<dyn Checker + Send + Sync>>,
) -> impl std::future::Future<Output = Option<WordAnswer>> {
    word_on_worker_inner(work, stop, checker)
}

fn word_on_worker_inner(
    work: WordWork,
    stop: Arc<AtomicBool>,
    checker: Option<Arc<dyn Checker + Send + Sync>>,
) -> impl std::future::Future<Output = Option<WordAnswer>> {
    submit(checker, move |checker, ignored| word_work(work, checker, &stop, ignored))
}

type Answer = Option<Vec<Checked>>;

/// Run on the spelling thread with its native checker (when it could make one) and the words
/// ignored so far.
type Work = Box<dyn FnOnce(Option<&dyn Checker>, &mut HashSet<String>) + Send>;

/// Queue `f` with `checker`, else the native one; `None` when there is neither or `f` panicked.
fn submit<T: Send + 'static>(
    checker: Option<Arc<dyn Checker + Send + Sync>>,
    f: impl FnOnce(&dyn Checker, &mut HashSet<String>) -> T + Send + 'static,
) -> impl std::future::Future<Output = Option<T>> {
    let slot = Slot::shared();
    let to = slot.clone();
    let work: Work = Box::new(move |native, ignored| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let checker = checker.as_deref().map(|c| c as &dyn Checker).or(native);
            checker.map(|checker| f(checker, ignored))
        }));
        Slot::fill(&to, result.ok().flatten());
    });
    if worker().send(work).is_err() {
        Slot::fill(&slot, None);
    }
    receive(slot)
}

static OWNER: OnceLock<std::thread::ThreadId> = OnceLock::new();

pub fn is_owner_thread() -> bool {
    OWNER.get() == Some(&std::thread::current().id())
}

fn worker() -> &'static std::sync::mpsc::Sender<Work> {
    static WORKER: OnceLock<std::sync::mpsc::Sender<Work>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Work>();
        std::thread::Builder::new()
            .name("orlo-spell".into())
            .spawn(move || {
                OWNER.set(std::thread::current().id()).expect("one spelling owner");
                let checker = platform::SpellChecker::new().ok();
                let mut ignored = HashSet::new();
                for work in rx {
                    work(checker.as_ref().map(|c| c as &dyn Checker), &mut ignored);
                }
            })
            .expect("start the spelling thread");
        tx
    })
}

/// One thread owns the shared native checker and every private tag; no Cocoa object crosses the channel.
pub fn on_worker(prepared: Vec<Prepared>, stop: Arc<AtomicBool>) -> impl std::future::Future<Output = Answer> {
    on_worker_inner(prepared, stop, None)
}

#[cfg(any(test, feature = "test-support"))]
pub fn on_worker_with_checker(
    prepared: Vec<Prepared>,
    stop: Arc<AtomicBool>,
    checker: Option<Arc<dyn Checker + Send + Sync>>,
) -> impl std::future::Future<Output = Answer> {
    on_worker_inner(prepared, stop, checker)
}

fn on_worker_inner(
    prepared: Vec<Prepared>,
    stop: Arc<AtomicBool>,
    checker: Option<Arc<dyn Checker + Send + Sync>>,
) -> impl std::future::Future<Output = Answer> {
    submit(checker, move |checker, ignored| check_ignoring(&prepared, checker, &stop, ignored))
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_on_worker(test: impl FnOnce() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel();
    worker()
        .send(Box::new(move |_, _| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test));
            let _ = tx.send(result);
        }))
        .expect("queue native spelling test");
    if let Err(panic) = rx.recv_timeout(std::time::Duration::from_secs(20)).expect("native spelling test completed") {
        std::panic::resume_unwind(panic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    use unicode_segmentation::UnicodeSegmentation;

    struct EveryWord(AtomicUsize);
    impl Checker for EveryWord {
        fn misspelled(&self, text: &str, _stop: &AtomicBool) -> Vec<Misspelling> {
            self.0.fetch_add(1, Ordering::Relaxed);
            text.unicode_word_indices()
                .map(|(b, w)| Misspelling { range: b..b + w.len(), source: b..b + w.len() })
                .collect()
        }
    }
    /// Plain-text preparation with the default selector; `complete` widens to whole words.
    fn plain(rope: &Rope, regions: Vec<Region>, complete: bool, stop: &AtomicBool) -> Vec<Prepared> {
        let plain = Highlighter::by_name("Plain Text");
        prepare_batch(rope, plain, regions, DEFAULT_SELECTOR, 0, complete, stop).1
    }
    fn line(line: usize, chars: std::ops::Range<usize>) -> Vec<Region> {
        vec![Region { line, chars }]
    }
    fn checked(text: &str, syntax: &str, selector: &str) -> Vec<String> {
        let rope = Rope::from_str(text);
        let regions =
            (0..rope.len_lines()).map(|line| Region { line, chars: 0..rope.line(line).len_chars() }).collect();
        let stop = AtomicBool::new(false);
        let prepared = prepare(&rope, Highlighter::by_name(syntax), regions, selector, 0, &stop);
        let words = check(prepared, &EveryWord(AtomicUsize::new(0)), &stop);
        words
            .iter()
            .flat_map(|w| w.ranges.iter().map(|r| rope.line(w.region.line).slice(r.clone()).to_string()))
            .collect()
    }
    #[test]
    fn default_selector_checks_prose_and_excludes_code_and_links() {
        let text = "// mispelled comment\nlet mispelled = \"mispelled string\";\n";
        assert_eq!(checked(text, "Rust", DEFAULT_SELECTOR), ["mispelled", "comment", "mispelled", "string"]);
        assert_eq!(checked("mispelled plain prose", "Plain Text", DEFAULT_SELECTOR), ["mispelled", "plain", "prose"]);
        assert_eq!(
            checked("[mispelled](https://mispelled.example) `mispelled`", "Markdown", DEFAULT_SELECTOR),
            ["mispelled", "mispelled"]
        );
        assert_eq!(checked(text, "Rust", "keyword"), ["let"]);
        assert!(checked(text, "Rust", "invalid selector (((").is_empty());
    }
    #[test]
    fn filters_identifiers_addresses_and_paths_but_keeps_unicode_prose() {
        assert_eq!(checked("mispelled word2 under_score camelCase PascalCase Capital HELLO café naïve don't https://mispelled.test mail@mispelled.test /mispelled/file ../mispelled ~/mispelled C:\\mispelled file.txt www.mispelled.test", "Plain Text", DEFAULT_SELECTOR), ["mispelled", "Capital", "HELLO", "café", "naïve", "don't"]);
        assert_eq!(checked("mispelled\u{2003}café.", "Plain Text", DEFAULT_SELECTOR), ["mispelled", "café"]);
    }

    #[test]
    fn all_bundled_code_families_exclude_identifiers_and_keep_prose() {
        let c_like = "let mispelled = \"mispelled\"; // mispelled\n";
        for (syntax, text) in [
            ("Rust", c_like),
            ("Go", "package main\nvar mispelled = \"mispelled\" // mispelled\n"),
            ("Python", "mispelled = \"mispelled\" # mispelled\n"),
            ("JavaScript", c_like),
            ("TypeScript", c_like),
            ("TypeScript (TSX)", c_like),
        ] {
            let words = checked(text, syntax, DEFAULT_SELECTOR);
            assert_eq!(words.iter().filter(|w| *w == "mispelled").count(), 2, "{syntax}: {words:?}");
            assert!(!words.iter().any(|w| ["let", "var", "package"].contains(&w.as_str())), "{syntax}: {words:?}");
        }
        let text = "fn example(arg: Thing) { arg.field; }";
        let rope = Rope::from_str(text);
        let highlighter = highlight::run_job(Highlighter::by_name("Rust"), &rope, &[0]);
        for (word, scope) in [
            ("example", "entity.name.function"),
            ("arg", "variable.parameter"),
            ("Thing", "entity.name.type"),
            ("field", "variable.member"),
        ] {
            let id = highlighter.scope_id_at(&rope, text.find(word).unwrap());
            assert!(highlight::matches(id, scope), "{word}: {:?}", highlight::scope_stack(id));
        }
        assert_eq!(highlight::scope_style("variable.other"), highlight::scope_style(""));
    }
    #[test]
    fn cache_rejects_stale_edits_scopes_and_dictionary_results() {
        let stamp = Stamp { version: 1, syntax: "Plain Text".into(), selector: DEFAULT_SELECTOR.into(), generation: 0 };
        let mut cache = Cache::default();
        cache.sync(stamp.clone());
        let rope = Rope::from_str("mispelled");
        let stop = AtomicBool::new(false);
        let region = Region { line: 0, chars: 0..9 };
        let prepared = plain(&rope, vec![region.clone()], false, &stop);
        assert_eq!(prepared[0].key.text, rope);
        assert_eq!(prepared[0].key.segments, std::slice::from_ref(&(0..9)));
        let entries = check(prepared, &EveryWord(AtomicUsize::new(0)), &stop);
        cache.adopt(&stamp, entries.clone());
        assert_eq!(cache.ranges(0).cloned().collect::<Vec<_>>(), std::slice::from_ref(&(0..9)));
        assert!(cache.missing(std::slice::from_ref(&region)).is_empty());
        for next in [
            Stamp { version: 2, ..stamp.clone() },
            Stamp { generation: 1, ..stamp.clone() },
            Stamp { syntax: "Rust".into(), ..stamp.clone() },
            Stamp { selector: "comment".into(), ..stamp.clone() },
        ] {
            cache.sync(next);
            cache.adopt(&stamp, entries.clone());
            assert!(cache.ranges(0).next().is_none());
            assert_eq!(cache.missing(std::slice::from_ref(&region)).len(), 1);
        }
        cache.sync(stamp.clone());
        cache.adopt(&stamp, entries);
        cache.clear();
        assert!(cache.ranges(0).next().is_none());
    }
    #[test]
    fn complete_preparation_checks_whole_words_once_and_keeps_token_filters() {
        for word in ["mispelled".repeat(400), "é".repeat(3_600)] {
            let text = format!("😀 {word} {word} tail");
            let rope = Rope::from_str(&text);
            let regions = (0..rope.len_chars())
                .step_by(CHUNK_CHARS)
                .map(|start| Region { line: 0, chars: start..(start + CHUNK_CHARS).min(rope.len_chars()) })
                .collect();
            let stop = AtomicBool::new(false);
            let prepared = plain(&rope, regions, true, &stop);
            assert_eq!(prepared.iter().filter(|p| p.text.contains(&word)).count(), 2);
            let checked = check(prepared, &EveryWord(AtomicUsize::new(0)), &stop);
            let words: Vec<_> =
                checked.iter().flat_map(|p| p.ranges.iter().map(|r| p.key.text.slice(r.clone()).to_string())).collect();
            assert_eq!(words, [word.clone(), word.clone(), "tail".into()]);
            for excluded in [format!("/{word}"), format!("{word}.example"), format!("{word}2"), format!("{word}Camel")]
            {
                let rope = Rope::from_str(&excluded);
                let prepared = plain(&rope, line(0, 0..CHUNK_CHARS), true, &stop);
                assert!(check(prepared, &EveryWord(AtomicUsize::new(0)), &stop).iter().all(|p| p.ranges.is_empty()));
            }
            stop.store(true, Ordering::Relaxed);
            assert!(complete_region(rope.line(0), Region { line: 0, chars: 1..CHUNK_CHARS }, &stop).is_none());
        }
    }

    #[test]
    fn native_subwords_cannot_bypass_identifier_filters_or_change_coordinates() {
        struct Subwords;
        impl Checker for Subwords {
            fn misspelled(&self, text: &str, _: &AtomicBool) -> Vec<Misspelling> {
                [
                    ("good", "good"),
                    ("under", "under"),
                    ("score", "score"),
                    ("camelCase", "camel"),
                    ("caféCase", "café"),
                    ("qzxword's", "qzxword"),
                    ("e\u{301}lan's", "e\u{301}lan"),
                ]
                .into_iter()
                .map(|(source, word)| {
                    let start = text.find(source).unwrap();
                    Misspelling { range: start..start + word.len(), source: start..start + source.len() }
                })
                .collect()
            }
        }
        let text = "😀 good,under_score camelCase caféCase qzxword's e\u{301}lan's";
        let rope = Rope::from_str(text);
        let stop = AtomicBool::new(false);
        let prepared = plain(&rope, line(0, 0..rope.len_chars()), true, &stop);
        let checked = check(prepared, &Subwords, &stop);
        let ranges = &checked[0].ranges;
        assert_eq!(
            ranges.iter().map(|r| rope.slice(r.clone()).to_string()).collect::<Vec<_>>(),
            ["good", "qzxword", "e\u{301}lan"]
        );
        for (range, word) in ranges.iter().zip(["good", "qzxword", "e\u{301}lan"]) {
            let start = text[..text.find(word).unwrap()].chars().count();
            assert_eq!(*range, start..start + word.chars().count());
        }
    }

    #[test]
    fn visible_context_keeps_complete_words_next_to_clipped_tokens() {
        let text = format!("{},mispelled,{}", "x".repeat(4_000), "é".repeat(4_000));
        let rope = Rope::from_str(&text);
        let stop = AtomicBool::new(false);
        let prepared = plain(&rope, line(0, 4001..4010), false, &stop);
        assert!(prepared[0].text.chars().count() <= 9 + 2 * CONTEXT_CHARS);
        let checked = check(prepared, &EveryWord(AtomicUsize::new(0)), &stop);
        assert_eq!(checked[0].ranges, std::slice::from_ref(&(4001..4010)));
    }

    #[test]
    fn inspection_stops_between_native_stages_in_both_discovery_paths() {
        struct Stages {
            stop: Arc<AtomicBool>,
            cancel: &'static str,
            error: bool,
            calls: Mutex<Vec<&'static str>>,
        }
        impl Stages {
            fn enter(&self, stage: &'static str) {
                self.calls.lock().unwrap().push(stage);
                if self.cancel == stage {
                    self.stop.store(true, Ordering::Relaxed);
                }
            }
        }
        impl Checker for Stages {
            fn misspelled(&self, _: &str, _: &AtomicBool) -> Vec<Misspelling> {
                self.enter("check");
                if self.error {
                    vec![Misspelling { range: 0..9, source: 0..9 }]
                } else {
                    vec![]
                }
            }
            fn word_at(&self, _: &str, _: usize, _: &AtomicBool) -> Option<Range<usize>> {
                self.enter("word_at");
                Some(0..9)
            }
            fn guesses(&self, _: &str) -> Vec<String> {
                self.enter("guesses");
                vec!["corrected".into()]
            }
            fn has_learned_word(&self, _: &str) -> bool {
                self.enter("learned");
                true
            }
        }
        for (error, cancel, expected) in [
            (true, "check", vec!["check"]),
            (true, "guesses", vec!["check", "guesses"]),
            (true, "learned", vec!["check", "guesses", "learned"]),
            (false, "word_at", vec!["check", "word_at"]),
            (false, "learned", vec!["check", "word_at", "learned"]),
            (false, "guesses", vec!["check", "word_at", "learned", "guesses"]),
        ] {
            let stop = Arc::new(AtomicBool::new(false));
            let rope = Rope::from_str("mispelled");
            let prepared = plain(&rope, line(0, 0..9), false, &stop);
            let checker = Stages { stop: stop.clone(), cancel, error, calls: Mutex::new(vec![]) };
            assert!(matches!(
                word_work(WordWork::Inspect { prepared, line_base: 10, at: 9 }, &checker, &stop, &mut HashSet::new()),
                WordAnswer::Suggestions(None)
            ));
            assert_eq!(*checker.calls.lock().unwrap(), expected);
        }
    }

    #[test]
    fn cancel_inside_giant_unicode_eligibility_releases_highlight_worker() {
        use std::time::{Duration, Instant};
        let text = "é".repeat(49_500_000);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (entered, stage) = std::sync::mpsc::channel();
        let (finished, done) = std::sync::mpsc::channel();
        let _preparation = highlight::on_worker(move || {
            assert_eq!(std::thread::current().name(), Some("orlo-highlight"));
            ELIGIBILITY_STAGE.with_borrow_mut(|hook| *hook = Some(entered));
            let result = eligible_ranges(&text, &flag);
            finished.send(result.is_none()).unwrap();
        });
        stage.recv_timeout(Duration::from_secs(60)).unwrap();
        // Cancel inside the traversal: a stop set before it starts would test nothing.
        std::thread::sleep(Duration::from_millis(15));
        let cancelled = Instant::now();
        stop.store(true, Ordering::Relaxed);
        let (highlighted, urgent) = std::sync::mpsc::channel();
        let _urgent = highlight::on_worker_urgent(move || {
            let rope = Rope::from_str("let current = 1;");
            let job = highlight::run_job(Highlighter::by_name("Rust"), &rope, &[0]);
            highlighted.send(job.cached_scope_runs(&rope, 0).is_some()).unwrap();
        });
        let prompt = urgent.recv_timeout(Duration::from_millis(500));
        let elapsed = cancelled.elapsed();
        eprintln!("99 MB eligibility cancellation to urgent highlighting: {elapsed:?}");
        // Drain the real worker before asserting, so a failure leaves no work queued on it.
        assert!(done.recv_timeout(Duration::from_secs(60)).unwrap());
        if prompt.is_err() {
            assert!(urgent.recv_timeout(Duration::from_secs(60)).unwrap());
        }
        assert_eq!(prompt.ok(), Some(true), "urgent highlighting waited {elapsed:?} after cancellation");
    }

    #[test]
    fn huge_lines_and_buffers_have_bounded_cancellable_requests() {
        let rope = Rope::from_str(&format!("mispelled {}\n{}", "x".repeat(4_000_000), "mispelled\n".repeat(100_000)));
        let visible =
            vec![Region { line: 0, chars: 0..rope.line(0).len_chars() }, Region { line: 90_000, chars: 0..9 }];
        let cache = Cache::default();
        let regions = cache.missing(&visible);
        assert!(regions.len() <= MAX_REQUESTS);
        assert!(regions.iter().all(|r| r.chars.len() <= CHUNK_CHARS));
        let stop = AtomicBool::new(false);
        let prepared = plain(&rope, regions, false, &stop);
        assert!(prepared.iter().all(|r| r.text.len() <= (CHUNK_CHARS + 2 * CONTEXT_CHARS) * 4));
        let checker = EveryWord(AtomicUsize::new(0));
        let active = check(prepared, &checker, &stop);
        assert_eq!(active[0].ranges, std::slice::from_ref(&(0..9)));
        assert_eq!(checker.0.load(Ordering::Relaxed), 1);
        let oversized = plain(&rope, line(0, 0..rope.line(0).len_chars()), false, &stop);
        assert_eq!(oversized[0].region.chars, 0..CHUNK_CHARS);
        assert!(oversized[0].text.len() <= (CHUNK_CHARS + 2 * CONTEXT_CHARS) * 4);
        let far = plain(&rope, line(90_000, 0..9), false, &stop);
        assert_eq!(check(far, &checker, &stop)[0].ranges, std::slice::from_ref(&(0..9)));
        let prepared = plain(&rope, line(0, 0..9), false, &stop);
        let before = checker.0.load(Ordering::Relaxed);
        stop.store(true, Ordering::Relaxed);
        assert!(check(prepared, &checker, &stop).is_empty());
        assert_eq!(checker.0.load(Ordering::Relaxed), before);
        assert!(plain(&rope, line(0, 0..9), false, &stop).is_empty());
    }
}
