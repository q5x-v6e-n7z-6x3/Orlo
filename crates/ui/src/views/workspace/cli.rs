//! Files from the `Orlo` command line: opened as any other open is (waiting on reads and writes
//! under way), and `--wait` waits until their buffers' last views close in every window.

use super::*;
use crate::app::cli::{Position, Target};
use crate::config::project::{PROJECT_EXT, WORKSPACE_EXT};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

type OpenResult = Rc<RefCell<Option<Result<Option<u64>, String>>>>;

struct Pending {
    result: OpenResult,
    position: Option<Position>,
    generation: u64,
}

struct Opening {
    result: OpenResult,
    window: WindowHandle<Workspace>,
    generation: u64,
}

#[derive(Default)]
pub(crate) struct State {
    pending: HashMap<PathBuf, Vec<Pending>>,
}

pub fn open_request(mut delivery: crate::app::handoff::Delivery, cx: &mut App) {
    let Some(request) = delivery.request.take() else {
        return;
    };
    let preferred = front_window(cx);
    let mut opening = Vec::new();
    for target in request.targets {
        let target = match target {
            Target::Path { path, position } => Target::Path { path: crate::editing::text::resolve(&path), position },
            other => other,
        };
        let existing = match &target {
            Target::Path { path, .. } => {
                workspace_windows(cx).find(|w| w.read(cx).is_ok_and(|ws| ws.editor_for_path(path, cx).is_some()))
            }
            _ => None,
        };
        let handle = existing.unwrap_or(preferred);
        let result: OpenResult = Rc::new(RefCell::new(None));
        let generation = handle.update(cx, |ws, window, cx| {
            window.activate_window();
            let generation = ws.restore_gen;
            match target {
                Target::Stdin(text) => {
                    ws.drop_blank_untitled(cx);
                    let mut state = EditorState::from_source(&text);
                    state.forget_saved_point();
                    let id = state.buffer_id();
                    ws.add_tab(state, window, cx);
                    *result.borrow_mut() = Some(Ok(Some(id)));
                }
                Target::Path { path, position } => {
                    let project = position.is_none()
                        && matches!(path.extension().and_then(|e| e.to_str()), Some(PROJECT_EXT | WORKSPACE_EXT));
                    if project {
                        *result.borrow_mut() = Some(
                            ws.try_open_project_path(&OpenProjectPath { path, here: false }, window, cx).map(|()| None),
                        );
                    } else if path.is_dir() {
                        ws.open_path(path, window, cx);
                        *result.borrow_mut() = Some(Ok(None));
                    } else {
                        ws.cli.pending.retain(|_, pending| {
                            pending.retain(|p| p.generation == generation && p.result.borrow().is_none());
                            !pending.is_empty()
                        });
                        ws.cli.pending.entry(path.clone()).or_default().push(Pending {
                            result: result.clone(),
                            position,
                            generation,
                        });
                        if let Some(kind) = ws.previewing.get_mut(&path) {
                            *kind = PendingOpen::Kept;
                        }
                        ws.open_path(path, window, cx);
                    }
                }
            }
            generation
        });
        match generation {
            Ok(generation) => opening.push(Opening { result, window: handle, generation }),
            Err(error) => {
                delivery.reject(format!("could not open the requested window: {error}"));
                return;
            }
        }
    }
    let mut delivery = Some(delivery);
    let mut completion: Option<crate::app::handoff::Completion> = None;
    let mut buffers = HashSet::new();
    cx.spawn(async move |cx| loop {
        let ended = cx
            .update(|cx| {
                if delivery.is_some() {
                    if !crate::app::handoff::accepting() {
                        delivery.take().unwrap().reject("Orlo began quitting before all requested files opened".into());
                        return true;
                    }
                    let mut pending = false;
                    for open in &opening {
                        if open.result.borrow().is_none()
                            && open.window.read(cx).map_or(true, |ws| ws.restore_gen != open.generation)
                        {
                            *open.result.borrow_mut() = Some(Err(
                                "the target window closed or switched projects before the file opened".into(),
                            ));
                        }
                        match open.result.borrow().as_ref() {
                            Some(Ok(Some(id))) => {
                                buffers.insert(*id);
                            }
                            Some(Ok(None)) => {}
                            Some(Err(error)) => {
                                delivery.take().unwrap().reject(error.clone());
                                return true;
                            }
                            None => pending = true,
                        }
                    }
                    if pending {
                        return false;
                    }
                    completion = Some(delivery.take().unwrap().begin_wait());
                }
                if !request.wait || completion.as_ref().is_some_and(|c| !c.connected()) || buffers_closed(&buffers, cx)
                {
                    if let Some(completion) = &completion {
                        completion.finish(Ok(()));
                    }
                    return true;
                }
                false
            })
            .unwrap_or(true);
        if ended {
            break;
        }
        cx.background_executor().timer(Duration::from_millis(20)).await;
    })
    .detach();
}

