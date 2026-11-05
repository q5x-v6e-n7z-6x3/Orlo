//! One debounced existing-file save per shared document, through the ordinary save engine.
use super::*;
use crate::documents::autosave::Key;
use std::rc::Rc;
use std::time::Duration;

impl Workspace {
    /// Stable oldest visible view owns policy/formatting even when focus changes.
    fn file_autosave_owner(&self, document: &Entity<OpenDocument>, cx: &App) -> Option<Entity<Editor>> {
        self.open_editors(cx)
            .into_iter()
            .filter(|e| e.read(cx).document == *document)
            .min_by_key(|e| e.read(cx).state.view_id())
    }

    pub(crate) fn schedule_changed_file_autosave(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(document) = Documents::get(id, cx) else { return };
        // Default-off typing does not scan windows or resolve file preferences.
        if document.read(cx).file_autosave.policy.as_ref().is_some_and(|p| !p.file_autosave) {
            return;
        }
        self.schedule_file_autosave(document, cx);
    }

    pub(crate) fn refresh_file_autosaves(&mut self, cx: &mut Context<Self>) {
        let editors = editors::one_per_document(self.views.tabs().cloned(), cx, |_| true);
        for editor in editors {
            self.schedule_file_autosave(editor.read(cx).document.clone(), cx);
        }
    }

    fn schedule_file_autosave(&mut self, document: Entity<OpenDocument>, cx: &mut Context<Self>) {
        let Some(editor) = self.file_autosave_owner(&document, cx) else { return };
        if self.views.find(&editor).is_none() {
            return;
        }
        let policy = self.editor_settings(editor.read(cx), cx);
        let delay = Duration::from_millis(u64::from(policy.file_autosave_delay_ms));
        let owner = editor.entity_id();
        let key = Key::of(&document.read(cx).model);
        let generation = document.update(cx, |d, _| {
            let a = &mut d.file_autosave;
            if a.owner != Some(owner) || a.policy.as_ref() != Some(&policy) {
                a.cancel();
                a.owner = Some(owner);
                a.policy = Some(policy.clone());
                a.failed = None;
            }
            if !policy.file_autosave
                || !d.model.dirty
                || d.model.path.is_none()
                || d.model.deleted
                || d.model.read_only
                || d.model.scratch
                || d.model.is_following()
                || a.baseline.is_none()
                || a.suspensions > 0
                || a.quitting
            {
                a.timer = None;
                a.pending = None;
                return None;
            }
            if d.saving.is_some() || a.failed.as_ref() == Some(&key) {
                return None;
            }
            if a.timer.is_some() && a.pending.as_ref() == Some(&key) {
                return None;
            }
            a.timer = None;
            a.pending = Some(key.clone());
            Some(a.generation)
        });
        let Some(generation) = generation else { return };
        let weak = document.downgrade();
        let handle = self.window.downcast::<Workspace>().unwrap();
        let task = cx.spawn(async move |_, cx| {
            cx.background_executor().timer(delay).await;
            handle
                .update(cx, |ws, _, cx| {
                    let Some(document) = weak.upgrade() else { return };
                    let current = document.read(cx);
                    if current.file_autosave.generation != generation
                        || current.file_autosave.pending.as_ref() != Some(&key)
                    {
                        return;
                    }
                    document.update(cx, |d, _| {
                        d.file_autosave.timer = None;
                        d.file_autosave.pending = None;
                    });
                    ws.file_autosave_ready(document, owner, generation, key, cx);
                })
                .ok();
        });
        document.update(cx, |d, _| d.file_autosave.timer = Some(task));
    }

