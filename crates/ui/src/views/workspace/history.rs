//! File history: a file's saved snapshots to browse, one restored into the active buffer.

use super::pickers::choice;
use super::*;
use crate::files::file_history;
use sha2::{Digest, Sha256};

actions!(workspace, [BrowseFileHistory, RestoreFileHistory, ClearFileHistory]);

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct PreviewFileHistory {
    pub file: PathBuf,
    pub editor: EntityId,
    pub buffer: u64,
    pub revision: u64,
    pub generation: u64,
    pub id: String,
}

pub struct HistoryPanel {
    pub viewer: Entity<Editor>,
    target: Target,
    id: String,
    snapshot: Option<(file_history::Version, String)>,
    busy: bool,
    note: String,
}

#[derive(Clone)]
struct Target {
    editor: Entity<Editor>,
    path: PathBuf,
    buffer: u64,
    revision: u64,
    generation: u64,
    encoding: crate::editing::encoding::Encoding,
    bom: bool,
    ending: &'static str,
}

impl Target {
    fn current(ws: &Workspace, cx: &App) -> Option<Self> {
        let editor = ws.active_editor()?;
        let e = editor.read(cx);
        if e.results || e.doc(cx).scratch {
            return None;
        }
        Some(Self {
            path: e.doc(cx).path.clone()?,
            buffer: e.doc(cx).id(),
            revision: e.doc(cx).version,
            generation: ws.restore_gen,
            encoding: e.doc(cx).encoding,
            bom: e.doc(cx).bom,
            ending: e.doc(cx).line_ending,
            editor: editor.clone(),
        })
    }

    fn validate(&self, ws: &Workspace, cx: &App, writable: bool) -> Result<(), String> {
        let e = self.editor.read(cx);
        let s = e.read_state(cx);
        if ws.restore_gen != self.generation
            || ws.active_editor().as_ref() != Some(&self.editor)
            || ws.views.find(&self.editor).is_none()
            || s.doc.path.as_ref() != Some(&self.path)
            || s.buffer_id() != self.buffer
            || s.doc.version != self.revision
            || e.results
            || s.doc.scratch
            || (s.doc.encoding, s.doc.bom, s.doc.line_ending) != (self.encoding, self.bom, self.ending)
        {
            return Err("The active file changed; browse its history again".into());
        }
        if writable && (s.doc.read_only || s.doc.edits_blocked()) {
            return Err("The active file is read-only".into());
        }
        if pipelines::writing(&self.path, cx) || ws.loading.contains(&self.path) {
            return Err("The active file is loading or being written; try again when it finishes".into());
        }
        Ok(())
    }

    fn action(&self, id: String) -> PreviewFileHistory {
        PreviewFileHistory {
            file: self.path.clone(),
            editor: self.editor.entity_id(),
            buffer: self.buffer,
            revision: self.revision,
            generation: self.generation,
            id,
        }
    }
}

struct Snapshot {
    version: file_history::Version,
    digest: String,
    text: String,
}

fn snapshot(root: &Path, path: &Path, id: &str) -> Result<Snapshot, String> {
    let record = file_history::read_in(root, path, id).map_err(|error| error.to_string())?;
    let decoded = crate::editing::encoding::decode(&record.bytes, Some(record.version.encoding));
    if decoded.lossy {
        return Err("This history snapshot contains malformed text and cannot be restored".into());
    }
    Ok(Snapshot {
        version: record.version,
        digest: format!("{:x}", Sha256::digest(&record.bytes)),
        text: crate::editing::text::decoded_file(&decoded.text).0.to_string(),
    })
}

