//! Saving off the UI thread. A document has one save at a time: one asked for while another is
//! under way waits, merged with the others to the same file, and is taken from the text as it
//! is when its turn comes, its optional asynchronous preparation and cleanup run just before.
//! Preparation reserves the save slot too, so close and quit wait for it. Each save holds its
//! destination's write lease (`pipelines::lease_file`, by path and by the file itself) from
//! before it writes until it has landed, so two writes to a file never overlap, a text
//! pipeline's or one to another name of it included; and saves to a file write in the order
//! they were taken, whichever document they are of, so an older write never lands after a
//! newer one.

use super::*;
use crate::editing::encoding::Encoding;
use crate::editing::text::{SaveFailure, SaveRequest, Written};
use crate::views::workspace::pipelines;
use gpui::Task;
use ropey::Rope;
use std::collections::VecDeque;
use std::sync::{Arc, Weak};

/// How a save ended: the text written, or the alert's text.
pub type SaveOutcome = Result<Rope, String>;

/// Told how a save ended, once it has landed.
pub type Then = Box<dyn FnOnce(&SaveOutcome, &mut App)>;

/// Preparation, through the document, before its text is taken. A returned task must finish
/// before capture. Runs at once or when a queued save gets its turn.
pub(crate) type Prepare = Box<dyn FnOnce(&mut App) -> Option<Task<()>>>;

/// Told once no save of a document is under way.
type Idle = Box<dyn FnOnce(&mut App)>;

pub(crate) type SaveValidation = Rc<dyn Fn(&App) -> bool>;

/// A background save can be canceled before capture; its disk check survives capture.
pub(crate) struct Guard {
    pub valid: SaveValidation,
    pub disk: crate::files::watch::Stamp,
}

/// A save asked for: where to (`None`: the document's own file, as its turn finds it), the
/// destination's encoding, the cleanup before it is taken, who waits for it, explicit bypass.
type Again = (Option<PathBuf>, Option<Encoding>, Vec<Prepare>, Vec<Then>, bool);

/// Where `again` joins the saves waiting: with the last one, when that is to the same file (a
/// second Cmd-S, or a second Save As to one file) and the same formatting policy, else after
/// it, so every file and explicit bypass asked for gets its own turn.
fn queue(waiting: &mut VecDeque<Again>, again: Again) {
    match waiting.back_mut() {
        Some(last) if last.0 == again.0 && last.4 == again.4 => {
            last.1 = again.1.or(last.1);
            last.2.extend(again.2);
            last.3.extend(again.3);
        }
        _ => waiting.push_back(again),
    }
}

#[derive(Default)]
pub(crate) struct Saving {
    /// The requests this save answers.
    waiters: Vec<Then>,
    guard: Option<Guard>,
    preparation_error: Option<String>,
    capture_validation: Option<SaveValidation>,
    /// Saves asked for while this one runs, in turn.
    waiting: VecDeque<Again>,
    /// Told once no save of the document is under way (a quit or a window closing waits).
    idle: Vec<Idle>,
}

/// Accepted copies have separate completion and share only the close/quit waiters.
#[derive(Default)]
pub(crate) struct Copying {
    pending: usize,
    idle: Vec<Idle>,
}

/// What follows a write once the document's update is over: the next save's turn (its cleanup
/// edits the document), then who waited is told (they read it).
#[derive(Default)]
#[must_use]
pub(crate) struct Landed {
    told: Vec<(Vec<Then>, SaveOutcome)>,
    idle: Vec<Idle>,
    waiting: VecDeque<Again>,
}

