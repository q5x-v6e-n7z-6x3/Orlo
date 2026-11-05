//! JSON commands on each nonempty selection, or the document when only carets exist.

use super::*;
use crate::editing::json::{self, Operation};
use crate::editing::text::Selection;
use ropey::Rope;
use std::ops::Range;

actions!(workspace, [FormatJson, MinifyJson, ValidateJson]);

const BIG: usize = 256 * 1024;
const MAX_SELECTIONS: usize = 4096;

#[derive(Clone)]
struct Request {
    view: EntityId,
    buffer: u64,
    version: u64,
    generation: u64,
    command_generation: u64,
    selections: Vec<Selection>,
    rope: Rope,
    ranges: Vec<Range<usize>>,
    bytes: usize,
    operation: Operation,
}

impl Request {
    fn capture(ws: &mut Workspace, operation: Operation, cx: &App) -> Result<Self, String> {
        let editor = ws.active_editor().ok_or("Open a tab to use JSON tools")?;
        let state = editor.read(cx).read_state(cx);
        if operation != Operation::Validate && state.doc.read_only {
            return Err("This document is read-only".into());
        }
        if state.view.selections.len() > MAX_SELECTIONS {
            return Err("JSON tools exceed the 4096-selection limit".into());
        }
        let mut ranges: Vec<_> =
            state.view.selections.iter().filter(|selection| !selection.is_empty()).map(Selection::range).collect();
        if ranges.is_empty() {
            ranges.push(0..state.len());
        }
        let mut bytes = 0;
        for range in &ranges {
            let len = state.doc.rope.slice(range.clone()).len_bytes();
            if len > json::MAX_INPUT - bytes {
                return Err("JSON input exceeds the 16 MiB total limit".into());
            }
            bytes += len;
        }
        ws.json_generation = ws.json_generation.wrapping_add(1);
        Ok(Self {
            view: editor.entity_id(),
            buffer: state.buffer_id(),
            version: state.doc.version,
            generation: ws.restore_gen,
            command_generation: ws.json_generation,
            selections: state.view.selections.clone(),
            rope: state.doc.rope.clone(),
            ranges,
            bytes,
            operation,
        })
    }

    fn run(&self) -> Result<Vec<(Range<usize>, String)>, String> {
        let mut edits = Vec::new();
        let mut bytes = 0;
        for (index, range) in self.ranges.iter().enumerate() {
            let text = self.rope.slice(range.clone()).to_string();
            let output = json::process(&text, self.operation).map_err(|error| {
                let line = self.rope.char_to_line(range.start) + 1;
                format!("Target {} (document line {line}): {error}", index + 1)
            })?;
            if let Some(output) = output {
                if output.len() > json::MAX_OUTPUT - bytes {
                    return Err("JSON output exceeds the 32 MiB total limit".into());
                }
                bytes += output.len();
                edits.push((range.clone(), output));
            }
        }
        Ok(edits)
    }

    fn current(&self, ws: &Workspace, cx: &App) -> bool {
        ws.json_generation == self.command_generation
            && ws.restore_gen == self.generation
            && ws.active_editor().is_some_and(|editor| {
                let state = editor.read(cx).read_state(cx);
                editor.entity_id() == self.view
                    && state.buffer_id() == self.buffer
                    && state.doc.version == self.version
                    && state.view.selections == self.selections
                    && (self.operation == Operation::Validate || !state.doc.read_only)
            })
    }
}

impl Workspace {
    forward_actions! {
    pub(crate) format_json(FormatJson, window, cx) => run_json(Operation::Format, window, cx);
    pub(crate) minify_json(MinifyJson, window, cx) => run_json(Operation::Minify, window, cx);
    pub(crate) validate_json(ValidateJson, window, cx) => run_json(Operation::Validate, window, cx);
    }

