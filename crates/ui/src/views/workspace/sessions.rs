use super::*;

/// A nonmutating snapshot excludes the provisional overtype/virtual representation.
struct Recoverable {
    rope: ropey::Rope,
    endings: Option<crate::editing::text::LineEndings>,
    removed: Vec<std::ops::Range<usize>>,
    owner: EntityId,
    selections: Vec<(usize, usize)>,
}
impl Recoverable {
    fn offset(&self, at: usize) -> usize {
        at.saturating_sub(self.removed.iter().map(|r| at.min(r.end).saturating_sub(r.start)).sum())
    }
}

/// Unsaved texts for the session: (file name, text), taken on the UI thread as rope clones.
pub type Texts = Vec<(String, ropey::Rope)>;

#[derive(Default)]
pub(crate) struct RestoringView {
    pub group: usize,
    /// Loaded tabs before this view; earlier pending insertions advance it.
    pub place: usize,
    pub active: bool,
    /// The loaded fallback must stay selected for the saved active claim to survive.
    pub fallback: Option<EntityId>,
    pub view: ViewState,
    /// Explicit Reopen creates its own view even if another Open finishes first.
    pub reopen: bool,
}

impl RestoringView {
    fn claims_active(&self, views: &Groups<Entity<Editor>>) -> bool {
        self.active
            && views
                .groups
                .get(self.group)
                .is_some_and(|g| g.tabs.get(g.active).map(Entity::entity_id) == self.fallback)
    }
}

/// Every window and the recent lists, for the session file. `current` is the window being
/// updated, which cannot be read through its handle. `hot`: unsaved buffers are kept too
/// (Sublime's hot exit).
pub(crate) fn capture_session(current: Option<(WindowState, Texts)>, hot: bool, cx: &mut App) -> (Session, Texts) {
    let mut texts = Texts::new();
    let mut windows = Vec::new();
    if let Some((w, t)) = current {
        windows.push(w);
        texts.extend(t);
    }
    for w in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        if let Ok(st) = w.update(cx, |ws, window, cx| ws.capture(window, hot, &mut texts, cx)) {
            windows.push(st);
        }
    }
    let r = cx.global::<Recent>();
    let s = Session {
        windows,
        recent_files: r.files.clone(),
        recent_folders: r.folders.clone(),
        recent_projects: r.projects.clone(),
    };
    (s, texts)
}

/// Captures are numbered: a write never replaces the file with an older one (a background
/// write finishing after the one Quit made).
pub(crate) static CAPTURED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// What this run may do to a session file (one per path: a test's support folder has its own).
#[derive(Default)]
pub(crate) struct Writer {
    /// The newest capture written.
    pub(crate) written: u64,
    /// Texts this run may delete once the session no longer names them: written by this run,
    /// or named by the session it restored cleanly and whole.
    pub(crate) owned: HashSet<String>,
    /// The session file could not be read at launch: why. It is not written over until it
    /// reads, or the user says to start afresh.
    pub(crate) blocked: Option<String>,
}

pub(crate) type Locked<T> = std::sync::Arc<std::sync::Mutex<T>>;

pub(crate) static WRITERS: std::sync::LazyLock<std::sync::Mutex<HashMap<PathBuf, Locked<Writer>>>> =
    std::sync::LazyLock::new(Default::default);

/// The writer of the session file at `path`, locked while it writes (the map only to look up).
pub(crate) fn writer(path: &Path) -> Locked<Writer> {
    let mut all = WRITERS.lock().unwrap_or_else(|e| e.into_inner());
    all.entry(path.to_path_buf()).or_default().clone()
}

/// A test's stuck disk: a write of the session file it names waits until it names none.
#[cfg(test)]
pub static STUCK: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// `f` on the session file's writer, once a write under way is done.
pub(crate) fn with_writer<R>(path: &Path, f: impl FnOnce(&mut Writer) -> R) -> R {
    let writer = writer(path);
    let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut w)
}

/// `with_writer`, or `None` at once while a write is under way.
pub(crate) fn try_with_writer<R>(path: &Path, f: impl FnOnce(&mut Writer) -> R) -> Option<R> {
    let writer = writer(path);
    let mut w = match writer.try_lock() {
        Ok(w) => w,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return None,
    };
    Some(f(&mut w))
}

/// Write capture `n`, unless a newer one was. A session that could not be read at launch is
/// not written over until it reads again; as Orlo quits it is moved aside first, whole
/// (`session::set_aside`), so this run's unsaved changes are kept too.
pub(crate) fn write_session(
    w: &mut Writer,
    path: &Path,
    n: u64,
    (s, texts): &(Session, Texts),
    quitting: bool,
) -> std::io::Result<()> {
    #[cfg(test)]
    while STUCK.lock().unwrap().as_deref() == Some(path) {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    if n < w.written {
        return Ok(());
    }
    if let Some(why) = &w.blocked {
        // readable again (a permission put back, a sync finished): kept, then written over
        if !quitting && !session::readable(path) {
            return Err(std::io::Error::other(format!(
                "the session file could not be read ({why}), so Orlo has not written over it"
            )));
        }
        session::set_aside(path)?;
        w.blocked = None;
    }
    session::save(path, s, texts, &mut w.owned)?;
    w.written = n;
    Ok(())
}

/// Write the session as Orlo goes (Quit, a signal, the last window closing; see
/// `capture_session`), on a thread of its own: a stuck disk gets two seconds, then Orlo says
/// so instead of hanging.
pub fn save_session(current: Option<(WindowState, Texts)>, hot: bool, cx: &mut App) -> std::io::Result<()> {
    use std::sync::mpsc::RecvTimeoutError;
    let n = CAPTURED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let captured = capture_session(current, hot, cx);
    let path = session::path();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = with_writer(&path, |w| write_session(w, &path, n, &captured, true));
        let _ = tx.send(r);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(r) => r,
        Err(RecvTimeoutError::Timeout) => {
            Err(std::io::Error::other("the session is still being written after 2 s (is the disk stuck?)"))
        }
        Err(RecvTimeoutError::Disconnected) => Err(std::io::Error::other("the session write stopped short")),
    }
}

/// Bring back the session at launch, as main.rs does: its windows (or a fresh one), the recent
/// lists, and what this run may do to its files. A view that cannot be placed keeps the
/// session aside (`session::keep_bad`). A session kept aside, or one that cannot be read, is
/// said.
pub fn restore_session(cx: &mut App) -> Vec<WindowHandle<Workspace>> {
    let path = session::path();
    let (saved, status, mut kept) = session::load_with(&path);
    let names = session::named(&saved);
    // temp files a crash left beside the files Orlo had open or used lately, and the session
    let views = saved.windows.iter().flat_map(|w| &w.groups);
    let files: Vec<PathBuf> = (views.flat_map(|g| &g.views))
        .map(|v| v.path.clone())
        .chain(saved.recent_files.iter().cloned())
        .chain([path.clone()])
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    cx.background_executor().spawn(async move { crate::editing::text::sweep_temp_files(&files) }).detach();
    cx.set_global(Recent { files: saved.recent_files, folders: saved.recent_folders, projects: saved.recent_projects });
    cx.default_global::<Unplaced>().0 = false;
    let mut windows: Vec<_> = (saved.windows.into_iter()).map(|st| open_window(Some(st), cx)).collect();
    if windows.is_empty() {
        windows.push(open_window(None, cx));
    }
    let unplaced = std::mem::take(&mut cx.default_global::<Unplaced>().0);
    let mut blocked = match &status {
        session::Status::Unreadable(e) => Some(e.clone()),
        _ => None,
    };
    if status == session::Status::Clean && unplaced {
        match session::keep_bad(&path, &names) {
            Ok(snapshot) => kept = Some(snapshot),
            Err(e) => blocked = Some(format!("the recovery snapshot could not be kept: {e}")),
        }
    }
    let owned = match status == session::Status::Clean && !unplaced {
        true => names,
        false => HashSet::new(),
    };
    with_writer(&path, |w| (w.owned, w.blocked) = (owned, blocked.clone()));
    cx.set_global(SessionBlocked(blocked.clone()));
    if let Some(e) = blocked {
        windows[0].update(cx, |_, window, cx| unreadable_session(&path, &e, window, cx)).ok();
    }
    if let Some(kept) = kept {
        windows[0].update(cx, |_, window, cx| session_kept(&kept, window, cx)).ok();
    }
    // this run's session from now on: a crash before any edit restores what it showed
    autosave(cx);
    windows
}