impl Landed {
    fn run(mut self, document: &Entity<OpenDocument>, cx: &mut App) {
        // the next save in turn, from the text as it is now; one refused at once lets the next go
        while let Some((path, previous, prepare, waiters, _)) = self.waiting.pop_front() {
            document.update(cx, |d, _| {
                d.saving = Some(Saving {
                    waiters,
                    guard: None,
                    preparation_error: None,
                    capture_validation: None,
                    waiting: std::mem::take(&mut self.waiting),
                    idle: std::mem::take(&mut self.idle),
                });
            });
            let next = OpenDocument::prepare_and_start(document, path, previous, prepare, cx);
            self.waiting = next.waiting;
            self.idle = next.idle;
            self.told.extend(next.told);
        }
        for (waiters, outcome) in self.told {
            waiters.into_iter().for_each(|w| w(&outcome, cx));
        }
        self.idle.into_iter().for_each(|f| OpenDocument::when_saved(document, f, cx));
    }
}

/// The saves taken to each file, in the order they were taken, whichever document they are of.
#[derive(Default)]
struct Turns(HashMap<PathBuf, VecDeque<Weak<()>>>);

impl Global for Turns {}

/// A place in line for a save just taken to `path`: it writes once those before it have.
fn take_turn(path: &Path, cx: &mut App) -> Arc<()> {
    let turn = Arc::new(());
    let turns = &mut cx.default_global::<Turns>().0;
    turns.retain(|_, line| {
        line.retain(|t| t.strong_count() > 0);
        !line.is_empty()
    });
    let line = turns.entry(path.to_path_buf()).or_default();
    line.push_back(Arc::downgrade(&turn));
    turn
}

/// Whether every save taken to `path` before `turn` has written.
fn first_in_line(path: &Path, turn: &Arc<()>, cx: &App) -> bool {
    let line = cx.try_global::<Turns>().and_then(|t| t.0.get(path));
    let first = line.and_then(|l| l.iter().find_map(Weak::upgrade));
    first.is_none_or(|f| Arc::ptr_eq(&f, turn))
}

impl OpenDocument {
    /// Whether a save of this document is under way.
    pub fn saving(&self) -> bool {
        self.saving.is_some() || self.copying.pending > 0
    }

    /// Refuse every live destination owner, across windows and filesystem aliases.
    pub(crate) fn check_copy_destination(document: &Entity<Self>, path: &Path, cx: &App) -> Result<(), String> {
        let source = document.read(cx);
        source.model.check_copy(path).map_err(|e| e.to_string())?;
        let target = pipelines::file_id(path);
        if cx.try_global::<Documents>().is_some_and(|all| {
            all.by_id.values().filter_map(|d| d.upgrade()).any(|other| {
                other != *document
                    && other.read(cx).model.path.as_ref().is_some_and(|p| {
                        crate::editing::text::resolve(p) == crate::editing::text::resolve(path)
                            || target.is_some_and(|id| pipelines::file_id(p) == Some(id))
                    })
            })
        }) {
            return Err("Copy destination belongs to another open document".into());
        }
        Ok(())
    }

