//! Tools > Text Pipelines: the pipeline list, running one on the document, previewing it, and
//! running it on files in the background.

use super::*;
use crate::searching::pipeline::{self, BatchReport, Outcome, Pipeline};
use crate::searching::search::Scope;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Weak};
use std::time::Duration;

/// Completed files and the end of the run (cancelled or not).
enum Msg {
    File(PathBuf, Result<Outcome, String>),
    Done(bool),
}

/// A closed-file worker owns its path until it finishes; opening or saving to it waits.
#[derive(Default)]
pub(crate) struct Writes(HashMap<PathBuf, Weak<()>>);
impl gpui::Global for Writes {}

/// The paths writes under way hold.
fn held(cx: &App) -> impl Iterator<Item = &PathBuf> {
    (cx.try_global::<Writes>().into_iter().flat_map(|w| &w.0))
        .filter(|(_, owner)| owner.strong_count() > 0)
        .map(|(path, _)| path)
}

pub(crate) fn writing(path: &Path, cx: &App) -> bool {
    held(cx).any(|p| path.starts_with(p))
}

/// New readers/owners must wait through publication, including hardlink aliases.
pub(crate) fn reading_blocked(path: &Path, cx: &App) -> bool {
    let files = cx.try_global::<Files>().is_some_and(|files| files.0.values().any(|owner| owner.strong_count() > 0));
    if held(cx).next().is_none() && !files {
        return false;
    }
    let path = crate::editing::text::resolve(path);
    writing(&path, cx) || (files && writing_file(file_id(&path), cx))
}

/// A pending disk read, invalidated only by an overlapping Orlo writer reservation.
/// External appends keep Loaded's existing pre-read stamp and ordinary polling policy.
pub(crate) struct Read {
    path: PathBuf,
    file: Option<(u64, u64)>,
    changed: AtomicBool,
}
impl Read {
    pub(crate) fn changed(&self) -> bool {
        self.changed.load(Ordering::Relaxed)
    }
}
#[derive(Default)]
struct Reads(Vec<Weak<Read>>);
impl gpui::Global for Reads {}

pub(crate) fn observe_read(path: &Path, cx: &mut App) -> Arc<Read> {
    let read = Arc::new(Read {
        path: crate::editing::text::resolve(path),
        file: file_id(path),
        changed: AtomicBool::new(reading_blocked(path, cx)),
    });
    let reads = &mut cx.default_global::<Reads>().0;
    reads.retain(|read| read.strong_count() > 0);
    reads.push(Arc::downgrade(&read));
    read
}

fn invalidate_reads(path: &Path, file: Option<(u64, u64)>, cx: &mut App) {
    if cx.try_global::<Reads>().is_none() {
        return;
    }
    let reads = cx.global_mut::<Reads>();
    reads.0.retain(|read| {
        let Some(read) = read.upgrade() else { return false };
        if read.path.starts_with(path) || file.is_some_and(|file| read.file == Some(file)) {
            read.changed.store(true, Ordering::Relaxed);
        }
        true
    });
}

pub(crate) fn writing_under(path: &Path, cx: &App) -> bool {
    held(cx).any(|p| p.starts_with(path) || path.starts_with(p))
}

/// The paths writes under way hold, for a comparison to check off the UI thread.
pub(crate) fn write_paths(cx: &App) -> Vec<PathBuf> {
    held(cx).cloned().collect()
}

/// Whether `path` is still held by `lease`, not by a write that took it over since.
pub(crate) fn owns_lease(path: &Path, lease: &Arc<()>, cx: &App) -> bool {
    cx.try_global::<Writes>()
        .and_then(|writes| writes.0.get(path))
        .and_then(Weak::upgrade)
        .is_some_and(|current| Arc::ptr_eq(&current, lease))
}

pub(crate) async fn wait_for_write(path: &Path, cx: &mut gpui::AsyncWindowContext) {
    while cx.update(|_, cx| reading_blocked(path, cx)).unwrap_or(false) {
        cx.background_executor().timer(Duration::from_millis(10)).await;
    }
}

/// Hold a file or a directory tree until an off-thread write finishes.
pub(crate) fn lease(path: &Path, cx: &mut App) -> Arc<()> {
    invalidate_reads(path, None, cx);
    let lease = Arc::new(());
    let writes = cx.default_global::<Writes>();
    writes.0.retain(|_, w| w.strong_count() > 0);
    writes.0.insert(path.to_path_buf(), Arc::downgrade(&lease));
    lease
}

/// The files saves are writing, by device and inode: another name of one (a hard link, written
/// in place) is the same file.
#[derive(Default)]
pub(crate) struct Files(HashMap<(u64, u64), Weak<()>>);
impl gpui::Global for Files {}

