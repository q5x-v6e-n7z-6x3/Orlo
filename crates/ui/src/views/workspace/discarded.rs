//! Explicit discarded-untitled rescue; normal close/save/hot exit never calls capture.

use super::pickers::choice;
use super::*;
use session::discarded::Store;

actions!(workspace, [BrowseDiscardedUntitled, ClearDiscardHistory, RestoreRescuePreview, DeleteRescuePreview]);

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct RestoreDiscardedUntitled {
    pub generation: u64,
    pub id: String,
}

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct PreviewDiscardedUntitled {
    pub generation: u64,
    pub id: String,
}

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct DeleteDiscardedUntitled {
    pub generation: u64,
    pub id: String,
}

pub struct RescuePanel {
    pub viewer: Entity<Editor>,
    entry: RestoreDiscardedUntitled,
}

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-enter", RestoreRescuePreview, Some("RescuePanel > Editor")),
        KeyBinding::new("cmd-backspace", DeleteRescuePreview, Some("RescuePanel > Editor")),
        KeyBinding::new("escape", ClosePanel, Some("RescuePanel > Editor")),
    ]
}

impl Workspace {
    pub(crate) fn rescue_then(
        &mut self,
        tabs: Vec<Entity<Editor>>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.rescue_discarded(&tabs, cx);
        let Err(error) = result else {
            return then(self, window, cx);
        };
        let window_tabs: HashSet<_> = self.views.tabs().cloned().collect();
        let versions: Vec<_> = tabs
            .iter()
            .map(|e| {
                let d = e.read(cx).doc(cx);
                (d.id(), d.version, d.path.clone())
            })
            .collect();
        let documents: Vec<_> = editors::one_per_document(tabs.iter().cloned(), cx, |_| true)
            .iter()
            .map(|e| e.read(cx).document.clone())
            .collect();
        for document in &documents {
            document.update(cx, |d, _| {
                d.file_autosave.cancel();
                d.file_autosave.suspensions += 1;
            });
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Could not keep discarded text for rescue",
            Some(&error),
            &["Cancel", "Discard Without Rescue"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            for document in documents {
                document
                    .update(cx, |d, _| d.file_autosave.suspensions = d.file_autosave.suspensions.saturating_sub(1))
                    .ok();
            }
            this.update_in(cx, |ws, window, cx| {
                if answer != Ok(1) {
                    return;
                }
                // A continuation may close every current tab. A new or moved view while
                // this additional loss prompt was open needs a fresh discard decision.
                if ws.views.tabs().cloned().collect::<HashSet<_>>() != window_tabs
                    || tabs.iter().zip(versions).any(|(e, before)| {
                        let d = e.read(cx).doc(cx);
                        ws.views.find(e).is_none() || (d.id(), d.version, d.path.clone()) != before
                    })
                {
                    return ws.status_message("The documents changed; close them again to discard", cx);
                }
                then(ws, window, cx);
            })
            .ok();
            cx.update(|_, cx| cx.defer(file_autosave::refresh_all)).ok();
        })
        .detach();
    }

    fn rescue_discarded(&self, tabs: &[Entity<Editor>], cx: &App) -> Result<(), String> {
        let mut texts = Vec::new();
        let mut seen = HashSet::new();
        for editor in tabs {
            if self.views.find(editor).is_none() {
                continue;
            }
            let e = editor.read(cx);
            let d = e.doc(cx);
            if d.path.is_some()
                || !d.dirty
                || d.rope.len_bytes() == 0
                || d.scratch
                || e.results
                || e.label.is_some()
                || self.views.is_preview(editor)
                || !seen.insert(d.id())
            {
                continue;
            }
            let closing =
                tabs.iter().filter(|t| self.views.find(t).is_some() && t.read(cx).document == e.document).count();
            if closing != e.document.read(cx).tabs {
                continue; // Another window/group keeps this document alive.
            }
            if self.open_editors(cx).iter().any(|other| {
                let other = other.read(cx);
                other.document == e.document && !other.marked.is_empty()
            }) {
                return Err("Finish or cancel text composition before discarding to keep a rescue copy".into());
            }
            if d.rope.len_bytes() > session::discarded::MAX_TEXT_BYTES {
                return Err("Rescue keeps drafts up to 1 MiB; this draft is larger".into());
            }
            if texts.len() == session::discarded::MAX_ENTRIES {
                texts.remove(0);
            }
            texts.push(d.text());
        }
        if texts.is_empty() {
            return Ok(());
        }
        // Complete before removing tabs: last-window close can immediately terminate Orlo.
        // At most fifty 1 MiB payloads; listing/pruning never reads older payloads.
        let store = Store::current();
        store.capture(store.generation(), texts).map_err(|error| error.to_string())
    }