    /// Capture now, then wait for the destination. Never prepare or finish a source save.
    pub(crate) fn save_copy(
        document: &Entity<Self>,
        path: PathBuf,
        then: impl FnOnce(&SaveOutcome, &mut App) + 'static,
        cx: &mut App,
    ) -> Option<SaveOutcome> {
        if let Err(error) = Self::check_copy_destination(document, &path, cx) {
            return Some(Err(error));
        }
        let request = match document.read(cx).model.capture_copy(path) {
            Ok(request) => Arc::new(request),
            Err(error) => return Some(Err(error.to_string())),
        };
        let path = request.save.path.clone();
        let file = pipelines::file_id(&path);
        let turn = take_turn(&path, cx);
        #[cfg(test)]
        if !gate::holds(&path)
            && !crate::testing::gate::holds("copy-write", &path)
            && first_in_line(&path, &turn, cx)
            && !pipelines::writing(&path, cx)
            && !pipelines::writing_file(file, cx)
        {
            let _lease = pipelines::lease_file(&path, file, cx);
            return Some(request.write().map(|_| request.save.rope.clone()).map_err(|e| e.error.to_string()));
        }
        document.update(cx, |d, _| d.copying.pending += 1);
        // Strong ownership survives removal of the last view or originating window.
        let document = document.clone();
        cx.spawn(async move |cx| {
            let wait = |cx: &mut App| {
                !first_in_line(&path, &turn, cx) || pipelines::writing(&path, cx) || pipelines::writing_file(file, cx)
            };
            while cx.update(wait).unwrap_or(false) {
                cx.background_executor().timer(std::time::Duration::from_millis(10)).await;
            }
            let Ok(lease) = cx.update(|cx| pipelines::lease_file(&path, file, cx)) else { return };
            drop(turn);
            #[cfg(test)]
            gate::wait(&path).await;
            // Opening/renaming a document while this copy waited must also refuse it.
            let validation = cx.update(|cx| Self::check_copy_destination(&document, &path, cx));
            let outcome = match validation {
                Ok(Ok(())) => {
                    #[cfg(test)]
                    crate::testing::gate::wait("copy-write", &path).await;
                    let write = request.clone();
                    cx.background_spawn(async move { write.write() })
                        .await
                        .map(|_| request.save.rope.clone())
                        .map_err(|e| e.error.to_string())
                }
                Ok(Err(error)) => Err(error),
                Err(_) => return,
            };
            drop(lease);
            cx.update(|cx| {
                let idle = document.update(cx, |d, _| {
                    d.copying.pending -= 1;
                    if d.copying.pending == 0 {
                        std::mem::take(&mut d.copying.idle)
                    } else {
                        Vec::new()
                    }
                });
                then(&outcome, cx);
                for f in idle {
                    Self::when_saved(&document, f, cx);
                }
            })
            .ok();
        })
        .detach();
        None
    }

    /// Write `document` to `path` (its own file when `None`; `previous`: the destination's
    /// encoding when another document has it open) off the UI thread, then `then` with how it
    /// went. `prepare` runs before capture; its optional task is awaited while this document
    /// remains saving. Preparation starts now, or, while a save
    /// runs, when this one's turn comes: it waits, merged with the one before it when that is
    /// to the same file, and its own file is then the one the saves before it leave it with. A
    /// save over at once (refused before anything is taken, or a test's write no test holds)
    /// is said here instead, without `then`.
    pub fn save(
        document: &Entity<Self>,
        path: Option<PathBuf>,
        previous: Option<Encoding>,
        prepare: impl FnOnce(&mut App) -> Option<Task<()>> + 'static,
        then: impl FnOnce(&SaveOutcome, &mut App) + 'static,
        cx: &mut App,
    ) -> Option<SaveOutcome> {
        Self::save_with_policy(document, path, previous, prepare, then, false, cx)
    }

    /// An explicit bypass still uses normal save preparation/capture, but must not merge
    /// with a queued save that requires successful formatting.
    pub(crate) fn save_without_formatting(
        document: &Entity<Self>,
        path: Option<PathBuf>,
        previous: Option<Encoding>,
        prepare: impl FnOnce(&mut App) -> Option<Task<()>> + 'static,
        then: impl FnOnce(&SaveOutcome, &mut App) + 'static,
        cx: &mut App,
    ) -> Option<SaveOutcome> {
        Self::save_with_policy(document, path, previous, prepare, then, true, cx)
    }