/// The file `path` names, by device and inode; a stat, so off the UI thread.
pub(crate) fn file_id(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// Whether a save is writing file `id`, under whatever name.
pub(crate) fn writing_file(id: Option<(u64, u64)>, cx: &App) -> bool {
    let files = cx.try_global::<Files>();
    id.and_then(|id| files?.0.get(&id)).is_some_and(|owner| owner.strong_count() > 0)
}

/// `lease` `path`, and file `id` with it.
pub(crate) fn lease_file(path: &Path, id: Option<(u64, u64)>, cx: &mut App) -> Arc<()> {
    invalidate_reads(path, id, cx);
    let lease = lease(path, cx);
    if let Some(id) = id {
        let files = cx.default_global::<Files>();
        files.0.retain(|_, w| w.strong_count() > 0);
        files.0.insert(id, Arc::downgrade(&lease));
    }
    lease
}

enum FileUse {
    Open(Result<Outcome, String>),
    Disk(Arc<()>, Option<crate::editing::encoding::Encoding>),
    Wait,
}

/// A Run Pipeline on Files going on in the background: its cancel flag and progress counters,
/// the report so far, and the 100 ms drain.
pub(crate) struct Run {
    pipeline: Arc<Pipeline>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
    /// Unknown (`usize::MAX`) until the walk is done.
    total: Arc<AtomicUsize>,
    rx: mpsc::Receiver<Msg>,
    report: BatchReport,
    _tasks: (Task<()>, Task<()>),
}

/// A window closing must not leave the run going.
impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The paths a batch is given as a search scope: folders walked for files matching the globs,
/// and the named files as they really are (so a file named twice is done once).
fn scope_of(paths: &[PathBuf], include: Option<&str>) -> Scope {
    let (roots, files): (Vec<_>, Vec<_>) = paths.iter().cloned().partition(|p| p.is_dir());
    let globs = include.unwrap_or_default().split(',');
    Scope {
        roots,
        files: files.iter().map(|f| crate::editing::text::resolve(f)).collect(),
        include: (globs.map(str::trim).filter(|g| !g.is_empty())).map(String::from).collect(),
        exclude: Vec::new(),
        gitignore: true,
        filter_files: false,
        root_excludes: Vec::new(),
    }
}

/// An include glob that does not parse would be left out by the walk, and take in every file.
fn bad_globs(include: &str) -> Option<String> {
    let bad = scope_of(&[], Some(include)).bad_globs();
    (!bad.is_empty()).then(|| format!("Not a valid pattern: {}", bad.join(", ")))
}

impl Workspace {
    /// Whether a view is open. The forms and the previews act on the active view, so without
    /// one a command says what it needs rather than doing nothing.
    pub(crate) fn need_tab(&self, what: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let open = self.active_editor().is_some();
        if !open {
            let detail = format!("Open a tab (File > New File) to {what}.");
            self.inform(window, cx, "No open tab", &detail);
        }
        open
    }

    /// Tools > Text Pipelines > New Pipeline…: asks for a name, then the template in a tab.
    pub(crate) fn new_pipeline(&mut self, _: &NewPipeline, window: &mut Window, cx: &mut Context<Self>) {
        if !self.need_tab("name a new pipeline", window, cx) {
            return;
        }
        let done: OnDone = Box::new(|v, _, ws, window, cx| {
            let name = pipeline::file_name(v.text("text"), pipeline::EXT, "pipeline")?;
            let file = pipeline::dir().join(format!("{name}.{}", pipeline::EXT));
            ws.open_or_template(file, &pipeline::template(), window, cx);
            Ok(())
        });
        self.show_input("Pipeline name", "", done, window, cx);
    }

    /// New Pipeline's and New Snippet's file: a missing one opens as `template` and is written
    /// only by the first save, so closing it never prompts; one that exists opens as it is.
    pub(crate) fn open_or_template(
        &mut self,
        file: PathBuf,
        template: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // a failure shows up as the save's error
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let path = crate::editing::text::resolve(&file);
        if reading_blocked(&path, cx) {
            let template = template.to_string();
            let gen = self.restore_gen;
            cx.spawn_in(window, async move |this, cx| {
                wait_for_write(&path, cx).await;
                this.update_in(cx, |ws, window, cx| {
                    if ws.restore_gen == gen {
                        ws.open_or_template(path, &template, window, cx);
                    }
                })
                .ok();
            })
            .detach();
            return;
        }
        if self.tab_for_path(&path, cx).is_some() || path.exists() {
            return self.open_path_here(path, window, cx);
        }
        let mut state = EditorState::from_source(template);
        state.doc.path = Some(path);
        self.drop_blank_untitled(cx);
        self.add_tab(state, window, cx);
    }

    forward_actions! {
    pub(crate) edit_pipeline(EditPipeline, window, cx) => choose_pipeline(PipelineUse::Edit, window, cx);
    pub(crate) preview_pipeline(PreviewPipeline, window, cx) => choose_pipeline(PipelineUse::Preview, window, cx);
    pub(crate) run_pipeline(RunPipeline, window, cx) => choose_pipeline(PipelineUse::Run, window, cx);
    pub(crate) run_pipeline_on_files(RunPipelineOnFiles, window, cx) => choose_pipeline(PipelineUse::RunOnFiles, window, cx);
    }

    /// The quick panel of the saved pipelines; choosing one dispatches `UsePipeline`. All but
    /// Edit act on a view, so without one they say so first.
    fn choose_pipeline(&mut self, how: PipelineUse, window: &mut Window, cx: &mut Context<Self>) {
        let what = match how {
            PipelineUse::Edit => None,
            PipelineUse::Preview => Some("preview a pipeline on"),
            PipelineUse::Run => Some("run a pipeline on"),
            // a folder asks which of its files in a form, which needs a view: said before the
            // chooser
            PipelineUse::RunOnFiles => Some("run a pipeline on files"),
        };
        if what.is_some_and(|what| !self.need_tab(what, window, cx)) {
            return;
        }
        let items: Vec<_> = (pipeline::list().into_iter())
            .map(|path| {
                quick_panel::Item::command(
                    path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
                    "",
                    UsePipeline { path, how },
                )
            })
            .collect();
        if items.is_empty() {
            let detail = "Create one with Tools > Text Pipelines > New Pipeline…";
            return self.inform(window, cx, "No pipelines", detail);
        }
        self.open_quick_panel(Kind::Commands, items, window, cx);
    }

    /// A pipeline chosen in the list. It is read and checked first, so a broken one is said
    /// before a document or a file is touched.
    pub(crate) fn use_pipeline(&mut self, a: &UsePipeline, window: &mut Window, cx: &mut Context<Self>) {
        if a.how == PipelineUse::Edit {
            return self.open_path_here(a.path.clone(), window, cx);
        }
        let p = match pipeline::load(&a.path) {
            Ok(p) => p,
            Err(errors) => {
                let detail = pipeline::problems(&errors);
                return self.alert(window, cx, "Pipeline has problems", &detail);
            }
        };
        match a.how {
            PipelineUse::Edit => {}
            PipelineUse::Preview => self.preview(&p, window, cx),
            PipelineUse::Run => self.run_on_document(&p, window, cx),
            PipelineUse::RunOnFiles => self.choose_files(Arc::new(p), window, cx),
        }
    }

    /// The pipeline's result on the active document (or selections) in a scratch tab; the
    /// document stays as it is.
    fn preview(&mut self, p: &Pipeline, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let texts = editor.update(cx, |e, cx| sel_of(&e.document, &mut e.state, cx).transform_texts(p.lines()));
        let mut out = String::new();
        for t in texts {
            match p.run(&t) {
                Ok(done) => {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out += &done;
                }
                Err(e) => return self.run_failed(&p.failure(e), window, cx),
            }
        }
        self.scratch_tab(&out, format!("Preview: {}", p.name), window, cx);
    }

    /// A tab for `text` that is never asked about when it closes.
    pub(crate) fn scratch_tab(&mut self, text: &str, title: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut state = EditorState::from_source(text);
        state.doc.scratch = true;
        let view = self.new_editor(state, cx);
        view.update(cx, |e, _| e.label = Some(title));
        self.attach(view, false, window, cx);
    }

    fn run_failed(&self, detail: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.alert(window, cx, "Could not run pipeline", detail);
    }

    /// Every step in memory, then one undoable edit of the document or each selection. A
    /// failing step changes nothing.
    /// Runs on the UI thread, as every Edit > Transform does, so a huge buffer waits
    /// for it; move it off with the batch's if that matters
    fn run_on_document(&mut self, p: &Pipeline, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        if p.adjusts_values() {
            let result = self.run_value_pipeline(&editor, p, false, cx);
            return match result {
                Ok(_) => self.status_message(format!("Pipeline {}: done", p.name), cx),
                Err(error) => self.run_failed(&error, window, cx),
            };
        }
        // the step's own message, not the line's
        let failure = RefCell::new(None);
        let done = transforms::apply(&editor, cx, p.lines(), |t| {
            p.run(t).map_err(|e| {
                failure.replace(Some(p.failure(e)));
                String::new()
            })
        });
        match (done, failure.take()) {
            (Ok(()), _) => self.status_message(format!("Pipeline {}: done", p.name), cx),
            (Err(m), failure) => self.run_failed(&failure.unwrap_or(m), window, cx),
        }
    }

    /// Run Pipeline on Files…: the files and folders, then (for a folder) an include glob.
    fn choose_files(&mut self, p: Arc<Pipeline>, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_paths(files::picker(true, true, true), window, cx, move |ws, paths, window, cx| {
            ws.ask_include(p, paths, window, cx)
        });
    }

    /// A folder among the paths asks which of its files, as a glob like Find in Files' Where;
    /// files alone start at once.
    pub(crate) fn ask_include(
        &mut self,
        p: Arc<Pipeline>,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !paths.iter().any(|f| f.is_dir()) {
            return self.start_batch(p, paths, None, window, cx);
        }
        // a batch on folders never starts without the glob having been asked (the tab may have
        // closed while the chooser was up)
        if !self.need_tab("choose which files of the folders to run on", window, cx) {
            return;
        }
        let done: OnDone = Box::new(move |v, _, ws, window, cx| {
            let glob = v.text("text").trim();
            if let Some(why) = bad_globs(glob) {
                return Err(why);
            }
            let include = (!glob.is_empty()).then(|| glob.to_string());
            ws.start_batch(p.clone(), paths.clone(), include, window, cx);
            Ok(())
        });
        let label = "Files in folders matching (*.txt, *.md; empty for all)";
        self.show_input(label, "", done, window, cx);
    }

    /// Run `p` on `paths` (files, and folders walked for files matching `include`) in the
    /// background; the report opens in a tab when it ends.
    pub(crate) fn start_batch(
        &mut self,
        p: Arc<Pipeline>,
        paths: Vec<PathBuf>,
        include: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pipeline_run.is_some() {
            let detail = "Cancel it first: Tools > Text Pipelines > Cancel Pipeline Run.";
            return self.alert(window, cx, "A pipeline is already running", detail);
        }
        if let Some(why) = include.as_deref().and_then(bad_globs) {
            return self.run_failed(&why, window, cx);
        }
        let scope = scope_of(&paths, include.as_deref());
        let history = cx.global::<Settings>().file_history;
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(usize::MAX));
        let work = cx.spawn_in(window, {
            let (p, cancel, done, total) = (p.clone(), cancel.clone(), done.clone(), total.clone());
            async move |this, cx| {
                let files = cx
                    .background_spawn({
                        let cancel = cancel.clone();
                        async move { pipeline::collect(&scope, &cancel) }
                    })
                    .await;
                pipeline::run_batch(
                    &files,
                    &cancel,
                    &done,
                    &total,
                    async |path| loop {
                        if cancel.load(Ordering::Relaxed) {
                            return None;
                        }
                        match this.update(cx, |ws, cx| ws.route_pipeline_file(path, &p, cx)).ok()? {
                            FileUse::Open(result) => return Some(result),
                            FileUse::Disk(lease, hint) => {
                                let (path, p, cancel) = (path.to_path_buf(), p.clone(), cancel.clone());
                                return cx
                                    .background_spawn(async move {
                                        let _lease = lease;
                                        (!cancel.load(Ordering::Relaxed))
                                            .then(|| pipeline::run_file_with_hint(&path, &p, history, hint))
                                    })
                                    .await;
                            }
                            FileUse::Wait => cx.background_executor().timer(Duration::from_millis(10)).await,
                        }
                    },
                    |path, result| {
                        let _ = tx.send(Msg::File(path, result));
                    },
                )
                .await;
                let _ = tx.send(Msg::Done(cancel.load(Ordering::Relaxed)));
            }
        });
        let drain = cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            let ended = this.update_in(cx, |ws, window, cx| ws.drain_pipeline_run(window, cx));
            if ended.unwrap_or(true) {
                break;
            }
        });
        self.pipeline_run =
            Some(Run { pipeline: p, cancel, done, total, rx, report: BatchReport::default(), _tasks: (work, drain) });
        cx.notify();
    }

    /// Where `path` is changed: in its open buffers, else on disk by the worker, held for it in
    /// this same update so nothing opens it first; a file being read or written waits.
    fn route_pipeline_file(&self, path: &Path, p: &Pipeline, cx: &mut Context<Self>) -> FileUse {
        let loading = |ws: &Workspace| ws.loading.contains(path) || ws.restoring.iter().any(|v| v.view.path == path);
        if writing(path, cx) || loading(self) || self.other_workspaces(cx).any(loading) {
            return FileUse::Wait;
        }
        if let Some(result) = self.change_open_buffers(path, p, cx) {
            return FileUse::Open(result);
        }
        FileUse::Disk(lease(path, cx), self.remembered_disk_encoding(path))
    }

    /// Take completed files into the report; open it in a tab when the run ends.
    fn drain_pipeline_run(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // out of the workspace while the buffers change, which needs `&self`
        let Some(mut run) = self.pipeline_run.take() else {
            return true;
        };
        let mut ended = None;
        loop {
            match run.rx.try_recv() {
                Ok(Msg::File(path, result)) => run.report.add(path, result),
                Ok(Msg::Done(cancelled)) => {
                    ended = Some(cancelled || run.cancel.load(Ordering::Relaxed));
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                // the background task is gone without its `Done` (it panicked): the run is over,
                // or it would wait for ever and block the next one
                Err(mpsc::TryRecvError::Disconnected) => {
                    let why = "the background run stopped unexpectedly";
                    run.report.aborted = Some(why.into());
                    ended = Some(false);
                    break;
                }
            }
        }
        let Some(cancelled) = ended else {
            self.pipeline_run = Some(run);
            cx.notify();
            return false;
        };
        let (name, r) = (run.pipeline.name.clone(), &run.report);
        let changed = r.changed.len() + r.open_unsaved.len();
        let stopped = (r.aborted.as_ref()).map_or(String::new(), |why| format!(", stopped: {why}"));
        let summary = format!(
            "Pipeline {name}: {changed} changed, {} skipped, {} failed{stopped}",
            r.skipped.len(),
            r.failed.len()
        );
        self.scratch_tab(&r.text(&name, cancelled), format!("Pipeline Report: {name}"), window, cx);
        self.status_message(summary, cx);
        true
    }

    /// `p` on the buffer of each editor that has `path` open, in any window: one undo step, left
    /// unsaved, whatever edits the buffer already has.
    fn change_open_buffers(
        &self,
        path: &Path,
        p: &Pipeline,
        cx: &mut Context<Self>,
    ) -> Option<Result<Outcome, String>> {
        // the batch's paths are resolved already (pipeline::collect)
        let buffers = editors::one_per_document(self.open_editors(cx), cx, |v| v.doc(cx).path.as_deref() == Some(path));
        // not open: the caller runs it on disk, off the UI thread
        if buffers.is_empty() {
            return None;
        }
        if buffers.iter().any(|editor| editor.read(cx).doc(cx).viewer_mode()) {
            return Some(Ok(Outcome::Skipped("viewer mode".into())));
        }
        let mut outcome = Outcome::Unchanged;
        for editor in buffers {
            if editor.read(cx).doc(cx).is_following() {
                return Some(Ok(Outcome::Skipped("following log".into())));
            }
            if editor.read(cx).doc(cx).read_only {
                return Some(Ok(Outcome::Skipped("not valid UTF-8".into())));
            }
            let changed = if p.adjusts_values() {
                self.run_value_pipeline(&editor, p, true, cx)
            } else {
                editor.update(cx, |e, cx| e.edit(cx, |s| s.transform_document(|t| p.run(t).map_err(|e| p.failure(e)))))
            };
            match changed {
                Ok(true) => outcome = Outcome::OpenUnsaved,
                Ok(false) => {}
                Err(m) => return Some(Err(m)),
            }
        }
        Some(Ok(outcome))
    }

    /// The other windows' workspaces. This window is being updated, so it cannot be read
    /// through its handle: it is `self`.
    pub(crate) fn other_workspaces<'a>(&self, cx: &'a App) -> impl Iterator<Item = &'a Workspace> {
        let me = self.window;
        (cx.windows().into_iter()).filter(move |w| *w != me).filter_map(|w| w.downcast::<Workspace>()?.read(cx).ok())
    }

    /// Every view in every window.
    pub(crate) fn open_editors(&self, cx: &App) -> Vec<Entity<Editor>> {
        let mut all: Vec<_> = self.views.tabs().cloned().collect();
        let mut seen: HashSet<EntityId> = all.iter().map(|t| t.entity_id()).collect();
        for ws in self.other_workspaces(cx) {
            all.extend((ws.views.tabs()).filter(|t| seen.insert(t.entity_id())).cloned());
        }
        all
    }

    /// Tools > Text Pipelines > Cancel Pipeline Run: the run stops before its next file, and
    /// its report says what was done.
    pub(crate) fn cancel_pipeline_run(&mut self, _: &CancelPipelineRun, _: &mut Window, cx: &mut Context<Self>) {
        match &self.pipeline_run {
            Some(run) => run.cancel.store(true, Ordering::Relaxed),
            None => self.status_message("No pipeline is running", cx),
        }
    }

    /// The status bar's "Pipeline tidy: 3 of 40 files" while a run goes on.
    pub(crate) fn pipeline_progress(&self) -> Option<String> {
        let run = self.pipeline_run.as_ref()?;
        let name = &run.pipeline.name;
        Some(match run.total.load(Ordering::Relaxed) {
            usize::MAX => format!("Pipeline {name}: listing files…"),
            total => format!("Pipeline {name}: {} of {total} files", run.done.load(Ordering::Relaxed)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{settle, settle_until, temp, window};
    use gpui::{TestAppContext, VisualTestContext};

    const SORT: &str = r#"{ "steps": [{ "op": "sort_lines" }] }"#;

    /// `data.txt` reading "b\na\n" in a fresh real folder, and a pipeline sorting lines.
    fn data(name: &str) -> (PathBuf, Pipeline) {
        let file = temp(name).canonicalize().unwrap().join("data.txt");
        std::fs::write(&file, "b\na\n").unwrap();
        (file, pipeline::parse(SORT).unwrap())
    }

    /// `file` held for the worker, as a run holds a closed file.
    fn reserve(ws: &Workspace, file: &Path, p: &Pipeline, cx: &mut Context<Workspace>) -> Arc<()> {
        let FileUse::Disk(lease, _) = ws.route_pipeline_file(file, p, cx) else {
            panic!("a closed file belongs to the worker");
        };
        lease
    }

    /// A run whose tasks are done, fed by `rx`.
    fn run(p: Pipeline, done: usize, total: usize, rx: mpsc::Receiver<Msg>) -> Run {
        Run {
            pipeline: Arc::new(p),
            cancel: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicUsize::new(done)),
            total: Arc::new(AtomicUsize::new(total)),
            rx,
            report: BatchReport::default(),
            _tasks: (Task::ready(()), Task::ready(())),
        }
    }

    #[gpui::test]
    fn a_cancel_after_worker_completion_is_reported_as_cancelled(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let (file, p) = data("orlo-pipeline-queued-cancel");
        let (tx, rx) = mpsc::channel();
        tx.send(Msg::Done(false)).unwrap();
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(file.clone(), window, cx);
            ws.pipeline_run = Some(run(p, 1, 1, rx));
            ws.cancel_pipeline_run(&CancelPipelineRun, window, cx);
            assert!(ws.drain_pipeline_run(window, cx));
        });
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(ws.views.tabs().next().unwrap().read(cx).doc(cx).text(), "b\na\n");
            let report = ws.active_editor().unwrap().read(cx).doc(cx).text();
            assert!(report.contains("Cancelled"), "{report}");
            assert!(!report.contains("Changed"), "{report}");
        });
    }

    #[gpui::test]
    fn cancelling_after_one_buffer_leaves_the_next_untouched(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let dir = temp("orlo-pipeline-cancel-between-buffers").canonicalize().unwrap();
        let files = [dir.join("a.txt"), dir.join("b.txt")];
        for file in &files {
            std::fs::write(file, "b\na\n").unwrap();
        }
        let p = pipeline::parse(SORT).unwrap();
        let _cancel_on_edit = ws.update_in(vcx, |ws, window, cx| {
            for file in &files {
                ws.open_path(file.clone(), window, cx);
            }
            let first = ws.views.tabs().next().unwrap().clone();
            ws.start_batch(Arc::new(p), files.to_vec(), None, window, cx);
            let cancel = ws.pipeline_run.as_ref().unwrap().cancel.clone();
            Editor::on_text_change(&first, cx, move |_, _| {
                cancel.store(true, Ordering::Relaxed);
            })
        });
        settle(vcx);
        ws.read_with(vcx, |ws, cx| {
            let texts: Vec<_> = ws.views.tabs().map(|e| e.read(cx).doc(cx).text()).collect();
            assert_eq!(&texts[..2], ["a\nb\n", "b\na\n"]);
            assert!(texts[2].contains("Cancelled"), "{}", texts[2]);
        });
        for file in &files {
            assert_eq!(std::fs::read_to_string(file).unwrap(), "b\na\n");
        }
    }

    #[gpui::test]
    fn opening_a_reserved_file_in_another_window_waits_for_the_worker(cx: &mut TestAppContext) {
        let (file, p) = data("orlo-pipeline-reserved-open");
        let (ws1, _) = window(cx);
        let (ws2, _) = cx.add_window_view(Workspace::new);
        let mut vcx1 = VisualTestContext::from_window(cx.windows()[0], cx);
        let mut vcx2 = VisualTestContext::from_window(cx.windows()[1], cx);
        let lease = ws1.update(&mut vcx1, |ws, cx| reserve(ws, &file, &p, cx));
        ws2.update_in(&mut vcx2, |ws, window, cx| ws.open_path(file.clone(), window, cx));
        vcx2.run_until_parked();
        ws2.read_with(&vcx2, |ws, cx| assert!(ws.views.tabs().all(|e| e.read(cx).doc(cx).path.is_none())));
        pipeline::run_file(&file, &p).unwrap();
        drop(lease);
        settle(&mut vcx2);
        ws2.read_with(&vcx2, |ws, cx| {
            let e = ws.active_editor().unwrap().read(cx);
            assert_eq!(e.doc(cx).path.as_ref(), Some(&file));
            assert_eq!(e.doc(cx).text(), "a\nb\n");
        });
    }

    #[gpui::test]
    fn save_as_waits_for_a_reserved_destination(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let (file, p) = data("orlo-pipeline-reserved-save");
        let lease = ws.update(vcx, |ws, cx| {
            let lease = reserve(ws, &file, &p, cx);
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("my draft")));
            lease
        });
        vcx.dispatch_action(SaveAs);
        vcx.simulate_new_path_selection(|_| Some(file.clone()));
        vcx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "b\na\n");
        ws.read_with(vcx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).doc(cx).path.is_none()));
        pipeline::run_file(&file, &p).unwrap();
        drop(lease);
        settle(vcx);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "my draft");
        ws.read_with(vcx, |ws, cx| assert!(ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref() == Some(&file)));
    }

    #[gpui::test]
    fn a_find_result_waits_for_a_reserved_file(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let (file, p) = data("orlo-pipeline-reserved-result");
        let lease = ws.update_in(vcx, |ws, window, cx| {
            let lease = reserve(ws, &file, &p, cx);
            let results = ws.active_editor().unwrap();
            ws.open_result(results, file.clone(), 1, None, window, cx);
            lease
        });
        vcx.run_until_parked();
        ws.read_with(vcx, |ws, cx| assert!(ws.views.tabs().all(|e| e.read(cx).doc(cx).path.is_none())));
        pipeline::run_file(&file, &p).unwrap();
        drop(lease);
        settle(vcx);
        ws.read_with(vcx, |ws, cx| {
            let e = ws.active_editor().unwrap().read(cx);
            assert_eq!(e.doc(cx).text(), "a\nb\n");
            assert_eq!(e.state.cursor(), 2);
        });
    }

    #[gpui::test]
    fn a_background_open_waits_for_a_reserved_file(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let (file, p) = data("orlo-pipeline-reserved-read");
        let lease = ws.update_in(vcx, |ws, window, cx| {
            let lease = reserve(ws, &file, &p, cx);
            ws.read_in_background(file.clone(), window, cx);
            assert!(!ws.loading.contains(&file));
            lease
        });
        vcx.run_until_parked();
        pipeline::run_file(&file, &p).unwrap();
        drop(lease);
        settle(vcx);
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| ws.views.tabs().any(|e| e.read(cx).doc(cx).path.as_ref() == Some(&file)))
        });
        ws.read_with(vcx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).text(), "a\nb\n"));
    }

    #[gpui::test]
    fn a_file_already_loading_is_not_reserved_for_a_disk_write(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let (file, p) = data("orlo-pipeline-already-loading");
        ws.update_in(vcx, |ws, window, cx| {
            ws.read_in_background(file.clone(), window, cx);
            assert!(matches!(ws.route_pipeline_file(&file, &p, cx), FileUse::Wait));
            assert!(!writing(&file, cx));
        });
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| !ws.loading.contains(&file)));
        ws.update(vcx, |ws, cx| match ws.route_pipeline_file(&file, &p, cx) {
            FileUse::Open(result) => assert_eq!(result, Ok(Outcome::OpenUnsaved)),
            FileUse::Wait => panic!("the read has not finished"),
            FileUse::Disk(..) => panic!("the loaded buffer was not found"),
        });
        assert_eq!(std::fs::read_to_string(file).unwrap(), "b\na\n");
    }

    #[gpui::test]
    fn a_run_whose_background_task_vanished_ends_with_a_report(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let p = pipeline::parse(r#"{ "name": "tidy", "steps": [ { "op": "sort_lines" } ] }"#);
        let (tx, rx) = mpsc::channel();
        tx.send(Msg::File("/x/a.txt".into(), Ok(Outcome::Changed))).unwrap();
        // the task panicked or was dropped: no `Done` comes
        drop(tx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.pipeline_run = Some(run(p.unwrap(), 0, usize::MAX, rx));
            assert!(ws.drain_pipeline_run(window, cx), "the run has ended");
            assert!(ws.pipeline_run.is_none(), "another can start");
        });
        let (title, report) = ws.read_with(vcx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            (e.title(cx), e.doc(cx).text())
        });
        assert_eq!(title, "Pipeline Report: tidy");
        assert!(report.contains("Stopped: "), "{report}");
        assert!(report.contains("/x/a.txt"), "{report}");
        assert!(!report.contains("Cancelled"), "{report}");
    }
}