/// Views a restore could not place (a group beyond the window's layout): the session keeps
/// them aside instead of letting a save delete their texts.
#[derive(Default)]
pub(crate) struct Unplaced(pub(crate) bool);

impl gpui::Global for Unplaced {}

/// The documents restores made from unsaved texts, by the text's name (`Unsaved::file`), in
/// every window: the session names one text for a document shown in several, so a window
/// restored later shows the document an earlier one made.
#[derive(Default)]
pub(crate) struct Restored(HashMap<String, gpui::WeakEntity<OpenDocument>>);

impl gpui::Global for Restored {}

impl Restored {
    /// The document made from text `name`, while a tab shows it.
    fn get(name: &str, cx: &App) -> Option<Entity<OpenDocument>> {
        let d = cx.try_global::<Self>()?.0.get(name)?.upgrade()?;
        (d.read(cx).tabs > 0).then_some(d)
    }

    fn insert(name: String, doc: &Entity<OpenDocument>, cx: &mut App) {
        let all = &mut cx.default_global::<Self>().0;
        all.retain(|_, d| d.upgrade().is_some());
        all.insert(name, doc.downgrade());
    }
}

/// Launch kept what it found of the session aside (`session::keep_bad`): where, with a way to
/// see it. Its drafts may be the only copy.
pub(crate) fn session_kept(kept: &Path, window: &mut Window, cx: &mut App) {
    let detail = format!(
        "Parts of the last session could not be brought back. What Orlo found, unsaved text \
         included, is kept as {} in {}.",
        kept.file_name().unwrap_or_default().to_string_lossy(),
        kept.parent().unwrap_or(kept).display()
    );
    let answer =
        window.prompt(PromptLevel::Warning, "The session was kept aside", Some(&detail), &["OK", "Show in Finder"], cx);
    let kept = kept.to_path_buf();
    cx.spawn(async move |cx| {
        if answer.await == Ok(1) {
            cx.update(|cx| cx.reveal_path(&kept)).ok();
        }
    })
    .detach();
}

/// The session file could not be read at launch: why, until it is written again. File > Session
/// Could Not Be Read… shows the alert again meanwhile.
#[derive(Default)]
pub struct SessionBlocked(pub Option<String>);

impl gpui::Global for SessionBlocked {}

/// The session file is there but could not be read at launch: Orlo leaves it and its texts
/// alone and says so; the user may start a new session (the old one kept aside). Said again
/// from the File menu while it lasts.
pub(crate) fn unreadable_session(path: &Path, e: &str, window: &mut Window, cx: &mut App) {
    let detail = format!(
        "{} could not be read ({e}). Orlo has not written over it, and the unsaved text in its \
         Session folder is untouched. Until it can be read, this run's windows and unsaved \
         changes are saved for next time only as Orlo quits, after the file is moved aside.",
        path.display()
    );
    let answer = window.prompt(
        PromptLevel::Warning,
        "Could not read the session",
        Some(&detail),
        &["Keep It Untouched", "Start a New Session"],
        cx,
    );
    let path = path.to_path_buf();
    cx.spawn(async move |cx| {
        if answer.await != Ok(1) {
            return;
        }
        // under the writer's lock: a quit, or a write that found it readable again, may have
        // moved it aside meanwhile, and a new session must not be
        let set_aside = cx.background_spawn(async move {
            with_writer(&path, |w| match w.blocked.is_some() {
                true => session::set_aside(&path).map(|()| w.blocked = None),
                false => Ok(()),
            })
        });
        match set_aside.await {
            Ok(()) => {
                cx.update(|cx| {
                    cx.set_global(SessionBlocked(None));
                    autosave(cx);
                })
                .ok();
            }
            Err(e) => eprintln!("Orlo: could not move the session aside: {e}"),
        }
    })
    .detach();
}

/// The hot-exit autosave: the wait for edits to settle (an edit starts it again), whether a
/// write is under way (an edit never drops it) and whether one more is due when it lands,
/// when the oldest change not yet captured was made, and how many writes in a row failed (the
/// status bar says so from 3).
#[derive(Default)]
pub(crate) struct Autosave {
    pub(crate) timer: Option<Task<()>>,
    pub(crate) writing: bool,
    pub(crate) again: bool,
    pub(crate) since: Option<std::time::Instant>,
    pub(crate) failures: u32,
}

impl gpui::Global for Autosave {}

/// Write the session, unsaved buffers and all, once edits have settled for a few seconds (and
/// at most 30 s after the oldest change not yet written, however long typing goes on), off the
/// UI thread and one write at a time: a crash, `kill` or a closed Terminal loses at most those
/// seconds. A failed write is tried again after 2 s, then 4, 8 ... up to 64; edits do not
/// bring that forward.
pub fn autosave(cx: &mut App) {
    use std::time::Duration;
    // the executor's clock: a test's moves when it advances it
    let now = cx.background_executor().now();
    let a = cx.default_global::<Autosave>();
    let since = *a.since.get_or_insert(now);
    if a.failures > 0 {
        return;
    }
    let waited = now.saturating_duration_since(since);
    let wait = Duration::from_secs(3).min(Duration::from_secs(30).saturating_sub(waited));
    autosave_after(wait, cx);
}

pub(crate) fn autosave_after(wait: std::time::Duration, cx: &mut App) {
    let timer = cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
        cx.update(write_autosave).ok();
    });
    cx.default_global::<Autosave>().timer = Some(timer);
}

/// Capture the session and write it in the background; while a write is under way, once that
/// one lands.
pub(crate) fn write_autosave(cx: &mut App) {
    let a = cx.default_global::<Autosave>();
    if a.writing {
        a.again = true;
        return;
    }
    #[cfg(test)]
    AUTOSAVES.set(AUTOSAVES.get() + 1);
    // this capture takes in every change made so far
    (a.writing, a.again, a.since) = (true, false, None);
    let n = CAPTURED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let captured = capture_session(None, true, cx);
    let path = session::path();
    let ex = cx.background_executor().clone();
    let write = cx.background_spawn(async move {
        loop {
            // Quit's write may hold it on a stuck disk: waited for without holding a thread
            match try_with_writer(&path, |w| write_session(w, &path, n, &captured, false)) {
                Some(r) => return r,
                None => ex.timer(std::time::Duration::from_millis(5)).await,
            }
        }
    });
    cx.spawn(async move |cx| {
        let result = write.await;
        cx.update(|cx| {
            let was = session_unsaved(cx);
            let a = cx.default_global::<Autosave>();
            a.writing = false;
            let again = std::mem::take(&mut a.again);
            match result {
                Ok(()) => {
                    a.failures = 0;
                    // written: the file reads, or was moved aside as it came to
                    if cx.try_global::<SessionBlocked>().is_some_and(|b| b.0.is_some()) {
                        cx.set_global(SessionBlocked(None));
                    }
                    if again {
                        write_autosave(cx);
                    }
                }
                Err(e) => {
                    eprintln!("Orlo: could not save the session: {e}");
                    a.failures += 1;
                    let retry = std::time::Duration::from_secs(1 << a.failures.min(6));
                    autosave_after(retry, cx);
                }
            }
            // the status bar's SESSION NOT SAVED comes or goes; otherwise nothing to redraw
            if session_unsaved(cx) != was {
                cx.refresh_windows();
            }
        })
        .ok();
    })
    .detach();
}

