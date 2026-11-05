//! Edit > Document Statistics and Character Info, and the Columns commands that ask first.

use super::*;

impl Workspace {
    pub(crate) fn document_statistics(&mut self, _: &DocumentStatistics, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let s = editor.read(cx).read_state(cx);
        let ranges: Vec<_> = s.view.selections.iter().map(|sel| sel.range()).collect();
        let d = &s.doc;
        let (rope, ending, bom, encoding) = (d.rope.clone(), d.line_ending, d.bom, d.encoding);
        let endings = d.endings.clone();
        let big = rope.len_chars() > stats::BIG;
        let detail = move || {
            let mut result = stats::stats(&rope, &ranges, ending, bom, encoding);
            result.bytes = stats::saved_len_with_endings(&rope, endings.as_ref(), ending, bom, encoding)
                .map_err(|e| e.to_string());
            result.detail()
        };
        if !big {
            return self.inform(window, cx, "Document Statistics", &detail());
        }
        // a rope clone is cheap and the counting is linear: a huge buffer must not freeze the UI
        self.status_message("Counting…", cx);
        let task = cx.background_spawn(async move { detail() });
        cx.spawn_in(window, async move |this, cx| {
            let detail = task.await;
            this.update_in(cx, |ws, window, cx| ws.inform(window, cx, "Document Statistics", &detail)).ok();
        })
        .detach();
    }

    /// Counts and source navigation use the shared map, including protected documents.
    pub(crate) fn inspect_line_endings(&mut self, _: &InspectLineEndings, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let d = editor.read(cx).doc(cx);
        let (id, version, rope, endings, preferred) =
            (d.id(), d.version, d.rope.clone(), d.endings.clone(), d.line_ending);
        let final_newline = rope.len_chars() > 0 && matches!(rope.char(rope.len_chars() - 1), '\n' | '\r');
        if rope.len_chars() > stats::BIG {
            self.status_message("Counting line endings…", cx);
            let task = cx.background_spawn(async move {
                orlo_project::documents::endings::physical_counts(&rope, endings.as_ref(), preferred)
            });
            cx.spawn_in(window, async move |this, cx| {
                let counts = task.await;
                this.update_in(cx, |ws, window, cx| {
                    if ws.active_editor().is_some_and(|e| {
                        let d = e.read(cx).doc(cx);
                        d.id() == id && d.version == version
                    }) {
                        ws.show_line_endings(counts, final_newline, window, cx);
                    }
                })
                .ok();
            })
            .detach();
        } else {
            self.show_line_endings(
                orlo_project::documents::endings::physical_counts(&rope, endings.as_ref(), preferred),
                final_newline,
                window,
                cx,
            );
        }
    }

    fn show_line_endings(
        &mut self,
        counts: [usize; 3],
        final_newline: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = ["\n", "\r\n", "\r"]
            .into_iter()
            .zip(["LF", "CRLF", "CR"])
            .zip(counts)
            .map(|((ending, name), count)| {
                super::pickers::choice(
                    &format!("{name}: {count}"),
                    "Navigate to next boundary (wraps)",
                    ItemAction::Dispatch(Box::new(NextLineEnding { ending })),
                )
            })
            .collect();
        self.open_quick_panel(Kind::Choices, items, window, cx);
        if let Some(panel) = &mut self.quick_panel {
            panel.title = Some(format!("Line endings — final newline: {}", if final_newline { "yes" } else { "no" }));
        }
    }

    pub(crate) fn next_line_ending(&mut self, a: &NextLineEnding, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        editor.update(cx, |e, cx| {
            e.catch_up(cx);
            let at = e.doc(cx).next_ending(a.ending, e.state.cursor());
            if let Some(at) = at {
                e.sel(cx).set_cursor(at);
                e.scroll_to_cursor(cx);
                cx.notify();
            }
        });
    }

