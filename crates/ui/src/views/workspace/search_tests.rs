//! GPUI tests of the search, replacement and results workflow.

use super::ui_tests::{find_text, replace_text, text, type_in, window, write_in};
use super::*;
use super::{playground, saved_ui};
use gpui::TestAppContext;

#[gpui::test]
fn lookaround_matches_are_outlined_after_the_background_search(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "AU$10 $20 $30");
    ws.update_in(cx, |ws, window, cx| {
        find_text(ws, r"(?<!AU)\$\d+", true, window, cx);
    });
    cx.run_until_parked();
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.find_panel().unwrap().error, None);
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.find_ranges, vec![6..9, 10..13]);
        assert_eq!(e.state.selections[0].range(), 6..9, "found as typed");
    });
}

#[gpui::test]
fn a_runaway_pattern_is_an_error_in_the_panel_not_zero_matches(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, &format!("{}c", "a".repeat(40)));
    ws.update_in(cx, |ws, window, cx| {
        replace_text(ws, r"^(a|a)*\1b", "x", true, window, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| {
        let error = ws.find_panel().unwrap().error.clone().unwrap_or_default();
        assert!(error.contains("backtracked"), "{error}");
        ws.replace_all(&ReplaceAll, window, cx);
    });
    assert_eq!(text(&ws, cx), format!("{}c", "a".repeat(40)));
}

#[gpui::test]
fn an_invalid_replacement_is_shown_and_replaces_nothing(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a1 a2");
    ws.update_in(cx, |ws, window, cx| {
        replace_text(ws, r"a(\d)", "$2", true, window, cx);
    });
    cx.run_until_parked();
    ws.update_in(cx, |ws, window, cx| {
        let error = ws.find_panel().unwrap().error.clone().unwrap_or_default();
        assert!(error.contains("group 2"), "{error}");
        ws.replace_all(&ReplaceAll, window, cx);
        ws.with_active(cx, |e, cx| e.sel(cx).set_selection(0, 2));
        ws.replace_one(&find_panel::ReplaceOne, window, cx);
    });
    assert_eq!(text(&ws, cx), "a1 a2");
}

#[gpui::test]
fn preserve_case_and_case_escapes_replace_all(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "color Color COLOR");
    ws.update_in(cx, |ws, window, cx| {
        replace_text(ws, "color", "hue", false, window, cx);
        ws.find_panel().unwrap().query.read(cx).focus(window);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-alt-a");
    ws.update_in(cx, |ws, window, cx| {
        assert!(ws.find_panel().unwrap().preserve_case);
        ws.replace_all(&ReplaceAll, window, cx);
    });
    assert_eq!(text(&ws, cx), "hue Hue HUE");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.undo()));
        let p = ws.find_panel_mut().unwrap();
        p.opts.regex = true;
        let r = p.replace.clone();
        r.update(cx, |e, cx| e.set_text(r"\U$0!", cx));
        ws.replace_all(&ReplaceAll, window, cx);
    });
    assert_eq!(text(&ws, cx), "COLOR! COLOR! COLOR!", "explicit case beats Preserve Case");
}

#[gpui::test]
fn the_find_field_takes_line_breaks_and_finds_across_lines(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "end;\nbegin x\nend;\nbegin y");
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(0));
        ws.find(&Find, window, cx);
    });
    cx.run_until_parked();
    // ctrl-enter inserts a line break; enter still finds
    cx.simulate_keystrokes("e n d ; ctrl-enter b e g");
    ws.read_with(cx, |ws, cx| {
        let p = ws.find_panel().unwrap();
        assert_eq!(p.query.read(cx).text(cx), "end;\nbeg");
        assert_eq!(find_panel::field_height(&p.query, cx), 41.);
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.find_ranges, vec![0..8, 13..21]);
    });
    cx.simulate_keystrokes("enter");
    ws.read_with(cx, |ws, cx| {
        let sel = ws.active_editor().unwrap().read(cx).state.selections[0];
        assert_eq!(sel.range(), 13..21, "the next match, over two lines");
    });
    cx.simulate_keystrokes("shift-enter");
    ws.read_with(cx, |ws, cx| {
        let sel = ws.active_editor().unwrap().read(cx).state.selections[0];
        assert_eq!(sel.range(), 0..8);
    });
}

#[gpui::test]
fn up_and_down_move_between_lines_before_recalling_history(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a\nb old");
    ws.update_in(cx, |ws, window, cx| {
        ws.find(&Find, window, cx);
        let q = ws.find_panel().unwrap().query.clone();
        q.update(cx, |e, cx| e.set_text("old", cx));
        ws.find_next(&FindNext, window, cx);
        q.update(cx, |e, cx| {
            e.set_text("a\nb", cx);
            e.sel(cx).set_cursor(3);
        });
    });
    cx.run_until_parked();
    let field = |ws: &Entity<Workspace>, cx: &mut gpui::VisualTestContext| {
        ws.read_with(cx, |ws, cx| {
            let q = ws.find_panel().unwrap().query.read(cx);
            (q.text(cx), q.state.cursor())
        })
    };
    cx.simulate_keystrokes("up");
    assert_eq!(field(&ws, cx), ("a\nb".into(), 1), "to the first line");
    cx.simulate_keystrokes("up");
    assert_eq!(field(&ws, cx).0, "old", "past it: the history");
    cx.simulate_keystrokes("down");
    assert_eq!(field(&ws, cx).0, "a\nb", "and back to the draft");
}

use super::ui_tests::{settle, temp};

/// Run the app, its clock going on 100 ms a step, until `done`.
fn settle_until(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    for _ in 0..600 {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        if done(cx) {
            return;
        }
    }
    panic!("not done after a minute of the app's time");
}
use crate::searching::results::{SearchFlags, Status};
use gpui::VisualTestContext;

/// Find in Files' fields set and Replace… run, until its preview is complete.
fn preview(
    ws: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    where_: &str,
    query: &str,
    rep: &str,
    set: impl FnOnce(&mut FindOpts, &mut SearchFlags),
) {
    ws.update_in(cx, |ws, window, cx| {
        ws.find_in_files(&FindInFiles, window, cx);
        let p = ws.fif_panel_mut().unwrap();
        set(&mut p.opts, &mut p.flags);
        let (q, w, r) = (p.query.clone(), p.where_.clone(), p.replace.clone());
        q.update(cx, |e, cx| e.set_text(query, cx));
        w.update(cx, |e, cx| e.set_text(where_, cx));
        r.update(cx, |e, cx| e.set_text(rep, cx));
        ws.replace_in_files(&ReplaceInFiles, window, cx);
        let error = ws.fif_panel().and_then(|p| p.error.clone());
        assert_eq!(error, None, "the run started");
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.result_set.as_ref().is_some_and(|r| r.done)));
}

fn apply(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
    ws.update_in(cx, |ws, window, cx| ws.apply_replacements(&ApplyReplacements, window, cx));
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
}