#[cfg(test)]
thread_local! {
    /// Autosave writes started on this thread (a test's).
    pub static AUTOSAVES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Three autosaves in a row failed: the status bar says the session is not saved.
pub fn session_unsaved(cx: &App) -> bool {
    cx.try_global::<Autosave>().is_some_and(|a| a.failures >= 3)
}

/// The quit under way wrote the session, or was told to go without it: the app's own quit
/// does not write it again.
pub(crate) struct QuitSaved;

impl gpui::Global for QuitSaved {}

/// SIGHUP (the Terminal that started Orlo closed), SIGINT (Ctrl-C) and SIGTERM (`kill`): the
/// session is written as Quit writes it, then Orlo exits. The handler only notes the signal;
/// a thread of its own waits for it, and the UI thread does the work.
pub fn quit_on_signals(cx: &mut App) {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let mut signals = match signal_hook::iterator::Signals::new([SIGHUP, SIGINT, SIGTERM]) {
        Ok(s) => s,
        Err(e) => return eprintln!("Orlo: cannot watch for SIGHUP, SIGINT and SIGTERM: {e}"),
    };
    let arrived = crate::app::handoff::Inbox::default();
    let tell = arrived.clone();
    std::thread::spawn(move || {
        let die = |signal| drop(signal_hook::low_level::emulate_default_handler(signal));
        let wait = std::time::Duration::from_secs(10);
        watch_signals(signals.forever(), tell, wait, die);
    });
    quit_on(arrived, cx);
}

/// Each of `signals` to `tell`. A second one, or `wait` going by with Orlo still running (a
/// stuck disk, a hang), ends it as the signal would have with no handler: `die`.
pub(crate) fn watch_signals(
    signals: impl Iterator<Item = i32>,
    tell: crate::app::handoff::Inbox,
    wait: std::time::Duration,
    die: impl Fn(i32) + Clone + Send + 'static,
) {
    for (k, signal) in signals.enumerate() {
        if k > 0 {
            eprintln!("Orlo: signal {signal} again: quitting at once");
            return die(signal);
        }
        eprintln!("Orlo: signal {signal}: saving the session and quitting");
        tell.push(Vec::new());
        let die = die.clone();
        std::thread::spawn(move || {
            std::thread::sleep(wait);
            eprintln!("Orlo: still running {wait:?} after signal {signal}: quitting at once");
            die(signal);
        });
    }
}

/// A signal `arrived`: the session is written as Quit writes it, then Orlo quits.
pub(crate) fn quit_on(arrived: crate::app::handoff::Inbox, cx: &mut App) {
    cx.spawn(async move |cx| {
        arrived.next().await;
        cx.update(|cx| {
            crate::app::handoff::stop_listening();
            quit_once_saved(cx);
        })
        .ok();
    })
    .detach();
}

/// Saves under way land first, as for Quit (the signal's watchdog ends one that hangs), then
/// the session is written and Orlo quits.
fn quit_once_saved(cx: &mut App) {
    Documents::cancel_file_autosaves(cx);
    if Documents::saving(cx) {
        return Documents::when_all_saved(quit_once_saved, cx);
    }
    let _ = save_for_quit(cx);
    cx.quit();
}

/// `save_session` for a quit: said on stderr when it fails, and not written again by the app's
/// quit when it worked.
pub(crate) fn save_for_quit(cx: &mut App) -> std::io::Result<()> {
    for window in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        window.update(cx, |ws, window, cx| ws.settle_compositions(window, cx)).ok();
    }
    save_workspaces(cx);
    let r = save_session(None, true, cx);
    match &r {
        Ok(()) => cx.set_global(QuitSaved),
        Err(e) => eprintln!("Orlo: could not save the session: {e}"),
    }
    r
}

/// Each project window's workspace file, unsaved text included, as Orlo quits: the project
/// opened later in another session finds its windows as they were.
pub(crate) fn save_workspaces(cx: &mut App) {
    for w in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        let r = w.update(cx, |ws, window, cx| match ws.project.is_some() {
            true => ws.save_workspace(window, true, cx),
            false => Ok(()),
        });
        if let Ok(Err(e)) = r {
            eprintln!("Orlo: could not save the workspace: {e}");
        }
    }
}

/// Quit, the Dock's Quit, logout: Sublime's hot exit keeps unsaved buffers in the session.
/// Closing the last window saves before it goes (after its prompt).
pub fn save_on_quit(cx: &mut App) {
    cx.on_app_quit(|cx| {
        app_quitting(cx);
        cx.background_executor().spawn(async {
            orlo_terminal::pty::wait_for_closing_sessions();
        })
    })
    .detach();
}

/// The app is quitting (Quit, the Dock's Quit, logout): the session keeps unsaved buffers.
/// Quit and the signals wrote it already, and after the last window closed it was written as
/// that window went. Hand-offs stop first.
pub(crate) fn app_quitting(cx: &mut App) {
    Documents::cancel_file_autosaves(cx);
    crate::app::handoff::finish_waiters();
    crate::app::handoff::stop_listening();
    if !cx.windows().is_empty() && !cx.has_global::<QuitSaved>() {
        let _ = save_for_quit(cx);
    }
    for window in workspace_windows(cx).collect::<Vec<_>>() {
        window.update(cx, |ws, _, cx| ws.close_terminals(cx)).ok();
    }
}

/// Quit: the session is written first, unsaved buffers kept (Sublime's hot exit); when it
/// cannot be, Orlo says so and stays, unless told to quit anyway.
pub fn quit(cx: &mut App) {
    Documents::cancel_file_autosaves(cx);
    // saves under way land first: the session then keeps what they left unsaved
    if Documents::saving(cx) {
        for w in cx.windows() {
            if let Some(w) = w.downcast::<Workspace>() {
                w.update(cx, |ws, _, cx| ws.status_message("Saving…", cx)).ok();
            }
        }
        return Documents::when_all_saved(quit, cx);
    }
    crate::app::handoff::stop_listening();
    let Err(e) = save_for_quit(cx) else {
        return cx.quit();
    };
    // staying for now: a launch meanwhile hands its files to this Orlo, behind the alert
    crate::app::handoff::resume_listening();
    let window = cx.active_window().or_else(|| cx.windows().into_iter().next()).and_then(|w| w.downcast::<Workspace>());
    let Some(window) = window else {
        crate::app::handoff::stop_listening();
        return cx.quit();
    };
    let detail = format!("{e}\n\nUnsaved changes and untitled tabs would be lost.");
    window
        .update(cx, |_, window, cx| {
            let answer = window.prompt(
                PromptLevel::Critical,
                "Could not save the session",
                Some(&detail),
                &["Cancel", "Quit Anyway"],
                cx,
            );
            cx.spawn(async move |_, cx| {
                if answer.await == Ok(1) {
                    cx.update(|cx| {
                        crate::app::handoff::stop_listening();
                        cx.set_global(QuitSaved);
                        cx.quit();
                    })
                    .ok();
                } else {
                    cx.update(|cx| {
                        Documents::resume_file_autosaves(cx);
                        super::file_autosave::refresh_all(cx);
                    })
                    .ok();
                }
            })
            .detach();
        })
        .ok();
}
pub(crate) fn push_recent(list: &mut Vec<PathBuf>, p: PathBuf) {
    list.retain(|x| x != &p);
    list.insert(0, p);
    list.truncate(12);
}