    pub(crate) fn character_info(&mut self, _: &CharacterInfo, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let s = editor.read(cx).read_state(cx);
        let head = s.view.selections[s.primary()].head;
        let next = s.next_cluster(head);
        let cluster = (next > head).then(|| s.doc.rope.slice(head..next).to_string());
        let mut detail = stats::char_report(cluster.as_deref(), s.doc.line_ending);
        if let Some(cluster) = &cluster {
            let view = &editor.read(cx).view;
            detail.push_str(&crate::config::settings::unicode::describe_with_ranges(
                cluster,
                &view.unicode_warning_characters,
                &view.unicode_warning_ranges,
                view.unicode_warnings,
            ));
        }
        self.inform(window, cx, "Character Info", &detail);
    }

    pub(crate) fn insert_unicode_character_prompt(
        &mut self,
        _: &InsertUnicodeCharacterPrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.active_editor() else { return };
        if let Err(error) = self.selection_set_admission(&target, cx) {
            return self.status_message(error, cx);
        }
        let captured = target.update(cx, |e, cx| {
            e.catch_up(cx);
            e.character_input_admission(cx)?;
            e.sel(cx).sel_checkpoint();
            let state = e.read_state(cx);
            Ok::<_, String>((
                state.buffer_id(),
                state.doc.version,
                state.selection_state(),
                state.selection_generation(),
            ))
        });
        let (buffer, version, selection, generation) = match captured {
            Ok(captured) => captured,
            Err(error) => return self.status_message(error, cx),
        };
        let done: OnDone = Box::new(move |values, editor, ws, window, cx| {
            editor::unicode_scalar(values.text("text"))?;
            ws.selection_set_admission(&editor, cx)?;
            editor.update(cx, |e, cx| {
                e.catch_up(cx);
                let state = e.read_state(cx);
                if state.buffer_id() != buffer
                    || state.doc.version != version
                    || state.selection_state() != selection
                    || state.selection_generation() != generation
                {
                    return Err("The document or selections changed; reopen Insert Unicode Character".into());
                }
                e.character_input_admission(cx)
            })?;
            ws.execute_command(
                &crate::keybindings::Command {
                    command: "editor::InsertUnicodeCharacter".into(),
                    args: serde_json::json!({"value": values.text("text")}),
                },
                window,
                cx,
            )
        });
        self.show_input("Unicode scalar (U+hex)", "", done, window, cx);
    }

    pub(crate) fn insert_unicode_character(
        &mut self,
        a: &editor::InsertUnicodeCharacter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor() else { return };
        if !editor.read(cx).is_focused(window) {
            return;
        }
        if let Err(error) = self.selection_set_admission(&editor, cx) {
            return self.status_message(error, cx);
        }
        editor.update(cx, |e, cx| e.insert_unicode_character(a, window, cx));
    }

    pub(crate) fn insert_literal_next_key(
        &mut self,
        _: &editor::InsertLiteralNextKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor() else { return };
        if !editor.read(cx).is_focused(window) {
            return;
        }
        if let Err(error) = self.selection_set_admission(&editor, cx) {
            return self.status_message(error, cx);
        }
        let peers = self.open_editors(cx);
        editor.update(cx, |e, cx| {
            e.insert_literal_next_key(&editor::InsertLiteralNextKey, window, cx);
            e.set_literal_input_peers(peers, cx);
        });
    }

    pub(crate) fn insert_literal_text(
        &mut self,
        a: &editor::InsertLiteralText,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.active_editor() else { return };
        if !editor.read(cx).is_focused(window) {
            return;
        }
        if let Err(error) = self.selection_set_admission(&editor, cx) {
            return self.status_message(error, cx);
        }
        editor.update(cx, |e, cx| e.insert_literal_text(a, window, cx));
    }

    pub(crate) fn show_character_viewer(
        &mut self,
        _: &ShowCharacterViewer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_quick_panel(cx);
        self.focus_editor(window, cx);
        // GPUI uses the existing asynchronous orderFrontCharacterPalette: selector on macOS.
        window.show_character_palette();
    }