    fn save_with_policy(
        document: &Entity<Self>,
        path: Option<PathBuf>,
        previous: Option<Encoding>,
        prepare: impl FnOnce(&mut App) -> Option<Task<()>> + 'static,
        then: impl FnOnce(&SaveOutcome, &mut App) + 'static,
        without_formatting: bool,
        cx: &mut App,
    ) -> Option<SaveOutcome> {
        if document.read(cx).model.is_following() {
            return Some(Err("Stop following before saving".into()));
        }
        let (prepare, then): (Prepare, Then) = (Box::new(prepare), Box::new(then));
        let again = (path, previous, vec![prepare], vec![then], without_formatting);
        let now = document.update(cx, |d, _| match &mut d.saving {
            Some(s) => {
                queue(&mut s.waiting, again);
                None
            }
            None => {
                // Reserve before preparation: edits, close and quit see this save immediately.
                d.saving = Some(Saving::default());
                Some(again)
            }
        });
        let (path, previous, prepare, then, _) = now?;
        document.update(cx, |d, _| d.saving.as_mut().unwrap().waiters = then);
        let mut landed = Self::prepare_and_start(document, path, previous, prepare, cx);
        // A synchronous save returns its outcome instead of calling its own callback.
        let now = (!landed.told.is_empty()).then(|| landed.told.remove(0).1);
        landed.run(document, cx);
        now
    }

    /// Never queue automatic work behind an explicit Save/Save As.
    pub(crate) fn save_automatically(
        document: &Entity<Self>,
        guard: Guard,
        prepare: impl FnOnce(&mut App) -> Option<Task<()>> + 'static,
        then: impl FnOnce(&SaveOutcome, &mut App) + 'static,
        cx: &mut App,
    ) -> Option<SaveOutcome> {
        if document.read(cx).saving() {
            return Some(Err("A save is already running".into()));
        }
        document.update(cx, |d, _| {
            d.file_autosave.attempt = Some(super::autosave::Key::of(&d.model));
            d.saving = Some(Saving { guard: Some(guard), waiters: vec![Box::new(then)], ..Default::default() });
        });
        let mut landed = Self::prepare_and_start(document, None, None, vec![Box::new(prepare)], cx);
        let now = (!landed.told.is_empty()).then(|| landed.told.remove(0).1);
        landed.run(document, cx);
        now
    }

    /// Fail this reserved save before cleanup/capture, completing all ordinary waiters.
    pub(crate) fn fail_preparation(document: &Entity<Self>, error: String, cx: &mut App) {
        document.update(cx, |d, _| {
            if let Some(saving) = &mut d.saving {
                saving.preparation_error = Some(error);
            }
        });
    }

    /// Preparation may yield once more before capture; validate its resulting snapshot then.
    pub(crate) fn validate_before_capture(document: &Entity<Self>, valid: SaveValidation, cx: &mut App) {
        document.update(cx, |d, _| {
            if let Some(saving) = &mut d.saving {
                saving.capture_validation = Some(valid);
                if saving.guard.is_some() {
                    d.file_autosave.attempt = Some(super::autosave::Key::of(&d.model));
                }
            }
        });
    }

    /// Preparation belongs to the save slot, including its asynchronous part. A detached task
    /// keeps the document alive until it has resumed capture, even when its last tab closes.
    fn prepare_and_start(
        document: &Entity<Self>,
        path: Option<PathBuf>,
        previous: Option<Encoding>,
        prepare: Vec<Prepare>,
        cx: &mut App,
    ) -> Landed {
        let valid = document.read(cx).saving.as_ref().and_then(|s| s.guard.as_ref()).map(|g| g.valid.clone());
        if valid.is_some_and(|valid| !valid(cx)) {
            return document.update(cx, |d, _| d.completed(Err("Automatic save canceled before capture".into())));
        }
        let mut prepare = prepare.into_iter();
        loop {
            let error = document.update(cx, |d, _| d.saving.as_mut().and_then(|s| s.preparation_error.take()));
            if let Some(error) = error {
                return document.update(cx, |d, _| d.completed(Err(error)));
            }
            let Some(next) = prepare.next() else {
                let valid = document.read(cx).saving.as_ref().and_then(|s| s.capture_validation.clone());
                if valid.is_some_and(|valid| !valid(cx)) {
                    return document.update(cx, |d, _| d.completed(Err(
                        "Save preparation expired or document changed; save again or use File > Save Without Formatting".into()
                    )));
                }
                break;
            };
            if let Some(task) = next(cx) {
                let document = document.clone();
                cx.spawn(async move |cx| {
                    task.await;
                    cx.update(|cx| {
                        let landed = Self::prepare_and_start(&document, path, previous, prepare.collect(), cx);
                        landed.run(&document, cx);
                    })
                    .ok();
                })
                .detach();
                return Landed::default();
            }
        }
        document.update(cx, |d, cx| d.start(path, previous, cx))
    }

