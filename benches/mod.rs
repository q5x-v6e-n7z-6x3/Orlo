//! Timings of the work behind the UI's hot paths, each against a budget so a regression fails.
//! Release build only (debug is 10–50× slower):
//!
//! ```sh
//! ORLO_INDEX_TREE="$PWD/crates" cargo test --workspace --release bench_ -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The budgets are about 3× the times measured on an M4 MacBook Air; a frame at
//! 120 Hz is 8.3 ms. The UI's own numbers come from `ORLO_PERF=1` (see the UI performance instrumentation).

use orlo_editor::display::WrapMap;
use orlo_editor::fold::Folds;
use orlo_editor::text::{EditorState, FindOpts, Selection};
use orlo_syntax::highlight::Highlighter;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Orlo's own sources: realistic Rust.
const RUST: &str = concat!(
    include_str!("../crates/editor/src/viewport/mod.rs"),
    include_str!("../crates/editor/src/search/files.rs"),
    include_str!("../crates/ui/src/views/editor/element.rs"),
);

fn rust_lines(n: usize) -> String {
    crate::testing::EDITING_MODEL.lines().chain(RUST.lines()).cycle().take(n).fold(String::new(), |mut s, l| {
        s.push_str(l);
        s.push('\n');
        s
    })
}

/// The test binary's allocator: the system's, counting bytes while `allocated_at_most` runs.
struct Counting;

#[global_allocator]
static COUNTING: Counting = Counting;

static COUNT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LIVE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static PEAK: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

fn counted(bytes: isize) {
    use std::sync::atomic::Ordering::Relaxed;
    if COUNT.load(Relaxed) {
        let now = LIVE.fetch_add(bytes, Relaxed) + bytes;
        PEAK.fetch_max(now, Relaxed);
    }
}

/// `p`, counting `bytes` when the allocation it came from succeeded.
fn allocated(p: *mut u8, bytes: isize) -> *mut u8 {
    if !p.is_null() {
        counted(bytes);
    }
    p
}

// SAFETY: every call goes to the system allocator unchanged; only the sizes are counted.
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::System.alloc(l) };
        allocated(p, l.size() as isize)
    }
    unsafe fn alloc_zeroed(&self, l: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::System.alloc_zeroed(l) };
        allocated(p, l.size() as isize)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(p, l) };
        counted(-(l.size() as isize));
    }
    unsafe fn realloc(&self, p: *mut u8, l: std::alloc::Layout, new: usize) -> *mut u8 {
        let q = unsafe { std::alloc::System.realloc(p, l, new) };
        allocated(q, new as isize - l.size() as isize)
    }
}

/// The most bytes `f` had allocated at once (net of what it freed), with what it returned.
/// Benchmarks run one at a time: another thread's allocations would count too.
fn allocated_at_most<T>(f: impl FnOnce() -> T) -> (T, usize) {
    use std::sync::atomic::Ordering::Relaxed;
    LIVE.store(0, Relaxed);
    PEAK.store(0, Relaxed);
    COUNT.store(true, Relaxed);
    let r = f();
    COUNT.store(false, Relaxed);
    (r, PEAK.load(Relaxed) as usize)
}

/// The fastest of `runs` (the least disturbed by the rest of the machine).
pub(crate) fn time<T>(name: &str, runs: usize, budget_ms: f64, mut f: impl FnMut() -> T) {
    let best = (0..runs)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed()
        })
        .min()
        .unwrap_or(Duration::ZERO);
    let ms = best.as_secs_f64() * 1000.;
    println!("bench {name:<40} {ms:>10.3} ms   (budget {budget_ms} ms)");
    assert!(ms <= budget_ms, "{name}: {ms:.3} ms is over its {budget_ms} ms budget");
}

/// 100 MB of Rust, in the temp folder.
pub(crate) fn big_file() -> PathBuf {
    temp_file("orlo-bench-100mb.rs", || {
        let chunk = rust_lines(100_000);
        chunk.repeat(100 * 1024 * 1024 / chunk.len() + 1)
    })
}

/// A fixture in the temp folder, written again when what `make` gives differs in size from
/// what is there (a fixture left by an older version of the bench).
fn temp_file(name: &str, make: impl FnOnce() -> String) -> PathBuf {
    let p = std::env::temp_dir().join(name);
    let text = make();
    if std::fs::metadata(&p).map_or(true, |m| m.len() != text.len() as u64) {
        std::fs::write(&p, text).unwrap();
    }
    p
}

mod editing;
mod filesystem;
mod rendering;