/// Closed history owns only folds. Keep the new view's preparation and the attached
/// document's current policy; default-filled closed metadata is not a full session snapshot.
pub(super) fn restore_closed_folds(editor: &Entity<Editor>, v: &ViewState, cx: &mut App) {
    editor.update(cx, |e, cx| {
        let folds = v.folds.as_ref().map(|f| f.restore(e.doc(cx))).unwrap_or_default();
        e.set_folds_quietly(folds, cx);
    });
}

/// A restored view takes its selections, bookmarks, scroll and wrap from `v`. The indentation
/// and line ending `v` kept go only to a document no tab shows yet: one open in another window
/// keeps its own.
pub(super) fn restore_view(editor: &Entity<Editor>, v: &ViewState, cx: &mut App) {
    let shown = editor.read(cx).document.read(cx).tabs > 0;
    editor.update(cx, |e, cx| {
        e.change(cx, |s| match shown {
            true => v.apply_view(s),
            false => {
                v.apply(s);
                s.doc.indent_override = v.indent_override.unwrap_or_default();
                s.doc.line_ending_override = v.line_ending_override.is_some();
            }
        });
        let folds = v.folds.as_ref().map(|f| f.restore(e.doc(cx))).unwrap_or_default();
        e.set_folds_quietly(folds, cx);
        e.set_top(v.top(e.doc(cx)));
        e.wrap_override = v.wrap;
        e.wrap_width_override = v.wrap_width;
    });
}

impl Workspace {
    /// A deliberate view change supersedes the saved active tab, even if the user returns.
    pub(crate) fn invalidate_restoring_active(&mut self) {
        for pending in &mut self.restoring {
            pending.active = pending.claims_active(&self.views);
        }
    }

    /// Closing the last window quits Orlo, so its state is saved first; another window's
    /// close leaves the session with the next autosave.
    pub(crate) fn save_if_last(&self, window: &Window, cx: &mut App) {
        if cx.windows().len() > 1 {
            return autosave(cx);
        }
        crate::app::handoff::stop_listening();
        let mut texts = Texts::new();
        let me = self.capture(window, false, &mut texts, cx);
        if let Err(e) = save_session(Some((me, texts)), false, cx) {
            eprintln!("Orlo: could not save the session: {e}");
        }
    }

    /// The window for the session file; with `hot`, buffers with unsaved changes keep their
    /// text, untitled ones included.
    pub fn capture(&self, window: &Window, hot: bool, texts: &mut Texts, cx: &App) -> WindowState {
        let recovery: HashMap<_, _> = if hot {
            self.open_editors(cx)
                .into_iter()
                .filter_map(|editor| {
                    let e = editor.read(cx);
                    let composition = e.recovery_composition(cx)?;
                    let (removed, selections) = (composition.removed, composition.selections);
                    let mut rope = e.doc(cx).rope.clone();
                    let mut endings = e.doc(cx).endings.clone();
                    for range in removed.iter().rev() {
                        if let Some(map) = &mut endings {
                            map.splice(&rope, range.clone(), "", e.doc(cx).line_ending);
                        }
                        rope.remove(range.clone());
                    }
                    Some((
                        e.doc(cx).id(),
                        Recoverable { rope, endings, removed, owner: editor.entity_id(), selections },
                    ))
                })
                .collect()
        } else {
            HashMap::new()
        };
        let (b, maximized) = match window.window_bounds() {
            WindowBounds::Windowed(b) | WindowBounds::Fullscreen(b) => (b, false),
            WindowBounds::Maximized(b) => (b, true),
        };
        // gpui reports the frame but opens a window by its content size
        // a full-screen window's restore bounds keep the title bar, so it comes back
        // that much taller
        let titlebar = window.bounds().size.height - window.viewport_size().height;
        let mut expanded: Vec<PathBuf> = self.sidebar.expanded().cloned().collect();
        expanded.sort();
        // in distraction-free mode the session keeps the window as it was before it
        let chrome = (self.distraction_free.as_ref()).map_or_else(|| self.chrome(), |m| m.normal);
        // the first view of each unsaved buffer, numbered from 1 (`ViewState::buffer`)
        let mut unsaved: Vec<&Entity<Editor>> = Vec::new();
        // Pending recovery ids belong to the previous session, not this capture's live ids.
        let mut pending_buffers = HashMap::new();
        let mut next_pending = self.views.tabs().count() + 1;
        let mut groups: Vec<GroupState> = self
            .views
            .groups
            .iter()
            .enumerate()
            .map(|(gi, g)| {
                let mut out = GroupState::default();
                let mut pending =
                    self.restoring.iter().filter(|p| p.group.min(self.views.groups.len() - 1) == gi).peekable();
                let pending_active = pending.clone().any(|p| p.claims_active(&self.views));
                for i in 0..=g.tabs.len() {
                    while pending.peek().is_some_and(|p| p.place.min(g.tabs.len()) <= i) {
                        let p = pending.next().unwrap();
                        if p.claims_active(&self.views) {
                            out.active = out.views.len();
                        }
                        let mut view = p.view.clone();
                        if view.unsaved.is_some() {
                            view.buffer =
                                *pending_buffers.entry((view.path.clone(), view.buffer)).or_insert_with(|| {
                                    let id = next_pending;
                                    next_pending += 1;
                                    id
                                });
                        }
                        out.views.push(view);
                    }
                    let Some(t) = g.tabs.get(i) else { break };
                    let e = t.read(cx);
                    let keep = hot && e.doc(cx).unsaved();
                    // untitled buffers come back only with hot exit
                    if !keep && e.doc(cx).path.is_none() {
                        continue;
                    }
                    if !pending_active && i <= g.active {
                        out.active = out.views.len();
                    }
                    let path = e.doc(cx).path.clone().unwrap_or_default();
                    let doc = e.doc(cx);
                    let mut v = ViewState {
                        indent_override: (doc.indent_override.iter()).any(|&v| v).then_some(doc.indent_override),
                        line_ending_override: (doc.line_ending_override).then(|| doc.line_ending.to_string()),
                        folds: e.capture_folds(cx),
                        wrap: e.wrap_override,
                        wrap_width: e.wrap_width_override,
                        pinned: g.pinned.contains(t),
                        preview: g.preview.as_ref() == Some(t),
                        ..ViewState::capture(path, &e.read_state(cx), e.top(cx))
                    };
                    if let Some(snapshot) = recovery.get(&doc.id()) {
                        v.selections = if snapshot.owner == t.entity_id() {
                            snapshot.selections.clone()
                        } else {
                            v.selections.into_iter().map(|(a, b)| (snapshot.offset(a), snapshot.offset(b))).collect()
                        };
                        v.top = v.top.map(|at| snapshot.offset(at));
                        if let Some(at) = v.top {
                            v.scroll =
                                snapshot.rope.char_to_line(at.min(snapshot.rope.len_chars())) as f32 + v.scroll.fract();
                        }
                        v.bookmarks = e
                            .state
                            .bookmarks
                            .iter()
                            .map(|&at| snapshot.rope.char_to_line(snapshot.offset(at).min(snapshot.rope.len_chars())))
                            .collect();
                        // The live fold fingerprint describes a different rope; restore can refold normally.
                        v.folds = None;
                    }
                    if keep {
                        let same = |u: &&Entity<Editor>| u.read(cx).same_document(e);
                        v.buffer = match unsaved.iter().position(same) {
                            Some(k) => k + 1,
                            None => {
                                unsaved.push(t);
                                let name = session::text_name(e.doc(cx));
                                texts.push((
                                    name.clone(),
                                    recovery.get(&doc.id()).map_or_else(|| doc.rope.clone(), |r| r.rope.clone()),
                                ));
                                v.unsaved = Some(Unsaved::capture(e.doc(cx), name));
                                if let Some(snapshot) = recovery.get(&doc.id()) {
                                    v.unsaved.as_mut().unwrap().endings =
                                        snapshot.endings.as_ref().map(|m| serde_json::Value::String(m.compact()));
                                }
                                unsaved.len()
                            }
                        };
                    }
                    out.views.push(v);
                }
                out
            })
            .collect();
        // New opens still being read have no saved place: keep them at the group's end.
        let reading = (self.loading.iter())
            .filter(|p| !self.restoring.iter().any(|v| &v.view.path == *p))
            .filter(|p| self.previewing.get(*p) != Some(&PendingOpen::Cancelled))
            .map(|p| {
                (
                    self.views.active,
                    ViewState {
                        path: p.clone(),
                        preview: self.previewing.get(p) == Some(&PendingOpen::Preview),
                        ..ViewState::default()
                    },
                )
            });
        let later: Vec<(usize, ViewState)> = reading.collect();
        for (g, v) in self.unavailable.iter().chain(&later) {
            let g = (*g).min(groups.len().saturating_sub(1));
            if let Some(group) = groups.get_mut(g) {
                group.views.push(v.clone());
            }
        }
        WindowState {
            closed_views: self.closed[self.closed.len().saturating_sub(session::folds::MAX_CLOSED_VIEWS)..].to_vec(),
            bounds: Some([
                f32::from(b.origin.x),
                f32::from(b.origin.y),
                f32::from(b.size.width),
                f32::from(b.size.height - titlebar),
            ]),
            display: window.display(cx).and_then(|d| d.uuid().ok()).map(|u| u.to_string()),
            maximized,
            folder: self.roots.first().map(|r| r.path.clone()),
            folders: self.roots.clone(),
            project: self.project.as_ref().map(|p| p.path.clone()),
            workspace: self.workspace_file.clone(),
            expanded,
            sidebar_visible: chrome.sidebar,
            outline_visible: self.outline.visible,
            sidebar_width: self.sidebar.width,
            show_open_files: self.sidebar.show_open_files,
            show_tabs: chrome.tabs,
            show_status_bar: chrome.status_bar,
            font_size: self.overrides.font_size,
            minimap: chrome.minimap,
            rulers: self.overrides.rulers.clone(),
            white_space: self.overrides.white_space,
            layout: self.views.layout.clone(),
            groups,
            active_group: self.views.active,
            sync_scroll: [self.sync_scroll.0, self.sync_scroll.1],
            panels: self.panel_places,
            git_sidebar_visible: chrome.git_sidebar,
            distraction_free: self.distraction_free.as_ref().map(|_| self.chrome()),
        }
    }