    fn run_json(&mut self, operation: Operation, window: &mut Window, cx: &mut Context<Self>) {
        let request = match Request::capture(self, operation, cx) {
            Ok(request) => request,
            Err(error) => return self.alert(window, cx, "JSON tools", &error),
        };
        // Pretty output can be much larger than even a small, deeply nested input.
        if operation != Operation::Format && request.bytes <= BIG && request.ranges.len() <= 128 {
            let result = request.run();
            return self.finish_json(request, result, window, cx);
        }
        self.status_message("JSON: working…", cx);
        let snapshot = request.clone();
        let task = cx.background_spawn(async move { snapshot.run() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, window, cx| ws.finish_json(request, result, window, cx)).ok();
        })
        .detach();
    }

    fn finish_json(
        &mut self,
        request: Request,
        result: Result<Vec<(Range<usize>, String)>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !request.current(self, cx) {
            return self.status_message("JSON result discarded: command, document or selections changed", cx);
        }
        let edits = match result {
            Ok(edits) => edits,
            Err(error) => return self.alert(window, cx, "Invalid JSON", &error),
        };
        if request.operation == Operation::Validate {
            return self.status_message("Valid JSON", cx);
        }
        self.with_active(cx, |editor, cx| {
            editor.edit(cx, |state| {
                state.replace_ranges(edits);
            })
        });
        self.status_message(
            match request.operation {
                Operation::Format => "JSON formatted (two spaces)",
                _ => "JSON minified",
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{settle_until, text, type_in, window};
    use super::*;
    use crate::editing::text::Selection;
    use gpui::{TestAppContext, VisualTestContext};

    fn undo(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
        ws.update(cx, |ws, cx| ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo())));
    }

    #[gpui::test]
    fn json_format_document_is_one_undo_and_shared_views_follow(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, r#"{"z":1,"a":[true,false]}"#);
        ws.update_in(cx, |ws, window, cx| {
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.format_json(&FormatJson, window, cx);
        });
        let want = "{\n  \"z\": 1,\n  \"a\": [\n    true,\n    false\n  ]\n}";
        settle_until(cx, |cx| text(&ws, cx) == want);
        let every_view = |ws: &Entity<Workspace>, cx: &mut VisualTestContext, want: &str| {
            assert_eq!(text(ws, cx), want);
            ws.read_with(cx, |ws, cx| {
                for editor in ws.views.tabs() {
                    assert_eq!(editor.read(cx).doc(cx).text(), want);
                }
            });
        };
        every_view(&ws, cx, want);
        undo(&ws, cx);
        every_view(&ws, cx, r#"{"z":1,"a":[true,false]}"#);
    }

    #[gpui::test]
    fn json_minify_transforms_each_nonempty_selection_and_ignores_carets(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let input = "prefix { \"a\" : 1 } between [ 2, 3 ] suffix";
        type_in(&ws, cx, input);
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| {
                e.sel(cx).set_selections(vec![Selection::caret(0), Selection::new(7, 18), Selection::new(27, 35)])
            });
            ws.minify_json(&MinifyJson, window, cx);
        });
        assert_eq!(text(&ws, cx), "prefix {\"a\":1} between [2,3] suffix");
        undo(&ws, cx);
        assert_eq!(text(&ws, cx), input);
    }

    #[gpui::test]
    fn json_invalid_selection_changes_no_text_and_reports_a_location(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let input = "{ \"a\" : 1 }\n[1,]";
        type_in(&ws, cx, input);
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.sel(cx).set_selections(vec![Selection::new(0, 11), Selection::new(12, 16)]));
            ws.format_json(&FormatJson, window, cx);
        });
        settle_until(cx, |cx| cx.pending_prompt().is_some());
        assert_eq!(text(&ws, cx), input);
        let (_, detail) = cx.pending_prompt().expect("invalid JSON must be reported");
        assert!(detail.contains("line ") && detail.contains("column "), "{detail}");
        cx.simulate_prompt_answer("Ok");
    }

    #[gpui::test]
    fn json_validate_preserves_text_selections_version_and_undo(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let input = " { \"n\" : 1e+999999 } ";
        type_in(&ws, cx, input);
        ws.update_in(cx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            let (version, selections) = {
                let s = editor.read(cx).read_state(cx);
                (s.doc.version, s.view.selections.clone())
            };
            ws.validate_json(&ValidateJson, window, cx);
            assert_eq!(editor.read(cx).doc(cx).version, version);
            assert_eq!(editor.read(cx).state.selections, selections);
        });
        assert_eq!(text(&ws, cx), input);
        assert_eq!(cx.pending_prompt(), None);
        undo(&ws, cx);
        assert_eq!(text(&ws, cx), "");
    }

    #[gpui::test]
    fn json_read_only_document_cannot_be_rewritten(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "[ 1 ]");
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.update_doc(cx, |d| d.read_only = true));
            ws.minify_json(&MinifyJson, window, cx);
        });
        assert_eq!(text(&ws, cx), "[ 1 ]");
        assert!(cx.pending_prompt().is_some());
        cx.simulate_prompt_answer("Ok");
    }

    #[gpui::test]
    fn json_results_reject_changed_text_selections_identity_and_restore_generation(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "[ 1 ]");
        ws.update_in(cx, |ws, window, cx| {
            for change in 0..4 {
                let request = Request::capture(ws, Operation::Minify, cx).unwrap();
                let result = request.run();
                match change {
                    0 => {
                        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert(" ")));
                    }
                    1 => {
                        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(0));
                    }
                    2 => {
                        ws.new_view_into_file(&NewViewIntoFile, window, cx);
                    }
                    _ => {
                        ws.restore_gen += 1;
                    }
                }
                let before = ws.active_editor().unwrap().read(cx).doc(cx).text();
                ws.finish_json(request, result, window, cx);
                assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).text(), before);
            }
        });
    }

    #[gpui::test]
    fn json_newer_noop_command_supersedes_pending_format(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let input = format!("[{}0]", "0,".repeat(150_000));
        assert!(input.len() > BIG);
        type_in(&ws, cx, &input);
        ws.update_in(cx, |ws, window, cx| {
            for latest in [Operation::Minify, Operation::Validate] {
                let pending = Request::capture(ws, Operation::Format, cx).unwrap();
                let result = pending.run();
                let version = ws.active_editor().unwrap().read(cx).doc(cx).version;
                match latest {
                    Operation::Minify => ws.minify_json(&MinifyJson, window, cx),
                    _ => ws.validate_json(&ValidateJson, window, cx),
                }
                // Deliver the older worker result after the newer no-op command started.
                ws.finish_json(pending, result, window, cx);
                assert!(
                    ws.active_editor().unwrap().read(cx).doc(cx).text() == input,
                    "an older Format result replaced the newer {latest:?} command"
                );
                assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).version, version);
            }
        });
    }

    #[gpui::test]
    fn json_large_document_uses_the_background_completion_path(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let input = format!("[ {}0 ]", "0, ".repeat(100_000));
        type_in(&ws, cx, &input);
        ws.update_in(cx, |ws, window, cx| ws.minify_json(&MinifyJson, window, cx));
        assert_eq!(text(&ws, cx), input, "large work must yield before applying");
        let want = format!("[{}0]", "0,".repeat(100_000));
        settle_until(cx, |cx| text(&ws, cx) == want);
        undo(&ws, cx);
        assert_eq!(text(&ws, cx), input);
    }
}