fn statuses(ws: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<(String, Option<Status>)> {
    ws.read_with(cx, |ws, _| {
        (ws.result_set.as_ref().unwrap().files.iter())
            .map(|f| (f.path.file_name().unwrap().to_string_lossy().into_owned(), f.status.clone()))
            .collect()
    })
}

#[gpui::test]
fn replace_in_files_previews_then_applies_only_what_is_included(cx: &mut TestAppContext) {
    let dir = temp("orlo-rif-basic");
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "foo Foo\nkeep\nfoo\n").unwrap();
    std::fs::write(dir.join("sub/b.txt"), "foo\r\n").unwrap();
    std::fs::write(dir.join("c.md"), "foo\n").unwrap();
    let (ws, cx) = window(cx);
    let where_ = format!("{}, *.txt", dir.display());
    preview(&ws, cx, &where_, "foo", "bar", |_, f| f.preserve_case = true);
    ws.update_in(cx, |ws, window, cx| {
        let r = ws.result_set.as_ref().unwrap();
        assert_eq!(r.summary(), "4 of 4 matches in 2 of 2 files selected", "c.md is filtered out");
        let reps: Vec<_> = (r.files[0].entries().iter()).map(|e| e.hit.replacement.clone().unwrap()).collect();
        assert_eq!(reps, ["bar", "Bar", "bar"], "the preview, Preserve Case included");
        assert!(ws.results_panel().is_some(), "the browser shows the preview");
        assert!(std::fs::read_to_string(dir.join("a.txt")).unwrap().contains("foo"), "nothing written yet");
        // the second match and the whole of b.txt left out
        let set = ws.result_set.as_mut().unwrap();
        set.toggle_match(0, 1);
        set.toggle_file(1);
        cx.notify();
        let _ = window;
    });
    apply(&ws, cx);
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "bar Foo\nkeep\nbar\n");
    assert_eq!(std::fs::read_to_string(dir.join("sub/b.txt")).unwrap(), "foo\r\n");
    assert_eq!(std::fs::read_to_string(dir.join("c.md")).unwrap(), "foo\n");
    assert_eq!(statuses(&ws, cx), [("a.txt".into(), Some(Status::Changed)), ("b.txt".into(), Some(Status::Excluded))]);
    ws.read_with(cx, |ws, cx| {
        let report = ws.active_editor().unwrap().read(cx);
        assert_eq!(report.title(cx), "Replace Report");
        let text = report.doc(cx).text();
        assert!(text.contains("Changed on disk (1)") && text.contains("Excluded (1)"), "{text}");
    });
}

#[gpui::test]
fn use_buffer_replaces_unsaved_text_in_every_window_as_one_undo_step(cx: &mut TestAppContext) {
    let dir = temp("orlo-rif-buffer");
    let file = write_in(&dir, "a.txt", "old foo\n");
    let (other, ocx) = window(cx);
    other.update_in(ocx, |ws, window, cx| {
        ws.open_path(file.clone(), window, cx);
        ws.with_active(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("unsaved foo ");
            })
        });
    });
    let (ws, wcx) = window(ocx);
    preview(&ws, wcx, &dir.display().to_string(), "foo", "bar", |_, _| {});
    ws.read_with(wcx, |ws, _| {
        assert_eq!(ws.result_set.as_ref().unwrap().counts().matches, 2, "the unsaved text is searched");
    });
    apply(&ws, wcx);
    assert_eq!(statuses(&ws, wcx), [("a.txt".into(), Some(Status::ChangedInBuffer))]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "old foo\n", "the disk is untouched");
    other.update(wcx, |ws, cx| {
        let e = ws.active_editor().unwrap();
        assert_eq!(e.read(cx).doc(cx).text(), "unsaved bar old bar\n");
        assert!(e.read(cx).doc(cx).dirty, "left unsaved for review");
        e.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(e.read(cx).doc(cx).text(), "unsaved foo old foo\n", "one undo step");
    });
}

#[gpui::test]
fn disk_mode_never_overwrites_an_open_buffer_with_unsaved_changes(cx: &mut TestAppContext) {
    let dir = temp("orlo-rif-conflict");
    let file = write_in(&dir, "a.txt", "foo\n");
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.open_path(file.clone(), window, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("draft ")));
    });
    preview(&ws, cx, &dir.display().to_string(), "foo", "bar", |_, f| f.use_buffer = false);
    apply(&ws, cx);
    let st = statuses(&ws, cx);
    assert!(matches!(&st[0].1, Some(Status::Skipped(why)) if why.contains("unsaved changes")), "{st:?}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "foo\n");
    ws.read_with(cx, |ws, cx| {
        let text = ws.views.tabs().find(|t| t.read(cx).doc(cx).path.is_some()).unwrap().read(cx).doc(cx).text();
        assert_eq!(text, "draft foo\n", "the buffer keeps its draft");
    });
}

#[gpui::test]
fn stale_previews_are_reported_and_never_applied(cx: &mut TestAppContext) {
    let dir = temp("orlo-rif-stale");
    let names = ["open.txt", "disk.txt", "later.txt", "edited.txt"];
    let path = |n: &str| dir.join(n);
    for n in names {
        std::fs::write(path(n), "foo\n").unwrap();
    }
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| ws.open_path(path("open.txt"), window, cx));
    preview(&ws, cx, &dir.display().to_string(), "foo", "bar", |_, _| {});
    // after the preview: the open buffer is edited, a file changes on disk, one is opened and
    // left as it was, and one is opened and edited
    ws.update_in(cx, |ws, window, cx| {
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
        std::fs::write(path("disk.txt"), "foo foo\n").unwrap();
        ws.open_path(path("later.txt"), window, cx);
        ws.open_path(path("edited.txt"), window, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("y")));
    });
    settle(cx);
    apply(&ws, cx);
    let st = statuses(&ws, cx);
    let get = |name: &str| st.iter().find(|(n, _)| n == name).unwrap().1.clone().unwrap();
    assert_eq!(get("open.txt"), Status::Stale("edited since the preview".into()));
    assert_eq!(get("disk.txt"), Status::Stale("changed on disk since the preview".into()));
    assert_eq!(get("edited.txt"), Status::Stale("edited in Orlo since the preview".into()));
    assert_eq!(get("later.txt"), Status::ChangedInBuffer, "opened unchanged: still the previewed text");
    assert_eq!(std::fs::read_to_string(path("disk.txt")).unwrap(), "foo foo\n");
    for n in ["later.txt", "edited.txt"] {
        assert_eq!(std::fs::read_to_string(path(n)).unwrap(), "foo\n", "{n} on disk");
    }
}