    /// Bring back a saved window. Files gone or unreadable are skipped; a path seen before in
    /// this window becomes another view of that buffer.
    pub fn restore(&mut self, mut st: WindowState, window: &mut Window, cx: &mut Context<Self>) {
        // Settle before restored settings can invalidate a live composition target.
        self.settle_compositions(window, cx);
        st.sanitize();
        self.restore_project(&mut st, cx);
        self.closed = std::mem::take(&mut st.closed_views);
        self.closed.drain(..self.closed.len().saturating_sub(session::folds::MAX_CLOSED_VIEWS));
        for view in &mut self.closed {
            view.unsaved = None;
            view.buffer = 0;
        }
        self.set_roots(std::mem::take(&mut st.folders), cx);
        // a root not found now keeps its expanded folders for when it is back
        let kept = |p: &PathBuf| {
            p.is_dir()
                || (self.roots.iter()).any(|r| r.missing && p.starts_with(&r.path))
                || session::on_missing_volume(p)
        };
        self.sidebar.set_expanded(st.expanded.into_iter().filter(kept).collect());
        self.sidebar.visible = st.sidebar_visible;
        self.outline.visible = st.outline_visible;
        self.sidebar.width = st.sidebar_width;
        self.sidebar.show_open_files = st.show_open_files;
        self.show_tabs = st.show_tabs;
        self.show_status_bar = st.show_status_bar;
        self.sync_scroll = (st.sync_scroll[0], st.sync_scroll[1]);
        self.panel_places = st.panels;
        self.git.sidebar_open = st.git_sidebar_visible;
        let distraction_free = st.distraction_free;
        self.overrides = Overrides {
            font_size: st.font_size,
            minimap: st.minimap,
            rulers: st.rulers,
            white_space: st.white_space,
            ..Overrides::default()
        };
        self.apply_view(cx);
        // the fresh window's untitled tab gives way
        let fresh: Vec<_> = self.views.tabs().cloned().collect();
        for t in &fresh {
            self.remove_view(t, cx);
        }
        if st.layout.groups() > 0 {
            self.views.set_layout(st.layout);
        }
        let mut loaded: Vec<Entity<Editor>> = Vec::new();
        let mut unsaved: HashMap<usize, Entity<Editor>> = HashMap::new();
        // a big file is read in the background, then takes its place: (group, place, active)
        let mut big: Vec<(usize, usize, bool, ViewState)> = Vec::new();
        for (g, group) in st.groups.into_iter().enumerate() {
            if g >= self.views.groups.len() {
                // more groups than the layout has (a hand-edited session): their views have no
                // place, so the session is kept aside rather than lose their texts
                if !group.views.is_empty() {
                    cx.default_global::<Unplaced>().0 = true;
                }
                continue;
            }
            self.views.focus(g);
            let mut active = 0;
            for (i, mut v) in group.views.into_iter().enumerate() {
                // Deferred siblings need the same recovery payload until a document exists.
                if v.buffer != 0 && v.unsaved.is_none() {
                    v.unsaved = big.iter().find(|b| b.3.buffer == v.buffer).and_then(|b| b.3.unsaved.clone());
                }
                let same = match v.buffer {
                    0 => loaded.iter().find(|e| e.read(cx).doc(cx).path.as_deref() == Some(v.path.as_path())).cloned(),
                    n => unsaved.get(&n).cloned(),
                };
                // the same text another window brought back: its document
                let name = v.unsaved.as_ref().and_then(|u| u.file.clone());
                let restored = name.as_deref().and_then(|n| Restored::get(n, cx));
                let size = std::fs::metadata(&v.path).map_or(0, |m| m.len());
                let later = (big.iter()).any(|b| b.3.path == v.path && b.3.buffer == v.buffer);
                if same.is_none()
                    && restored.is_none()
                    && (pipelines::reading_blocked(&v.path, cx)
                        || later
                        || (v.unsaved.is_none() && size > LOAD_IN_BACKGROUND))
                {
                    let at = self.views.groups[g].tabs.len();
                    big.push((g, at, i == group.active, v));
                    continue;
                }
                let state = match &v.unsaved {
                    _ if same.is_some() || restored.is_some() => None,
                    Some(u) => Some(u.state(&v.path)),
                    None => Some(
                        Loaded::read_with_hint(&v.path, v.disk_encoding_hint.or(v.encoding))
                            .map(|l| EditorState::from_loaded(&v.path, l)),
                    ),
                };
                let editor = match (same, restored, state) {
                    (Some(e), _, _) => e.update(cx, |src, cx| cx.new(|cx| src.another_view(cx))),
                    (None, Some(document), _) => {
                        let attached = document.read(cx).model.attach_view();
                        let e = self.view_of(document, attached, cx);
                        loaded.push(e.clone());
                        e
                    }
                    (None, None, Some(Ok(state))) => {
                        let e = self.new_editor(state, cx);
                        if let Some(name) = name {
                            let document = e.read(cx).document.clone();
                            Restored::insert(name, &document, cx);
                        }
                        loaded.push(e.clone());
                        e
                    }
                    _ => {
                        if session::on_missing_volume(&v.path) {
                            self.unavailable.push((g, v));
                        }
                        continue;
                    }
                };
                if v.buffer != 0 {
                    unsaved.entry(v.buffer).or_insert_with(|| editor.clone());
                }
                restore_view(&editor, &v, cx);
                self.attach(editor.clone(), false, window, cx);
                if v.pinned {
                    self.views.set_pinned(&editor, true);
                }
                if v.preview && !v.pinned {
                    self.views.groups[g].preview = Some(editor.clone());
                }
                if i <= group.active {
                    active = (self.views.groups[g].tabs.iter()).position(|t| *t == editor).unwrap_or(0);
                }
            }
            self.views.select(g, active);
        }
        self.views.focus(st.active_group);
        self.restore_later(big, window, cx);
        if let Some(shown) = distraction_free {
            self.distraction_free = None;
            self.enter_distraction_free(Some(shown), window, cx);
        }
        if self.views.tabs().next().is_none() && self.restoring.is_empty() {
            self.add_tab(EditorState::new(), window, cx);
        }
        self.focus_editor(window, cx);
        self.refresh_menus(cx);
        self.poll_disk(window, cx);
        cx.notify();
    }

