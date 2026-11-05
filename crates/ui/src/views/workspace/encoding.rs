//! Reopen with Encoding and Save with Encoding, through the same
//! reads and saves as any file.

use super::*;
use crate::editing::encoding::Encoding;

actions!(workspace, [ReopenWithEncoding, SaveWithEncoding]);

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct ChangeFileEncoding {
    pub encoding: Encoding,
    pub bom: bool,
    pub reopen: bool,
    pub editor: EntityId,
    pub buffer: u64,
    pub revision: u64,
    pub point: crate::editing::text::SavedPoint,
    pub path: Option<PathBuf>,
    pub disk: Option<crate::files::watch::Stamp>,
    pub disk_encoding: Encoding,
    pub read_only: bool,
}

impl Workspace {
    forward_actions! {
    pub(crate) reopen_with_encoding(ReopenWithEncoding, window, cx) => encoding_choices(true, window, cx);
    pub(crate) save_with_encoding(SaveWithEncoding, window, cx) => encoding_choices(false, window, cx);
    }

    fn encoding_choices(&mut self, reopen: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let doc = editor.read(cx).doc(cx);
        if doc.edits_blocked() {
            return self.status_message(
                if doc.viewer_mode() {
                    "Turn off viewer mode before changing encoding"
                } else {
                    "Stop following before changing encoding"
                },
                cx,
            );
        }
        let (buffer, revision) = (doc.id(), doc.version);
        let id = editor.entity_id();
        let point = doc.save_point();
        let path = doc.path.clone();
        let disk = doc.disk;
        let disk_encoding = doc.disk_encoding;
        let read_only = doc.read_only;
        let items = Encoding::ALL
            .into_iter()
            .flat_map(|encoding| {
                [false, true]
                    .into_iter()
                    .filter(move |bom| !*bom || (!reopen && !encoding.bom_bytes().is_empty()))
                    .map(move |bom| (encoding, bom))
            })
            .map(|(encoding, bom)| {
                crate::chrome::quick_panel::Item::command(
                    format!("{}{}", encoding.name(), if bom { " with BOM" } else { "" }),
                    if reopen {
                        "Reinterpret disk bytes; keep the file unchanged"
                    } else {
                        "Convert and use ordinary Save"
                    },
                    ChangeFileEncoding {
                        encoding,
                        bom,
                        reopen,
                        editor: id,
                        buffer,
                        revision,
                        point,
                        path: path.clone(),
                        disk,
                        disk_encoding,
                        read_only,
                    },
                )
            })
            .collect();
        self.open_quick_panel(Kind::Choices, items, window, cx);
    }