fn buffers_closed(buffers: &HashSet<u64>, cx: &App) -> bool {
    workspace_windows(cx)
        .all(|w| w.read(cx).map_or(true, |ws| ws.views.tabs().all(|e| !buffers.contains(&e.read(cx).doc(cx).id()))))
}

impl Workspace {
    /// A command line waiting on `path` gets its answer, the caret at its line and column.
    /// Every open calls this as it ends: a background read, a session restore, a view reused.
    pub(crate) fn finish_cli_open(&mut self, path: &Path, error: Option<String>, cx: &mut Context<Self>) {
        let Some(pending) = self.cli.pending.remove(path) else {
            return;
        };
        let editor = self.editor_for_path(path, cx);
        for pending in pending {
            if pending.result.borrow().is_some() {
                continue;
            }
            let result = if pending.generation != self.restore_gen {
                Err("the project changed before the file opened".into())
            } else if let Some(error) = &error {
                Err(error.clone())
            } else if let Some(editor) = &editor {
                editor.update(cx, |e, cx| {
                    if let Some(p) = &pending.position {
                        let d = e.doc(cx);
                        let line = p.line.saturating_sub(1).min(d.line_count() - 1);
                        let start = d.line_start(line).saturating_add(p.column.saturating_sub(1));
                        e.jump_to(start.min(d.line_end(line)), cx);
                    }
                    Ok(Some(e.doc(cx).id()))
                })
            } else {
                Err(format!("{} did not open", path.display()))
            };
            *pending.result.borrow_mut() = Some(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{settle, settle_until, temp, type_in, window};
    use super::*;
    use crate::app::cli::{Position, Request, Target};
    use crate::app::handoff::Delivery;
    use gpui::{TestAppContext, VisualTestContext};

    fn request(wait: bool, targets: Vec<Target>) -> Delivery {
        Delivery::local(Request { wait, targets })
    }

    fn file(path: &Path, position: Option<Position>) -> Target {
        Target::Path { path: path.to_path_buf(), position }
    }

    /// A session of `path` alone, restored in the background (the file is large).
    fn restore(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, path: &Path) {
        ws.update_in(vcx, |ws, window, cx| {
            let views = vec![ViewState { path: path.to_path_buf(), ..Default::default() }];
            let groups = vec![GroupState { views, active: 0 }];
            let state = WindowState { groups, ..Default::default() };
            ws.restore(state, window, cx)
        });
    }

    fn loaded(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
        settle_until(vcx, |cx| ws.read_with(cx, |ws, _| ws.loading.is_empty()));
    }

    #[gpui::test]
    fn cli_joins_background_session_restore(cx: &mut TestAppContext) {
        let dir = temp("g65-restore");
        let path = dir.join("large.txt");
        std::fs::write(&path, format!("first\nsecond\n{}", "x\n".repeat(2_100_000))).unwrap();
        let path = path.canonicalize().unwrap();
        let (ws, vcx) = window(cx);
        restore(&ws, vcx, &path);
        let delivery = request(true, vec![file(&path, Some(Position { line: 2, column: 2 }))]);
        let completion = delivery.completion();
        vcx.cx.update(|cx| open_request(delivery, cx));
        loaded(&ws, vcx);
        ws.update(vcx, |ws, cx| {
            assert!(ws.cli.pending.is_empty(), "restored file must resolve its CLI open");
            let editor = ws.editor_for_path(&path, cx).unwrap();
            assert_eq!(editor.read(cx).state.cursor(), 7);
            assert!(completion.outcome().is_none());
            ws.remove_view(&editor, cx);
        });
        settle(vcx);
        assert_eq!(completion.outcome(), Some(Ok(())));
    }

    #[gpui::test]
    fn cli_joined_restore_failure_is_reported(cx: &mut TestAppContext) {
        let path = temp("g65-restore-failure").join("large.txt");
        std::fs::write(&path, "x\n".repeat(2_100_000)).unwrap();
        let path = path.canonicalize().unwrap();
        let (ws, vcx) = window(cx);
        restore(&ws, vcx, &path);
        let delivery = request(false, vec![file(&path, None)]);
        let completion = delivery.completion();
        vcx.cx.update(|cx| open_request(delivery, cx));
        std::fs::remove_file(path).unwrap();
        loaded(&ws, vcx);
        settle(vcx);
        assert!(completion.outcome().is_some_and(|r| r.is_err()), "failed restore must reject CLI open");
    }

    #[gpui::test]
    fn cli_keeps_a_joined_sidebar_preview(cx: &mut TestAppContext) {
        let dir = temp("g65-preview");
        let path = dir.join("large.txt");
        let other = dir.join("other.txt");
        std::fs::write(&path, "x\n".repeat(2_100_000)).unwrap();
        std::fs::write(&other, "other").unwrap();
        let path = path.canonicalize().unwrap();
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| ws.open_preview(path.clone(), window, cx));
        let delivery = request(false, vec![file(&path, None)]);
        let completion = delivery.completion();
        vcx.cx.update(|cx| open_request(delivery, cx));
        ws.update_in(vcx, |ws, window, cx| ws.open_preview(other, window, cx));
        loaded(&ws, vcx);
        settle(vcx);
        assert_eq!(completion.outcome(), Some(Ok(())));
        assert!(ws.read_with(vcx, |ws, cx| ws.editor_for_path(&path, cx).is_some()));
    }

    #[gpui::test]
    fn cli_reports_malformed_project_and_workspace(cx: &mut TestAppContext) {
        let dir = temp("g65-project-error");
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, "keep this draft");
        for ext in [PROJECT_EXT, WORKSPACE_EXT] {
            let path = dir.join(format!("bad.{ext}"));
            std::fs::write(&path, "{").unwrap();
            let delivery = request(true, vec![file(&path, None)]);
            let completion = delivery.completion();
            vcx.cx.update(|cx| open_request(delivery, cx));
            settle(vcx);
            assert!(completion.outcome().is_some_and(|r| r.is_err()));
            assert_eq!(
                ws.read_with(vcx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).text()),
                "keep this draft"
            );
        }
    }

