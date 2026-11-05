//! Explicit log following belongs to the shared document, independently of its windows.
use super::*;
use crate::files::watch::{self, follow::Read};
use crate::views::workspace::pipelines;
use gpui::Task;
use std::time::Duration;

pub(crate) struct Following {
    pub(crate) status: String,
    pub(crate) paused: bool,
    path: PathBuf,
    _task: Task<()>,
}

impl OpenDocument {
    pub(crate) fn start_follow(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.model.is_following() {
            return Ok(());
        }
        let path =
            self.model.path.clone().filter(|_| self.model.disk.is_some()).ok_or("Follow Log requires a saved file")?;
        if self.model.dirty || self.model.deleted || self.saving() {
            return Err("Save or discard changes and wait for pending saves before following".into());
        }
        if pipelines::writing(&path, cx) {
            return Err("Wait for the file's write to finish before following".into());
        }
        let before = Facts::of(&self.model);
        if let Some(change) = self.model.set_following(true) {
            cx.emit(DocumentEvent::Edited(change));
        }
        let p = path.clone();
        let mut view = self.model.attach_view();
        let task = cx.spawn(async move |this, cx| {
            let mut observed = None;
            loop {
                let plan = this.update(cx, |d, cx| {
                    if !d.model.is_following() || d.model.path.as_ref() != Some(&p) || d.tabs == 0 {
                        return None;
                    }
                    if d.model.viewer_mode() || pipelines::writing(&p, cx) {
                        return Some(None);
                    }
                    Some(Some((d.model.rope.clone(), d.model.version, d.model.disk, d.model.disk_encoding)))
                });
                let Ok(Some(plan)) = plan else {
                    break;
                };
                if let Some((rope, version, disk, encoding)) = plan {
                    let path = p.clone();
                    let read = cx
                        .background_spawn(async move {
                            let result = watch::follow::read(&path, observed, encoding, &rope);
                            (result, watch::stamp(&path))
                        })
                        .await;
                    #[cfg(test)]
                    gate::wait(&p).await;
                    // A queued delivery can outlive rotation/truncation. Validate the disk
                    // stamp off-thread again before offering the snapshot to the document.
                    let path = p.clone();
                    let current = cx.background_spawn(async move { watch::stamp(&path) }).await;
                    let applied = this.update(cx, |d, cx| {
                        if !d.model.is_following() || d.model.path.as_ref() != Some(&p) || d.tabs == 0 {
                            return None;
                        }
                        if d.model.version != version
                            || d.model.disk != disk
                            || d.model.disk_encoding != encoding
                            || d.model.dirty
                            || d.model.viewer_mode()
                            || d.saving()
                            || pipelines::writing(&p, cx)
                            || read.1 != current
                        {
                            return Some(true);
                        }
                        match read.0 {
                            Ok(Read::Same | Read::Retry) => {}
                            Ok(Read::Missing) => d.follow_status("Waiting for log to reappear", false, cx),
                            Ok(Read::Incomplete { disk, file }) => {
                                if pipelines::writing_file(Some(file), cx) || Some(disk) != current {
                                    return Some(true);
                                }
                                observed = Some(disk);
                                d.update_file(
                                    |m| {
                                        m.disk = None;
                                        m.read_only = true;
                                    },
                                    cx,
                                );
                                d.follow_status("Following log; waiting for an incomplete BOM", false, cx);
                            }
                            Ok(Read::Snapshot { mut loaded, changes, partial, file }) => {
                                if pipelines::writing_file(Some(file), cx) {
                                    return Some(true);
                                }
                                if loaded.disk != current {
                                    return Some(true);
                                }
                                observed = loaded.disk;
                                // A prefix is safe to show, never safe to overwrite as a complete
                                // file. Stop leaves this protection for ordinary reload to resolve.
                                if partial {
                                    loaded.disk = None;
                                }
                                d.edit(&mut view, |s| s.follow_from(loaded, &changes), cx);
                                d.follow_status(
                                    if partial {
                                        "Following log; waiting for an incomplete character"
                                    } else {
                                        "Following log"
                                    },
                                    false,
                                    cx,
                                );
                            }
                            Err(failure) => {
                                if failure.disk != current || pipelines::writing_file(failure.file, cx) {
                                    return Some(true);
                                }
                                d.update_file(
                                    |m| {
                                        m.read_only = true;
                                        m.disk = None;
                                    },
                                    cx,
                                );
                                d.follow_status(&format!("Follow paused: {}", failure.error), true, cx);
                                return Some(false);
                            }
                        }
                        Some(true)
                    });
                    if !matches!(applied, Ok(Some(true))) {
                        break;
                    }
                }
                cx.background_executor().timer(Duration::from_millis(500)).await;
            }
        });
        self.following = Some(Following { status: "Following log".into(), paused: false, path, _task: task });
        self.report(before, cx);
        Ok(())
    }

    fn follow_status(&mut self, status: &str, paused: bool, cx: &mut Context<Self>) {
        if let Some(follow) = &mut self.following {
            if follow.status != status || follow.paused != paused {
                follow.status = status.into();
                follow.paused = paused;
                cx.emit(DocumentEvent::File { document: self.id(), renamed: false });
                cx.notify();
            }
        }
    }

    pub(crate) fn stop_follow(&mut self, cx: &mut Context<Self>) {
        let before = Facts::of(&self.model);
        self.following = None; // dropping the task cancels its timer and queued delivery
        if let Some(change) = self.model.set_following(false) {
            cx.emit(DocumentEvent::Edited(change));
        }
        self.report(before, cx);
    }

    pub(crate) fn stop_follow_if_moved(&mut self, cx: &mut Context<Self>) {
        if self.following.as_ref().is_some_and(|f| self.model.path.as_ref() != Some(&f.path)) {
            self.stop_follow(cx);
        }
    }
}

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

    /// Follow deliveries wait after their read until `release`.
    pub(crate) struct Pause(Rc<Shared>, PathBuf);

    pub(crate) fn pause(path: &Path) -> Pause {
        let shared = Rc::new(Shared::default());
        GATES.with(|g| g.borrow_mut().insert(path.to_path_buf(), shared.clone()));
        Pause(shared, path.to_path_buf())
    }

    impl Pause {
        /// How many reads have reached the gate.
        pub(crate) fn arrived(&self) -> usize {
            self.0.arrived.get()
        }

        /// The waiting read delivery goes on, and later ones do not wait.
        pub(crate) fn release(&self) {
            self.0.open.set(true);
            GATES.with(|g| {
                let mut gates = g.borrow_mut();
                if gates.get(&self.1).is_some_and(|current| Rc::ptr_eq(current, &self.0)) {
                    gates.remove(&self.1);
                }
            });
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
