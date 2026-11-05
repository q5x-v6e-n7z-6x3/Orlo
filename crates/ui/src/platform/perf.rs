//! `ORLO_PERF=1` prints timings to stderr, one `orlo-perf <what> ms=<n>` line each: `main()` to
//! the first painted editor, the oldest unpainted key or scroll to the paint that shows it, and
//! each frame's work from the workspace render to the last editor paint. Off, it costs a branch.
//! Main thread only. The input timestamp is per thread, not per view: with several editors
//! open, another view's paint can take it. This is app timing, not compositor presentation.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<Instant> = OnceLock::new();

thread_local! {
    static INPUT: Cell<Option<Instant>> = const { Cell::new(None) };
    /// (render start, last editor paint end) of the frame being drawn.
    static FRAME: Cell<Option<(Instant, Option<Instant>)>> = const { Cell::new(None) };
    static FIRST: Cell<bool> = const { Cell::new(true) };
}

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ORLO_PERF").is_some())
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.
}

/// First thing in `main`.
pub fn start() {
    START.get_or_init(Instant::now);
}

/// A key or scroll arrived; the next editor paint reports how long it took to show.
pub fn input() {
    if enabled() {
        INPUT.set(INPUT.get().or_else(|| Some(Instant::now())));
    }
}

/// The workspace starts rendering a frame: report the previous one.
pub fn frame_begin() {
    if !enabled() {
        return;
    }
    if let Some((start, Some(end))) = FRAME.get() {
        let at = START.get().map_or(0., |s| ms(end - *s));
        eprintln!("orlo-perf frame ms={:.3} at={at:.1}", ms(end - start));
    }
    FRAME.set(Some((Instant::now(), None)));
}

/// An editor finished painting.
pub fn painted() {
    if !enabled() {
        return;
    }
    let now = Instant::now();
    if let Some((start, _)) = FRAME.get() {
        FRAME.set(Some((start, Some(now))));
    }
    if let Some(t) = INPUT.take() {
        eprintln!("orlo-perf input-to-paint ms={:.3}", ms(now - t));
    }
    if FIRST.replace(false) {
        let since_main = START.get().map_or(0., |s| ms(now - *s));
        let epoch = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros());
        eprintln!("orlo-perf first-paint ms={since_main:.3} epoch_us={epoch}");
    }
}