#[gpui::test]
fn files_that_cannot_be_replaced_are_reported(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp("orlo-rif-hostile");
    std::fs::write(dir.join("latin1.txt"), b"foo caf\xe9\n").unwrap();
    std::fs::write(dir.join("bin.dat"), b"foo\0\n").unwrap();
    let ro = write_in(&dir, "ro.txt", "foo\n");
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o444)).unwrap();
    let (ws, cx) = window(cx);
    preview(&ws, cx, &dir.display().to_string(), "foo", "bar", |_, _| {});
    apply(&ws, cx);
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o644)).unwrap();
    let st = statuses(&ws, cx);
    let get = |n: &str| st.iter().find(|(x, _)| x == n).unwrap().1.clone().unwrap();
    assert_eq!(get("latin1.txt"), Status::Skipped("malformed text; reopen with the correct encoding".into()));
    assert_eq!(get("bin.dat"), Status::Skipped("binary file".into()));
    assert_eq!(get("ro.txt"), Status::Failed("the file is read-only".into()));
    assert_eq!(std::fs::read(dir.join("latin1.txt")).unwrap(), b"foo caf\xe9\n");
    assert_eq!(std::fs::read_to_string(&ro).unwrap(), "foo\n");
}

#[gpui::test]
fn a_cancelled_apply_reports_the_files_it_did_not_reach(cx: &mut TestAppContext) {
    let dir = temp("orlo-rif-cancel");
    for n in ["a.txt", "b.txt"] {
        std::fs::write(dir.join(n), "foo\n").unwrap();
    }
    let (ws, cx) = window(cx);
    preview(&ws, cx, &dir.display().to_string(), "foo", "bar", |_, _| {});
    ws.update_in(cx, |ws, window, cx| {
        ws.apply_replacements(&ApplyReplacements, window, cx);
        ws.cancel_replacements(&CancelReplacements, window, cx);
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.applying.is_none()));
    assert!(statuses(&ws, cx).iter().all(|(_, s)| *s == Some(Status::NotReached)));
    for n in ["a.txt", "b.txt"] {
        assert_eq!(std::fs::read_to_string(dir.join(n)).unwrap(), "foo\n");
    }
}

#[gpui::test]
fn scope_rules_apply_to_every_source(cx: &mut TestAppContext) {
    let dir = temp("orlo-scope-rules");
    std::fs::write(dir.join("a.rs"), "needle\n").unwrap();
    std::fs::write(dir.join("b.txt"), "needle\n").unwrap();
    std::fs::write(dir.join(".gitignore"), "a.rs\n").unwrap();
    let link = std::env::temp_dir().join("orlo-scope-rules-link");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| ws.open_path(dir.join("b.txt"), window, cx));
    let run =
        |ws: &Entity<Workspace>, cx: &mut VisualTestContext, where_: &str, git: bool| -> Result<Vec<String>, String> {
            ws.update_in(cx, |ws, window, cx| {
                ws.find_in_files(&FindInFiles, window, cx);
                let flags = SearchFlags { gitignore: git, ..SearchFlags::default() };
                ws.run_search("needle", where_, None, FindOpts::default(), flags, window, cx)
            })?;
            settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.result_set.as_ref().is_some_and(|r| r.done)));
            Ok(ws.read_with(cx, |ws, _| {
                (ws.result_set.as_ref().unwrap().files.iter())
                    .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
                    .collect()
            }))
        };
    let both = format!("{}, {}", dir.display(), link.display());
    assert_eq!(run(&ws, cx, &both, true).unwrap(), ["b.txt"], "gitignored, and the link's alias once");
    assert_eq!(run(&ws, cx, &both, false).unwrap(), ["a.rs", "b.txt"], "Use gitignore off");
    // globs filter the open files too
    assert!(run(&ws, cx, "<open files>, -*.txt", true).unwrap().is_empty());
    assert_eq!(run(&ws, cx, "<open files>", true).unwrap(), ["b.txt"]);
    // the files of the previous results; a bad glob and an empty scope are errors
    assert_eq!(run(&ws, cx, "<previous results>", true).unwrap(), ["b.txt"]);
    let e = run(&ws, cx, &format!("{}, [a", dir.display()), true).unwrap_err();
    assert!(e.contains("Not a valid pattern in Where: [a"), "{e}");
    ws.update_in(cx, |ws, window, cx| {
        ws.result_set = None;
        ws.find_in_files(&FindInFiles, window, cx);
    });
    let e = run(&ws, cx, "<previous results>", true).unwrap_err();
    assert!(e.contains("no results"), "{e}");
}

#[gpui::test]
fn the_playground_searches_a_copy_and_inspects_captures(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "id=7 x id=42");
    let doc = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(cx, |ws, window, cx| {
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        let p = ws.playground_panel().unwrap();
        p.pattern.update(cx, |e, cx| e.set_text(r"(?<=id=)(?<n>\d+)", cx));
        p.replace.update(cx, |e, cx| e.set_text("[${n}]", cx));
    });
    settle(cx);
    ws.update_in(cx, |ws, window, cx| {
        let sample = ws.active_editor().unwrap();
        assert!(sample.read(cx).playground && sample.read(cx).doc(cx).scratch);
        assert_eq!(sample.read(cx).find_ranges, vec![3..4, 10..12]);
        ws.playground_next(&playground::PlaygroundNext, window, cx);
        assert_eq!(sample.read(cx).state.selections[0].range(), 10..12);
        // the sample is edited: the document it came from is not
        sample.update(cx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("id=1 ");
            })
        });
    });
    settle(cx);
    ws.update_in(cx, |ws, window, cx| {
        let sample = ws.active_editor().unwrap();
        assert_eq!(sample.read(cx).find_ranges.len(), 3, "the search follows the sample");
        assert_eq!(doc.read(cx).doc(cx).text(), "id=7 x id=42", "the document is untouched");
        ws.playground_preview(&playground::PlaygroundPreview, window, cx);
        let preview = ws.active_editor().unwrap().read(cx).doc(cx).text();
        assert_eq!(preview, "id=[1] id=[7] x id=[42]");
        // into Replace, recorded in the find history
        ws.playground_to_replace(&playground::PlaygroundToReplace, window, cx);
        let p = ws.find_panel().unwrap();
        assert_eq!(p.query.read(cx).text(cx), r"(?<=id=)(?<n>\d+)");
        assert_eq!(p.replace.read(cx).text(cx), "[${n}]");
        assert_eq!(ws.query_history.entries()[0], r"(?<=id=)(?<n>\d+)");
    });
}

#[gpui::test]
fn a_playground_result_for_an_older_pattern_is_dropped(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "aaa bbb");
    ws.update_in(cx, |ws, window, cx| {
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        let p = ws.playground_panel().unwrap().pattern.clone();
        p.update(cx, |e, cx| e.set_text("a+", cx));
    });
    // typing on before the pause ends: only the last pattern is searched and shown
    ws.update_in(cx, |ws, _, cx| {
        let p = ws.playground_panel().unwrap().pattern.clone();
        p.update(cx, |e, cx| e.set_text("b+", cx));
    });
    settle(cx);
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.active_editor().unwrap().read(cx).find_ranges, vec![4..7]);
    });
}