    #[gpui::test]
    fn cli_opens_a_named_missing_file_and_clamps_unicode_location(cx: &mut TestAppContext) {
        let dir = temp("orlo-cli-open");
        let existing = dir.join("unicode.txt");
        let missing = dir.join("new-file.txt");
        std::fs::write(&existing, "first\r\né界\r\nend").unwrap();
        let (ws, vcx) = window(cx);
        let position = Some(Position { line: 2, column: 90 });
        let targets = vec![file(&existing, position), file(&missing, None)];
        vcx.cx.update(|cx| open_request(request(false, targets), cx));
        ws.read_with(vcx, |ws, cx| {
            let existing =
                ws.editor_for_path(&crate::editing::text::resolve(&existing), cx).expect("existing file opened");
            assert_eq!(existing.read(cx).doc(cx).text(), "first\né界\nend");
            assert_eq!(existing.read(cx).doc(cx).line_ending, "\r\n");
            assert_eq!(
                existing.read(cx).state.cursor(),
                8,
                "column clamps at the normalized line end, in Unicode characters"
            );
            let missing = ws
                .editor_for_path(&crate::editing::text::resolve(&missing), cx)
                .expect("missing file becomes named buffer");
            assert_eq!(missing.read(cx).doc(cx).text(), "");
            assert!(!missing.read(cx).doc(cx).path.as_ref().unwrap().exists());
        });
    }

    #[gpui::test]
    fn stdin_opens_unsaved_and_does_not_replace_an_existing_draft(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, "draft");
        let targets = vec![Target::Stdin("piped café".into())];
        vcx.cx.update(|cx| open_request(request(false, targets), cx));
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 2);
            let editor = ws.active_editor().unwrap();
            let state = editor.read(cx).read_state(cx);
            assert_eq!(state.text(), "piped café");
            assert!(state.doc.path.is_none());
            assert!(state.doc.dirty);
        });
    }

    #[gpui::test]
    fn piped_text_stays_unsaved_through_an_edit_and_its_undo(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let targets = vec![Target::Stdin("piped".into())];
        vcx.cx.update(|cx| open_request(request(false, targets), cx));
        type_in(&ws, vcx, "x");
        ws.update(vcx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
            assert_eq!(e.read(cx).text(cx), "piped");
            assert!(e.read(cx).doc(cx).dirty, "no file holds it: closing asks");
        });
    }

    #[gpui::test]
    fn wait_tracks_the_final_shared_view_in_another_window(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let (first, second, id) = ws.update_in(vcx, |ws, window, cx| {
            let first = ws.active_editor().unwrap();
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            let second = ws.active_editor().unwrap();
            let id = first.read(cx).doc(cx).id();
            (first, second, id)
        });
        let other_window = vcx.cx.update(|cx| open_window(None, cx));
        let mut other = VisualTestContext::from_window(other_window.into(), &vcx.cx);
        let other_ws = other_window
            .update(&mut other, |ws, window, cx| {
                ws.drop_blank_untitled(cx);
                ws.attach(second.clone(), false, window, cx);
                cx.entity()
            })
            .unwrap();
        ws.update(vcx, |ws, cx| {
            ws.remove_view(&second, cx);
            ws.remove_view(&first, cx);
        });
        let buffers = HashSet::from([id]);
        assert!(!vcx.cx.update(|cx| buffers_closed(&buffers, cx)), "a moved or cloned view still holds the wait");
        other_ws.update(&mut other, |ws, cx| ws.remove_view(&second, cx));
        assert!(
            vcx.cx.update(|cx| buffers_closed(&buffers, cx)),
            "the final view releases the wait even while Entity handles stay alive"
        );
    }
}