    /// A session's big files, read in the background; each view takes its place in its group
    /// when the read lands. The saved active tab yields to a newer view selection.
    pub(crate) fn restore_later(
        &mut self,
        big: Vec<(usize, usize, bool, ViewState)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_later_kind(big, false, window, cx);
    }

    pub(crate) fn reopen_later(&mut self, view: ViewState, window: &mut Window, cx: &mut Context<Self>) {
        let group = self.views.active;
        let place = self.views.groups[group].tabs.len();
        self.restore_later_kind(vec![(group, place, true, view)], true, window, cx);
    }

    fn restore_later_kind(
        &mut self,
        big: Vec<(usize, usize, bool, ViewState)>,
        reopen: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // each file read once, wherever its views are
        let mut seen = HashSet::new();
        let paths: Vec<_> =
            (big.iter().map(|b| (b.3.path.clone(), b.3.buffer))).filter(|key| seen.insert(key.clone())).collect();
        let already_restoring: HashSet<_> =
            self.restoring.iter().map(|v| (v.view.path.clone(), v.view.buffer)).collect();
        self.restoring.extend(big.into_iter().map(|(group, place, active, view)| RestoringView {
            group,
            place,
            active,
            fallback: self.views.groups.get(group).and_then(|g| g.tabs.get(g.active).map(Entity::entity_id)),
            view,
            reopen,
        }));
        cx.notify();
        let gen = self.restore_gen;
        for (path, buffer) in paths {
            if already_restoring.contains(&(path.clone(), buffer)) {
                continue;
            }
            // Closed history carries only an explicit disk hint; never apply its default-filled policy.
            let encoding =
                self.restoring.iter().find(|v| v.view.path == path && v.view.buffer == buffer).and_then(|v| {
                    v.view.disk_encoding_hint.or_else(|| (!v.reopen).then_some(v.view.encoding).flatten())
                });
            let recovery = self
                .restoring
                .iter()
                .filter(|v| v.view.path == path && v.view.buffer == buffer)
                .find_map(|v| v.view.unsaved.clone());
            self.loading.insert(path.clone());
            let p = path.clone();
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    // Restoring a project must respect the same file/repository writes as Open.
                    pipelines::wait_for_write(&path, cx).await;
                    if !this.update(cx, |ws, _| ws.restore_gen == gen).unwrap_or(false) {
                        return;
                    }
                    let Ok(observed) = cx.update(|_, cx| pipelines::observe_read(&path, cx)) else { return };
                    let p = p.clone();
                    let recovered = recovery.is_some();
                    let read = cx
                        .background_spawn(async move {
                            if recovered {
                                Ok(None)
                            } else {
                                Loaded::read_with_hint(&p, encoding).map(Some)
                            }
                        })
                        .await;
                    #[cfg(test)]
                    crate::testing::gate::wait("restore-read", &path).await;
                    let retry = this
                        .update_in(cx, |ws, window, cx| {
                            // the window moved on to another project meanwhile
                            if ws.restore_gen != gen {
                                return false;
                            }
                            if pipelines::reading_blocked(&path, cx) || (!recovered && observed.changed()) {
                                return true;
                            }
                            ws.invalidate_restoring_active();
                            ws.loading.remove(&path);
                            // a panel up: it keeps the keys, as for a file opened in the background
                            let focused = window.focused(cx);
                            let was_view = focused.as_ref().is_some_and(|f| ws.is_view_focus(f, cx));
                            let busy = ws.panel.is_some() || ws.quick_panel.is_some();
                            let browser = ws.pending_jumps.get(&path).is_some_and(|j| j.browser.is_some());
                            let show_result = ws.browser_jump_is_current(&path, window, cx);
                            let foreground = ws.active_editor().filter(|_| browser && !show_result);
                            let open = ws.views.tabs().any(|t| t.read(cx).doc(cx).path.as_ref() == Some(&path));
                            if open {
                                // A normal restore yields to a newer Open, but explicitly reopened
                                // views still need their own independent fold state.
                                ws.restoring.retain(|v| {
                                    v.view.path != path
                                        || v.view.buffer != buffer
                                        || v.reopen
                                        || v.view.unsaved.is_some()
                                });
                            }
                            let reopen = ws.restoring.iter().any(|v| {
                                v.view.path == path && v.view.buffer == buffer && (v.reopen || v.view.unsaved.is_some())
                            });
                            if open && !reopen && !ws.pending_jumps.contains_key(&path) {
                                ws.finish_cli_open(&path, None, cx);
                                return false;
                            }
                            if !open || reopen {
                                let loaded = match read {
                                    Ok(loaded) => loaded,
                                    Err(error) => {
                                        ws.restoring.retain(|v| v.view.path != path || v.view.buffer != buffer);
                                        ws.pending_jumps.remove(&path);
                                        ws.finish_cli_open(&path, Some(format!("{}: {error}", path.display())), cx);
                                        return false;
                                    }
                                };
                                let name = recovery.as_ref().and_then(|u| u.file.clone());
                                let first =
                                    if let Some(document) = name.as_deref().and_then(|name| Restored::get(name, cx)) {
                                        let attached = document.read(cx).model.attach_view();
                                        ws.view_of(document, attached, cx)
                                    } else {
                                        let state = match (&recovery, loaded) {
                                            (Some(u), _) => match u.state(&path) {
                                                Ok(state) => state,
                                                Err(error) => {
                                                    ws.status_message(error.to_string(), cx);
                                                    return false;
                                                }
                                            },
                                            (_, Some(loaded)) => EditorState::from_loaded(&path, loaded),
                                            _ => return false,
                                        };
                                        let editor = ws.new_editor(state, cx);
                                        if let Some(name) = name {
                                            let document = editor.read(cx).document.clone();
                                            Restored::insert(name, &document, cx);
                                        }
                                        editor
                                    };
                                let mut first_view = true;
                                while let Some(order) =
                                    ws.restoring.iter().position(|v| v.view.path == path && v.view.buffer == buffer)
                                {
                                    let RestoringView { group: g, place: at, active, view: v, reopen, .. } =
                                        ws.restoring.remove(order);
                                    let editor = match first_view {
                                        true => first.clone(),
                                        false => first.update(cx, |src, cx| cx.new(|cx| src.another_view(cx))),
                                    };
                                    first_view = false;
                                    if reopen {
                                        restore_closed_folds(&editor, &v, cx);
                                    } else {
                                        restore_view(&editor, &v, cx);
                                    }
                                    ws.wire(&editor, window, cx);
                                    let blank = ws.views.groups.get(g).is_some_and(|gr| match gr.tabs.as_slice() {
                                        [only] => Self::is_blank_untitled(only, cx),
                                        _ => false,
                                    });
                                    for later in ws.restoring.iter_mut().skip(order) {
                                        if later.group == g {
                                            later.place += 1;
                                        }
                                    }
                                    let (g, at) = ws.views.insert_at(g, at, editor.clone());
                                    if active || blank {
                                        ws.views.groups[g].active = at;
                                    }
                                    // The first loaded view becomes an empty group's restore fallback.
                                    let group = &ws.views.groups[g];
                                    if group.tabs.len() == 1 {
                                        let fallback = group.tabs.get(group.active).map(Entity::entity_id);
                                        for pending in &mut ws.restoring {
                                            if pending.group == g && pending.fallback.is_none() {
                                                pending.fallback = fallback;
                                            }
                                        }
                                    }
                                    if v.pinned {
                                        // Restoring a pin keeps its saved slot; interactive pinning moves it.
                                        let group = &mut ws.views.groups[g];
                                        group.pinned.insert(at.min(group.pinned.len()), editor);
                                    } else if v.preview {
                                        ws.views.groups[g].preview = Some(editor.clone());
                                    }
                                }
                            }
                            ws.restoring.retain(|v| v.view.path != path || v.view.buffer != buffer);
                            if let Some((g, i)) = foreground.and_then(|e| ws.views.find(&e)) {
                                ws.views.select(g, i);
                            } else {
                                ws.drop_blank_untitled(cx);
                            }
                            if show_result {
                                let target = ws.editor_for_path(&path, cx);
                                if let Some((g, i)) = target.and_then(|e| ws.views.find(&e)) {
                                    ws.views.select(g, i);
                                }
                            }
                            ws.finish_pending_jump(&path, cx);
                            ws.finish_cli_open(&path, None, cx);
                            // a blank untitled tab that had the keys and gave way: they go to the editor
                            let gone = |f: &FocusHandle| was_view && !ws.is_view_focus(f, cx);
                            match focused.filter(|f| busy && !gone(f)) {
                                Some(f) => window.focus(&f),
                                None => ws.focus_editor(window, cx),
                            }
                            cx.notify();
                            false
                        })
                        .unwrap_or(false);
                    if !retry {
                        break;
                    }
                }
            })
            .detach();
        }
    }
}