    pub(crate) fn change_file_encoding(&mut self, a: &ChangeFileEncoding, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor().filter(|e| e.entity_id() == a.editor) else {
            return;
        };
        let state = editor.read(cx).read_state(cx);
        if state.doc.edits_blocked() {
            return self.status_message(
                if state.doc.viewer_mode() {
                    "Turn off viewer mode before changing encoding"
                } else {
                    "Stop following before changing encoding"
                },
                cx,
            );
        }
        if state.buffer_id() != a.buffer
            || state.doc.version != a.revision
            || state.doc.save_point() != a.point
            || state.doc.path != a.path
            || state.doc.disk != a.disk
            || state.doc.disk_encoding != a.disk_encoding
            || state.doc.read_only != a.read_only
        {
            return self.status_message("The document changed; choose its encoding again", cx);
        }
        if !a.reopen {
            editor.update(cx, |e, cx| {
                e.update_doc(cx, |d| d.set_encoding(a.encoding, a.bom));
                cx.notify();
            });
            autosave(cx);
            return self.save(&Save, window, cx);
        }
        if editor.read(cx).document.read(cx).saving() {
            return self.status_message("The document is saving; reopen it when the save finishes", cx);
        }
        if state.unsaved() {
            return self.alert(
                window,
                cx,
                "Cannot reopen",
                "Save or discard unsaved changes before reopening with another encoding.",
            );
        }
        let Some(path) = state.doc.path.clone() else {
            return self.status_message("Save the file before reopening it with an encoding", cx);
        };
        if pipelines::writing(&path, cx) {
            return self.status_message("The file is being written; reopen it when the operation finishes", cx);
        }
        let (buffer, version, disk, rope, generation, encoding) = (
            state.buffer_id(),
            state.doc.version,
            state.doc.disk,
            state.doc.rope.clone(),
            self.restore_gen,
            a.encoding,
        );
        let point = state.doc.save_point();
        let disk_encoding = state.doc.disk_encoding;
        let read_only = state.doc.read_only;
        let source = path.clone();
        let task =
            cx.background_spawn(
                async move { Loaded::read_encoding(&source, Some(encoding)).map(|l| (l.diff(&rope), l)) },
            );
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, window, cx| {
                let s = editor.read(cx).read_state(cx);
                if ws.restore_gen != generation
                    || ws.views.find(&editor).is_none()
                    || ws.active_editor().as_ref() != Some(&editor)
                    || s.buffer_id() != buffer
                    || s.doc.version != version
                    || s.doc.disk != disk
                    || s.doc.save_point() != point
                    || s.doc.disk_encoding != disk_encoding
                    || s.doc.read_only != read_only
                    || editor.read(cx).document.read(cx).saving()
                    || pipelines::writing(&path, cx)
                    || s.doc.path.as_ref() != Some(&path)
                    || s.unsaved()
                {
                    return;
                }
                match result {
                    Ok((changes, loaded)) if crate::files::watch::stamp(&path) == loaded.disk => {
                        // the reload goes through the document: every view and window hears of it
                        editor.update(cx, |e, cx| e.reload(loaded, &changes, cx));
                        autosave(cx);
                        ws.status_message(format!("Reopened as {}", encoding.name()), cx);
                    }
                    Ok(_) => ws.status_message("The file changed while reopening; try again", cx),
                    Err(e) => ws.alert(window, cx, "Could not reopen file", &e.to_string()),
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{settle_until, temp, window};
    use super::*;
    use gpui::TestAppContext;

    fn action(ws: &Workspace, cx: &App, encoding: Encoding, bom: bool, reopen: bool) -> ChangeFileEncoding {
        let e = ws.active_editor().unwrap();
        ChangeFileEncoding {
            encoding,
            bom,
            reopen,
            editor: e.entity_id(),
            buffer: e.read(cx).doc(cx).id(),
            revision: e.read(cx).doc(cx).version,
            point: e.read(cx).doc(cx).save_point(),
            path: e.read(cx).doc(cx).path.clone(),
            disk: e.read(cx).doc(cx).disk,
            disk_encoding: e.read(cx).doc(cx).disk_encoding,
            read_only: e.read(cx).doc(cx).read_only,
        }
    }

    #[gpui::test]
    fn pending_reinterpretation_expires_when_read_only_policy_changes(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let path = temp("f01-reopen-stale").join("text.txt");
        std::fs::write(&path, b"cafe").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(path.clone(), window, cx);
            ws.change_file_encoding(&action(ws, cx, Encoding::Windows1252, false, true), window, cx);
            ws.active_editor().unwrap().update(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true));
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).doc(cx).encoding, Encoding::Utf8);
            assert!(editor.read(cx).doc(cx).read_only);
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"cafe");
    }

    #[gpui::test]
    fn encoding_menus_offer_reinterpretation_and_bomless_conversion(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.reopen_with_encoding(&ReopenWithEncoding, window, cx);
            let labels = || ws.quick_panel.as_ref().unwrap().items.iter().map(|i| i.label.as_ref()).collect::<Vec<_>>();
            assert!(labels().contains(&"Windows-1252"));
            assert!(labels().contains(&"ISO-8859-1"));
            assert!(labels().contains(&"UTF-32 LE"));
            assert!(!labels().iter().any(|label| label.contains("with BOM")));
            ws.save_with_encoding(&SaveWithEncoding, window, cx);
            let labels = ws.quick_panel.as_ref().unwrap().items.iter().map(|i| i.label.as_ref()).collect::<Vec<_>>();
            assert!(labels.contains(&"UTF-32 LE"));
            assert!(labels.contains(&"UTF-32 LE with BOM"));
            assert!(!labels.contains(&"Windows-1252 with BOM"));
        });
    }

    #[gpui::test]
    fn closed_legacy_reopen_keeps_disk_hint_but_new_bom_wins(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let p = temp("f01-closed-hint").join("text.txt");
        std::fs::write(&p, b"caf\xe9\r\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(p.clone(), window, cx);
            ws.change_file_encoding(&action(ws, cx, Encoding::Windows1252, false, true), window, cx);
        });
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).encoding == Encoding::Windows1252)
        });
        ws.update_in(cx, |ws, window, cx| {
            let e = ws.active_editor().unwrap();
            ws.close_editor(&e, window, cx);
            ws.reopen_closed_file(&ReopenClosedFile, window, cx);
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).doc(cx).text(), "café\n");
            assert_eq!(e.read(cx).doc(cx).encoding, Encoding::Windows1252);
            ws.close_editor(&e, window, cx);
            std::fs::write(&p, [0, 0, 0xfe, 0xff, 0, 0, 0, 0x41]).unwrap();
            ws.reopen_closed_file(&ReopenClosedFile, window, cx);
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).doc(cx).text(), "A");
            assert_eq!(e.read(cx).doc(cx).encoding, Encoding::Utf32Be);
        });
    }

    #[gpui::test]
    fn encoding_choice_rejects_metadata_changes_without_text_revision(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let p = temp("f01-stale-choice").join("text.txt");
        std::fs::write(&p, "café\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(p.clone(), window, cx);
            let choice = action(ws, cx, Encoding::Utf32Be, true, false);
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Windows1252, false)));
            ws.change_file_encoding(&choice, window, cx);
            assert_eq!(editor.read(cx).doc(cx).encoding, Encoding::Windows1252);
            assert!(editor.read(cx).doc(cx).unsaved());
        });
        assert_eq!(std::fs::read(&p).unwrap(), "café\n".as_bytes());
    }

    #[gpui::test]
    fn reload_and_session_restore_follow_changed_boms(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let path = temp("g60-new-bom").join("text.txt");
        std::fs::write(&path, "café 🦀\n").unwrap();
        let path = path.canonicalize().unwrap();
        ws.update_in(cx, |ws, window, cx| ws.open_path(path.clone(), window, cx));
        std::fs::write(&path, crate::editing::encoding::encode("café 🦀\n", Encoding::Utf16Be, true).unwrap()).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.reload_editor(ws.active_editor().unwrap(), |_, e, _, _| panic!("{e}"), window, cx);
        });
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.reloads.is_empty()));
        let saved = ws.update_in(cx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).doc(cx).text(), "café 🦀\n");
            assert_eq!(editor.read(cx).doc(cx).encoding, Encoding::Utf16Be);
            assert!(!editor.read(cx).doc(cx).read_only);
            ws.capture(window, false, &mut Texts::new(), cx)
        });
        std::fs::write(&path, b"\xef\xbb\xbfcaf\xc3\xa9 \xf0\x9f\xa6\x80\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.restore(saved, window, cx);
            let editor = ws.active_editor().unwrap();
            assert_eq!(editor.read(cx).doc(cx).text(), "café 🦀\n");
            assert_eq!(editor.read(cx).doc(cx).encoding, Encoding::Utf8);
            assert!(!editor.read(cx).doc(cx).read_only);
        });
    }

    #[gpui::test]
    fn ui_reopens_bomless_utf16_and_refuses_dirty_buffer(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let p = temp("g60-reopen").join("text.txt");
        let bytes = crate::editing::encoding::encode("café 🦀\r\n", Encoding::Utf16Le, false).unwrap();
        std::fs::write(&p, &bytes).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(p.clone(), window, cx);
            ws.change_file_encoding(&action(ws, cx, Encoding::Utf16Le, false, true), window, cx);
        });
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, cx| ws.active_editor().is_some_and(|e| e.read(cx).doc(cx).text() == "café 🦀\n"))
        });
        ws.update_in(cx, |ws, window, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| e.edit(cx, |s| s.insert("draft")));
            ws.change_file_encoding(&action(ws, cx, Encoding::Utf8, false, true), window, cx);
            assert!(e.read(cx).doc(cx).text().contains("draft"));
            assert_eq!(e.read(cx).doc(cx).encoding, Encoding::Utf16Le);
        });
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
    }

    #[gpui::test]
    fn reopen_offers_only_what_it_can_do_and_an_untitled_buffer_says_why_not(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let offered = |ws: &Workspace| {
            let p = ws.quick_panel.as_ref().unwrap();
            (p.items.iter())
                .map(|i| match &i.action {
                    ItemAction::Dispatch(a) => {
                        let a = a.as_any().downcast_ref::<ChangeFileEncoding>().unwrap();
                        (i.label.clone(), a.encoding, a.bom)
                    }
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
        };
        ws.update_in(cx, |ws, window, cx| {
            ws.reopen_with_encoding(&ReopenWithEncoding, window, cx);
            let reopen = offered(ws);
            assert_eq!(reopen.len(), Encoding::ALL.len());
            assert!(reopen.iter().all(|(_, _, bom)| !bom), "a reopen cannot add a BOM");
            assert!(reopen.contains(&("UTF-16 LE".into(), Encoding::Utf16Le, false)));
            ws.cancel_quick_panel(cx);
            ws.save_with_encoding(&SaveWithEncoding, window, cx);
            let with_bom = Encoding::ALL.iter().filter(|e| !e.bom_bytes().is_empty()).count();
            assert_eq!(offered(ws).len(), Encoding::ALL.len() + with_bom);
            ws.cancel_quick_panel(cx);
            ws.change_file_encoding(&action(ws, cx, Encoding::Utf16Le, false, true), window, cx);
            assert!(ws.status.is_some());
        });
    }

    #[gpui::test]
    fn ui_save_converts_using_safe_save_and_updates_shared_views(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let p = temp("g60-save").join("text.txt");
        std::fs::write(&p, "café 🦀\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(p.clone(), window, cx);
            let original = ws.active_editor().unwrap();
            let clone = original.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
            ws.attach(clone.clone(), false, window, cx);
            ws.change_file_encoding(&action(ws, cx, Encoding::Utf16Be, true, false), window, cx);
            assert_eq!(original.read(cx).doc(cx).encoding, Encoding::Utf16Be);
            assert_eq!(clone.read(cx).doc(cx).encoding, Encoding::Utf16Be);
            assert!(!clone.read(cx).doc(cx).unsaved());
        });
        assert_eq!(
            std::fs::read(&p).unwrap(),
            crate::editing::encoding::encode("café 🦀\n", Encoding::Utf16Be, true).unwrap()
        );
    }
}