    pub(crate) fn browse_discarded_untitled(
        &mut self,
        _: &BrowseDiscardedUntitled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_panel(Kind::Choices, Vec::new(), window, cx);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some("Rescue · Discarded Untitled".into());
        panel.progress = Some("Loading rescue drafts…".into());
        let input = panel.input.clone();
        let store = Store::current();
        let reading = cx.background_spawn(async move {
            let catalog = store.catalog()?;
            let generation = catalog.publication.generation;
            let items = catalog
                .entries
                .into_iter()
                .map(|(entry, summary)| {
                    let excerpt = summary.unwrap_or_else(|error| format!("[Preview unavailable: {error}]"));
                    choice(
                        &format!(
                            "Untitled · {} · {} bytes · {excerpt}",
                            super::history::captured_at(entry.created.saturating_mul(1000)),
                            entry.bytes
                        ),
                        &format!("{} bytes · Select to preview, restore or permanently delete", entry.bytes),
                        ItemAction::Dispatch(Box::new(PreviewDiscardedUntitled { generation, id: entry.id })),
                    )
                })
                .collect();
            Ok::<_, std::io::Error>((catalog.publication, items))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = reading.await;
            #[cfg(test)]
            crate::testing::gate::wait("rescue-catalog", &session::support_dir()).await;
            this.update_in(cx, |ws, _, cx| {
                if ws.quick_panel.as_ref().is_none_or(|p| p.input != input) {
                    return;
                }
                match result {
                    Ok((publication, items)) if publication.is_current() => {
                        let panel = ws.quick_panel.as_mut().unwrap();
                        panel.set_items(Kind::Choices, items);
                        panel.refilter(&panel.query.clone());
                        panel.progress = Some(
                            if panel.items.is_empty() {
                                "No discarded untitled drafts to restore"
                            } else {
                                "Search the first 1 KiB of each draft, capture time or size; select for preview"
                            }
                            .into(),
                        );
                        cx.notify();
                    }
                    Ok(_) => {
                        ws.rescue_error("Could not read discarded drafts: Rescue changed; browse again".into(), cx)
                    }
                    Err(error) => ws.rescue_error(format!("Could not read discarded drafts: {error}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn preview_discarded_untitled(
        &mut self,
        action: &PreviewDiscardedUntitled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.read_rescue(
            RestoreDiscardedUntitled { generation: action.generation, id: action.id.clone() },
            true,
            window,
            cx,
        );
    }

    pub(crate) fn restore_discarded_untitled(
        &mut self,
        action: &RestoreDiscardedUntitled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.read_rescue(action.clone(), false, window, cx);
    }

    fn read_rescue(
        &mut self,
        action: RestoreDiscardedUntitled,
        preview: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_panel(Kind::Choices, Vec::new(), window, cx);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some("Rescue · Discarded Untitled".into());
        panel.progress = Some(if preview { "Loading preview…" } else { "Restoring draft…" }.into());
        let input = panel.input.clone();
        let store = Store::current();
        let selected = action.clone();
        let reading = cx.background_spawn(async move { store.load(selected.generation, &selected.id) });
        cx.spawn_in(window, async move |this, cx| {
            let result = reading.await;
            #[cfg(test)]
            crate::testing::gate::wait("rescue-read", &session::support_dir()).await;
            this.update_in(cx, |ws, window, cx| {
                if ws.quick_panel.as_ref().is_none_or(|p| p.input != input) {
                    return;
                }
                let loaded = match result {
                    Ok(loaded) if loaded.publication.is_current() => loaded,
                    Ok(_) => {
                        return ws.rescue_error(
                            "Could not restore or preview discarded draft: Rescue changed; browse again".into(),
                            cx,
                        )
                    }
                    Err(error) => {
                        return ws.rescue_error(format!("Could not restore or preview discarded draft: {error}"), cx)
                    }
                };
                match loaded.text {
                    Ok(text) if preview => {
                        ws.cancel_quick_panel(cx);
                        ws.show_rescue_preview(&text, action, window, cx);
                    }
                    Ok(text) => {
                        ws.cancel_quick_panel(cx);
                        let mut state = EditorState::from_source(&text);
                        state.forget_saved_point();
                        if matches!(ws.panel, Some(Panel::Rescue(_))) {
                            ws.close_panel(&ClosePanel, window, cx);
                        }
                        ws.drop_blank_untitled(cx);
                        ws.add_tab(state, window, cx);
                        autosave(cx);
                    }
                    Err(error) => {
                        let message = format!("Could not restore or preview discarded draft: {error}");
                        if preview {
                            ws.cancel_quick_panel(cx);
                            ws.show_rescue_preview(&message, action, window, cx);
                            ws.status_message(message, cx);
                        } else {
                            ws.rescue_error(message, cx);
                        }
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Keep the loading input alive/focused, with an error visible even without a status bar.
    fn rescue_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(panel) = &mut self.quick_panel {
            panel.set_items(
                Kind::Choices,
                vec![
                    choice(
                        "Browse Rescue Again",
                        "Reload available drafts",
                        ItemAction::Dispatch(Box::new(BrowseDiscardedUntitled)),
                    ),
                    choice(
                        "Return to Document",
                        "Escape also returns to the document",
                        ItemAction::Dispatch(Box::new(quick_panel::Dismiss)),
                    ),
                ],
            );
            let query = panel.input.read(cx).text(cx);
            panel.refilter(&query);
            panel.progress = Some(message.clone());
        }
        self.status_message(message, cx);
        cx.notify();
    }

    fn show_rescue_preview(
        &mut self,
        text: &str,
        entry: RestoreDiscardedUntitled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let viewer = cx.new(|cx| {
            let mut state = EditorState::from_source(text);
            state.doc.read_only = true;
            state.doc.scratch = true;
            let mut viewer = Editor::new(state, false, cx);
            viewer.label = Some("Rescue Preview".into());
            viewer.view.minimap = false;
            viewer
        });
        self.show_panel(Panel::Rescue(Box::new(RescuePanel { viewer: viewer.clone(), entry })), cx);
        viewer.read(cx).focus(window);
    }

    pub(crate) fn restore_rescue_preview(&mut self, _: &RestoreRescuePreview, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(Panel::Rescue(panel)) = &self.panel {
            self.restore_discarded_untitled(&panel.entry.clone(), w, cx);
        }
    }

    pub(crate) fn delete_rescue_preview(&mut self, _: &DeleteRescuePreview, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(Panel::Rescue(panel)) = &self.panel {
            let entry = &panel.entry;
            self.delete_discarded_untitled(
                &DeleteDiscardedUntitled { generation: entry.generation, id: entry.id.clone() },
                w,
                cx,
            );
        }
    }

    pub(crate) fn delete_discarded_untitled(
        &mut self,
        action: &DeleteDiscardedUntitled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = action.clone();
        let store = Store::current();
        let answer = window.prompt(
            PromptLevel::Warning,
            "Permanently delete this rescue draft?",
            Some("This rescue copy will be removed. Open documents and other rescue drafts are kept."),
            &["Cancel", "Delete Permanently"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(1) {
                return;
            }
            let selected = action.clone();
            let result =
                cx.background_executor().spawn(async move { store.delete(selected.generation, &selected.id) }).await;
            this.update_in(cx, |ws, w, cx| match result {
                Ok(()) => {
                    if matches!(&ws.panel, Some(Panel::Rescue(p)) if p.entry.id == action.id) {
                        ws.close_panel(&ClosePanel, w, cx);
                    }
                    ws.status_message("Rescue draft permanently deleted", cx);
                }
                Err(error) => ws.status_message(format!("Could not delete rescue draft: {error}"), cx),
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn render_rescue(&self, _cx: &mut Context<Self>) -> AnyElement {
        let Some(Panel::Rescue(panel)) = &self.panel else {
            return div().into_any_element();
        };
        let button = |id: &'static str, label: &'static str, symbol: Option<Icon>, action: Box<dyn Action>| {
            let control = match symbol {
                Some(symbol) => kit::icon_button(id, symbol, label),
                None => kit::ghost_button(id, "Clear…").tooltip(kit::tooltip(label.into())),
            };
            control
                .debug_selector(move || id.into())
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(move |_: &ClickEvent, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        };
        div()
            .debug_selector(|| "rescue-preview-root".into())
            .key_context("RescuePanel")
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(ui(theme::PANEL_BG))
            .child(
                kit::panel_header()
                    .debug_selector(|| "rescue-preview-header".into())
                    .text_size(px(11.))
                    .child(
                        div()
                            .id("rescue-preview-title")
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .child("Rescue")
                            .tooltip(kit::tooltip("Rescue · Read-only Preview".into())),
                    )
                    .children(
                        [
                            (
                                "rescue-browse",
                                "Browse Rescue History",
                                Some(Icon::FolderOpen),
                                BrowseDiscardedUntitled.boxed_clone(),
                            ),
                            (
                                "rescue-restore",
                                "Restore as New Untitled · Cmd-Enter",
                                Some(Icon::FilePlus),
                                RestoreRescuePreview.boxed_clone(),
                            ),
                            (
                                "rescue-delete",
                                "Permanently Delete · Cmd-Backspace",
                                Some(Icon::Trash),
                                DeleteRescuePreview.boxed_clone(),
                            ),
                            ("rescue-clear", "Clear Rescue History", None, ClearDiscardHistory.boxed_clone()),
                            ("rescue-close", "Close Preview · Escape", Some(Icon::Close), ClosePanel.boxed_clone()),
                        ]
                        .map(|(id, label, symbol, action)| button(id, label, symbol, action)),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "rescue-preview-keys".into())
                    .px(px(8.))
                    .py(px(3.))
                    .text_size(px(11.))
                    .text_color(ui(theme::TEXT_MUTED))
                    .child("Read-only · ⌘↩ Restore · ⌘⌫ Delete · Esc Close"),
            )
            .child(
                div()
                    .debug_selector(|| "rescue-preview-body".into())
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(panel.viewer.clone()),
            )
            .into_any_element()
    }

    pub(crate) fn clear_discard_history(
        &mut self,
        _: &ClearDiscardHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = Store::current();
        let picker = self
            .quick_panel
            .as_ref()
            .filter(|p| p.title.as_deref() == Some("Rescue · Discarded Untitled"))
            .map(|p| p.input.clone());
        let viewer = match &self.panel {
            Some(Panel::Rescue(p)) => Some(p.viewer.clone()),
            _ => None,
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            "Clear discarded untitled history?",
            Some("These rescue copies will be deleted. Open documents, session recovery and file history are kept."),
            &["Cancel", "Clear History"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(1) {
                return;
            }
            let result = cx.background_executor().spawn(async move { store.clear() }).await;
            #[cfg(test)]
            crate::testing::gate::wait("rescue-clear", &session::support_dir()).await;
            this.update_in(cx, |ws, window, cx| {
                if ws.quick_panel.as_ref().is_some_and(|p| picker.as_ref() == Some(&p.input)) {
                    ws.quick_panel_dismiss(&quick_panel::Dismiss, window, cx);
                }
                match result {
                    Ok(()) => {
                        if matches!(&ws.panel, Some(Panel::Rescue(p)) if viewer.as_ref() == Some(&p.viewer)) {
                            let replacement = ws.quick_panel.as_ref().map(|p| p.input.clone());
                            ws.close_panel(&ClosePanel, window, cx);
                            if let Some(input) = replacement {
                                input.read(cx).focus(window);
                            }
                        }
                        ws.status_message("Discard history cleared", cx);
                    }
                    Err(error) => ws.status_message(format!("Could not clear discard history: {error}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{temp, two_windows, type_in, window};
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    #[gpui::test]
    fn n25_discarded_untitled_restores_text_as_new_document_with_default_format(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            let (rope, preferred, map) = crate::editing::text::decoded_file("a\r\nb\nc\rd");
            let mut state = EditorState::from_text(&rope.to_string());
            state.doc.endings = Some(map);
            state.doc.line_ending = preferred;
            state.forget_saved_point();
            ws.add_tab(state, w, cx);
        });
        close(&ws, cx, "Don't Save");
        let store = Store::current();
        let (generation, all) = store.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(store.restore(generation, &all[0].id).unwrap(), "a\nb\nc\nd");
        ws.update_in(cx, |ws, w, cx| {
            ws.restore_discarded_untitled(&RestoreDiscardedUntitled { generation, id: all[0].id.clone() }, w, cx)
        });
        super::super::ui_tests::settle_until(cx, |cx| {
            ws.read_with(cx, |ws, cx| ws.active_editor().is_some_and(|e| e.read(cx).doc(cx).dirty))
        });
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            let d = e.read(cx).doc(cx);
            assert_eq!(d.text(), "a\nb\nc\nd");
            assert!(d.path.is_none());
            assert_eq!(d.endings.as_ref().unwrap().compact(), "lll");
            assert_eq!(d.bytes_as_saved(None).unwrap(), b"a\nb\nc\nd");
        });
    }

    fn entries() -> Vec<session::discarded::Entry> {
        Store::current().list().unwrap().1
    }

    fn close(ws: &Entity<Workspace>, cx: &mut VisualTestContext, answer: &str) {
        ws.update_in(cx, |ws, w, cx| ws.close_tab(&CloseTab, w, cx));
        cx.simulate_prompt_answer(answer);
        cx.run_until_parked();
    }

    fn offered(ws: &Workspace) -> RestoreDiscardedUntitled {
        ws.quick_panel
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find_map(|item| match &item.action {
                ItemAction::Dispatch(action) => action
                    .as_any()
                    .downcast_ref::<PreviewDiscardedUntitled>()
                    .map(|a| RestoreDiscardedUntitled { generation: a.generation, id: a.id.clone() }),
                _ => None,
            })
            .unwrap()
    }

    #[gpui::test]
    fn discarded_admission_does_not_wait_for_storage_critical_section(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let store = Store::current();
        store.capture(0, vec!["ready preview 界".into()]).unwrap();
        let id = store.list().unwrap().1[0].id.clone();
        for catalog in [false, true] {
            let gate = crate::testing::gate::pause(
                if catalog { "rescue-catalog" } else { "rescue-read" },
                &session::support_dir(),
            );
            ws.update_in(cx, |ws, w, cx| {
                if catalog {
                    ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx);
                } else {
                    ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation: 0, id: id.clone() }, w, cx);
                }
            });
            cx.run_until_parked();
            assert_eq!(gate.arrived(), 1);
            let (held_tx, held_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let timed = timed_out.clone();
            let store = store.clone();
            let blocked = std::thread::spawn(move || {
                store.with_storage_lock_for_test(|| {
                    held_tx.send(()).unwrap();
                    if release_rx.recv_timeout(std::time::Duration::from_secs(2)).is_err() {
                        timed.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                })
            });
            held_rx.recv().unwrap();
            gate.release();
            cx.run_until_parked();
            // Foreground can paint and dispatch while a worker owns the actual storage mutex.
            ws.update_in(cx, |ws, w, cx| {
                ws.command_palette(&CommandPalette, w, cx);
            });
            cx.simulate_keystrokes("x");
            release_tx.send(()).ok();
            blocked.join().unwrap();
            assert!(
                !timed_out.load(std::sync::atomic::Ordering::SeqCst),
                "foreground admission waited for filesystem critical section"
            );
            ws.read_with(cx, |ws, cx| assert_eq!(ws.quick_panel.as_ref().unwrap().input.read(cx).text(cx), "x"));
        }
    }

    #[gpui::test]
    fn discarded_already_loaded_results_refuse_delete_clear_and_capture_before_admission(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        for operation in 0..3 {
            let store = Store::current();
            let generation = store.generation();
            store.capture(generation, vec!["loaded but not admitted 界".into()]).unwrap();
            let id = store.list().unwrap().1[0].id.clone();
            let gate = crate::testing::gate::pause("rescue-read", &session::support_dir());
            ws.update_in(cx, |ws, w, cx| {
                ws.restore_discarded_untitled(&RestoreDiscardedUntitled { generation, id: id.clone() }, w, cx)
            });
            cx.run_until_parked();
            assert_eq!(gate.arrived(), 1);
            match operation {
                0 => store.delete(generation, &id).unwrap(),
                1 => store.clear().unwrap(),
                _ => store.capture(generation, vec!["new capture".into()]).unwrap(),
            }
            gate.release();
            cx.run_until_parked();
            ws.update_in(cx, |ws, w, cx| {
                assert!(!ws.active_editor().unwrap().read(cx).doc(cx).dirty);
                let panel = ws.quick_panel.as_ref().unwrap();
                assert!(panel.progress.as_ref().unwrap().contains("Rescue changed"));
                assert!(panel.input.read(cx).is_focused(w));
            });
        }
    }

    #[gpui::test]
    fn discarded_stale_preview_error_returns_keyboard_to_live_document(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "working");
        ws.update(cx, |ws, _| ws.show_status_bar = false);
        for clear in [false, true] {
            let store = Store::current();
            let generation = store.generation();
            store.capture(generation, vec!["private stale preview".into()]).unwrap();
            let id = store.list().unwrap().1[0].id.clone();
            ws.update_in(cx, |ws, w, cx| {
                ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation, id: id.clone() }, w, cx)
            });
            cx.run_until_parked();
            if clear {
                store.clear().unwrap();
            } else {
                store.delete(generation, &id).unwrap();
            }
            cx.dispatch_action(RestoreRescuePreview);
            cx.run_until_parked();
            ws.update_in(cx, |ws, w, cx| {
                let panel = ws.quick_panel.as_ref().unwrap();
                assert!(panel.progress.as_ref().unwrap().starts_with("Could not restore"));
                assert!(panel.input.read(cx).is_focused(w));
            });
            cx.simulate_keystrokes("escape");
            ws.update_in(cx, |ws, w, cx| assert!(ws.active_editor().unwrap().read(cx).is_focused(w)));
            cx.simulate_keystrokes("y");
        }
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "workingyy"));
    }

    #[gpui::test]
    fn discarded_clear_matching_preview_keeps_replacement_picker_keyboard_focus(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let store = Store::current();
        store.capture(0, vec!["clear this preview".into()]).unwrap();
        let id = store.list().unwrap().1[0].id.clone();
        ws.update_in(cx, |ws, w, cx| {
            ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation: 0, id }, w, cx)
        });
        cx.run_until_parked();
        let gate = crate::testing::gate::pause("rescue-clear", &session::support_dir());
        ws.update_in(cx, |ws, w, cx| ws.clear_discard_history(&ClearDiscardHistory, w, cx));
        cx.simulate_prompt_answer("Clear History");
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| ws.command_palette(&CommandPalette, w, cx));
        gate.release();
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert!(ws.panel.is_none(), "the matching old preview is closed");
            assert!(
                ws.quick_panel.as_ref().unwrap().input.read(cx).is_focused(w),
                "Clear must preserve replacement focus"
            );
        });
        cx.simulate_keystrokes("x");
        ws.read_with(cx, |ws, cx| assert_eq!(ws.quick_panel.as_ref().unwrap().input.read(cx).text(cx), "x"));
    }

    #[gpui::test]
    fn discarded_catalog_admission_refuses_removed_or_replaced_snapshot(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        for clear in [false, true] {
            let store = Store::current();
            let generation = store.generation();
            store.capture(generation, vec!["catalog not yet shown".into()]).unwrap();
            let id = store.list().unwrap().1[0].id.clone();
            let gate = crate::testing::gate::pause("rescue-catalog", &session::support_dir());
            ws.update_in(cx, |ws, w, cx| ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx));
            cx.run_until_parked();
            assert_eq!(gate.arrived(), 1);
            if clear {
                store.clear().unwrap();
            } else {
                store.delete(generation, &id).unwrap();
            }
            gate.release();
            cx.run_until_parked();
            ws.update_in(cx, |ws, w, cx| {
                let panel = ws.quick_panel.as_ref().unwrap();
                assert!(panel.progress.as_ref().unwrap().contains("Rescue changed"));
                assert!(panel.input.read(cx).is_focused(w));
                assert!(!panel.keys.iter().any(|s| s.contains("catalog not yet shown")));
            });
        }
    }

    #[gpui::test]
    fn discarded_clear_completion_preserves_replacement_pickers_and_new_preview(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        for commands in [true, false] {
            let gate = crate::testing::gate::pause("rescue-clear", &session::support_dir());
            ws.update_in(cx, |ws, w, cx| ws.clear_discard_history(&ClearDiscardHistory, w, cx));
            cx.simulate_prompt_answer("Clear History");
            cx.run_until_parked();
            assert_eq!(gate.arrived(), 1);
            let input = ws.update_in(cx, |ws, w, cx| {
                if commands {
                    ws.command_palette(&CommandPalette, w, cx);
                } else {
                    ws.open_quick_panel(
                        Kind::Choices,
                        vec![choice("Replacement", "", ItemAction::Dispatch(Box::new(ClosePanel)))],
                        w,
                        cx,
                    );
                }
                ws.quick_panel.as_ref().unwrap().input.clone()
            });
            gate.release();
            cx.run_until_parked();
            ws.read_with(cx, |ws, _| {
                assert!(
                    ws.quick_panel.as_ref().is_some_and(|p| p.input == input),
                    "old Clear must keep replacement picker"
                )
            });
        }
        let gate = crate::testing::gate::pause("rescue-clear", &session::support_dir());
        ws.update_in(cx, |ws, w, cx| ws.clear_discard_history(&ClearDiscardHistory, w, cx));
        cx.simulate_prompt_answer("Clear History");
        cx.run_until_parked();
        let store = Store::current();
        let generation = store.generation();
        store.capture(generation, vec!["new generation preview".into()]).unwrap();
        let id = store.list().unwrap().1[0].id.clone();
        ws.update_in(cx, |ws, w, cx| {
            ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation, id }, w, cx)
        });
        cx.run_until_parked();
        let viewer = ws.read_with(cx, |ws, _| match &ws.panel {
            Some(Panel::Rescue(p)) => p.viewer.clone(),
            _ => panic!("new preview"),
        });
        gate.release();
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(
                matches!(&ws.panel, Some(Panel::Rescue(p)) if p.viewer == viewer),
                "old Clear must keep new-generation preview"
            )
        });
    }

    #[gpui::test]
    fn prefinal_rescue_read_error_retains_typed_query_and_coherent_filter(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        std::fs::write(session::support_dir().join("DiscardedUntitled"), "unsafe directory").unwrap();
        let gate = crate::testing::gate::pause("rescue-catalog", &session::support_dir());
        ws.update_in(cx, |ws, w, cx| {
            ws.show_status_bar = false;
            ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx);
        });
        cx.run_until_parked();
        assert_eq!(gate.arrived(), 1);
        cx.simulate_input("needle界");
        gate.release();
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let panel = ws.quick_panel.as_ref().unwrap();
            assert_eq!(panel.input.read(cx).text(cx), "needle界");
            assert_eq!(panel.query, "needle界");
            assert!(panel.filtered.is_empty());
            assert!(panel.progress.as_ref().unwrap().starts_with("Could not"));
            assert!(panel.input.read(cx).is_focused(w));
        });
        cx.simulate_keystrokes("cmd-a backspace");
        ws.read_with(cx, |ws, cx| {
            let panel = ws.quick_panel.as_ref().unwrap();
            assert!(panel.input.read(cx).text(cx).is_empty());
            assert!(panel.query.is_empty());
            assert_eq!(panel.filtered.len(), 2, "clearing the query reveals retry/return");
        });
        cx.simulate_keystrokes("escape");
        ws.read_with(cx, |ws, _| assert!(ws.quick_panel.is_none()));
        assert!(super::super::ui_tests::text(&ws, cx).is_empty());
    }

    #[gpui::test]
    fn discarded_terminal_read_failure_keeps_visible_focused_error_and_keys(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| {
            ws.show_status_bar = false;
            ws.restore_discarded_untitled(&RestoreDiscardedUntitled { generation: 0, id: "missing".into() }, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let p = ws.quick_panel.as_ref().expect("visible terminal rescue error");
            assert!(p.progress.as_ref().unwrap().starts_with("Could not"));
            assert!(p.input.read(cx).is_focused(w));
        });
        cx.simulate_keystrokes("x");
        ws.read_with(cx, |ws, cx| assert_eq!(ws.quick_panel.as_ref().unwrap().input.read(cx).text(cx), "x"));
    }

    #[gpui::test]
    fn discarded_catalog_private_failure_keeps_visible_focused_error_and_keys(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        std::fs::write(session::support_dir().join("DiscardedUntitled"), "unsafe directory").unwrap();
        ws.update_in(cx, |ws, w, cx| {
            ws.show_status_bar = false;
            ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let p = ws.quick_panel.as_ref().expect("visible terminal catalog error");
            assert!(p.progress.as_ref().unwrap().starts_with("Could not"));
            assert!(p.input.read(cx).is_focused(w));
        });
        cx.simulate_keystrokes("x");
        ws.read_with(cx, |ws, cx| assert_eq!(ws.quick_panel.as_ref().unwrap().input.read(cx).text(cx), "x"));
    }

    #[gpui::test]
    fn discarded_preview_controls_fit_and_work_in_narrow_docks(cx: &mut TestAppContext) {
        use crate::app::session::Dock;
        let (ws, cx) = window(cx);
        let text = "narrow rescue 界 🦀\nsecond line\n";
        type_in(&ws, cx, text);
        close(&ws, cx, "Don't Save");
        let (generation, records) = Store::current().list().unwrap();
        let action = PreviewDiscardedUntitled { generation, id: records[0].id.clone() };
        ws.update_in(cx, |ws, w, cx| {
            ws.new_file(&NewFile, w, cx);
            ws.preview_discarded_untitled(&action, w, cx);
        });
        cx.run_until_parked();
        for (dock, width, height) in [(Dock::Right, 220., 240.), (Dock::Right, 440., 240.), (Dock::Bottom, 440., 80.)] {
            cx.simulate_resize(gpui::size(px(if dock == Dock::Right { 1000. } else { 440. }), px(600.)));
            ws.update_in(cx, |ws, w, cx| {
                ws.sidebar.visible = false;
                ws.move_panel(&MovePanel { dock }, w, cx);
                ws.panel_places.changes.width = width;
                ws.panel_places.changes.height = height;
                cx.notify();
            });
            cx.run_until_parked();
            let root = cx.debug_bounds("rescue-preview-root").expect("preview is drawn in its dock");
            let header = cx.debug_bounds("rescue-preview-header").unwrap();
            let mut controls = Vec::new();
            for id in ["rescue-browse", "rescue-restore", "rescue-delete", "rescue-clear", "rescue-close"] {
                let bounds = cx.debug_bounds(id).unwrap_or_else(|| panic!("missing {id}"));
                assert!(
                    bounds.left() >= root.left() && bounds.right() <= root.right(),
                    "{dock:?}/{width}: {id} outside {root:?}: {bounds:?}"
                );
                assert!(bounds.top() >= header.top() && bounds.bottom() <= header.bottom(), "{id} outside header");
                assert!(bounds.size.width >= px(22.) && bounds.size.height >= px(22.), "{id} has no usable hit target");
                for old in &controls {
                    let old: &gpui::Bounds<gpui::Pixels> = old;
                    assert!(
                        bounds.right() <= old.left()
                            || bounds.left() >= old.right()
                            || bounds.bottom() <= old.top()
                            || bounds.top() >= old.bottom(),
                        "controls overlap: {bounds:?}/{old:?}"
                    );
                }
                controls.push(bounds);
            }
            let body = cx.debug_bounds("rescue-preview-body").unwrap();
            assert!(body.size.height >= px(20.), "{dock:?}: preview lost its text viewport: {body:?}");
            assert!(body.bottom() <= root.bottom(), "preview escapes the dock");
        }
        for (id, answer) in [("rescue-delete", "Cancel"), ("rescue-clear", "Cancel")] {
            let button = cx.debug_bounds(id).unwrap();
            cx.simulate_click(button.center(), gpui::Modifiers::none());
            cx.simulate_prompt_answer(answer);
            cx.run_until_parked();
            ws.update_in(cx, |ws, w, cx| {
                let Some(Panel::Rescue(p)) = &ws.panel else {
                    panic!("Cancel closed preview");
                };
                assert!(p.viewer.read(cx).is_focused(w), "button stole preview focus");
                assert_eq!(p.viewer.read(cx).text(cx), text);
            });
        }
        let restore = cx.debug_bounds("rescue-restore").unwrap();
        cx.simulate_click(restore.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), text));
        ws.update_in(cx, |ws, w, cx| ws.preview_discarded_untitled(&action, w, cx));
        cx.run_until_parked();
        let browse = cx.debug_bounds("rescue-browse").unwrap();
        cx.simulate_click(browse.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.quick_panel.is_some()), "Browse hit missed the picker");
        cx.simulate_keystrokes("escape");
        ws.update_in(cx, |ws, w, cx| ws.preview_discarded_untitled(&action, w, cx));
        cx.run_until_parked();
        let close = cx.debug_bounds("rescue-close").unwrap();
        cx.simulate_click(close.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            assert!(ws.panel.is_none(), "Close hit missed the preview");
            assert!(ws.active_editor().unwrap().read(cx).is_focused(w));
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), text);
        });
    }

    #[gpui::test]
    fn discarded_real_close_fresh_store_search_preview_restore_and_permanent_delete(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let text = "rescue needle 救助 🦀 e\u{301}\n";
        type_in(&ws, cx, text);
        let original = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        close(&ws, cx, "Don't Save");
        let fresh = Store::new(session::support_dir().join("DiscardedUntitled"));
        let (generation, records) = fresh.list().unwrap();
        assert_eq!(records.len(), 1);
        let action = RestoreDiscardedUntitled { generation, id: records[0].id.clone() };
        ws.update_in(cx, |ws, w, cx| ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx));
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| {
            let panel = ws.quick_panel.as_mut().unwrap();
            panel.refilter("needle 救助");
            assert_eq!(panel.filtered.len(), 1);
            let offered = offered(ws);
            assert_eq!(offered, action);
            ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation, id: offered.id }, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, _, cx| {
            let Some(Panel::Rescue(panel)) = &ws.panel else {
                panic!("preview missing");
            };
            assert_eq!(panel.viewer.read(cx).text(cx), text);
            let d = panel.viewer.read(cx).doc(cx);
            assert!(d.read_only && d.scratch);
            assert!(ws.views.tabs().next().is_none(), "preview creates no working tab");
            assert!(ws.render_panel(cx).is_some(), "actual dock rendering is reachable");
        });
        cx.dispatch_action(RestoreRescuePreview);
        cx.run_until_parked();
        let restored = ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            let d = editor.read(cx).doc(cx);
            assert_eq!(d.text(), text);
            assert!(d.dirty && !d.read_only && !d.scratch && d.path.is_none());
            assert_ne!(editor.read(cx).document, original.read(cx).document);
            editor
        });
        restored.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.insert("x");
                s.undo();
                assert_eq!(s.text(), text);
                assert!(s.unsaved());
            })
        });
        assert_eq!(original.read_with(cx, |e, cx| e.text(cx)), text);
        let deleting = DeleteDiscardedUntitled { generation, id: action.id.clone() };
        ws.update_in(cx, |ws, w, cx| ws.delete_discarded_untitled(&deleting, w, cx));
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(fresh.restore(generation, &action.id).unwrap(), text);
        ws.update_in(cx, |ws, w, cx| ws.delete_discarded_untitled(&deleting, w, cx));
        cx.simulate_prompt_answer("Delete Permanently");
        cx.run_until_parked();
        assert!(!session::support_dir().join("DiscardedUntitled").join(&action.id).exists());
        assert!(fresh.restore(generation, &action.id).is_err());
        ws.update_in(cx, |ws, w, cx| ws.restore_discarded_untitled(&action, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 1);
            assert_eq!(restored.read(cx).text(cx), text);
            assert!(ws.status.as_ref().unwrap().0.starts_with("Could not restore"));
        });
    }

    #[gpui::test]
    fn discarded_pending_preview_restore_delete_clear_and_replacement_refuse_stale_text(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "queued Unicode 界");
        close(&ws, cx, "Don't Save");
        let store = Store::current();
        let (generation, records) = store.list().unwrap();
        let id = records[0].id.clone();
        let preview = PreviewDiscardedUntitled { generation, id: id.clone() };
        ws.update_in(cx, |ws, w, cx| {
            ws.preview_discarded_untitled(&preview, w, cx);
            ws.open_quick_panel(
                Kind::Choices,
                vec![choice("Replacement", "", ItemAction::Dispatch(Box::new(ClosePanel)))],
                w,
                cx,
            );
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(ws.panel.is_none());
            assert_eq!(ws.quick_panel.as_ref().unwrap().items[0].label, "Replacement");
        });
        ws.update_in(cx, |ws, w, cx| ws.preview_discarded_untitled(&preview, w, cx));
        store.delete(generation, &id).unwrap();
        cx.run_until_parked();
        assert!(ws.read_with(cx, |ws, _| ws.panel.is_none()));
        store.capture(generation, vec!["another queued draft".into()]).unwrap();
        let id = store.list().unwrap().1[0].id.clone();
        let restore = RestoreDiscardedUntitled { generation, id };
        ws.update_in(cx, |ws, w, cx| ws.restore_discarded_untitled(&restore, w, cx));
        store.clear().unwrap();
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |ws, _| ws.views.tabs().count()), 0);
        ws.update_in(cx, |ws, w, cx| ws.restore_discarded_untitled(&restore, w, cx));
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |ws, _| ws.views.tabs().count()), 0);
    }

    #[gpui::test]
    fn discarded_corrupt_payload_is_local_and_still_permanently_deletable(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "valid sibling 界");
        close(&ws, cx, "Don't Save");
        let store = Store::current();
        let (generation, records) = store.list().unwrap();
        let good = records[0].id.clone();
        store.capture(generation, vec!["damaged".into()]).unwrap();
        let bad = store.list().unwrap().1[0].id.clone();
        std::fs::write(session::support_dir().join("DiscardedUntitled").join(&bad), [0xff, 0xfe]).unwrap();
        ws.update_in(cx, |ws, w, cx| ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let panel = ws.quick_panel.as_ref().unwrap();
            assert_eq!(panel.items.len(), 2);
            assert!(panel.keys.iter().any(|s| s.contains("valid sibling 界")));
            assert!(panel.keys.iter().any(|s| s.contains("Preview unavailable")));
        });
        ws.update_in(cx, |ws, w, cx| {
            ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation, id: bad.clone() }, w, cx)
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            let Some(Panel::Rescue(panel)) = &ws.panel else {
                panic!("corrupt entry must remain deletable");
            };
            assert!(panel.viewer.read(cx).text(cx).starts_with("Could not restore"));
        });
        cx.dispatch_action(DeleteRescuePreview);
        cx.run_until_parked();
        cx.simulate_prompt_answer("Delete Permanently");
        cx.run_until_parked();
        assert!(store.restore(generation, &bad).is_err());
        assert_eq!(store.restore(generation, &good).unwrap(), "valid sibling 界");
    }

    #[gpui::test]
    fn discarded_cross_window_preview_cannot_restore_after_delete_or_clear(cx: &mut TestAppContext) {
        let (a, b, mut acx, mut bcx) = two_windows(cx);
        type_in(&a, &mut acx, "cross-window 救助");
        close(&a, &mut acx, "Don't Save");
        let store = Store::current();
        let (generation, records) = store.list().unwrap();
        let id = records[0].id.clone();
        b.update_in(&mut bcx, |ws, w, cx| {
            ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation, id: id.clone() }, w, cx)
        });
        bcx.run_until_parked();
        assert!(b.read_with(&bcx, |ws, _| matches!(ws.panel, Some(Panel::Rescue(_)))));
        b.update_in(&mut bcx, |ws, w, cx| ws.restore_rescue_preview(&RestoreRescuePreview, w, cx));
        bcx.run_until_parked();
        let working = b.read_with(&bcx, |ws, _| ws.active_editor().unwrap());
        b.update_in(&mut bcx, |ws, w, cx| {
            ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation, id: id.clone() }, w, cx)
        });
        bcx.run_until_parked();
        a.update_in(&mut acx, |ws, w, cx| {
            ws.delete_discarded_untitled(&DeleteDiscardedUntitled { generation, id: id.clone() }, w, cx)
        });
        acx.simulate_prompt_answer("Delete Permanently");
        acx.run_until_parked();
        b.update_in(&mut bcx, |ws, w, cx| ws.restore_rescue_preview(&RestoreRescuePreview, w, cx));
        bcx.run_until_parked();
        b.read_with(&bcx, |ws, cx| {
            assert_eq!(ws.active_editor(), Some(working.clone()));
            assert_eq!(working.read(cx).text(cx), "cross-window 救助");
            assert!(working.read(cx).doc(cx).dirty);
        });
        b.update_in(&mut bcx, |ws, w, cx| ws.delete_rescue_preview(&DeleteRescuePreview, w, cx));
        bcx.simulate_prompt_answer("Delete Permanently");
        bcx.run_until_parked();
        assert!(b.read_with(&bcx, |ws, _| ws.status.as_ref().unwrap().0.starts_with("Could not delete")));
        store.capture(generation, vec!["cleared across windows".into()]).unwrap();
        let id = store.list().unwrap().1[0].id.clone();
        b.update_in(&mut bcx, |ws, w, cx| {
            ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation, id }, w, cx)
        });
        bcx.run_until_parked();
        a.update_in(&mut acx, |ws, w, cx| ws.clear_discard_history(&ClearDiscardHistory, w, cx));
        acx.simulate_prompt_answer("Clear History");
        acx.run_until_parked();
        b.update_in(&mut bcx, |ws, w, cx| ws.restore_rescue_preview(&RestoreRescuePreview, w, cx));
        bcx.run_until_parked();
        b.read_with(&bcx, |ws, cx| {
            assert_eq!(ws.active_editor(), Some(working.clone()));
            assert_eq!(working.read(cx).text(cx), "cross-window 救助");
            assert!(working.read(cx).doc(cx).dirty);
            assert!(ws.status.as_ref().unwrap().0.starts_with("Could not restore"));
        });
        b.update_in(&mut bcx, |ws, w, cx| ws.delete_rescue_preview(&DeleteRescuePreview, w, cx));
        bcx.simulate_prompt_answer("Delete Permanently");
        bcx.run_until_parked();
        assert!(b.read_with(&bcx, |ws, _| ws.status.as_ref().unwrap().0.starts_with("Could not delete")));
        assert!(store.list().unwrap().1.is_empty());
    }

    #[gpui::test]
    fn discarded_displayed_preview_is_never_persisted_by_hot_exit_after_remote_clear(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "ordinary working recovery 界");
        let store = Store::current();
        store.capture(store.generation(), vec!["private preview only 救助".into()]).unwrap();
        let (generation, entries) = store.list().unwrap();
        ws.update_in(cx, |ws, w, cx| {
            ws.preview_discarded_untitled(&PreviewDiscardedUntitled { generation, id: entries[0].id.clone() }, w, cx)
        });
        cx.run_until_parked();
        // Equivalent to another window clearing this Store: keep this viewer displayed.
        Store::new(session::support_dir().join("DiscardedUntitled")).clear().unwrap();
        assert!(ws.read_with(cx, |ws, _| matches!(ws.panel, Some(Panel::Rescue(_)))));
        cx.cx.update(|cx| sessions::save_for_quit(cx).unwrap());
        let recovered = session::load(&session::path());
        let texts: Vec<_> = recovered
            .windows
            .iter()
            .flat_map(|w| &w.groups)
            .flat_map(|g| &g.views)
            .filter_map(|v| v.unsaved.as_ref().map(|u| u.text.as_str()))
            .collect();
        assert_eq!(texts, ["ordinary working recovery 界"]);
        assert!(!std::fs::read_to_string(session::path()).unwrap().contains("private preview only"));
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "ordinary working recovery 界");
            assert!(ws.active_editor().unwrap().read(cx).doc(cx).dirty);
        });
        assert!(store.list().unwrap().1.is_empty());
    }

    #[gpui::test]
    fn discarded_fifty_escaped_unicode_rows_load_once_and_filter_without_storage(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let store = Store::current();
        let text = "quoted \" slash \\ control\0\n\t 救助 🦀 ".repeat(100);
        store.capture(store.generation(), (0..50).map(|n| format!("{n}: {text}")).collect()).unwrap();
        ws.update_in(cx, |ws, w, cx| ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx));
        cx.run_until_parked();
        store.clear().unwrap();
        ws.update(cx, |ws, _| {
            let panel = ws.quick_panel.as_mut().unwrap();
            assert_eq!(panel.items.len(), 50);
            assert!(panel
                .keys
                .iter()
                .all(|s| s.len() < 1200 && s.contains('"') && s.contains('\\') && !s.chars().any(char::is_control)));
            panel.refilter("救助 🦀");
            assert_eq!(panel.filtered.len(), 50, "filter uses bounded keys even after payloads are gone");
        });
        assert!(store.list().unwrap().1.is_empty());
    }

    #[gpui::test]
    fn discarded_dirty_empty_draft_never_creates_entry(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "deleted");
        ws.update(cx, |ws, cx| {
            ws.active_editor()
                .unwrap()
                .update(cx, |e, cx| e.edit(cx, |s| s.replace_ranges(vec![(0..s.len(), String::new())])))
        });
        close(&ws, cx, "Don't Save");
        assert!(entries().is_empty());
    }

    #[gpui::test]
    fn discarded_picker_searches_captured_unicode_content(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "rescue needle 救助 🦀");
        close(&ws, cx, "Don't Save");
        ws.update_in(cx, |ws, w, cx| ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let panel = ws.quick_panel.as_ref().unwrap();
            assert!(
                panel.keys.iter().any(|key| key.contains("needle 救助 🦀")),
                "search must include captured content"
            );
        });
    }

    #[gpui::test]
    fn discarded_virtual_composing_sibling_settles_before_survivor_saves(cx: &mut TestAppContext) {
        use crate::editing::text::Selection;
        use gpui::EntityInputHandler;
        let (ws, cx) = window(cx);
        let path = temp("orlo-shared-virtual-close").join("document.txt");
        std::fs::write(&path, "word").unwrap();
        ws.update_in(cx, |ws, w, cx| ws.open_path(path.clone(), w, cx));
        let first = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(cx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
        first.update_in(cx, |e, w, cx| {
            e.set_virtual_space(true, cx);
            e.sel(cx).set_selections(vec![Selection::from_points((4, 3), (4, 3))]);
            e.replace_and_mark_text_in_range(None, "界", None, w, cx);
        });
        ws.update_in(cx, |ws, w, cx| ws.request_close(first.clone(), After::Close, w, cx));
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        assert!(first.read_with(cx, |e, _| e.marked.is_empty()));
        ws.update_in(cx, |ws, w, cx| {
            assert_eq!(ws.views.tabs().count(), 1);
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "word   界");
            ws.save(&Save, w, cx);
        });
        super::super::ui_tests::settle_until(cx, |_| std::fs::read_to_string(&path).unwrap() == "word   界");
        assert!(entries().is_empty(), "named documents never enter rescue");
    }

    #[gpui::test]
    fn discarded_composing_sibling_close_cannot_orphan_provisional_source(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "original");
        let first = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(cx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
        first.update_in(cx, |e, w, cx| {
            e.perform_action(&editor::ToggleOvertype, w, cx);
            e.sel(cx).set_cursor(0);
            e.replace_and_mark_text_in_range(None, "界", None, w, cx);
        });
        ws.update_in(cx, |ws, w, cx| ws.request_close(first.clone(), After::Close, w, cx));
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        assert!(entries().is_empty());
        let survivor = ws.read_with(cx, |ws, cx| {
            let survivor = ws.active_editor().unwrap();
            assert_eq!(survivor.read(cx).text(cx), "界riginal");
            assert!(survivor.read(cx).doc(cx).dirty);
            assert!(first.read(cx).marked.is_empty());
            survivor
        });
        survivor.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(survivor.read_with(cx, |e, cx| e.text(cx)), "original");
        survivor.update(cx, |e, cx| e.edit(cx, |s| s.redo()));
        assert_eq!(survivor.read_with(cx, |e, cx| e.text(cx)), "界riginal");
        close(&ws, cx, "Don't Save");
        let (generation, entries) = Store::current().list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(Store::current().restore(generation, &entries[0].id).unwrap(), "界riginal");
    }

    #[gpui::test]
    fn discarded_clear_failure_is_reported_and_open_text_stays_usable(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "live draft");
        let root = session::support_dir().join("DiscardedUntitled");
        std::fs::create_dir_all(root.join("00000000000000000000-1-1-1.txt")).unwrap();
        ws.update_in(cx, |ws, w, cx| ws.clear_discard_history(&ClearDiscardHistory, w, cx));
        cx.simulate_prompt_answer("Clear History");
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert!(ws.status.as_ref().unwrap().0.starts_with("Could not clear discard history:"));
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "live draft");
        });
    }

    #[gpui::test]
    fn discarded_virtual_padding_preview_is_not_retained(cx: &mut TestAppContext) {
        use crate::editing::text::Selection;
        use gpui::EntityInputHandler;
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "word");
        let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        editor.update_in(cx, |e, w, cx| {
            e.set_virtual_space(true, cx);
            e.sel(cx).set_selections(vec![Selection::from_points((4, 3), (4, 3))]);
            e.replace_and_mark_text_in_range(None, "界", None, w, cx);
            assert_eq!(e.text(cx), "word   界");
        });
        close(&ws, cx, "Don't Save");
        assert!(cx.has_pending_prompt());
        assert!(entries().is_empty());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        editor.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(None, "", None, w, cx));
        assert_eq!(editor.read_with(cx, |e, cx| e.text(cx)), "word");
        close(&ws, cx, "Don't Save");
        let (generation, entries) = Store::current().list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(Store::current().restore(generation, &entries[0].id).unwrap(), "word");
    }

    #[gpui::test]
    fn discarded_preview_is_excluded_and_read_only_working_text_restores_writable(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "preview text");
        cx.run_until_parked();
        ws.update(cx, |ws, _| {
            ws.views.groups[ws.views.active].preview = ws.active_editor();
        });
        close(&ws, cx, "Don't Save");
        assert!(entries().is_empty(), "a tool preview is never rescued");
        ws.update_in(cx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
        type_in(&ws, cx, "read-only working text");
        ws.update(cx, |ws, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true))
        });
        close(&ws, cx, "Don't Save");
        ws.update_in(cx, |ws, w, cx| {
            ws.browse_discarded_untitled(&BrowseDiscardedUntitled, w, cx);
        });
        cx.run_until_parked();
        ws.update_in(cx, |ws, w, cx| ws.restore_discarded_untitled(&offered(ws), w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            let d = editor.read(cx).doc(cx);
            assert_eq!(d.text(), "read-only working text");
            assert!(d.dirty && !d.read_only && d.path.is_none());
        });
    }

    #[gpui::test]
    fn discarded_picker_restores_dirty_usable_unicode_and_clear_rejects_old_actions(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "救助 🦀 e\u{301}\n");
        close(&ws, cx, "Don't Save");
        cx.dispatch_action(BrowseDiscardedUntitled);
        cx.run_until_parked();
        let action = ws.read_with(cx, |ws, _| offered(ws));
        cx.dispatch_action(action.clone());
        cx.run_until_parked();
        let editor = ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            let d = editor.read(cx).doc(cx);
            assert_eq!(d.text(), "救助 🦀 e\u{301}\n");
            assert!(d.path.is_none() && d.dirty && !d.read_only);
            editor
        });
        editor.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.insert("edited");
                s.undo();
                assert_eq!(s.text(), "救助 🦀 e\u{301}\n");
                assert!(s.unsaved(), "undo cannot make a rescued draft clean");
                s.redo();
            })
        });
        let text = editor.read_with(cx, |e, cx| e.text(cx));
        ws.update_in(cx, |ws, w, cx| ws.clear_discard_history(&ClearDiscardHistory, w, cx));
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(entries().len(), 1);
        ws.update_in(cx, |ws, w, cx| ws.clear_discard_history(&ClearDiscardHistory, w, cx));
        cx.simulate_prompt_answer("Clear History");
        cx.run_until_parked();
        assert!(entries().is_empty());
        ws.update_in(cx, |ws, w, cx| ws.restore_discarded_untitled(&action, w, cx));
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 1);
            assert_eq!(editor.read(cx).text(cx), text);
            assert!(ws.status.as_ref().unwrap().0.starts_with("Could not restore"));
        });
    }

    #[gpui::test]
    fn discarded_cancel_shared_siblings_and_close_all_capture_only_last_document(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "shared draft");
        close(&ws, cx, "Cancel");
        assert!(entries().is_empty());
        ws.update_in(cx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
        close(&ws, cx, "Don't Save");
        assert!(entries().is_empty(), "remaining sibling is not discarded");
        ws.update_in(cx, |ws, w, cx| {
            ws.new_view_into_file(&NewViewIntoFile, w, cx);
            ws.close_all(&CloseAll, w, cx);
        });
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        assert_eq!(entries().len(), 1, "Close All records a shared document once");
        assert!(ws.read_with(cx, |ws, _| ws.views.tabs().next().is_none()));
    }

    #[gpui::test]
    fn discarded_all_preserves_other_window_ownership_and_captures_local_draft(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (a, b, mut acx, mut bcx) = two_windows(cx);
        type_in(&a, &mut acx, "shared");
        let first = a.read_with(&acx, |ws, _| ws.active_editor().unwrap());
        let peer = first.update(&mut acx, |e, cx| cx.new(|cx| e.another_view(cx)));
        b.update_in(&mut bcx, |ws, w, cx| {
            ws.attach(peer.clone(), true, w, cx);
        });
        first.update_in(&mut acx, |e, w, cx| {
            e.perform_action(&editor::ToggleOvertype, w, cx);
            e.sel(cx).set_cursor(0);
            e.replace_and_mark_text_in_range(None, "界", None, w, cx);
        });
        a.update_in(&mut acx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
        type_in(&a, &mut acx, "only here");
        a.update_in(&mut acx, |ws, w, cx| ws.confirm_close(w, cx));
        acx.simulate_prompt_answer("Discard All");
        acx.run_until_parked();
        let store = Store::current();
        let (generation, listed) = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(store.restore(generation, &listed[0].id).unwrap(), "only here");
        assert_eq!(first.read_with(&bcx, |e, cx| e.document.read(cx).tabs), 1);
        assert!(first.read_with(&bcx, |e, _| e.marked.is_empty()));
        assert_eq!(peer.read_with(&bcx, |e, cx| e.text(cx)), "界hared");
        assert!(a.read_with(&bcx, |ws, _| ws.views.tabs().next().is_none()));
        close(&b, &mut bcx, "Don't Save");
        assert_eq!(entries().len(), 2);
        assert_eq!(peer.read_with(&bcx, |e, cx| e.document.read(cx).tabs), 0);
    }

    #[gpui::test]
    fn discarded_hot_exit_and_named_or_tool_documents_are_excluded(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "hot exit draft");
        cx.cx.update(|cx| sessions::save_for_quit(cx).unwrap());
        let recovered = session::load(&session::path());
        assert_eq!(recovered.windows[0].groups[0].views[0].unsaved.as_ref().unwrap().text, "hot exit draft");
        assert!(entries().is_empty());
        ws.update_in(cx, |ws, w, cx| {
            let editor = ws.active_editor().unwrap();
            ws.close_editor(&editor, w, cx); // ordinary removal has no discard authorization
            ws.new_file(&NewFile, w, cx);
        });
        assert!(entries().is_empty());
        let path = temp("orlo-rescue-named").join("named.txt");
        std::fs::write(&path, "saved").unwrap();
        ws.update_in(cx, |ws, w, cx| ws.open_path(path, w, cx));
        type_in(&ws, cx, "changed");
        close(&ws, cx, "Don't Save");
        for kind in 0..3 {
            ws.update_in(cx, |ws, w, cx| {
                ws.new_file(&NewFile, w, cx);
                ws.active_editor().unwrap().update(cx, |e, cx| {
                    e.edit(cx, |s| s.insert("tool text"));
                    match kind {
                        0 => e.update_doc(cx, |d| d.scratch = true),
                        1 => e.results = true,
                        _ => e.label = Some("Preview".into()),
                    }
                });
            });
            close(&ws, cx, "Don't Save");
        }
        assert!(entries().is_empty());
    }

    #[gpui::test]
    fn discarded_oversize_failure_can_cancel_or_explicitly_discard_without_rescue(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        editor.update(cx, |e, cx| e.edit(cx, |s| s.insert(&"🦀".repeat(262_145))));
        close(&ws, cx, "Don't Save");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |ws, _| ws.views.tabs().count()), 1);
        assert!(entries().is_empty());
        close(&ws, cx, "Don't Save");
        cx.simulate_prompt_answer("Discard Without Rescue");
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |ws, _| ws.views.tabs().count()), 0);
        assert!(entries().is_empty());
    }

    #[gpui::test]
    fn discarded_storage_error_does_not_close_new_edits_after_second_prompt(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let store = session::support_dir().join("DiscardedUntitled");
        std::fs::write(&store, "not a directory").unwrap();
        type_in(&ws, cx, "draft");
        close(&ws, cx, "Don't Save");
        assert!(cx.has_pending_prompt());
        type_in(&ws, cx, "new edit");
        cx.simulate_prompt_answer("Discard Without Rescue");
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 1);
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "draftnew edit");
            assert!(ws.status.as_ref().unwrap().0.contains("documents changed"));
        });
        std::fs::remove_file(store).unwrap();
        close(&ws, cx, "Don't Save");
        assert_eq!(entries().len(), 1);
    }

    fn added_tab_during_rescue_error_keeps_documents_open(
        cx: &mut TestAppContext,
        start: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>),
        answer: &str,
    ) {
        let (ws, cx) = window(cx);
        std::fs::write(session::support_dir().join("DiscardedUntitled"), "not a directory").unwrap();
        type_in(&ws, cx, "original draft");
        let original = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(cx, start);
        cx.simulate_prompt_answer(answer);
        cx.run_until_parked();
        assert!(cx.has_pending_prompt(), "rescue failure must await an explicit loss decision");

        ws.update_in(cx, |ws, w, cx| ws.new_file(&NewFile, w, cx));
        type_in(&ws, cx, "new draft 救助 🦀");
        let added = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        cx.simulate_prompt_answer("Discard Without Rescue");
        cx.run_until_parked();

        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.views.tabs().count(), 2, "a changed close set must remain open");
            assert!(cx.windows().contains(&ws.window));
            assert_eq!(original.read(cx).text(cx), "original draft");
            assert_eq!(added.read(cx).text(cx), "new draft 救助 🦀");
            assert!(original.read(cx).doc(cx).dirty && added.read(cx).doc(cx).dirty);
            assert!(ws.status.as_ref().unwrap().0.contains("documents changed"));
        });
        assert!(!cx.has_pending_prompt(), "stale loss confirmation cancels rather than closing another tab");
    }

    #[gpui::test]
    fn discarded_window_error_rejects_added_tab(cx: &mut TestAppContext) {
        added_tab_during_rescue_error_keeps_documents_open(cx, Workspace::confirm_close, "Discard All");
    }

    #[gpui::test]
    fn discarded_single_tab_error_rejects_added_tab(cx: &mut TestAppContext) {
        added_tab_during_rescue_error_keeps_documents_open(
            cx,
            |ws, w, cx| ws.close_tab(&CloseTab, w, cx),
            "Don't Save",
        );
    }

    #[gpui::test]
    fn discarded_close_all_error_rejects_added_tab(cx: &mut TestAppContext) {
        added_tab_during_rescue_error_keeps_documents_open(
            cx,
            |ws, w, cx| ws.close_all(&CloseAll, w, cx),
            "Don't Save",
        );
    }

    #[gpui::test]
    fn discarded_ime_in_shared_view_never_retains_provisional_overtype_source(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "original");
        let first = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(cx, |ws, w, cx| ws.new_view_into_file(&NewViewIntoFile, w, cx));
        first.update_in(cx, |e, w, cx| {
            e.perform_action(&editor::ToggleOvertype, w, cx);
            e.sel(cx).set_cursor(0);
            e.replace_and_mark_text_in_range(None, "界", None, w, cx);
        });
        ws.update_in(cx, |ws, w, cx| ws.close_all(&CloseAll, w, cx));
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        assert!(cx.has_pending_prompt());
        assert!(entries().is_empty());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(ws.read_with(cx, |ws, _| ws.views.tabs().count()), 2);
        first.update_in(cx, |e, w, cx| {
            let marked = e.marked_text_range(w, cx);
            e.replace_text_in_range(marked, "界", w, cx);
        });
        ws.update_in(cx, |ws, w, cx| ws.close_all(&CloseAll, w, cx));
        cx.simulate_prompt_answer("Don't Save");
        cx.run_until_parked();
        let (generation, entries) = Store::current().list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(Store::current().restore(generation, &entries[0].id).unwrap(), "界riginal");
    }
}