impl Workspace {
    fn run_value_pipeline(
        &self,
        editor: &Entity<Editor>,
        p: &Pipeline,
        whole: bool,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        self.adjust_values_admission(editor, cx)?;
        editor.update(cx, |e, cx| {
            if !e.input_enabled || e.doc(cx).edits_blocked() {
                return Err("This view does not allow text edits".into());
            }
            e.catch_up(cx);
            if whole {
                let old = e.text(cx);
                let new = p.run(&old).map_err(|error| p.failure(error))?;
                if old == new {
                    return Ok(false);
                }
                return e.edit(cx, |s| s.transform_document(|_| Ok(new.clone())));
            }
            let Some(outputs) = e.read_state(cx).value_pipeline_outputs(p)? else { return Ok(false) };
            let outputs = RefCell::new(outputs.into_iter());
            e.edit(cx, |s| {
                s.transform_selections(p.lines(), |_| {
                    Ok(outputs.borrow_mut().next().expect("prepared pipeline target"))
                })
            })?;
            Ok(true)
        })
    }
}

#[cfg(test)]
mod n09_tests {
    use super::*;
    use crate::workspace::ui_tests::{settle, temp, window};
    use gpui::{EntityInputHandler, TestAppContext};

    #[gpui::test]
    fn n09_open_buffer_file_pipeline_keeps_disk_and_undo_noop_failure_and_shared_ime(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let path = temp("orlo-n09-file-pipeline").canonicalize().unwrap().join("value.txt");
        std::fs::write(&path, "009").unwrap();
        ws.update_in(vcx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        settle(vcx);
        let source = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        let p = pipeline::parse(r#"{"steps":[{"op":"adjust_values","step":"1"}]}"#).unwrap();
        ws.update_in(vcx, |ws, w, cx| ws.start_batch(Arc::new(p), vec![path.clone()], None, w, cx));
        settle(vcx);
        assert_eq!(source.read_with(vcx, |e, cx| e.text(cx)), "010");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "009");
        source.update_in(vcx, |e, w, cx| {
            e.perform_action(&editor::Undo, w, cx);
            assert_eq!(e.text(cx), "009");
        });
        let before = source.read_with(vcx, |e, cx| {
            (
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation(),
            )
        });
        let zero = pipeline::parse(r#"{"steps":[{"op":"adjust_values","step":"0"}]}"#).unwrap();
        ws.update(vcx, |ws, cx| {
            assert!(matches!(ws.route_pipeline_file(&path, &zero, cx), FileUse::Open(Ok(Outcome::Unchanged))))
        });
        assert!(
            source.read_with(vcx, |e, cx| (
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.read_state(cx).selection_generation()
            )) == before
        );
        source.update_in(vcx, |e, w, cx| {
            e.perform_action(&editor::Redo, w, cx);
            assert_eq!(e.text(cx), "010");
            e.sel(cx).set_cursor(3);
            e.replace_and_mark_text_in_range(None, "中", None, w, cx);
        });
        let before = source.read_with(vcx, |e, cx| {
            (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.marked.clone(),
            )
        });
        let p = pipeline::parse(r#"{"steps":[{"op":"adjust_values"}]}"#).unwrap();
        ws.update(vcx, |ws, cx| assert!(matches!(ws.route_pipeline_file(&path, &p, cx), FileUse::Open(Err(_)))));
        assert!(
            source.read_with(vcx, |e, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state(),
                e.marked.clone()
            )) == before
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "009");
        source.update_in(vcx, |e, w, cx| e.replace_text_in_range(None, "", w, cx));
        let peer = ws.update_in(vcx, |ws, w, cx| {
            let (group, tab) = ws.views.find(&source).unwrap();
            ws.views.select(group, tab);
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
            ws.active_editor().unwrap()
        });
        peer.update_in(vcx, |e, w, cx| {
            e.sel(cx).set_cursor(3);
            e.replace_and_mark_text_in_range(None, "中", None, w, cx);
        });
        let before = source.read_with(vcx, |e, cx| e.text(cx));
        ws.update(vcx, |ws, cx| match ws.route_pipeline_file(&path, &p, cx) {
            FileUse::Open(Err(error)) => assert!(error.contains("composition")),
            _ => panic!("shared IME must refuse the file pipeline"),
        });
        assert_eq!(source.read_with(vcx, |e, cx| e.text(cx)), before);
        assert!(!peer.read_with(vcx, |e, _| e.marked.is_empty()));
    }
}

#[cfg(test)]
#[gpui::test]
fn n23_viewer_pipeline_preflights_same_path_documents_before_mutation(cx: &mut gpui::TestAppContext) {
    use super::ui_tests::{temp, window};
    let path = temp("orlo-n23-pipeline").canonicalize().unwrap().join("data.txt");
    std::fs::write(&path, "b\na\n").unwrap();
    let p = pipeline::parse(r#"{"steps":[{"op":"sort_lines"}]}"#).unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
    let first = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(cx, |ws, w, cx| {
        ws.new_file(&NewFile, w, cx);
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| s.insert("z\ny\n"));
            e.update_doc(cx, |d| d.path = Some(path.clone()));
        });
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
        assert!(matches!(ws.change_open_buffers(&path, &p, cx), Some(Ok(Outcome::Skipped(_)))));
    });
    assert_eq!(first.read_with(cx, |e, cx| e.text(cx)), "b\na\n");
    assert_eq!(ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).text(cx)), "z\ny\n");
    ws.update_in(cx, |ws, w, cx| {
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
        assert!(matches!(ws.change_open_buffers(&path, &p, cx), Some(Ok(Outcome::OpenUnsaved))));
    });
    assert_eq!(first.read_with(cx, |e, cx| e.text(cx)), "a\nb\n");
}