pub(super) fn captured_at(milliseconds: u64) -> String {
    let seconds: libc::time_t = (milliseconds / 1000).try_into().unwrap_or(libc::time_t::MAX);
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    let mut buffer = [0u8; 64];
    // SAFETY: localtime_r writes the valid tm before strftime reads it; both output buffers
    // have the advertised sizes and the fixed format is NUL-terminated.
    let length = unsafe {
        if libc::localtime_r(&seconds, local.as_mut_ptr()).is_null() {
            return "Unknown capture time".into();
        }
        libc::strftime(buffer.as_mut_ptr().cast(), buffer.len(), c"%Y-%m-%d %H:%M:%S".as_ptr(), local.as_ptr())
    };
    String::from_utf8_lossy(&buffer[..length]).into_owned()
}

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-enter", RestoreFileHistory, Some("FileHistoryPanel > Editor")),
        KeyBinding::new("escape", ClosePanel, Some("FileHistoryPanel > Editor")),
    ]
}

impl Workspace {
    fn file_history_panel(&self) -> Option<&HistoryPanel> {
        match &self.panel {
            Some(Panel::History(panel)) => Some(panel),
            _ => None,
        }
    }

    fn file_history_panel_mut(&mut self) -> Option<&mut HistoryPanel> {
        match &mut self.panel {
            Some(Panel::History(panel)) => Some(panel),
            _ => None,
        }
    }

    fn history_error(&mut self, viewer: &Entity<Editor>, error: String, cx: &mut Context<Self>) {
        if let Some(p) = self.file_history_panel_mut().filter(|p| &p.viewer == viewer) {
            p.busy = false;
            p.note = error;
            cx.notify();
        }
    }

