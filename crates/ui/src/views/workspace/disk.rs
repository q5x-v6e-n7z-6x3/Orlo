//! File watching: while the window is focused, a background poll stats the open files, lists
//! the side bar folders on screen, reads the root .gitignore and stats the settings file, and
//! the window acts on what changed. Reloads read and diff off the UI thread too.

use super::*;
use crate::editing::text::Loaded;
use crate::files::watch::{self, Act, Disk, Known, Stamp};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// The settings file's stamp, app-wide, so an outside edit is applied once, not per window.
pub struct SettingsDisk(pub Option<Stamp>);

impl gpui::Global for SettingsDisk {}

/// What a poll found besides the open files: the shown folders listed again because their
/// names changed, each root's .gitignore names, the settings file (and its path with links
/// resolved, as a save of it leases it), and for each root whether it is a folder and its git
/// branch.
type Polled = (
    Vec<(PathBuf, Vec<(PathBuf, bool)>)>,
    HashMap<PathBuf, HashSet<String>>,
    (Disk, PathBuf),
    Vec<(PathBuf, bool, Option<String>)>,
);

/// What that poll checked against: the settings file's stamp and the window's roots, as they
/// were when it started.
type Then = (Option<Stamp>, Vec<PathBuf>);

impl Workspace {
    /// Poll every 500 ms while this window is focused; the task ends with the window.
    pub(crate) fn start_polling(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.poller = Some(cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let alive = this.update_in(cx, |ws, window, cx| {
                if window.is_window_active() {
                    ws.poll_disk(window, cx);
                    ws.index_rescan(true, cx);
                    ws.report_grammar_failures(window, cx);
                }
            });
            if alive.is_err() {
                break;
            }
        }));
    }

    pub(crate) fn poll_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_git(cx);
        let tabs = self.views.tabs().cloned();
        let buffers = editors::one_per_document(tabs, cx, |e| e.doc(cx).path.is_some() && !e.doc(cx).edits_blocked());
        // each file on its own: a stat hung on a dropped network share holds up only its
        // file, and that file is not asked about again until the stat returns
        for editor in buffers {
            let s = editor.read(cx).doc(cx);
            let path = s.path.clone().unwrap_or_default();
            // being written (a save of Orlo's own, a text pipeline): its new stamp is not news
            if pipelines::writing(&path, cx) || !self.checking.insert(path.clone()) {
                continue;
            }
            let then: Known = (s.disk, s.deleted);
            let p = path.clone();
            let task = cx.background_spawn(async move { watch::check(&p, then.0, then.1) });
            cx.spawn_in(window, async move |this, cx| {
                let disk = task.await;
                this.update_in(cx, |ws, window, cx| {
                    ws.checking.remove(&path);
                    ws.apply_file(editor, then, disk, window, cx);
                })
                .ok();
            })
            .detach();
        }
        if self.polling {
            return;
        }
        let roots: Vec<PathBuf> = self.roots.iter().map(|r| r.path.clone()).collect();
        let shown: Vec<PathBuf> = crate::config::project::present(&self.roots);
        let dirs = self.sidebar.folders_to_poll(&shown);
        let known = cx.global::<SettingsDisk>().0;
        // being written (a save of Orlo's own): its new stamp is not news, and it may be half
        // written
        let settings_written = pipelines::writing(&settings::path(), cx);
        self.polling = true;
        let then = roots.clone();
        let task = cx.background_spawn(async move {
            let listings = dirs
                .into_iter()
                .filter_map(|(d, shown)| sidebar::reread(&d, shown.as_ref().map(|n| n.as_slice())).map(|l| (d, l)))
                .collect();
            let ignored = (shown.iter()).map(|r| (r.clone(), sidebar::ignored_names(r))).collect();
            let path = settings::path();
            let settings = match settings_written {
                true => Disk::Same,
                false => watch::check(&path, known, false),
            };
            let settings = (settings, crate::editing::text::resolve(&path));
            // a root deleted, renamed or unmounted, or back; a checkout in a terminal changes
            // the status bar's branch
            let roots = (roots.into_iter())
                .map(|r| {
                    let here = r.is_dir();
                    let branch = here.then(|| git_branch(&r)).flatten();
                    (r, here, branch)
                })
                .collect();
            (listings, ignored, settings, roots)
        });
        cx.spawn_in(window, async move |this, cx| {
            let polled: Polled = task.await;
            this.update_in(cx, |ws, window, cx| {
                ws.polling = false;
                ws.apply_disk((known, then), polled, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Act on what the poll found for one buffer's file.
    fn apply_file(
        &mut self,
        editor: Entity<Editor>,
        then: Known,
        disk: Disk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // closed while the poll ran, still being read, or being written (Orlo's own save
        // started meanwhile): the next poll decides
        let busy = |(e, _): &(Entity<Editor>, usize)| e.read(cx).same_document(editor.read(cx));
        let writing = (editor.read(cx).doc(cx).path.as_deref()).is_some_and(|p| pipelines::writing(p, cx));
        if self.views.find(&editor).is_none()
            || self.reloads.iter().any(busy)
            || writing
            || editor.read(cx).doc(cx).edits_blocked()
        {
            return;
        }
        let (dirty, watching, now) = {
            let s = editor.read(cx).read_state(cx);
            (s.doc.dirty, s.doc.watching, (s.doc.disk, s.doc.deleted))
        };
        match watch::act(disk, then, now, dirty, watching) {
            Act::Nothing => {}
            // only the ctime moved: chmod, a Finder tag, a sync client's attributes
            Act::Ask(s) if then.0.is_some_and(|k| k.moved(&s)) => self.ask_if_text_changed(editor, s, window, cx),
            Act::Mark(stamp, deleted) => self.mark_disk(&editor, stamp, deleted, cx),
            Act::Reload(s) => {
                // unreadable: take the stamp, so the next poll does not try again
                let e = editor.clone();
                let failed = move |ws: &mut Self, _, _: &mut Window, cx: &mut Context<Self>| {
                    ws.mark_disk(&e, Some(s), false, cx)
                };
                self.reload_editor(editor, failed, window, cx);
            }
            Act::Ask(s) => self.ask_reload(editor, s, window, cx),
        }
    }

    fn apply_disk(
        &mut self,
        (settings_then, roots_then): Then,
        (listings, ignored, settings, roots): Polled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // what the side bar sees change, Goto Anything walks again for
        if self.sidebar.apply(listings, ignored) {
            self.walk_files(cx);
            cx.notify();
        }
        // roots added or removed while the poll ran wait for the next one
        let now: Vec<&PathBuf> = self.roots.iter().map(|r| &r.path).collect();
        if now.iter().copied().eq(roots_then.iter()) {
            let mut found = false;
            for (r, (path, here, branch)) in self.roots.iter_mut().zip(roots) {
                found |= r.missing == here;
                r.missing = !here;
                match branch {
                    Some(b) if self.branches.get(&path) != Some(&b) => {
                        self.branches.insert(path, b);
                        cx.notify();
                    }
                    None if self.branches.remove(&path).is_some() => cx.notify(),
                    _ => {}
                }
            }
            if found {
                self.roots_changed(window, cx);
            }
        }
        // Orlo saved the settings file, or another window applied it, while the poll ran
        if cx.global::<SettingsDisk>().0 != settings_then {
            return;
        }
        // being written now: the next poll decides
        let (settings, resolved) = settings;
        if pipelines::writing(&settings::path(), cx) || pipelines::writing(&resolved, cx) {
            return;
        }
        // the settings file edited elsewhere applies as a save in Orlo would
        match settings {
            Disk::Same => {}
            // only a parse of a file that is there changes the settings
            Disk::Gone => cx.global_mut::<SettingsDisk>().0 = None,
            Disk::Changed(s) | Disk::Back(s) => {
                cx.global_mut::<SettingsDisk>().0 = Some(s);
                if let Ok(text) = std::fs::read_to_string(settings::path()) {
                    self.apply_settings(&text, window, cx);
                }
            }
        }
    }

    /// Set the buffer's stamp and red dot in every view of it.
    fn mark_disk(&mut self, editor: &Entity<Editor>, stamp: Option<Stamp>, deleted: bool, cx: &mut Context<Self>) {
        let d = editor.read(cx).doc(cx);
        if (d.disk, d.deleted) == (stamp, deleted) {
            return;
        }
        editor.update(cx, |e, cx| {
            e.update_doc(cx, |d| (d.disk, d.deleted) = (stamp, deleted));
            cx.notify();
        });
    }

    /// Read the buffer's file again off the UI thread and swap it in as one undoable step
    /// (Revert, Reload, an outside change); carets, bookmarks and scroll stay on their lines in
    /// every view. A file being written (Orlo's own save, a text pipeline) is read once the
    /// write has landed. A buffer edited, saved or reloaded meanwhile is left alone, for the
    /// next poll to decide, and so is a reload overtaken by a later one (a Revert). `failed`
    /// gets the read error.
    pub(crate) fn reload_editor(
        &mut self,
        editor: Entity<Editor>,
        failed: impl FnOnce(&mut Self, std::io::Error, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if editor.read(cx).doc(cx).edits_blocked() {
            return;
        }
        let Some(path) = editor.read(cx).doc(cx).path.clone() else {
            return;
        };
        self.reloads.retain(|(e, _)| !e.read(cx).same_document(editor.read(cx)));
        self.next_reload += 1;
        let ticket = self.next_reload;
        self.reloads.push((editor.clone(), ticket));
        cx.spawn_in(window, async move |this, cx| {
            // half written, or about to be replaced by a save that would then mark the reloaded
            // text unsaved against it: what the write leaves is what is read
            pipelines::wait_for_write(&path, cx).await;
            let reading = this.update(cx, |ws, cx| {
                // overtaken meanwhile, or moved by a Save As
                let i = ws.reloads.iter().position(|(_, t)| *t == ticket)?;
                let s = editor.read(cx).read_state(cx);
                if s.doc.path.as_ref() != Some(&path) || s.doc.edits_blocked() {
                    ws.reloads.remove(i);
                    return None;
                }
                let (rope, before) = (s.doc.rope.clone(), (s.doc.version, s.doc.disk, s.doc.dirty));
                let encoding = s.doc.disk_encoding;
                let read = cx.background_spawn(async move {
                    Loaded::read_with_hint(&path, Some(encoding)).map(|l| {
                        let changes = l.diff(&rope);
                        (l, changes)
                    })
                });
                Some((before, read))
            });
            let Ok(Some((before, read))) = reading else {
                return;
            };
            let read = read.await;
            this.update_in(cx, |ws, window, cx| {
                let Some(i) = ws.reloads.iter().position(|(_, t)| *t == ticket) else {
                    return;
                };
                ws.reloads.remove(i);
                let s = editor.read(cx).read_state(cx);
                if (s.doc.version, s.doc.disk, s.doc.dirty) != before
                    || ws.views.find(&editor).is_none()
                    || s.doc.edits_blocked()
                {
                    return;
                }
                let (loaded, changes) = match read {
                    Ok(r) => r,
                    Err(e) => return failed(ws, e, window, cx),
                };
                // through the document, so every view of it follows, the same text too
                editor.update(cx, |e, cx| e.reload(loaded, &changes, cx));
            })
            .ok();
        })
        .detach();
    }

    /// The file's text compared, off the UI thread, with what it was when last read or saved:
    /// the same takes the stamp quietly, another asks. Unknown (undo no longer holds it) asks.
    pub(crate) fn ask_if_text_changed(
        &mut self,
        editor: Entity<Editor>,
        s: Stamp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let st = editor.read(cx).read_state(cx);
        let (Some(path), Some(saved)) = (st.doc.path.clone(), st.saved_text()) else {
            return self.ask_reload(editor, s, window, cx);
        };
        // not polled again until this lands
        if !self.checking.insert(path.clone()) {
            return;
        }
        let (le, bom, since) = (st.doc.line_ending, st.doc.bom, st.doc.disk);
        let endings = st.doc.endings.clone();
        let encoding = st.doc.disk_encoding;
        let p = path.clone();
        let same = cx.background_spawn(async move {
            // the stamp of the text compared: the file may change again after the poll
            let now = watch::stamp(&p);
            let l = Loaded::read_with_hint(&p, Some(encoding));
            (
                now,
                l.is_ok_and(|l| {
                    l.rope == saved && l.endings == endings && (l.line_ending, l.bom, l.encoding) == (le, bom, encoding)
                }),
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            let (now, same) = same.await;
            this.update_in(cx, |ws, window, cx| {
                ws.checking.remove(&path);
                // gone, or saved, reloaded or marked meanwhile: the comparison is out of date
                let Some(s) = now.filter(|_| editor.read(cx).doc(cx).disk == since) else {
                    return;
                };
                // closed, or being written now: the next poll decides
                if ws.views.find(&editor).is_none()
                    || pipelines::writing(&path, cx)
                    || editor.read(cx).doc(cx).edits_blocked()
                {
                    return;
                }
                match same {
                    true => ws.mark_disk(&editor, Some(s), false, cx),
                    false => ws.ask_reload(editor, s, window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sublime's sheet for a dirty buffer changed on disk. The stamp is taken first, so the
    /// next poll does not ask again; Cancel asks again only on the next change.
    fn ask_reload(&mut self, editor: Entity<Editor>, s: Stamp, window: &mut Window, cx: &mut Context<Self>) {
        // one sheet at a time: another buffer is asked about on a later poll
        if self.asking {
            return;
        }
        self.mark_disk(&editor, Some(s), false, cx);
        let path = editor.read(cx).doc(cx).path.clone().unwrap_or_default();
        self.asking = true;
        let rx = window.prompt(
            PromptLevel::Warning,
            &path.to_string_lossy(),
            Some("Has changed on disk.\nDo you want to reload it?"),
            &["Reload", "Always Ignore", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = rx.await;
            this.update_in(cx, |ws, window, cx| {
                ws.asking = false;
                match answer {
                    Ok(0) => ws.reload_editor(
                        editor,
                        |ws, e, window, cx| ws.alert(window, cx, "Could not reload file", &e.to_string()),
                        window,
                        cx,
                    ),
                    Ok(1) => {
                        editor.update(cx, |e, cx| e.update_doc(cx, |d| d.watching = false));
                    }
                    _ => {}
                }
            })
            .ok();
        })
        .detach();
    }
}