#[gpui::test]
fn saved_searches_save_load_import_and_refuse_bad_files(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "doc");
    ws.update_in(cx, |ws, window, cx| {
        ws.replace(&Replace, window, cx);
        let p = ws.find_panel_mut().unwrap();
        p.opts.regex = true;
        p.preserve_case = true;
        let (q, r) = (p.query.clone(), p.replace.clone());
        q.update(cx, |e, cx| e.set_text(r"(\w+)@", cx));
        r.update(cx, |e, cx| e.set_text("$1 at ", cx));
        ws.save_search(&SaveSearch, window, cx);
        assert!(matches!(ws.panel, Some(Panel::Form(_))), "the name is asked for");
    });
    cx.run_until_parked();
    cx.simulate_input("Emails");
    cx.simulate_keystrokes("enter");
    let list = crate::searching::saved::list(&crate::searching::saved::path()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].name.as_str(), list[0].query.as_str(), list[0].regex, list[0].flags.preserve_case),
        ("Emails", r"(\w+)@", true, true)
    );
    // used again: into Replace with its toggles
    ws.update_in(cx, |ws, window, cx| {
        ws.close_panel(&ClosePanel, window, cx);
        ws.use_saved_search(
            &saved_ui::UseSavedSearch { name: "Emails".into(), how: saved_ui::SavedUse::Use },
            window,
            cx,
        );
        let p = ws.find_panel().unwrap();
        assert!(p.show_replace && p.opts.regex && p.preserve_case);
        assert_eq!(p.replace.read(cx).text(cx), "$1 at ");
    });
    // import: a taken name gets a number; a bad file imports nothing
    let dir = temp("orlo-saved-import");
    let good = write_in(
        &dir,
        "good.json",
        r#"{"version": 1, "searches": [{"name": "Emails", "query": "x"}, {"name": "Other", "query": "y", "where": "~/src, *.rs"}]}"#,
    );
    let bad = dir.join("bad.json");
    std::fs::write(&bad, r#"{"version": 1, "searches": [{"name": "B", "query": "(", "regex": true}]}"#).unwrap();
    ws.update_in(cx, |ws, window, cx| {
        ws.import_from(&bad, window, cx);
        ws.import_from(&good, window, cx);
    });
    let names: Vec<String> =
        crate::searching::saved::list(&crate::searching::saved::path()).unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["Emails", "Emails 2", "Other"]);
    // a search with a Where opens Find in Files with it
    ws.update_in(cx, |ws, window, cx| {
        ws.use_saved_search(
            &saved_ui::UseSavedSearch { name: "Other".into(), how: saved_ui::SavedUse::Use },
            window,
            cx,
        );
        assert_eq!(ws.fif_panel().unwrap().where_.read(cx).text(cx), "~/src, *.rs");
    });
}

#[gpui::test]
fn the_history_picker_fills_the_field_it_was_opened_from(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "alpha beta");
    ws.update_in(cx, |ws, window, cx| {
        ws.replace(&Replace, window, cx);
        let p = ws.find_panel().unwrap();
        let (q, r) = (p.query.clone(), p.replace.clone());
        q.update(cx, |e, cx| e.set_text("alpha", cx));
        r.update(cx, |e, cx| e.set_text("omega", cx));
        ws.replace_all(&ReplaceAll, window, cx);
        r.update(cx, |e, cx| e.set_text("", cx));
        r.read(cx).focus(window);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("alt-down");
    ws.update_in(cx, |ws, window, cx| {
        let qp = ws.quick_panel.as_ref().unwrap();
        let labels: Vec<_> = qp.items.iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, ["omega", "Clear Search History"], "the Replace field's history");
        ws.confirm_quick_panel(window, cx);
        assert_eq!(ws.find_panel().unwrap().replace.read(cx).text(cx), "omega");
        ws.clear_search_history(&ClearSearchHistory, window, cx);
        assert!(ws.replace_history.entries().is_empty() && ws.query_history.entries().is_empty());
    });
}

#[gpui::test]
fn extraction_copies_matches_or_lines_and_leaves_the_document(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "a=1 b=2\nnone\nc=3\n");
    ws.update_in(cx, |ws, window, cx| {
        find_text(ws, r"(?<==)\d", true, window, cx);
        let a = |what, to_clipboard, locations| Extract { what, from: ExtractFrom::Document, to_clipboard, locations };
        ws.extract(&a(crate::searching::extract::What::Matches, true, false), window, cx);
    });
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "1\n2\n3\n");
    ws.update_in(cx, |ws, window, cx| {
        let a = Extract {
            what: crate::searching::extract::What::Lines,
            from: ExtractFrom::Document,
            to_clipboard: false,
            locations: true,
        };
        ws.extract(&a, window, cx);
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.title(cx), "Extracted Lines");
        assert_eq!(e.doc(cx).text(), "untitled:1: a=1 b=2\nuntitled:3: c=3\n");
        let history = cx.global::<crate::editing::clipboard_history::ClipboardHistory>().entries().to_vec();
        assert_eq!(history[0].text, "1\n2\n3\n", "the copy is in the clipboard history");
    });
}

#[gpui::test]
fn extraction_from_file_results_reads_each_file_as_searched(cx: &mut TestAppContext) {
    let dir = temp("orlo-extract-results");
    std::fs::write(dir.join("a.txt"), "x TODO one\nok\n").unwrap();
    std::fs::write(dir.join("b.txt"), "TODO two\r\n").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.find_in_files(&FindInFiles, window, cx);
        let w = dir.display().to_string();
        ws.run_search("TODO", &w, None, FindOpts::default(), SearchFlags::default(), window, cx).unwrap();
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.result_set.as_ref().is_some_and(|r| r.done)));
    ws.update_in(cx, |ws, window, cx| {
        let a = Extract {
            what: crate::searching::extract::What::Lines,
            from: ExtractFrom::Results,
            to_clipboard: true,
            locations: true,
        };
        ws.extract(&a, window, cx);
    });
    settle(cx);
    let got = cx.read_from_clipboard().unwrap().text().unwrap();
    // listed under the folder as Where named it
    let a = search::tilde(&dir.join("a.txt"));
    let b = search::tilde(&dir.join("b.txt"));
    assert_eq!(got, format!("{a}:1: x TODO one\n{b}:1: TODO two\n"));
    // a file changed since: nothing is extracted from stale results
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("before".into()));
    ws.update_in(cx, |ws, window, cx| {
        let a = Extract {
            what: crate::searching::extract::What::Matches,
            from: ExtractFrom::Results,
            to_clipboard: true,
            locations: false,
        };
        ws.extract(&a, window, cx);
    });
    settle(cx);
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "before");
}