    pub(crate) fn browse_file_history(&mut self, _: &BrowseFileHistory, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = Target::current(self, cx) else {
            return self.status_message("Open a file to browse its history", cx);
        };
        if let Err(error) = target.validate(self, cx, false) {
            return self.status_message(error, cx);
        }
        self.open_quick_panel(Kind::Choices, Vec::new(), window, cx);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some("File History".into());
        panel.progress = Some("Loading saved versions…".into());
        let input = panel.input.clone();
        let (storage, path) = (file_history::root(), target.path.clone());
        let reading = cx.background_spawn(async move { file_history::list_in(&storage, &path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = reading.await;
            this.update_in(cx, |ws, _, cx| {
                if ws.quick_panel.as_ref().is_none_or(|p| p.input != input) {
                    return;
                }
                if let Err(error) = target.validate(ws, cx, false) {
                    ws.cancel_quick_panel(cx);
                    return ws.status_message(error, cx);
                }
                let versions = match result {
                    Ok(versions) if !versions.is_empty() => versions,
                    Ok(_) => {
                        ws.cancel_quick_panel(cx);
                        return ws.status_message(
                            "No saved versions for this file; enable file_history to keep originals before saves",
                            cx,
                        );
                    }
                    Err(error) => {
                        ws.cancel_quick_panel(cx);
                        return ws.status_message(format!("Could not read file history: {error}"), cx);
                    }
                };
                let items = versions
                    .into_iter()
                    .enumerate()
                    .map(|(i, version)| {
                        choice(
                            &format!("{} · {}", i + 1, captured_at(version.created_ms)),
                            &format!("{} · {} bytes · Preview saved text", version.encoding.name(), version.bytes),
                            ItemAction::Dispatch(Box::new(target.action(version.id))),
                        )
                    })
                    .collect();
                let panel = ws.quick_panel.as_mut().unwrap();
                panel.set_items(Kind::Choices, items);
                panel.progress =
                    Some("Choose a version to preview; restore changes the buffer in one Undo step".into());
                panel.refilter(&panel.query.clone());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn preview_file_history(
        &mut self,
        action: &PreviewFileHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = Target::current(self, cx) else {
            return;
        };
        if target.action(action.id.clone()) != *action {
            return self.status_message("The active file changed; browse its history again", cx);
        }
        if let Err(error) = target.validate(self, cx, false) {
            return self.status_message(error, cx);
        }
        self.cancel_quick_panel(cx);
        let viewer = cx.new(|cx| {
            let mut state = EditorState::new();
            state.doc.read_only = true;
            state.doc.scratch = true;
            let mut editor = Editor::new(state, false, cx);
            editor.view.minimap = false;
            editor.highlighter = Highlighter::new(Some(&target.path));
            editor
        });
        self.show_panel(
            Panel::History(Box::new(HistoryPanel {
                viewer: viewer.clone(),
                target: target.clone(),
                id: action.id.clone(),
                snapshot: None,
                busy: true,
                note: "Loading saved text…".into(),
            })),
            cx,
        );
        viewer.read(cx).focus(window);
        let (storage, path, id) = (file_history::root(), target.path.clone(), action.id.clone());
        let reading = cx.background_spawn(async move { snapshot(&storage, &path, &id) });
        cx.spawn(async move |this, cx| {
            let result = reading.await;
            this.update(cx, |ws, cx| {
                if ws.file_history_panel().is_none_or(|p| p.viewer != viewer) {
                    return;
                }
                let result = result.and_then(|snapshot| target.validate(ws, cx, false).map(|()| snapshot));
                let loaded = match result {
                    Ok(loaded) => loaded,
                    Err(error) => return ws.history_error(&viewer, error, cx),
                };
                viewer.update(cx, |e, cx| {
                    e.set_text(&loaded.text, cx);
                    e.update_doc(cx, |d| {
                        d.read_only = true;
                        d.scratch = true;
                    });
                    e.sel(cx).set_cursor(0);
                });
                let p = ws.file_history_panel_mut().unwrap();
                p.note = format!(
                    "{} · {} · Restores text only; current output encoding and line endings stay",
                    captured_at(loaded.version.created_ms),
                    loaded.version.encoding.name()
                );
                p.snapshot = Some((loaded.version, loaded.digest));
                p.busy = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn restore_file_history(&mut self, _: &RestoreFileHistory, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.file_history_panel().filter(|p| !p.busy) else {
            return;
        };
        let Some(expected) = panel.snapshot.clone() else {
            return;
        };
        let (target, id, viewer) = (panel.target.clone(), panel.id.clone(), panel.viewer.clone());
        if let Err(error) = target.validate(self, cx, true) {
            return self.history_error(&viewer, error, cx);
        }
        self.file_history_panel_mut().unwrap().busy = true;
        self.file_history_panel_mut().unwrap().note = "Checking saved version…".into();
        cx.notify();
        let (storage, path) = (file_history::root(), target.path.clone());
        let reading = cx.background_spawn(async move {
            let loaded = snapshot(&storage, &path, &id)?;
            if (loaded.version.clone(), loaded.digest.clone()) != expected {
                return Err("The history snapshot changed; browse its history again".to_string());
            }
            Ok(loaded.text)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = reading.await;
            this.update_in(cx, |ws, window, cx| {
                if ws.file_history_panel().is_none_or(|p| p.viewer != viewer) {
                    return;
                }
                let result = result.and_then(|text| target.validate(ws, cx, true).map(|()| text));
                let text = match result {
                    Ok(text) => text,
                    Err(error) => return ws.history_error(&viewer, error, cx),
                };
                target.editor.update(cx, |e, cx| {
                    e.edit(cx, |s| {
                        s.replace_ranges(vec![(0..s.len(), text)]);
                    })
                });
                ws.close_panel(&ClosePanel, window, cx);
                ws.status_message("Restored historical text into the buffer; save when ready", cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn clear_file_history(&mut self, _: &ClearFileHistory, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = Target::current(self, cx) else {
            return self.status_message("Open a file to clear its history", cx);
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Clear file history for {}?", search::tilde(&target.path)),
            Some("Permanently delete this file's stored versions."),
            &["Clear History", "Cancel"],
            cx,
        );
        let storage = file_history::root();
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let valid = this
                .update(cx, |ws, cx| match target.validate(ws, cx, false) {
                    Ok(()) => true,
                    Err(error) => {
                        ws.status_message(error, cx);
                        false
                    }
                })
                .unwrap_or(false);
            if !valid {
                return;
            }
            let path = target.path.clone();
            let result = cx.background_executor().spawn(async move { file_history::clear_in(&storage, &path) }).await;
            this.update_in(cx, |ws, window, cx| match result {
                Ok(()) => {
                    if ws.file_history_panel().is_some_and(|p| p.target.path == target.path) {
                        ws.close_panel(&ClosePanel, window, cx);
                    }
                    ws.status_message("File history cleared", cx);
                }
                Err(error) => ws.status_message(format!("Could not clear file history: {error}"), cx),
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn render_file_history(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(p) = self.file_history_panel() else {
            return div().into_any_element();
        };
        let button = |id: &'static str, label: &'static str, action: Box<dyn Action>| {
            kit::ghost_button(id, label)
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(move |_: &ClickEvent, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        };
        div()
            .key_context("FileHistoryPanel")
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(ui(theme::PANEL_BG))
            .child(
                kit::panel_header()
                    .child(div().flex_1().min_w_0().overflow_hidden().child("File History"))
                    .children(
                        [
                            ("history-browse", "Versions…", BrowseFileHistory.boxed_clone()),
                            ("history-restore", "Restore Text", Box::new(RestoreFileHistory)),
                            ("history-clear", "Clear…", Box::new(ClearFileHistory)),
                        ]
                        .map(|(id, label, action)| button(id, label, action)),
                    )
                    .child(
                        kit::icon_button("history-close", Icon::Close, "Close File History (Escape)")
                            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                            .on_click(
                                cx.listener(|ws, _: &ClickEvent, window, cx| ws.close_panel(&ClosePanel, window, cx)),
                            ),
                    ),
            )
            .child(
                div().px(px(8.)).py(px(3.)).text_size(px(11.)).text_color(ui(theme::TEXT_MUTED)).child(p.note.clone()),
            )
            .child(div().flex_1().min_h_0().min_w_0().child(p.viewer.clone()))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{settle_until, temp, text, window};
    use super::*;
    use crate::editing::encoding::Encoding;
    use gpui::{TestAppContext, VisualTestContext};

    fn document<'a>(cx: &'a mut TestAppContext, name: &str) -> (Entity<Workspace>, &'a mut VisualTestContext, PathBuf) {
        let path = temp(name).join("document.txt");
        let (ws, cx) = window(cx);
        let old = crate::editing::encoding::encode("old café\r\n", Encoding::Utf16Le, false).unwrap();
        std::fs::write(&path, old).unwrap();
        file_history::capture_in(&file_history::root(), &path, Encoding::Utf16Le).unwrap();
        std::fs::write(&path, "current\n").unwrap();
        ws.update_in(cx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        (ws, cx, path)
    }

    /// The first version the File History picker offers.
    fn offered(ws: &Workspace) -> Option<PreviewFileHistory> {
        let items = &ws.quick_panel.as_ref()?.items;
        items.iter().find_map(|item| match &item.action {
            ItemAction::Dispatch(action) => action.as_any().downcast_ref().cloned(),
            _ => None,
        })
    }

    fn settled(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.file_history_panel().is_some_and(|p| !p.busy)));
    }

    fn preview(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> PreviewFileHistory {
        ws.update_in(cx, |ws, w, cx| ws.browse_file_history(&BrowseFileHistory, w, cx));
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| offered(ws).is_some()));
        let action = ws.update_in(cx, |ws, w, cx| {
            let action = offered(ws).unwrap();
            ws.preview_file_history(&action, w, cx);
            action
        });
        settled(ws, cx);
        action
    }

    #[gpui::test]
    fn n25_history_restores_text_and_keeps_destination_boundary_kinds(cx: &mut TestAppContext) {
        let path = temp("n25-history-text-only").join("document.txt");
        let (ws, cx) = window(cx);
        std::fs::write(&path, "A\nB\rC\r\nD").unwrap();
        file_history::capture_in(&file_history::root(), &path, Encoding::Utf8).unwrap();
        std::fs::write(&path, "a\r\nb\nc\rd").unwrap();
        ws.update_in(cx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        preview(&ws, cx);
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(ws.file_history_panel().unwrap().viewer.read(cx).text(cx), "A\nB\nC\nD");
            ws.restore_file_history(&RestoreFileHistory, w, cx);
        });
        settle_until(cx, |cx| text(&ws, cx) == "A\nB\nC\nD");
        ws.update(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).doc(cx).bytes_as_saved(None).unwrap(), b"A\r\nB\nC\rD");
            editor.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
            assert_eq!(editor.read(cx).doc(cx).bytes_as_saved(None).unwrap(), b"a\r\nb\nc\rd");
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"a\r\nb\nc\rd");
    }

    #[gpui::test]
    fn legacy_history_preview_restores_text_without_changing_output_codec(cx: &mut TestAppContext) {
        let path = temp("f01-history-preview").join("document.txt");
        let (ws, cx) = window(cx);
        std::fs::write(&path, b"old caf\xe9\r\n").unwrap();
        file_history::capture_in(&file_history::root(), &path, Encoding::Windows1252).unwrap();
        std::fs::write(&path, "current\n").unwrap();
        ws.update_in(cx, |ws, window, cx| ws.open_path(path.clone(), window, cx));
        preview(&ws, cx);
        ws.update_in(cx, |ws, window, cx| {
            let panel = ws.file_history_panel().unwrap();
            assert_eq!(panel.viewer.read(cx).text(cx), "old café\n");
            ws.restore_file_history(&RestoreFileHistory, window, cx);
        });
        settle_until(cx, |cx| text(&ws, cx) == "old café\n");
        ws.update(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).doc(cx).encoding, Encoding::Utf8);
            assert!(editor.read(cx).doc(cx).unsaved());
            editor.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
            assert_eq!(editor.read(cx).text(cx), "current\n");
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"current\n");
    }

    #[gpui::test]
    fn preview_decodes_recorded_encoding_and_restore_is_one_unsaved_edit(cx: &mut TestAppContext) {
        let (ws, cx, path) = document(cx, "orlo-history-restore");
        preview(&ws, cx);
        ws.update_in(cx, |ws, w, cx| {
            let p = ws.file_history_panel().unwrap();
            assert_eq!(p.viewer.read(cx).text(cx), "old café\n");
            assert!(p.viewer.read(cx).doc(cx).read_only);
            assert!(p.note.contains("text") && p.note.contains("UTF-16"));
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).text(cx), "current\n");
            ws.restore_file_history(&RestoreFileHistory, w, cx);
        });
        settle_until(cx, |cx| text(&ws, cx) == "old café\n");
        ws.update(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            assert!(e.read(cx).doc(cx).dirty);
            assert_eq!(e.read(cx).doc(cx).encoding, Encoding::Utf8);
            assert!(!e.read(cx).doc(cx).bom);
            e.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
            assert_eq!(e.read(cx).text(cx), "current\n");
            assert!(!e.read(cx).doc(cx).dirty);
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"current\n");
    }

    #[gpui::test]
    fn restore_and_undo_reach_other_views_of_the_same_buffer(cx: &mut TestAppContext) {
        let (ws, cx, path) = document(cx, "orlo-history-shared");
        let original = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(cx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
        preview(&ws, cx);
        ws.update_in(cx, |ws, w, cx| ws.restore_file_history(&RestoreFileHistory, w, cx));
        settle_until(cx, |cx| original.read_with(cx, |e, cx| e.text(cx) == "old café\n"));
        original.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        settle_until(cx, |cx| text(&ws, cx) == "current\n");
        assert_eq!(std::fs::read(&path).unwrap(), b"current\n");
    }

    #[gpui::test]
    fn restore_refuses_a_target_edited_after_preview(cx: &mut TestAppContext) {
        let (ws, cx, _) = document(cx, "orlo-history-stale-edit");
        preview(&ws, cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("draft ")));
            let before = ws.active_editor().unwrap().read(cx).text(cx);
            ws.restore_file_history(&RestoreFileHistory, w, cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), before);
            assert!(ws.file_history_panel().unwrap().note.contains("changed"));
        });
    }

    #[gpui::test]
    fn old_picker_action_cannot_restore_a_reopened_buffer_with_the_same_version(cx: &mut TestAppContext) {
        let (ws, cx, path) = document(cx, "orlo-history-reopened");
        let action = preview(&ws, cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.close_tab(&CloseTab, w, cx);
            ws.open_path(path, w, cx);
            assert_ne!(ws.active_editor().unwrap().read(cx).doc(cx).id(), action.buffer);
            ws.preview_file_history(&action, w, cx);
            ws.restore_file_history(&RestoreFileHistory, w, cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "current\n");
        });
    }

    #[gpui::test]
    fn restore_refuses_a_snapshot_removed_after_preview(cx: &mut TestAppContext) {
        let (ws, cx, path) = document(cx, "orlo-history-stale-source");
        preview(&ws, cx);
        file_history::clear_in(&file_history::root(), &path).unwrap();
        ws.update_in(cx, |ws, w, cx| ws.restore_file_history(&RestoreFileHistory, w, cx));
        settled(&ws, cx);
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "current\n");
            assert!(!ws.file_history_panel().unwrap().note.is_empty());
        });
    }

    #[gpui::test]
    fn save_as_through_hardlink_retains_known_bomless_history_encoding(cx: &mut TestAppContext) {
        for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
            let dir = temp(encoding.name());
            let path = dir.join("original.txt");
            let alias = dir.join("alias.txt");
            let original = crate::editing::encoding::encode("old café 🦀\r\n", encoding, false).unwrap();
            std::fs::write(&path, &original).unwrap();
            std::fs::hard_link(&path, &alias).unwrap();
            let loaded = crate::editing::text::Loaded::read_encoding(&path, Some(encoding)).unwrap();
            let (ws, cx) = window(cx);
            ws.update_in(cx, |ws, w, cx| {
                cx.global_mut::<Settings>().file_history = true;
                ws.add_tab(EditorState::from_loaded(&path, loaded), w, cx);
                ws.new_file(&NewFile, w, cx);
                ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("replacement\n")));
                ws.save_as(&SaveAs, w, cx);
            });
            cx.simulate_new_path_selection(|_| Some(alias.clone()));
            cx.run_until_parked();
            assert_eq!(std::fs::read(&alias).unwrap(), b"replacement\n");
            let versions = file_history::list_in(&file_history::root(), &alias).unwrap();
            assert_eq!(versions.len(), 1);
            assert_eq!(versions[0].encoding, encoding);
            let record = file_history::read_in(&file_history::root(), &alias, &versions[0].id).unwrap();
            assert_eq!(record.bytes, original);
            let restored = snapshot(&file_history::root(), &alias, &versions[0].id).unwrap();
            assert_eq!(restored.text, "old café 🦀\n");
        }
    }

    #[gpui::test]
    fn clear_requires_confirmation_and_cancel_keeps_history(cx: &mut TestAppContext) {
        let (ws, cx, path) = document(cx, "orlo-history-clear");
        ws.update_in(cx, |ws, w, cx| ws.clear_file_history(&ClearFileHistory, w, cx));
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(file_history::list_in(&file_history::root(), &path).unwrap().len(), 1);
        ws.update_in(cx, |ws, w, cx| ws.clear_file_history(&ClearFileHistory, w, cx));
        cx.simulate_prompt_answer("Clear History");
        settle_until(cx, |_| file_history::list_in(&file_history::root(), &path).unwrap().is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), b"current\n");
    }
}
