//! Copy File to Right / Left in a folder comparison: a file on one side only is copied to the
//! other once the prompt is answered, under the comparison's ownership checks and the app-wide
//! write leases.
use super::*;
use crate::files::fsops::reviewed_copy::ReviewedCopy;
use std::rc::Rc;

#[cfg(test)]
type StageHook = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

#[cfg(test)]
thread_local! {
    pub(crate) static AFTER_STAGE: std::cell::RefCell<Option<StageHook>> = const { std::cell::RefCell::new(None) };
    pub(crate) static AFTER_GUARD: std::cell::RefCell<Option<StageHook>> = const { std::cell::RefCell::new(None) };
}

struct CopyContext {
    generation: u64,
    project: u64,
    inventory: Arc<Inventory>,
    index: usize,
    ownership: Vec<(u64, PathBuf)>,
    environment: Environment,
}

/// `path` resolved through its nearest ancestor that exists, the missing names put back after
/// it. Errors other than NotFound are returned.
fn location(path: &Path) -> Result<PathBuf, String> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut physical) => {
                for child in missing.into_iter().rev() {
                    physical.push(child);
                }
                return Ok(physical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(ancestor.file_name().ok_or_else(|| error.to_string())?);
                ancestor = ancestor.parent().ok_or_else(|| error.to_string())?;
            }
            Err(error) => return Err(format!("Cannot validate {}: {error}", path.display())),
        }
    }
}

fn copy_owner(
    review: &ReviewedCopy,
    buffers: &[Buffer],
    busy: &[PathBuf],
    stop: &AtomicBool,
) -> Result<Option<Owner>, String> {
    let source = location(&review.source)?;
    let destination = location(&review.destination)?;
    let reservations = review.reservations();
    let targets: Vec<_> = reservations
        .iter()
        .map(|p| location(p))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|p| p != &source)
        .collect();
    let mut owner = None;
    for buffer in buffers {
        stopped(stop)?;
        let path = location(&buffer.owner.path)?;
        let same_source = matches_snapshot(&buffer.owner.path, &review.source, &review.snapshot)?;
        if path == destination || targets.iter().any(|target| path.starts_with(target) || target.starts_with(&path)) {
            return Err("The destination or a missing parent has an open buffer in an app window".into());
        }
        if same_source {
            if owner.is_some() {
                return Err("The source has ambiguous open buffers".into());
            }
            if buffer.owner.dirty
                || buffer.owner.deleted
                || buffer.rope != review.snapshot.text.rope
                || buffer.owner.format != review.snapshot.text.format
            {
                return Err("Save the source buffer and Refresh before copying its disk file".into());
            }
            owner = Some(buffer.owner.clone());
        }
    }
    for path in busy {
        stopped(stop)?;
        let physical = location(path)?;
        if [source.as_path(), destination.as_path()]
            .into_iter()
            .chain(targets.iter().map(PathBuf::as_path))
            .any(|target| target.starts_with(&physical) || physical.starts_with(target))
            || matches_snapshot(path, &review.source, &review.snapshot)?
        {
            return Err("The source or destination is loading, restoring or reserved for writing".into());
        }
    }
    stopped(stop)?;
    Ok(owner)
}

impl CopyContext {
    fn watch_publication(
        self: &Rc<Self>,
        ws: &Workspace,
        owner: &Option<Owner>,
        reserved: &[(PathBuf, Arc<()>)],
        stop: &Arc<AtomicBool>,
        cx: &mut Context<Workspace>,
    ) -> Vec<Subscription> {
        let context = self.clone();
        let owner = owner.clone();
        let leases = reserved.to_vec();
        let changed = stop.clone();
        // only the owners and environment already at hand are compared: no path I/O, no text
        // scan
        let invalidate = Rc::new(move |ws: &Workspace, cx: &App| {
            if !context.current(ws, &changed) || !context.intact(ws, &owner, &leases, cx) {
                changed.store(true, Ordering::Relaxed);
            }
        });
        let own = invalidate.clone();
        let mut guards = vec![cx.observe_self(move |ws, cx| own(ws, cx))];
        let others: Vec<_> = ws.other_workspace_handles(cx).filter_map(|w| w.entity(cx).ok()).collect();
        for other in others {
            let check = invalidate.clone();
            guards.push(cx.observe(&other, move |ws, _, cx| check(ws, cx)));
        }
        let writes_changed = stop.clone();
        guards.push(cx.observe_global::<pipelines::Writes>(move |_, _| writes_changed.store(true, Ordering::Relaxed)));
        let new_workspace = stop.clone();
        guards.push(cx.observe_new::<Workspace>(move |_, _, _| new_workspace.store(true, Ordering::Relaxed)));
        guards
    }