    /// An information alert (the caution look of `alert` is for failures).
    pub(crate) fn inform(&self, window: &mut Window, cx: &mut Context<Self>, message: &str, detail: &str) {
        drop(window.prompt(PromptLevel::Info, message, Some(detail), &["OK"], cx));
    }

    /// Edit > Columns > Column Editor…: text, or a run of numbers, at every selection.
    pub(crate) fn column_editor(&mut self, _: &ColumnEditor, window: &mut Window, cx: &mut Context<Self>) {
        let fields = vec![
            Field::choice("mode", "Mode", vec!["Text", "Numbers"]),
            Field::text("text", "Text", ""),
            Field::text("initial", "Initial", "1"),
            Field::text("increment", "Increment", "1"),
            Field::text("repeat", "Repeat", "1"),
            Field::choice("base", "Base", vec!["10", "16", "8", "2"]),
            Field::toggle("uppercase", "Uppercase", false),
            Field::text("width", "Pad width", "0"),
            Field::choice("pad", "Pad with", vec!["zeros", "spaces"]),
            Field::toggle("pad_short", "Pad short lines", true),
        ];
        let done: OnDone = Box::new(|v, editor, _, _, cx| {
            let count = editor.read(cx).state.selections.len();
            let texts = match v.choice("mode") {
                0 => vec![v.text("text").to_string(); count],
                _ => columns::sequence(&sequence_of(v)?, count)?,
            };
            let (pad_short, tab_size) = (v.flag("pad_short"), editor.read(cx).doc(cx).tab_size);
            editor.update(cx, |e, cx| e.edit(cx, |s| s.insert_per_selection(&texts, pad_short, tab_size)));
            Ok(())
        });
        self.show_form("Column Editor", fields, done, window, cx);
    }

    /// Edit > Columns > Pad to Column…: spaces at each caret up to a column.
    pub(crate) fn pad_to_column(&mut self, _: &PadToColumn, window: &mut Window, cx: &mut Context<Self>) {
        let done: OnDone = Box::new(|v, editor, _, _, cx| {
            // the number is the column as Sublime counts them, from 1
            let max = columns::MAX_WIDTH;
            let col = match v.text("text").trim() {
                "" => None,
                t => match t.parse::<usize>() {
                    Ok(n) if (1..=max).contains(&n) => Some(n - 1),
                    _ => return Err(format!("Column must be a whole number from 1 to {max}")),
                },
            };
            let tab_size = editor.read(cx).doc(cx).tab_size;
            editor.update(cx, |e, cx| e.edit(cx, |s| s.pad_to_column(col, tab_size)));
            Ok(())
        });
        self.show_input("Pad to column (empty = rightmost caret)", "", done, window, cx);
    }

    /// Edit > Columns > Delimiter: for the active buffer, every view of it.
    pub(crate) fn set_delimiter(&mut self, a: &SetDelimiter, _: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        editor.read(cx).doc(cx).set_delimiter(a.delim);
        self.refresh_menus(cx);
        let name = a.delim.map_or("Automatic", Delim::name);
        self.status_message(format!("Delimiter: {name}"), cx);
    }
}

/// The Column Editor's numbers from its form.
fn sequence_of(v: &FormValues) -> Result<columns::Sequence, String> {
    fn number<T: std::str::FromStr>(v: &FormValues, key: &str, error: &str) -> Result<T, String> {
        v.text(key).trim().parse().map_err(|_| error.to_string())
    }
    Ok(columns::Sequence {
        initial: number(v, "initial", "Initial must be an integer")?,
        step: number(v, "increment", "Increment must be an integer")?,
        repeat: number(v, "repeat", "Repeat must be a whole number from 1")?,
        base: [10, 16, 8, 2][v.choice("base").min(3)],
        upper: v.flag("uppercase"),
        width: number(v, "width", "Pad width must be a whole number from 0")?,
        zero_pad: v.choice("pad") == 0,
    })
}