#[cfg(test)]
mod audit_state_tests {
    use super::super::ui_tests::{settle, settle_until, temp, window};
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    /// A window of one group showing `views`, `active` the selected one.
    fn one_group(views: impl IntoIterator<Item = ViewState>, active: usize) -> WindowState {
        WindowState { groups: vec![GroupState { views: views.into_iter().collect(), active }], ..Default::default() }
    }

    /// A view of `path` at `selection`.
    fn view(path: &Path, selection: Option<usize>) -> ViewState {
        ViewState {
            path: path.to_path_buf(),
            selections: selection.map_or_else(Vec::new, |i| vec![(i, i)]),
            ..Default::default()
        }
    }

    /// A file at `path` big enough to be read in the background.
    fn write_big(path: &Path) {
        let text = "line\n".repeat((LOAD_IN_BACKGROUND / 5 + 1) as usize);
        std::fs::write(path, text).unwrap();
    }

    fn paths_of(views: &[ViewState]) -> Vec<PathBuf> {
        views.iter().map(|v| v.path.clone()).collect()
    }

    fn tab_paths(tabs: &[Entity<Editor>], cx: &App) -> Vec<PathBuf> {
        (tabs.iter()).map(|e| e.read(cx).doc(cx).path.clone().unwrap()).collect()
    }

    /// Waits for every background read of the restore to land.
    fn restored(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
        settle(cx);
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
    }