#[gpui::test]
fn bookmarking_matches_then_deleting_the_lines_is_one_undo_step(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "keep\nERROR a\nkeep\nERROR b\n");
    ws.update_in(cx, |ws, window, cx| {
        find_text(ws, "ERROR", false, window, cx);
        ws.bookmark_matches(&BookmarkMatches, window, cx);
        ws.close_panel(&ClosePanel, window, cx);
        let e = ws.active_editor().unwrap();
        assert_eq!(e.read(cx).read_state(cx).bookmarked_lines(), [1, 3]);
        window.dispatch_action(Box::new(editor::BookmarkedLines { op: editor::MarkedLines::Cut }), cx);
    });
    cx.run_until_parked();
    assert_eq!(text(&ws, cx), "keep\nkeep\n");
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "ERROR a\nERROR b\n");
    ws.update_in(cx, |ws, _, cx| {
        let e = ws.active_editor().unwrap();
        e.update(cx, |e, cx| e.edit(cx, |s| s.undo()));
        assert_eq!(e.read(cx).doc(cx).text(), "keep\nERROR a\nkeep\nERROR b\n");
        assert_eq!(e.read(cx).read_state(cx).bookmarked_lines(), [1, 3]);
    });
}

#[gpui::test]
fn find_matches_take_a_highlight_style_that_outlives_the_panel(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "warn x warn y");
    ws.update_in(cx, |ws, window, cx| {
        ws.find(&Find, window, cx);
        let q = ws.find_panel().unwrap().query.clone();
        q.update(cx, |e, cx| e.set_text("warn", cx));
        ws.highlight_matches(&HighlightMatches { style: 2 }, window, cx);
        q.update(cx, |e, cx| e.set_text("x", cx));
        ws.close_panel(&ClosePanel, window, cx);
        let e = ws.active_editor().unwrap();
        assert_eq!(e.read(cx).state.highlights[2], vec![0..4, 7..11]);
        assert!(e.read(cx).find_ranges.is_empty(), "the search outline went with the panel");
        // the selection's occurrences, without the panel
        e.update(cx, |e, cx| e.sel(cx).set_selection(5, 6));
        ws.highlight_matches(&HighlightMatches { style: 0 }, window, cx);
        assert_eq!(ws.active_editor().unwrap().read(cx).state.highlights[0], vec![5..6]);
    });
}

#[gpui::test]
fn the_browser_filters_without_changing_what_apply_changes(cx: &mut TestAppContext) {
    let dir = temp("orlo-browser-filter");
    std::fs::write(dir.join("a.txt"), "foo one\nfoo two\n").unwrap();
    std::fs::write(dir.join("b.txt"), "foo three\n").unwrap();
    let (ws, cx) = window(cx);
    preview(&ws, cx, &dir.display().to_string(), "foo", "bar", |_, f| f.context = false);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("two");
    ws.update_in(cx, |ws, window, cx| {
        let rows = ws.results_rows(cx);
        assert_eq!(rows, vec![crate::searching::results::Row::File(0), crate::searching::results::Row::Match(0, 1)]);
        let _ = window;
    });
    // F4 walks the rows shown and opens each match
    cx.simulate_keystrokes("f4");
    ws.read_with(cx, |ws, cx| {
        let e = ws.active_editor().unwrap().read(cx);
        assert_eq!(e.doc(cx).path.as_deref().map(|p| p.ends_with("a.txt")), Some(true));
        assert_eq!(e.state.selections[0].range(), 8..11, "the second line's match, selected");
    });
    apply(&ws, cx);
    // a.txt is open now (F4 opened it): changed in its buffer, unsaved; b.txt on disk
    let a = ws.read_with(cx, |ws, cx| {
        let open = ws.views.tabs().find(|t| (t.read(cx).doc(cx).path.as_deref()).is_some_and(|p| p.ends_with("a.txt")));
        open.unwrap().read(cx).doc(cx).text()
    });
    assert_eq!(a, "bar one\nbar two\n");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "foo one\nfoo two\n");
    assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "bar three\n", "filtered out, still included");
}

#[gpui::test]
fn the_side_bar_selection_becomes_where(cx: &mut TestAppContext) {
    let dir = temp("orlo-sidebar-where");
    std::fs::create_dir(dir.join("src")).unwrap();
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        ws.find_in_sidebar_selection(&FindInSidebarSelection, window, cx);
        assert!(ws.fif_panel().is_none(), "nothing marked: nothing opens");
        ws.sidebar.marked = vec![dir.join("src"), dir.join("notes.txt"), dir.join("gone")];
        ws.find_in_sidebar_selection(&FindInSidebarSelection, window, cx);
        let w = ws.fif_panel().unwrap().where_.read(cx).text(cx);
        let want = format!("{}, {}", search::tilde(&dir.join("src")), search::tilde(&dir.join("notes.txt")));
        assert_eq!(w, want, "the rows that still exist");
    });
}

#[gpui::test]
fn reopened_buffer_cannot_reuse_preview_version(cx: &mut TestAppContext) {
    let dir = temp("orlo-reopen-stale");
    let path = write_in(&dir, "a.txt", "foo\n");
    let path = crate::editing::text::resolve(&path);
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| ws.open_path(path.clone(), window, cx));
    preview(&ws, cx, &path.display().to_string(), "foo", "bar", |_, _| {});
    ws.update_in(cx, |ws, window, cx| {
        ws.close_tab(&CloseTab, window, cx);
        std::fs::write(&path, "x foo\n").unwrap();
        ws.open_path(path.clone(), window, cx);
        ws.step_browser(true, window, cx);
        assert!(ws.results_panel().unwrap().note.is_some(), "the location is stale");
    });
    apply(&ws, cx);
    let status = statuses(&ws, cx)[0].1.clone().unwrap();
    let contents = ws.read_with(cx, |ws, cx| {
        ws.views.tabs().find(|e| e.read(cx).doc(cx).path.as_ref() == Some(&path)).unwrap().read(cx).doc(cx).text()
    });
    assert!(matches!(status, Status::Stale(_)), "reopened source was {status:?}; contents {contents:?}");
    assert_eq!(contents, "x foo\n");
}

#[gpui::test]
fn replacements_still_apply_when_a_sibling_view_remains(cx: &mut TestAppContext) {
    let dir = temp("orlo-rif-sibling");
    let path = write_in(&dir, "a.txt", "foo\n");
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| ws.open_path(path.clone(), window, cx));
    preview(&ws, cx, &path.display().to_string(), "foo", "bar", |_, _| {});
    ws.update_in(cx, |ws, window, cx| {
        let original = ws.active_editor().unwrap();
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        let (group, tab) = ws.views.find(&original).unwrap();
        ws.views.select(group, tab);
        ws.close_tab(&CloseTab, window, cx);
    });
    apply(&ws, cx);
    assert_eq!(statuses(&ws, cx)[0].1, Some(Status::ChangedInBuffer));
    ws.read_with(cx, |ws, cx| {
        let e =
            ws.views.tabs().find(|e| e.read(cx).doc(cx).path.as_deref().is_some_and(|p| p.ends_with("a.txt"))).unwrap();
        assert_eq!(e.read(cx).doc(cx).text(), "bar\n");
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "foo\n");
}