    fn current(&self, ws: &Workspace, stop: &Arc<AtomicBool>) -> bool {
        ws.comparison.generation == self.generation
            && ws.restore_gen == self.project
            && !stop.load(Ordering::Relaxed)
            && active(&ws.comparison.copy, stop)
            && matches!(&ws.panel, Some(Panel::Comparison(p)) if p.visible_pair() && p.folder.as_ref().is_some_and(|f|
                f.selected == Some(self.index) && f.inventory.as_ref().is_some_and(|i| Arc::ptr_eq(i, &self.inventory))))
    }

    fn intact(&self, ws: &Workspace, owner: &Option<Owner>, reserved: &[(PathBuf, Arc<()>)], cx: &App) -> bool {
        let buffers = ws.comparison_buffers(cx);
        if !reserved.iter().all(|(path, lease)| pipelines::owns_lease(path, lease, cx)) {
            return false;
        }
        let writes =
            pipelines::write_paths(cx).into_iter().filter(|p| !reserved.iter().any(|(path, _)| p == path)).collect();
        let environment = ws.comparison_environment_with_writes(writes, cx);
        ownership(&buffers) == self.ownership
            && environment == self.environment
            && owner.as_ref().is_none_or(|o| buffers.iter().any(|b| &b.owner == o))
    }
}

