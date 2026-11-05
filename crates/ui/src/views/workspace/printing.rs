//! Print options and the snapshot boundary. Native presentation never borrows GPUI state.
use super::*;
use crate::platform::{
    macos::printing as native,
    printing::{Options, Snapshot},
};

impl Workspace {
    pub(crate) fn print(&mut self, _: &Print, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let snapshot = match self.print_snapshot(&editor, cx) {
            Ok(snapshot) => snapshot,
            Err(error) => return self.alert(window, cx, "Cannot Print", &error),
        };
        let done: OnDone = Box::new(move |values, _, ws, window, cx| {
            let options = Options {
                font_size: values
                    .text("font_size")
                    .trim()
                    .parse()
                    .map_err(|_| "Font size must be a whole number from 8 to 24")?,
                tab_width: values
                    .text("tab_width")
                    .trim()
                    .parse()
                    .map_err(|_| "Tab width must be a whole number from 1 to 16")?,
                line_numbers: values.flag("line_numbers"),
                wrap: values.flag("wrap"),
                syntax_colors: values.flag("syntax_colors"),
            };
            options.validate()?;
            let selection = values.choice("scope") == 1;
            let values_page_setup = values.flag("page_setup");
            let snapshot = snapshot.clone();
            let title = snapshot.title.clone();
            ws.status_message("Preparing print snapshot…", cx);
            let task = cx.background_spawn(async move { snapshot.prepare(selection, options) });
            cx.spawn_in(window, async move |this, cx| {
                let source = task.await;
                // This future runs on the foreground executor, with NO update_in/read
                // borrow alive. AppKit's nested modal loop may now safely re-enter GPUI.
                let result = source.and_then(|source| {
                    native::run_prepared(&title, &source, options, values_page_setup, native::Destination::Panel)
                });
                this.update_in(cx, |ws, window, cx| match result {
                    Ok(true) => ws.status_message("Print job completed", cx),
                    Ok(false) => ws.status_message("Printing cancelled or could not finish", cx),
                    Err(error) => ws.alert(window, cx, "Cannot Print", &error),
                })
                .ok();
            })
            .detach();
            Ok(())
        });
        self.show_form(
            "Print snapshot — Enter opens macOS Print / PDF",
            vec![
                Field::text("font_size", "Font size (8–24)", "11"),
                Field::choice("scope", "Scope", vec!["Whole document", "Selected text"]),
                Field::text("tab_width", "Tab width (1–16)", "4"),
                Field::toggle("line_numbers", "Source line numbers", false),
                Field::toggle("wrap", "Wrap lines", true),
                Field::toggle("syntax_colors", "Syntax colors", false),
                Field::toggle("page_setup", "Page setup before printing", false),
            ],
            done,
            window,
            cx,
        );
    }