    fn file_autosave_valid(
        &self,
        document: &Entity<OpenDocument>,
        owner: gpui::EntityId,
        generation: u64,
        cx: &App,
    ) -> bool {
        let d = document.read(cx);
        if d.tabs == 0
            || !d.model.dirty
            || d.file_autosave.generation != generation
            || d.file_autosave.suspensions > 0
            || d.file_autosave.quitting
            || d.model.deleted
            || d.model.read_only
            || d.model.is_following()
            || d.model.scratch
            || self.choosing_save_for(document, cx)
            || self.other_workspaces(cx).any(|ws| ws.choosing_save_for(document, cx))
            || self.closing
        {
            return false;
        }
        let Some(editor) =
            self.file_autosave_owner(document, cx).filter(|e| e.entity_id() == owner && self.views.find(e).is_some())
        else {
            return false;
        };
        let policy = self.editor_settings(editor.read(cx), cx);
        policy.file_autosave
            && d.file_autosave.policy.as_ref() == Some(&policy)
            && !self.open_editors(cx).iter().any(|e| {
                let e = e.read(cx);
                e.document == *document && (!e.marked.is_empty() || e.state.in_snippet())
            })
    }

    fn file_autosave_ready(
        &mut self,
        document: Entity<OpenDocument>,
        owner: gpui::EntityId,
        generation: u64,
        key: Key,
        cx: &mut Context<Self>,
    ) {
        let d = document.read(cx);
        if Key::of(&d.model) != key
            || d.saving()
            || !self.file_autosave_valid(&document, owner, generation, cx)
            || d.model.path.as_deref().is_some_and(|p| pipelines::writing(p, cx))
        {
            self.schedule_file_autosave(document, cx);
            return;
        }
        let Some(disk) = d.file_autosave.baseline else { return };
        let editor = self.file_autosave_owner(&document, cx).unwrap();
        let handle = self.window.downcast::<Workspace>().unwrap();
        let weak = document.downgrade();
        let start_key = key.clone();
        let path = key.path;
        let valid: crate::documents::save::SaveValidation = Rc::new(move |cx| {
            let Some(document) = weak.upgrade() else { return false };
            document.read(cx).model.path == path
                && handle.read(cx).is_ok_and(|ws| ws.file_autosave_valid(&document, owner, generation, cx))
        });
        let tidy = self.cleanup_for_save(&editor, None, cx);
        let prepare = self.format_for_save_guarded(&editor, None, tidy, Some(valid.clone()), cx);
        let id = document.read(cx).id();
        let landed = move |outcome: &crate::documents::SaveOutcome, cx: &mut App| {
            finish(id, outcome.clone(), cx);
        };
        let guard = crate::documents::save::Guard { valid, disk };
        cx.defer(move |cx| {
            if !(guard.valid)(cx) || Key::of(&document.read(cx).model) != start_key {
                return;
            }
            if let Some(outcome) = OpenDocument::save_automatically(&document, guard, prepare, landed, cx) {
                finish(id, outcome, cx);
            }
        });
    }

    fn file_autosave_finished(
        &mut self,
        editor: Entity<Editor>,
        outcome: crate::documents::SaveOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = editor.read(cx).document.clone();
        if let Err(error) = outcome {
            self.status_message(
                format!("Automatic save paused: {error}. Edit, change save settings, or save explicitly to retry."),
                cx,
            );
        } else {
            self.saved(editor, outcome, false, After::Nothing, window, cx);
        }
        self.schedule_file_autosave(document, cx);
    }
}

/// View removal can transfer policy to another window; run after its current update.
pub(crate) fn refresh_all(cx: &mut App) {
    for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        handle.update(cx, |ws, _, cx| ws.refresh_file_autosaves(cx)).ok();
    }
}

/// The visible owner receives completion even if the initiating window was removed.
fn finish(id: u64, outcome: crate::documents::SaveOutcome, cx: &mut App) {
    let mut views = Vec::new();
    for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        if let Ok(ws) = handle.read(cx) {
            for editor in ws.views.tabs().filter(|e| e.read(cx).doc(cx).id() == id) {
                views.push((editor.read(cx).state.view_id(), handle, editor.clone()));
            }
        }
    }
    if let Some((_, handle, editor)) = views.into_iter().min_by_key(|(id, _, _)| *id) {
        handle.update(cx, |ws, window, cx| ws.file_autosave_finished(editor, outcome, window, cx)).ok();
    }
}