    /// `f` once no save of `document` is under way, at once if none is.
    pub fn when_saved(document: &Entity<Self>, f: impl FnOnce(&mut App) + 'static, cx: &mut App) {
        let f: Idle = Box::new(f);
        let now = document.update(cx, |d, _| match &mut d.saving {
            Some(s) => {
                s.idle.push(f);
                None
            }
            None if d.copying.pending > 0 => {
                d.copying.idle.push(f);
                None
            }
            None => Some(f),
        });
        if let Some(f) = now {
            f(cx)
        }
    }

    fn start(&mut self, path: Option<PathBuf>, previous: Option<Encoding>, cx: &mut Context<Self>) -> Landed {
        let Some(path) = path.or_else(|| self.model.path.clone()) else {
            return self.completed(Err("untitled buffer has no path".into()));
        };
        let expected = self.saving.as_ref().and_then(|s| s.guard.as_ref()).map(|g| g.disk);
        if expected.is_some() {
            self.file_autosave.attempt = Some(super::autosave::Key::of(&self.model));
        }
        let request = match self.update_file(|d| d.capture_save(path, previous), cx) {
            Ok(r) => Arc::new(r),
            Err(e) => return self.completed(Err(e.to_string())),
        };
        // A test that does not hold the write sees it land before `save` returns, as every save
        // did when it was written on the UI thread; a held one (`gate`) goes the app's way.
        #[cfg(test)]
        if !gate::holds(&request.path) {
            let lease = pipelines::lease(&request.path, cx);
            let written = expected.map_or_else(|| request.write(), |disk| request.write_if_unchanged(disk));
            let outcome = self.finish(&request, written, cx);
            drop(lease);
            return self.completed(outcome);
        }
        let turn = take_turn(&request.path, cx);
        // not the document's to cancel: a save asked for lands though its last view closes
        cx.spawn(async move |this, cx| {
            let path = request.path.clone();
            // the file itself too: another document may have it open by another name (a hard
            // link), and two writes in place at once would leave the end of the longer
            let p = path.clone();
            let file = cx.background_spawn(async move { pipelines::file_id(&p) }).await;
            // the saves to it taken before this one, and any other writer of the file (a text
            // pipeline, another document's save)
            let wait = |cx: &mut App| {
                !first_in_line(&path, &turn, cx) || pipelines::writing(&path, cx) || pipelines::writing_file(file, cx)
            };
            while cx.update(wait).unwrap_or(false) {
                cx.background_executor().timer(std::time::Duration::from_millis(10)).await;
            }
            let Ok(lease) = cx.update(|cx| pipelines::lease_file(&path, file, cx)) else {
                return;
            };
            drop(turn);
            #[cfg(test)]
            gate::wait(&path).await;
            let write = request.clone();
            let written = cx
                .background_spawn(async move {
                    expected.map_or_else(|| write.write(), |disk| write.write_if_unchanged(disk))
                })
                .await;
            let Some(document) = this.upgrade() else {
                return;
            };
            cx.update(|cx| {
                let landed = document.update(cx, |d, cx| d.landed(&request, written, lease, cx));
                landed.run(&document, cx);
            })
            .ok();
        })
        .detach();
        Landed::default()
    }

    /// What reached disk, recorded, or the stamp a failure left: how the save ended.
    fn finish(
        &mut self,
        request: &SaveRequest,
        written: Result<Written, SaveFailure>,
        cx: &mut Context<Self>,
    ) -> SaveOutcome {
        match written {
            Ok(w) => {
                self.update_file(|d| d.finish_save(request, &w), cx);
                self.file_autosave.baseline = w.disk;
                self.file_autosave.failed = None;
                Ok(request.rope.clone())
            }
            Err(f) => {
                self.update_file(|d| d.fail_save(request, &f), cx);
                Err(f.error.to_string())
            }
        }
    }