    fn print_snapshot(&self, editor: &Entity<Editor>, cx: &App) -> Result<Snapshot, String> {
        let document = &editor.read(cx).document;
        // Overtype/virtual-space composition temporarily retains source alongside its
        // preview. Refuse even when it belongs to another window sharing this document.
        if self.open_editors(cx).iter().chain(std::iter::once(editor)).any(|other| {
            let other = other.read(cx);
            other.document == *document && !other.marked.is_empty()
        }) {
            return Err("Finish or cancel text composition before opening Print".into());
        }
        let state = editor.read(cx).read_state(cx);
        if state.view.revision() != state.doc.version {
            return Err("A shared edit is updating this view; try Print again".into());
        }
        let title = state
            .doc
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        let mut snapshot =
            Snapshot::new(state.doc.rope.clone(), state.view.selections.iter().map(|s| s.range()).collect(), title);
        snapshot.set_syntax(editor.read(cx).highlighter.syntax_name().to_string());
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{type_in, window};
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn print_options_are_reachable_and_cancel_without_changing_shared_read_only_source(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, "α\t中😀\nsource");
        ws.update_in(vcx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
        let editor = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        editor.update(vcx, |e, cx| e.document.update(cx, |d, cx| d.update_file(|d| d.read_only = true, cx)));
        let before = editor
            .read_with(vcx, |e, cx| (e.text(cx), e.doc(cx).version, e.doc(cx).encoding, e.state.selections.clone()));
        vcx.dispatch_action(Print);
        assert!(ws.read_with(vcx, |ws, _| matches!(ws.panel, Some(Panel::Form(_)))));
        vcx.run_until_parked();
        for key in ["wrap", "syntax_colors", "page_setup"] {
            assert!(vcx.debug_bounds(key).is_some(), "missing print option {key}");
        }
        vcx.simulate_keystrokes("escape");
        assert!(ws.read_with(vcx, |ws, _| ws.panel.is_none()));
        let after = editor
            .read_with(vcx, |e, cx| (e.text(cx), e.doc(cx).version, e.doc(cx).encoding, e.state.selections.clone()));
        assert_eq!(before, after);
    }

    #[gpui::test]
    fn print_preparation_preserves_dirty_path_selection_and_undo_after_tab_switch(cx: &mut TestAppContext) {
        let path = super::super::ui_tests::temp("orlo-print-state").join("source.rs");
        std::fs::write(&path, "before").unwrap();
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| ws.open_path(path.clone(), window, cx));
        vcx.run_until_parked();
        type_in(&ws, vcx, "let x = \"😀\";\n");
        let editor = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        let snapshot = ws.read_with(vcx, |ws, cx| ws.print_snapshot(&editor, cx).unwrap());
        let before = editor.read_with(vcx, |e, cx| {
            (e.text(cx), e.doc(cx).path.clone(), e.doc(cx).dirty, e.doc(cx).version, e.state.selections.clone())
        });
        ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
        let prepared = snapshot.prepare(false, Options { syntax_colors: true, ..Options::default() }).unwrap();
        assert_eq!(prepared.text, before.0);
        assert_eq!(
            before,
            editor.read_with(vcx, |e, cx| (
                e.text(cx),
                e.doc(cx).path.clone(),
                e.doc(cx).dirty,
                e.doc(cx).version,
                e.state.selections.clone()
            ))
        );
        editor.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(editor.read_with(vcx, |e, cx| e.text(cx)), "before");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "before");
        assert_eq!(snapshot.source(false, Options::default()).unwrap(), before.0);
    }

    #[gpui::test]
    fn print_invalid_options_stay_in_form_and_stale_target_cancels(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        vcx.dispatch_action(Print);
        vcx.run_until_parked();
        vcx.simulate_input("0");
        vcx.simulate_keystrokes("enter");
        assert!(ws.read_with(vcx, |ws, _| matches!(&ws.panel, Some(Panel::Form(p)) if p.error.is_some())));
        ws.update_in(vcx, |ws, window, cx| {
            ws.new_file(&NewFile, window, cx);
            ws.submit_form(&form::Submit, window, cx);
        });
        assert!(ws.read_with(vcx, |ws, _| ws.panel.is_none()));
    }
    #[gpui::test]
    fn print_refuses_native_composition_in_another_shared_view_without_committing_it(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, "original");
        let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(vcx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
        first.update_in(vcx, |e, window, cx| {
            e.perform_action(&editor::ToggleOvertype, window, cx);
            e.sel(cx).set_cursor(0);
            e.replace_and_mark_text_in_range(None, "界", None, window, cx);
        });
        let before = first.read_with(vcx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
        let refused = ws.read_with(vcx, |ws, cx| ws.print_snapshot(&ws.active_editor().unwrap(), cx).is_err());
        assert!(refused, "another view's provisional composition must never reach print");
        assert_eq!(before, first.read_with(vcx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)));
        first.update_in(vcx, |e, window, cx| {
            let marked = e.marked_text_range(window, cx);
            e.replace_text_in_range(marked, "界", window, cx);
        });
        let snapshot = ws.read_with(vcx, |ws, cx| ws.print_snapshot(&ws.active_editor().unwrap(), cx).unwrap());
        assert_eq!(snapshot.source(false, Options::default()).unwrap(), "界riginal");
        first.update(vcx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(first.read_with(vcx, |e, cx| e.text(cx)), "original");
        assert_eq!(snapshot.source(false, Options::default()).unwrap(), "界riginal", "immutable after undo");
        first.update(vcx, |e, cx| e.edit(cx, |s| s.redo()));
        assert_eq!(first.read_with(vcx, |e, cx| e.text(cx)), "界riginal");
    }

    #[gpui::test]
    fn print_snapshot_waits_for_shared_selection_mapping_and_includes_folded_source(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, "abc\nhidden\nend");
        let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(vcx, |ws, window, cx| ws.new_view_into_file(&NewViewIntoFile, window, cx));
        let second = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        second.update(vcx, |e, cx| {
            e.sel(cx).set_selections(vec![crate::editing::text::Selection::new(4, 10)]);
            e.fold_chars_now(std::iter::once(4..10), cx);
        });
        ws.update(vcx, |ws, cx| {
            first.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.set_cursor(0);
                    s.insert("X");
                })
            });
            assert!(ws.print_snapshot(&second, cx).is_err(), "never combine new text with unmapped old selections");
        });
        vcx.run_until_parked();
        let snapshot = ws.read_with(vcx, |ws, cx| ws.print_snapshot(&second, cx).unwrap());
        assert_eq!(snapshot.source(true, Options::default()).unwrap(), "hidden");
        assert_eq!(snapshot.source(false, Options::default()).unwrap(), "Xabc\nhidden\nend");
        assert!(second.read_with(vcx, |e, _| !e.folds().is_empty()));
    }

    #[gpui::test]
    fn print_refuses_composition_from_another_window(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let path = super::super::ui_tests::temp("orlo-print-shared-ime").join("source.txt");
        std::fs::write(&path, "source").unwrap();
        let (first, second, mut a, mut b) = super::super::ui_tests::two_windows(cx);
        first.update_in(&mut a, |ws, window, cx| ws.open_path(path.clone(), window, cx));
        second.update_in(&mut b, |ws, window, cx| ws.open_path(path.clone(), window, cx));
        let composing = first.read_with(&a, |ws, _| ws.active_editor().unwrap());
        composing.update_in(&mut a, |e, window, cx| e.replace_and_mark_text_in_range(None, "界", None, window, cx));
        assert!(second.read_with(&b, |ws, cx| ws.print_snapshot(&ws.active_editor().unwrap(), cx).is_err()));
        assert_eq!(composing.read_with(&a, |e, cx| e.text(cx)), "界source");
        assert!(!composing.read_with(&a, |e, _| e.marked.is_empty()));
    }
}