#[gpui::test]
fn reopened_buffers_cannot_supply_old_extraction_ranges(cx: &mut TestAppContext) {
    let dir = temp("orlo-extract-reopen");
    let path = write_in(&dir, "a.txt", "foo\n");
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| ws.open_path(path.clone(), window, cx));
    preview(&ws, cx, &path.display().to_string(), "foo", "bar", |_, _| {});
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("before".into()));
    ws.update_in(cx, |ws, window, cx| {
        ws.close_tab(&CloseTab, window, cx);
        std::fs::write(&path, "x foo\n").unwrap();
        ws.open_path(path, window, cx);
        ws.extract(
            &extract_ui::Extract {
                what: crate::searching::extract::What::Matches,
                from: extract_ui::ExtractFrom::Results,
                to_clipboard: true,
                locations: false,
            },
            window,
            cx,
        );
    });
    settle(cx);
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "before");
}

#[gpui::test]
fn deleting_a_saved_search_keeps_changes_from_another_window(cx: &mut TestAppContext) {
    let (other, ocx) = window(cx);
    let (ws, vcx) = window(ocx);
    let initial =
        crate::searching::saved::parse(r#"{"version":1,"searches":[{"name":"Delete me","query":"foo"}]}"#).unwrap();
    crate::searching::saved::write(&crate::searching::saved::path(), &initial).unwrap();
    let dir = temp("orlo-delete-saved");
    let incoming = dir.join("incoming.json");
    std::fs::write(&incoming, r#"{"version":1,"searches":[{"name":"Keep me","query":"bar"}]}"#).unwrap();
    ws.update_in(vcx, |ws, window, cx| {
        ws.use_saved_search(
            &saved_ui::UseSavedSearch { name: "Delete me".into(), how: saved_ui::SavedUse::Delete },
            window,
            cx,
        );
    });
    vcx.run_until_parked();
    // A sheet only blocks its own window. Another window can import while it waits.
    other.update_in(vcx, |ws, window, cx| ws.import_from(&incoming, window, cx));
    assert_eq!(crate::searching::saved::list(&crate::searching::saved::path()).unwrap().len(), 2);
    vcx.simulate_prompt_answer("Delete");
    vcx.run_until_parked();
    let names: Vec<_> =
        crate::searching::saved::list(&crate::searching::saved::path()).unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["Keep me"], "confirming Delete must preserve searches saved after the sheet opened");
}

#[gpui::test]
fn document_paste_preserves_lone_cr_content(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, _, cx| {
        ws.with_active(cx, |e, cx| {
            assert!(!e.single_line);
            e.paste_entry(
                &crate::editing::clipboard_history::ClipEntry { text: "a\rb\nc\n".into(), whole_lines: false },
                cx,
            );
        });
    });
    assert_eq!(text(&ws, cx), "a\rb\nc\n");
}

#[gpui::test]
fn replacing_saved_search_keeps_a_file_that_became_invalid(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let path = crate::searching::saved::path();
    let initial =
        crate::searching::saved::parse(r#"{"version":1,"searches":[{"name":"Mine","query":"old"}]}"#).unwrap();
    crate::searching::saved::write(&path, &initial).unwrap();
    ws.update_in(vcx, |ws, window, cx| {
        find_text(ws, "new", false, window, cx);
        ws.save_search(&SaveSearch, window, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_input("Mine");
    vcx.simulate_keystrokes("enter");
    assert!(vcx.has_pending_prompt());
    std::fs::write(&path, "damaged but recoverable searches").unwrap();
    vcx.simulate_prompt_answer("Replace");
    vcx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "damaged but recoverable searches");
}

#[gpui::test]
fn saved_search_overwrite_recognizes_unicode_case(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    let initial =
        crate::searching::saved::parse(r#"{"version":1,"searches":[{"name":"Éxample","query":"old"}]}"#).unwrap();
    crate::searching::saved::write(&crate::searching::saved::path(), &initial).unwrap();
    ws.update_in(vcx, |ws, window, cx| {
        find_text(ws, "new", false, window, cx);
        ws.save_search(&SaveSearch, window, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_input("éxample");
    vcx.simulate_keystrokes("enter");
    assert!(vcx.has_pending_prompt(), "the same name offers to replace, regardless of Unicode case");
    vcx.simulate_prompt_answer("Replace");
    vcx.run_until_parked();
    let list = crate::searching::saved::list(&crate::searching::saved::path()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].query, "new");
}

#[gpui::test]
fn playground_replace_returns_to_the_copied_document(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "alpha unrelated");
    let unrelated = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, vcx, "alpha source alpha");
    let source = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        let sample = ws.active_editor().unwrap();
        assert_eq!(sample.read(cx).doc(cx).text(), "alpha source alpha");
        let p = ws.playground_panel().unwrap();
        p.pattern.update(cx, |e, cx| e.set_text("alpha", cx));
        p.replace.update(cx, |e, cx| e.set_text("OMEGA", cx));
        ws.playground_to_replace(&playground::PlaygroundToReplace, window, cx);
        assert_eq!(
            ws.active_editor(),
            Some(source.clone()),
            "Use in Replace must return to the document copied into this sample"
        );
        ws.replace_all(&ReplaceAll, window, cx);
        assert_eq!(source.read(cx).doc(cx).text(), "OMEGA source OMEGA");
        assert_eq!(unrelated.read(cx).doc(cx).text(), "alpha unrelated");
        assert_eq!(sample.read(cx).doc(cx).text(), "alpha source alpha");
    });
}

#[gpui::test]
fn playground_origin_survives_panel_reopen_and_group_moves(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "unrelated alpha");
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, vcx, "source alpha");
    let source = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(vcx, |ws, window, cx| {
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        let sample = ws.active_editor().unwrap();
        ws.new_view_into_file(&NewViewIntoFile, window, cx);
        let mirrored = ws.active_editor().unwrap();
        assert!(mirrored.read(cx).playground);
        assert_eq!(
            mirrored.read(cx).playground_source.as_ref().and_then(|origin| origin.upgrade()),
            Some(source.clone())
        );
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        ws.playground_panel().unwrap().pattern.update(cx, |e, cx| e.set_text("alpha", cx));
        ws.playground_to_find(&playground::PlaygroundToFind, window, cx);
        assert_eq!(ws.active_editor(), Some(source.clone()));
        assert_eq!(ws.find_panel().unwrap().query.read(cx).text(cx), "alpha");
        ws.move_to_new_group(&MoveToNewGroup { axis: Axis::Rows }, window, cx);
        assert_eq!(ws.views.find(&source).unwrap().0, 1);
        ws.open_playground(&OpenPlayground, window, cx);
        assert_eq!(ws.active_editor(), Some(sample));
        let p = ws.playground_panel().unwrap();
        p.pattern.update(cx, |e, cx| e.set_text("alpha", cx));
        p.replace.update(cx, |e, cx| e.set_text("OMEGA", cx));
        ws.playground_to_replace(&playground::PlaygroundToReplace, window, cx);
        assert_eq!(ws.active_editor(), Some(source.clone()));
        assert_eq!(ws.views.active, 1);
        ws.replace_all(&ReplaceAll, window, cx);
        assert_eq!(source.read(cx).doc(cx).text(), "source OMEGA");
    });
}