    fn pending_restore_keeps_explicit_selection(cx: &mut TestAppContext, capture: bool, choice: &str) {
        let dir = temp(&format!("orlo-audit-state-restore-choice-{capture}-{choice}")).canonicalize().unwrap();
        let paths = [dir.join("A.txt"), dir.join("B.txt"), dir.join("C.txt")];
        write_big(&paths[0]);
        for path in &paths[1..] {
            std::fs::write(path, "text").unwrap();
        }
        let (ws, cx) = window(cx);
        let lease = ws.update_in(cx, |ws, window, cx| {
            let lease = pipelines::lease(&paths[0], cx);
            let views = paths.iter().map(|path| view(path, None));
            ws.restore(one_group(views, 0), window, cx);
            assert_eq!(ws.restoring.len(), 1);
            lease
        });
        cx.run_until_parked();
        let chosen = if choice.ends_with("return") { 1 } else { 2 };
        ws.update_in(cx, |ws, window, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref(), Some(&paths[1]));
            match choice {
                "tab" => ws.select_view(0, 1, window, cx),
                "next" => ws.next_tab(&NextTab, window, cx),
                "range" => {
                    ws.views.select_range(0, 1);
                    ws.tabs_changed(window, cx);
                }
                "toggle" => {
                    ws.views.toggle_selected(0, 1);
                    ws.tabs_changed(window, cx);
                }
                "focus" => {
                    ws.views.toggle_selected(0, 1);
                    ws.views.select(0, 0);
                    cx.notify();
                }
                "open" => ws.open_path_here(paths[2].clone(), window, cx),
                "return" => {
                    ws.select_view(0, 1, window, cx);
                    ws.select_view(0, 0, window, cx);
                }
                "panel-return" => {
                    ws.views.groups[0].tabs[1].update(cx, |e, _| e.playground = true);
                    ws.open_playground(&OpenPlayground, window, cx);
                    ws.open_path_here(paths[1].clone(), window, cx);
                }
                _ => unreachable!(),
            }
        });
        if choice == "focus" {
            cx.update(|window, _| window.activate_window());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear());
            let target = ws.read_with(cx, |ws, _| ws.views.groups[0].tabs[1].clone());
            cx.update(|window, cx| {
                target.read(cx).focus(window);
                window.draw(cx).clear();
            });
        }
        cx.run_until_parked();
        ws.update_in(cx, |ws, window, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref(), Some(&paths[chosen]));
            if capture {
                for hot in [false, true] {
                    let saved = ws.capture(window, hot, &mut Texts::new(), cx);
                    assert_eq!(paths_of(&saved.groups[0].views), paths);
                    assert_eq!(saved.groups[0].active, chosen, "capture must keep the newer choice (hot={hot})");
                }
            }
        });
        ws.read_with(cx, |ws, _| assert_eq!(ws.restoring.len(), 1));
        drop(lease);
        restored(&ws, cx);
        ws.update_in(cx, |ws, window, cx| {
            let active = ws.active_editor().unwrap();
            assert_eq!(
                active.read(cx).doc(cx).path.as_ref(),
                Some(&paths[chosen]),
                "restore completion must keep the newer choice"
            );
            assert!(active.read(cx).is_focused(window), "restore completion must keep the chosen editor's keys");
        });
    }

    #[gpui::test]
    fn capturing_during_restore_keeps_explicit_selection(cx: &mut TestAppContext) {
        for choice in ["tab", "next", "range", "toggle", "focus", "open", "return", "panel-return"] {
            pending_restore_keeps_explicit_selection(cx, true, choice);
        }
    }

    #[gpui::test]
    fn completing_restore_keeps_explicit_selection(cx: &mut TestAppContext) {
        pending_restore_keeps_explicit_selection(cx, false, "tab");
    }

    #[gpui::test]
    fn capturing_during_restore_preserves_tab_order_and_active_view(cx: &mut TestAppContext) {
        let dir = temp("orlo-audit-state-capture-order").canonicalize().unwrap();
        let paths = [dir.join("first.txt"), dir.join("deferred.txt"), dir.join("last.txt")];
        for path in &paths {
            std::fs::write(path, "text").unwrap();
        }
        for (active, all_deferred) in (0..paths.len()).flat_map(|a| [false, true].map(|all| (a, all))) {
            let (ws, cx) = window(cx);
            let mut leases = ws.update_in(cx, |ws, window, cx| {
                let leases: Vec<_> = paths
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| all_deferred || *i == 1)
                    .map(|(_, p)| pipelines::lease(p, cx))
                    .collect();
                let views = paths.iter().map(|path| view(path, None));
                ws.restore(one_group(views, active), window, cx);
                for hot in [false, true] {
                    let captured = ws.capture(window, hot, &mut Texts::new(), cx);
                    assert_eq!(paths_of(&captured.groups[0].views), paths);
                    assert_eq!(captured.groups[0].active, active);
                }
                leases
            });
            if all_deferred {
                drop(leases.swap_remove((active + 1) % paths.len()));
                settle(cx);
                settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.restoring.len() == 2));
                ws.update_in(cx, |ws, window, cx| {
                    for hot in [false, true] {
                        let captured = ws.capture(window, hot, &mut Texts::new(), cx);
                        assert_eq!(captured.groups[0].active, active);
                    }
                });
            }
            drop(leases);
            restored(&ws, cx);
            ws.read_with(cx, |ws, cx| {
                assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref(), Some(&paths[active]));
            });
        }
    }

    #[gpui::test]
    fn deferred_pinned_views_keep_order_when_reads_finish_out_of_order(cx: &mut TestAppContext) {
        let dir = temp("orlo-audit-state-pinned-restore-order").canonicalize().unwrap();
        let large = dir.join("large.txt");
        let pinned = dir.join("pinned.txt");
        let later = dir.join("later.txt");
        let last = dir.join("last.txt");
        write_big(&large);
        for path in [&pinned, &later, &last] {
            std::fs::write(path, "text").unwrap();
        }
        let paths = [large.clone(), pinned, large.clone(), later.clone(), last];
        let (ws, cx) = window(cx);
        let (first_lease, later_lease) = ws.update_in(cx, |ws, window, cx| {
            let leases = (pipelines::lease(&large, cx), pipelines::lease(&later, cx));
            let views = paths.iter().enumerate().map(|(i, path)| ViewState { pinned: i < 4, ..view(path, Some(i)) });
            ws.restore(one_group(views, 2), window, cx);
            leases
        });
        drop(later_lease);
        settle(cx);
        settle_until(cx, |cx| ws.read_with(cx, |ws, cx| ws.editor_for_path(&later, cx).is_some()));
        ws.update_in(cx, |ws, window, cx| {
            let captured = ws.capture(window, false, &mut Texts::new(), cx);
            assert_eq!(paths_of(&captured.groups[0].views), paths);
            assert_eq!(captured.groups[0].active, 2);
            assert!(captured.groups[0].views[..4].iter().all(|v| v.pinned));
        });
        drop(first_lease);
        restored(&ws, cx);
        ws.update_in(cx, |ws, window, cx| {
            let group = &ws.views.groups[0];
            assert_eq!(tab_paths(&group.tabs, cx), paths);
            assert_eq!(group.pinned, group.tabs[..4]);
            assert_eq!(group.active, 2);
            assert_eq!(group.tabs[0].read(cx).state.cursor(), 0);
            assert_eq!(group.tabs[2].read(cx).state.cursor(), 2);
            assert!(group.tabs[0].read(cx).same_document(group.tabs[2].read(cx)));
            let captured = ws.capture(window, true, &mut Texts::new(), cx);
            assert_eq!(captured.groups[0].active, 2);
            assert!(captured.groups[0].views[..4].iter().all(|v| v.pinned));
        });
    }

    #[gpui::test]
    fn background_views_keep_their_order_around_synchronous_tabs(cx: &mut TestAppContext) {
        let dir = temp("orlo-audit-state-restore-order").canonicalize().unwrap();
        let large = dir.join("large.txt");
        let middle = dir.join("middle.txt");
        let last = dir.join("last.txt");
        write_big(&large);
        std::fs::write(&middle, "middle").unwrap();
        std::fs::write(&last, "last").unwrap();
        let paths = [large.clone(), middle, large, last];
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            let views = paths.iter().enumerate().map(|(i, path)| view(path, Some(i)));
            ws.restore(one_group(views, 2), window, cx);
        });
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.restoring.is_empty()));
        ws.read_with(cx, |ws, cx| {
            let tabs = &ws.views.groups[0].tabs;
            assert_eq!(tab_paths(tabs, cx), paths);
            assert_eq!(ws.views.groups[0].active, 2);
            assert_eq!(tabs[0].read(cx).state.cursor(), 0);
            assert_eq!(tabs[2].read(cx).state.cursor(), 2);
            assert!(tabs[0].read(cx).same_document(tabs[2].read(cx)));
        });
    }

    #[gpui::test]
    fn restoration_waits_for_file_writers(cx: &mut TestAppContext) {
        for large in [false, true] {
            let dir = temp(&format!("orlo-audit-state-restore-write-{large}")).canonicalize().unwrap();
            let path = dir.join("document.txt");
            if large {
                write_big(&path);
            } else {
                std::fs::write(&path, "before\n").unwrap();
            }
            let (ws, cx) = window(cx);
            let lease = ws.update_in(cx, |ws, window, cx| {
                let lease = pipelines::lease(&path, cx);
                ws.restore(one_group([view(&path, None)], 0), window, cx);
                lease
            });
            cx.run_until_parked();
            ws.read_with(cx, |ws, cx| {
                assert!(
                    ws.views.tabs().all(|e| e.read(cx).doc(cx).path.as_ref() != Some(&path)),
                    "a restoring file cannot be read while its writer owns it (large={large})"
                );
            });
            std::fs::write(&path, "after writer\n").unwrap();
            drop(lease);
            restored(&ws, cx);
            ws.read_with(cx, |ws, cx| {
                assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "after writer\n");
            });
        }
    }
}

#[cfg(test)]
#[path = "fold_persistence_tests.rs"]
mod fold_persistence_tests;