impl Workspace {
    pub(crate) fn copy_comparison_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.comparison.copy.is_some() {
            return;
        }
        let Some(Panel::Comparison(panel)) = &self.panel else {
            return;
        };
        let Some((source, index)) = panel.copy_candidate() else {
            return;
        };
        let folder = panel.folder.as_ref().unwrap();
        let inventory = folder.inventory.as_ref().unwrap().clone();
        let relative = inventory.rows[index].path.clone();
        let roots = folder.roots.clone();
        let remembered = self.remembered_disk_encoding(&roots[source].join(&relative));
        let buffers = self.comparison_buffers(cx);
        let context = Rc::new(CopyContext {
            generation: self.comparison.generation,
            project: self.restore_gen,
            inventory,
            index,
            ownership: ownership(&buffers),
            environment: self.comparison_environment(cx),
        });
        let busy = context.environment.busy.clone();
        let stop = renew(&mut self.comparison.copy);
        let worker_stop = stop.clone();
        let task = cx.background_spawn(async move {
            let hint = disk_hint(&roots[source].join(&relative), &buffers, &worker_stop)?;
            let review = Arc::new(ReviewedCopy::capture_with_hint(
                roots,
                relative,
                source,
                hint.map(|h| h.0).or(remembered),
                &worker_stop,
            )?);
            if hint.is_some_and(|(_, device, inode)| {
                device != review.snapshot.device || inode != review.snapshot.stamp.ino
            }) {
                return Err("The source changed while matching its open buffer; Refresh".into());
            }
            let owner = copy_owner(&review, &buffers, &busy, &worker_stop)?;
            Ok::<_, String>((review, owner))
        });
        let closed_stop = stop.clone();
        let closed = cx.on_window_closed(move |_| closed_stop.store(true, Ordering::Relaxed));
        let app = cx.to_async();
        cx.spawn_in(window, async move |this, cx| {
            let _closed = closed;
            let result = task.await;
            let confirmation = this.update_in(cx, |ws, window, cx| {
                if !context.current(ws, &stop) { ws.retire_cancelled_copy(&stop, window, cx); return None; }
                let (review, owner) = match result {
                    Ok(result) => result,
                    Err(error) => { ws.copy_result(format!("Copy rejected: {error}; Refresh to review current files"), false, window, cx); return None; }
                };
                if !context.intact(ws, &owner, &[], cx) {
                    ws.copy_result("Copy rejected: open buffers or reservations changed; Refresh".into(), false, window, cx); return None;
                }
                let parents = if review.missing.is_empty() { "None".into() } else { review.missing.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n") };
                let detail = format!("Source: {}\nDestination: {}\n\nMissing parent directories to create:\n{parents}\n\nThis creates a new file on disk. Existing files will never be replaced.", review.source.display(), review.destination.display());
                let answer = window.prompt(PromptLevel::Warning, if source == 0 { "Copy File to Right?" } else { "Copy File to Left?" }, Some(&detail), &["Copy File", "Cancel"], cx);
                Some((review, owner, answer))
            }).ok().flatten();
            let Some((review, owner, answer)) = confirmation else { return; };
            let confirmed = answer.await == Ok(0);
            let reserved = this.update_in(cx, |ws, window, cx| {
                if !context.current(ws, &stop) { ws.retire_cancelled_copy(&stop, window, cx); return None; }
                if !confirmed {
                    ws.copy_result("Copy cancelled — no files created".into(), false, window, cx); return None;
                }
                if !context.intact(ws, &owner, &[], cx) {
                    ws.copy_result("Copy rejected: source owners or reservations changed; Refresh".into(), true, window, cx); return None;
                }
                let mut paths = review.reservations();
                paths.extend(owner.iter().map(|o| o.path.clone()));
                paths.sort(); paths.dedup();
                let reserved = paths.into_iter().map(|path| { let lease = pipelines::lease(&path, cx); (path, lease) }).collect::<Vec<_>>();
                let editors: Vec<_> = ws.open_editors(cx).into_iter().filter(|e| owner.as_ref().is_some_and(|owner| e.read(cx).doc(cx).id() == owner.buffer)).collect();
                let changes: Vec<_> = editors.into_iter().map(|e| { let changed_stop = stop.clone(); Editor::on_text_change(&e, cx, move |_, _| {
                        changed_stop.store(true, Ordering::Relaxed);
                    }) }).collect();
                Some((reserved, changes))
            }).ok().flatten();
            let Some((reserved, _changes)) = reserved else { return; };
            let worker_review = review.clone();
            let worker_stop = stop.clone();
            let stage = cx.background_spawn(async move { worker_review.stage(&worker_stop) }).await;
            let result = match stage {
                Ok(staged) => {
                    let (allowed, _guard) = this.update_in(cx, |ws, _window, cx| {
                        #[cfg(test)]
                        AFTER_STAGE.with_borrow_mut(|hook| { if let Some(hook) = hook.take() { hook(ws, _window, cx); } });
                        let allowed = context.current(ws, &stop) && context.intact(ws, &owner, &reserved, cx);
                        let guard = if allowed { context.watch_publication(ws,&owner,&reserved,&stop,cx) } else { Vec::new() };
                        (allowed,guard)
                    }).unwrap_or((false,Vec::new()));
                    #[cfg(test)]
                    if allowed {
                        this.update_in(cx, |ws,w,cx| AFTER_GUARD.with_borrow_mut(|hook| {
                            if let Some(hook) = hook.take() { hook(ws,w,cx); }
                        })).ok();
                    }
                    let worker_stop = stop.clone();
                    cx.background_spawn(async move {
                        let _reserved = reserved;
                        staged.finish(allowed, &worker_stop)
                    }).await
                }
                Err(error) => { drop(reserved); Err(error) }
            };
            let message = match result { Ok(()) => format!("Copied file to {}", review.destination.display()), Err(error) if error.starts_with("Copied file") => error, Err(error) => format!("Copy stopped: {error}") };
            let delivered = this.update_in(cx, |ws, window, cx| {
                if active(&ws.comparison.copy, &stop) { ws.copy_result(message.clone(), true, window, cx); }
                else { ws.status_message(message.clone(), cx); ws.sidebar.refresh(); cx.notify(); }
            });
            if delivered.is_err() {
                app.update(|cx| {
                    for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
                        if handle.update(cx, |ws,_,cx| { ws.status_message(message.clone(),cx); ws.sidebar.refresh(); cx.notify(); }).is_ok() { break; }
                    }
                }).ok();
            }
        }).detach();
        cx.notify();
    }

    fn retire_cancelled_copy(&mut self, stop: &Arc<AtomicBool>, window: &mut Window, cx: &mut Context<Self>) {
        if active(&self.comparison.copy, stop) {
            self.copy_result("Copy cancelled — no files created".into(), false, window, cx);
        }
    }

    fn copy_result(&mut self, message: String, refresh: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.comparison.copy = None;
        if let Some(folder) = self.folder_review_mut() {
            folder.copy_message = Some(message.clone());
        }
        if refresh {
            self.refresh_comparison(&RefreshComparison, window, cx);
            self.sidebar.refresh();
        }
        self.status_message(message, cx);
        cx.notify();
    }
}