#[gpui::test]
fn playground_refuses_a_closed_origin_but_honors_an_explicit_target(cx: &mut TestAppContext) {
    let (ws, vcx) = window(cx);
    type_in(&ws, vcx, "alpha unrelated");
    let unrelated = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
    ws.update_in(vcx, |ws, window, cx| ws.new_file(&NewFile, window, cx));
    type_in(&ws, vcx, "alpha source");
    ws.update_in(vcx, |ws, window, cx| {
        let source = ws.active_editor().unwrap();
        ws.open_playground_with_document(&OpenPlaygroundWithDocument, window, cx);
        let sample = ws.active_editor().unwrap();
        let p = ws.playground_panel().unwrap();
        p.pattern.update(cx, |e, cx| e.set_text("alpha", cx));
        p.replace.update(cx, |e, cx| e.set_text("OMEGA", cx));
        ws.views.remove(&source);
        for find in [true, false] {
            if find {
                ws.playground_to_find(&playground::PlaygroundToFind, window, cx);
            } else {
                ws.playground_to_replace(&playground::PlaygroundToReplace, window, cx);
            }
            assert_eq!(ws.active_editor(), Some(sample.clone()));
            assert!(ws.playground_panel().is_some());
            assert_eq!(unrelated.read(cx).doc(cx).text(), "alpha unrelated");
            assert!(
                ws.status.as_ref().is_some_and(|(text, _)| text.contains("select a document")),
                "a missing origin must be explained instead of choosing another tab"
            );
        }
        drop(source);
        ws.playground_to_replace(&playground::PlaygroundToReplace, window, cx);
        assert_eq!(ws.active_editor(), Some(sample.clone()));
        ws.playground_to_find_in_files(&playground::PlaygroundToFindInFiles, window, cx);
        assert_eq!(ws.fif_panel().unwrap().query.read(cx).text(cx), "alpha");
        ws.open_playground(&OpenPlayground, window, cx);
        let p = ws.playground_panel().unwrap();
        p.pattern.update(cx, |e, cx| e.set_text("alpha", cx));
        p.replace.update(cx, |e, cx| e.set_text("OMEGA", cx));
        let (g, i) = ws.views.find(&unrelated).unwrap();
        ws.views.select(g, i);
        ws.playground_to_replace(&playground::PlaygroundToReplace, window, cx);
        assert_eq!(ws.active_editor(), Some(unrelated.clone()));
        ws.replace_all(&ReplaceAll, window, cx);
        assert_eq!(unrelated.read(cx).doc(cx).text(), "OMEGA unrelated");
        assert_eq!(sample.read(cx).doc(cx).text(), "alpha source");
    });
}

#[gpui::test]
fn disk_search_uses_known_legacy_codec_even_with_pending_conversion(cx: &mut TestAppContext) {
    use crate::editing::encoding::Encoding;
    let dir = temp("f01-disk-search");
    let path = dir.join("source.txt");
    std::fs::write(&path, b"caf\xe9\r\n").unwrap();
    let path = path.canonicalize().unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, window, cx| {
        let loaded = Loaded::read_encoding(&path, Some(Encoding::Windows1252)).unwrap();
        ws.add_tab(EditorState::from_loaded(&path, loaded), window, cx);
        let editor = ws.active_editor().unwrap();
        editor.update(cx, |e, cx| e.update_doc(cx, |d| d.set_encoding(Encoding::Utf32Be, true)));
    });
    preview(&ws, cx, &path.to_string_lossy(), "café", "été", |_, flags| flags.use_buffer = false);
    ws.read_with(cx, |ws, _| {
        let set = ws.result_set.as_ref().unwrap();
        assert_eq!(set.files.len(), 1);
        assert!(matches!(
            set.files[0].source,
            crate::searching::results::Source::Disk { encoding: Encoding::Windows1252, lossy: false, .. }
        ));
        assert_eq!(set.files[0].entries().len(), 1);
    });
    apply(&ws, cx);
    assert_eq!(std::fs::read(&path).unwrap(), b"caf\xe9\r\n", "pending conversion is unsaved");
    ws.update_in(cx, |ws, window, cx| {
        let e = ws.open_editors(cx).into_iter().find(|e| e.read(cx).doc(cx).path.as_ref() == Some(&path)).unwrap();
        ws.close_editor(&e, window, cx);
    });
    preview(&ws, cx, &path.to_string_lossy(), "café", "été", |_, flags| flags.use_buffer = false);
    apply(&ws, cx);
    assert_eq!(std::fs::read(&path).unwrap(), b"\xe9t\xe9\r\n", "closed named hint retained");
}