    /// A write landed: record it, let the file go, and hand on the saves asked for meanwhile,
    /// to take their turn once the update is over.
    fn landed(
        &mut self,
        request: &SaveRequest,
        written: Result<Written, SaveFailure>,
        lease: Arc<()>,
        cx: &mut Context<Self>,
    ) -> Landed {
        let outcome = self.finish(request, written, cx);
        drop(lease);
        self.completed(outcome)
    }

    fn completed(&mut self, outcome: SaveOutcome) -> Landed {
        let Some(saving) = self.saving.take() else {
            return Landed::default();
        };
        if saving.guard.is_some() {
            let attempted = self.file_autosave.attempt.take();
            if outcome.is_err() {
                self.file_autosave.failed = attempted;
            }
        }
        Landed { told: vec![(saving.waiters, outcome)], idle: saving.idle, waiting: saving.waiting }
    }
}

impl Documents {
    /// Whether any document's save is under way.
    pub fn saving(cx: &App) -> bool {
        Self::first_saving(cx).is_some()
    }

    fn first_saving(cx: &App) -> Option<Entity<OpenDocument>> {
        (cx.try_global::<Documents>()?.by_id.values()).filter_map(|d| d.upgrade()).find(|d| d.read(cx).saving())
    }

    /// `f` once no document's save is under way (Quit, a window closing).
    pub fn when_all_saved(f: impl FnOnce(&mut App) + 'static, cx: &mut App) {
        let Some(first) = Self::first_saving(cx) else {
            return f(cx);
        };
        // each in turn: the rest are looked at once the first is done
        let f: Idle = Box::new(f);
        OpenDocument::when_saved(&first, move |cx| Documents::when_all_saved(f, cx), cx);
    }
}

/// Tests: a save's write held at its start until released, so a test can act while it runs.
#[cfg(test)]
pub(crate) mod gate {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::task::{Poll, Waker};

    #[derive(Default)]
    struct Shared {
        arrived: Cell<usize>,
        open: Cell<bool>,
        waker: RefCell<Option<Waker>>,
    }

    thread_local! {
        static GATES: RefCell<HashMap<PathBuf, Rc<Shared>>> = RefCell::default();
    }

    /// Writes to `path` wait at their start until `release`.
    pub(crate) struct Pause(Rc<Shared>, PathBuf);

    pub(crate) fn pause(path: &Path) -> Pause {
        let shared = Rc::new(Shared::default());
        GATES.with(|g| g.borrow_mut().insert(path.to_path_buf(), shared.clone()));
        Pause(shared, path.to_path_buf())
    }

    impl Pause {
        /// How many writes have reached the gate.
        pub(crate) fn arrived(&self) -> usize {
            self.0.arrived.get()
        }

        /// The waiting write goes on, and later ones do not wait.
        pub(crate) fn release(&self) {
            self.0.open.set(true);
            GATES.with(|g| g.borrow_mut().remove(&self.1));
            if let Some(w) = self.0.waker.borrow_mut().take() {
                w.wake();
            }
        }
    }

    impl Drop for Pause {
        fn drop(&mut self) {
            self.release();
        }
    }

    /// Whether a test holds writes to `path`.
    pub(crate) fn holds(path: &Path) -> bool {
        GATES.with(|g| g.borrow().contains_key(path))
    }

    pub(crate) async fn wait(path: &Path) {
        let Some(shared) = GATES.with(|g| g.borrow().get(path).cloned()) else {
            return;
        };
        shared.arrived.set(shared.arrived.get() + 1);
        std::future::poll_fn(|cx| match shared.open.get() {
            true => Poll::Ready(()),
            false => {
                *shared.waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
}