#[gpui::test]
fn replace_in_files_rechecks_follow_across_windows_and_aliases_before_either_route(cx: &mut TestAppContext) {
    for use_buffer in [false, true] {
        for before_preview in [false, true] {
            for alias in ["same", "symlink", "hardlink"] {
                let dir = temp(&format!("rif-follow-{use_buffer}-{before_preview}-{alias}"));
                let file = write_in(&dir, "a.txt", "foo\n");
                let followed = if alias == "same" {
                    file.clone()
                } else {
                    let path =
                        temp(&format!("rif-follow-alias-{use_buffer}-{before_preview}-{alias}")).join("alias.txt");
                    if alias == "symlink" {
                        std::os::unix::fs::symlink(&file, &path).unwrap();
                    } else {
                        std::fs::hard_link(&file, &path).unwrap();
                    }
                    path
                };
                let (other, ws, mut a, mut b) = ui_tests::two_windows(cx);
                other.update_in(&mut a, |ws, w, cx| ws.open_path(followed.clone(), w, cx));
                let editor = other.read_with(&a, |ws, _| ws.active_editor().unwrap());
                editor.update(&mut a, |e, cx| {
                    e.edit(cx, |s| s.insert("history "));
                    e.edit(cx, |s| s.undo());
                    assert!(!e.doc(cx).dirty);
                });
                if before_preview {
                    other.update_in(&mut a, |ws, w, cx| ws.follow_log(&FollowLog, w, cx));
                }
                preview(&ws, &mut b, &file.to_string_lossy(), "foo", "bar", |_, flags| flags.use_buffer = use_buffer);
                if !before_preview {
                    other.update_in(&mut a, |ws, w, cx| ws.follow_log(&FollowLog, w, cx));
                }
                let before = editor.read_with(&a, |e, cx| (e.text(cx), e.doc(cx).version));
                apply(&ws, &mut b);
                let statuses = statuses(&ws, &mut b);
                assert!(
                    statuses.iter().all(
                        |(_, status)| matches!(status, Some(Status::Skipped(why)) if why.contains("Stop Following Log"))
                    ),
                    "{statuses:?}"
                );
                assert_eq!(std::fs::read_to_string(&file).unwrap(), "foo\n");
                editor.read_with(&a, |e, cx| assert_eq!((e.text(cx), e.doc(cx).version), before));
                other.update_in(&mut a, |ws, w, cx| ws.stop_following_log(&StopFollowingLog, w, cx));
                editor.update(&mut a, |e, cx| {
                    e.edit(cx, |s| s.redo());
                    assert_eq!(e.text(cx), "history foo\n", "Follow refusal preserves redo history");
                    e.edit(cx, |s| s.undo());
                    assert_eq!(e.text(cx), "foo\n");
                });
                preview(&ws, &mut b, &file.to_string_lossy(), "foo", "bar", |_, flags| flags.use_buffer = use_buffer);
                apply(&ws, &mut b);
                let statuses = self::statuses(&ws, &mut b);
                assert!(
                    statuses
                        .iter()
                        .all(|(_, status)| matches!(status, Some(Status::Changed | Status::ChangedInBuffer))),
                    "{statuses:?}"
                );
                if !use_buffer {
                    assert_eq!(std::fs::read_to_string(&file).unwrap(), "bar\n");
                }
            }
        }
    }
}

#[gpui::test]
fn expanded_f01_disk_find_reports_unsupported_instead_of_substitution_matches(cx: &mut TestAppContext) {
    use crate::searching::results::Body;
    let dir = temp("f01-search-unsupported");
    let path = dir.join("unknown.txt");
    let bytes = b"foo caf\xe9\n";
    std::fs::write(&path, bytes).unwrap();
    let path = path.canonicalize().unwrap();
    let (ws, cx) = window(cx);
    for query in ["foo", "�", "absent"] {
        ws.update_in(cx, |ws, window, cx| {
            ws.find_in_files(&FindInFiles, window, cx);
            let panel = ws.fif_panel_mut().unwrap();
            panel.flags.use_buffer = false;
            panel.query.update(cx, |e, cx| e.set_text(query, cx));
            panel.where_.update(cx, |e, cx| e.set_text(path.to_str().unwrap(), cx));
            ws.run_find_in_files(&RunFindInFiles, window, cx);
        });
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.result_set.as_ref().is_some_and(|r| r.done)));
        ws.read_with(cx, |ws, cx| {
            let results = ws.result_set.as_ref().unwrap();
            assert_eq!(results.files.len(), 1, "unsupported input must remain visible");
            assert!(matches!(&results.files[0].body, Body::Failed(why) if why == "malformed text; reopen with the correct encoding"));
            assert!(results.files[0].entries().is_empty());
            assert!(ws.active_editor().unwrap().read(cx).doc(cx).text().contains("malformed text; reopen with the correct encoding"));
        });
    }
    // The same file becomes searchable with its actual disk codec, even while a
    // different output codec is pending. This is still a real Find, not a replace preview.
    ws.update_in(cx, |ws, window, cx| {
        let loaded = Loaded::read_encoding(&path, Some(crate::editing::encoding::Encoding::Windows1252)).unwrap();
        ws.add_tab(EditorState::from_loaded(&path, loaded), window, cx);
        let editor = ws.active_editor().unwrap();
        editor.update(cx, |e, cx| {
            e.update_doc(cx, |d| d.set_encoding(crate::editing::encoding::Encoding::Utf32Be, true))
        });
        ws.find_in_files(&FindInFiles, window, cx);
        let panel = ws.fif_panel_mut().unwrap();
        panel.flags.use_buffer = false;
        panel.query.update(cx, |e, cx| e.set_text("café", cx));
        panel.where_.update(cx, |e, cx| e.set_text(path.to_str().unwrap(), cx));
        ws.run_find_in_files(&RunFindInFiles, window, cx);
    });
    settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.result_set.as_ref().is_some_and(|r| r.done)));
    ws.read_with(cx, |ws, _| {
        let results = ws.result_set.as_ref().unwrap();
        assert_eq!(results.files.len(), 1);
        assert_eq!(results.files[0].entries().len(), 1);
        assert!(matches!(
            &results.files[0].source,
            crate::searching::results::Source::Disk {
                encoding: crate::editing::encoding::Encoding::Windows1252,
                lossy: false,
                ..
            }
        ));
    });
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[gpui::test]
fn n23_viewer_replace_files_reports_protected_buffer_and_keeps_dirty_text(cx: &mut TestAppContext) {
    let dir = temp("orlo-n23-replace-files").canonicalize().unwrap();
    let path = dir.join("a.txt");
    std::fs::write(&path, "foo😀\n").unwrap();
    let (ws, cx) = window(cx);
    ws.update_in(cx, |ws, w, cx| {
        ws.open_path(path.clone(), w, cx);
        ws.with_active(cx, |e, cx| e.edit(cx, |s| s.insert("dirty ")));
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
    });
    let source = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
    let before = source.read_with(cx, |e, cx| (e.text(cx), e.doc(cx).save_point(), e.doc(cx).unsaved()));
    preview(&ws, cx, &dir.to_string_lossy(), "foo", "BAD", |_, f| f.use_buffer = true);
    apply(&ws, cx);
    assert!(matches!(&statuses(&ws, cx)[0].1, Some(Status::Skipped(_))));
    assert_eq!(source.read_with(cx, |e, cx| (e.text(cx), e.doc(cx).save_point(), e.doc(cx).unsaved())), before);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "foo😀\n");
}

#[gpui::test]
fn n23_viewer_find_reads_and_replace_commands_cannot_mutate(cx: &mut TestAppContext) {
    let (ws, cx) = window(cx);
    type_in(&ws, cx, "foo😀 foo");
    ws.update_in(cx, |ws, w, cx| {
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
        replace_text(ws, "foo", "BAD", false, w, cx);
        ws.replace_all(&ReplaceAll, w, cx);
        ws.replace_one(&find_panel::ReplaceOne, w, cx);
    });
    assert_eq!(text(&ws, cx), "foo😀 foo");
    ws.update_in(cx, |ws, w, cx| {
        ws.toggle_viewer_mode(&ToggleViewerMode, w, cx);
        ws.replace_all(&ReplaceAll, w, cx);
    });
    assert_eq!(text(&ws, cx), "BAD😀 BAD");
}
