//! The window's project index: the roots, the views, Orlo's saves and the window's focus
//! keep it current, and the status bar shows its first build and its file limit.

use super::*;

impl Workspace {
    /// The active file's folder (else the last one focused): the index reads the files nearest
    /// it first.
    fn active_dir(&self, cx: &App) -> Option<PathBuf> {
        let path = (self.active_editor()).and_then(|e| e.read(cx).doc(cx).path.clone());
        (path.as_deref().and_then(Path::parent)).map(Path::to_path_buf).or_else(|| self.last_dir.clone())
    }

    /// The index follows the roots: other roots are indexed afresh, the same ones rescanned.
    pub(crate) fn index_roots(&mut self, cx: &mut Context<Self>) {
        let (roots, near) = (self.roots.clone(), self.active_dir(cx));
        self.index.update(cx, |i, cx| i.set_roots(roots, near, cx));
    }

    /// The folders walked again for what changed outside Orlo's open files (the window became
    /// active, or the poll's rescan is due).
    pub(crate) fn index_rescan(&mut self, due_only: bool, cx: &mut Context<Self>) {
        if due_only && !self.index.read(cx).rescan_due(cx) {
            return;
        }
        let near = self.active_dir(cx);
        self.index.update(cx, |i, cx| i.rescan(near, cx));
    }

    /// The status bar's word on the index: its first build's count, and its file limit.
    pub(crate) fn index_status(&self, cx: &App) -> Option<String> {
        let i = self.index.read(cx);
        status(i.progress(), i.limited())
    }
}

/// "Indexing n/m" while the first build runs (n read of m found), and "Index limited to
/// 100,000 files" while the walk stops at its limit.
fn status(progress: Option<(usize, usize)>, limited: Option<usize>) -> Option<String> {
    let building = progress.map(|(n, m)| format!("Indexing {n}/{m}"));
    let limit = crate::searching::pattern::group_digits;
    let limited = limited.map(|n| format!("Index limited to {} files", limit(n)));
    let both: Vec<String> = building.into_iter().chain(limited).collect();
    (!both.is_empty()).then(|| both.join("    "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{settle_rows, settle_until, syntax, temp, window};
    use gpui::{TestAppContext, VisualTestContext};
    use std::time::Duration;

    #[gpui::test]
    fn an_empty_group_still_indexes_nearest_the_last_folder(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.last_dir = Some("/last".into());
            ws.close_editor(&ws.active_editor().unwrap(), window, cx);
            assert!(ws.active_editor().is_none());
            assert_eq!(ws.active_dir(cx), Some("/last".into()));
        });
    }

    /// `path` opened in `ws`'s window: its view.
    fn open(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, path: PathBuf) -> Entity<Editor> {
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(path, window, cx);
            ws.active_editor().unwrap()
        })
    }

    /// `path` opened in `ws`'s window, made the active window: its view.
    fn open_active(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, path: PathBuf) -> Entity<Editor> {
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(path, window, cx);
            window.activate_window();
            ws.active_editor().unwrap()
        })
    }

    /// Every indexed definition's name, sorted.
    fn names(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Vec<String> {
        ws.read_with(vcx, |ws, cx| {
            let index = ws.index.read(cx);
            let mut v: Vec<String> = index.symbols().map(|(_, s)| s.name.clone()).collect();
            v.sort();
            v
        })
    }

    /// A fresh folder holding `files`, opened in a new window, its index built.
    fn project<'a>(
        cx: &'a mut TestAppContext,
        name: &str,
        files: &[(&str, &str)],
    ) -> (Entity<Workspace>, &'a mut VisualTestContext, PathBuf) {
        let dir = temp(&format!("orlo-index-{name}")).canonicalize().unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.clone(), window, cx));
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, cx| ws.index.read(cx).progress().is_none()));
        (ws, vcx, dir)
    }

    #[gpui::test]
    fn index_candidates_flow_to_completion_from_closed_same_syntax_files(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "completion-candidates",
            &[
                ("main.rs", "fn main() { let remote_local = 1;\nrem\n}\n"),
                ("closed.rs", "fn remote_member() {}\n"),
                ("foreign.py", "def remote_foreign(): pass\n"),
            ],
        );
        let editor = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("main.rs"), window, cx);
            let editor = ws.active_editor().unwrap();
            editor.update(cx, |e, cx| {
                sel_of(&e.document, &mut e.state, cx)
                    .set_cursor(doc_of(&e.document, cx).text().find("\nrem").unwrap() + 4);
                cx.notify();
            });
            editor
        });
        editor.update_in(vcx, |e, window, cx| {
            window.activate_window();
            e.focus(window);
            cx.notify();
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("ctrl-space");
        settle_until(vcx, |vcx| editor.read_with(vcx, |e, _| !e.completion_rows().is_empty()));
        let rows = editor.read_with(vcx, |e, _| e.completion_rows());
        assert!(
            rows.iter().any(|r| r.label == "remote_member"
                && matches!(&r.kind, crate::languages::complete::Kind::Symbol(k) if k == "function")),
            "{rows:?}"
        );
        assert!(!rows.iter().any(|r| r.label == "remote_foreign"));
    }

    #[gpui::test]
    fn final_closed_plain_text_words_follow_family_and_index_setting(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "completion-plain-closed",
            &[
                ("main.txt", "remote_local\nremote_\n"),
                ("closed.txt", "remote_identifier\n"),
                ("foreign.py", "def remote_foreign(): pass\n"),
            ],
        );
        let editor = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("main.txt"), window, cx);
            assert_eq!(ws.views.tabs().count(), 1, "word source stays closed");
            ws.active_editor().unwrap()
        });
        let rows = complete_at(&editor, vcx, "\nremote_");
        assert!(
            rows.iter().any(|r| r.label == "remote_identifier" && r.kind == crate::languages::complete::Kind::Word),
            "{rows:?}"
        );
        assert!(!rows.iter().any(|r| r.label == "remote_foreign"));
        vcx.update(|_, cx| {
            let mut settings = cx.global::<Settings>().clone();
            settings.index_files = false;
            cx.set_global(settings);
        });
        let rows = complete_at(&editor, vcx, "\nremote_");
        assert!(!rows.iter().any(|r| r.label == "remote_identifier"), "{rows:?}");
        assert!(rows.iter().any(|r| r.label == "remote_local"));
    }

    #[gpui::test]
    fn final_plain_text_unsaved_buffer_overrides_disk_words(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "completion-plain-unsaved",
            &[("main.txt", "remote_local\nremote_\n"), ("other.txt", "remote_disk\n")],
        );
        let editor = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("other.txt"), window, cx);
            let other = ws.active_editor().unwrap();
            other.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("remote_unsaved\n");
                })
            });
            ws.index.update(cx, |i, cx| i.ensure_fresh(&other, cx)).detach();
            ws.open_path(dir.join("main.txt"), window, cx);
            ws.active_editor().unwrap()
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                ws.index
                    .read(cx)
                    .entries()
                    .any(|e| e.path.as_ref() == Some(&dir.join("other.txt")) && e.idents.contains_key("remote_unsaved"))
            })
        });
        let rows = complete_at(&editor, vcx, "\nremote_");
        assert!(rows.iter().any(|r| r.label == "remote_unsaved"), "{rows:?}");
        assert!(!rows.iter().any(|r| r.label == "remote_disk"), "{rows:?}");
    }

    fn differing_syntax_views<'a>(
        cx: &'a mut TestAppContext,
        name: &str,
    ) -> (Entity<Workspace>, &'a mut VisualTestContext, Entity<Editor>, PathBuf) {
        let (ws, vcx, dir) = project(cx, name, &[("main.txt", "fn remote_function() {}\nremote_\n")]);
        let requester = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("main.txt"), window, cx);
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.set_syntax(&syntax("Rust"), window, cx);
            assert!(!ws.outline.visible);
            ws.active_editor().unwrap()
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                ws.index.read(cx).entry_for(&requester, cx).is_some_and(|e| e.syntax == "Plain Text")
            })
        });
        (ws, vcx, requester, dir)
    }

    #[gpui::test]
    fn final_completion_refreshes_requesting_cloned_views_syntax(cx: &mut TestAppContext) {
        let (ws, vcx, requester, dir) = differing_syntax_views(cx, "completion-cloned-syntax");
        let rows = complete_at(&requester, vcx, "\nremote_");
        assert!(
            rows.iter().any(|r| r.label == "remote_function"
                && matches!(&r.kind, crate::languages::complete::Kind::Symbol(k) if k == "function")
                && r.detail == dir.join("main.txt").display().to_string()),
            "{rows:?}"
        );
        ws.read_with(vcx, |ws, cx| {
            let entry = ws.index.read(cx).entry_for(&requester, cx).unwrap();
            assert_eq!(entry.syntax, "Rust");
            assert_eq!(
                entry.key,
                crate::languages::index::EntryKey::Buffer(requester.entity_id(), requester.read(cx).doc(cx).version)
            );
        });
    }

    #[gpui::test]
    fn final_pending_completion_refresh_expires_on_source_and_syntax_change(cx: &mut TestAppContext) {
        let (ws, vcx, requester, _) = differing_syntax_views(cx, "completion-cloned-stale");
        requester.update_in(vcx, |e, window, cx| {
            sel_of(&e.document, &mut e.state, cx)
                .set_cursor(doc_of(&e.document, cx).text().find("\nremote_").unwrap() + "\nremote_".len());
            window.activate_window();
            e.focus(window);
            cx.notify();
        });
        vcx.run_until_parked();
        let held = HeldWorker::new();
        vcx.simulate_keystrokes("ctrl-space");
        requester.update(vcx, |e, cx| {
            e.edit(cx, |s| {
                s.select_all();
                s.insert("def newer_function(): pass\nnewer_\n");
            });
            e.highlighter = crate::languages::highlight::Highlighter::by_name("Python");
            cx.notify();
        });
        drop(held);
        vcx.run_until_parked();
        assert!(
            requester.read_with(vcx, |e, _| e.completion_waiting().is_none()),
            "superseded Rust refresh published a popup"
        );
        let rows = complete_at(&requester, vcx, "\nnewer_");
        assert!(
            rows.iter().any(|r| r.label == "newer_function"
                && matches!(&r.kind, crate::languages::complete::Kind::Symbol(k) if k == "function")),
            "{rows:?}"
        );
        assert!(!rows.iter().any(|r| r.label == "remote_function"));
        ws.read_with(vcx, |ws, cx| assert_eq!(ws.index.read(cx).entry_for(&requester, cx).unwrap().syntax, "Python"));
    }

    #[gpui::test]
    fn final_pending_completion_refresh_expires_on_source_edit(cx: &mut TestAppContext) {
        let (_, vcx, requester, _) = differing_syntax_views(cx, "completion-cloned-edit");
        requester.update_in(vcx, |e, window, cx| {
            sel_of(&e.document, &mut e.state, cx)
                .set_cursor(doc_of(&e.document, cx).text().find("\nremote_").unwrap() + "\nremote_".len());
            window.activate_window();
            e.focus(window);
            cx.notify();
        });
        vcx.run_until_parked();
        let held = HeldWorker::new();
        vcx.simulate_keystrokes("ctrl-space");
        vcx.simulate_input("f");
        drop(held);
        vcx.run_until_parked();
        assert!(
            requester.read_with(vcx, |e, _| e.completion_waiting().is_none()),
            "the old refresh published after the source version changed"
        );
    }

    fn invalidate_completion_context(cx: &mut TestAppContext, change: &str) {
        let (ws, vcx, dir) = project(
            cx,
            &format!("completion-invalidate-{change}"),
            &[("main.rs", "fn main() { let remote_local = 0;\nrem\n}\n"), ("closed.rs", "fn remote_member() {}\n")],
        );
        let editor = open(&ws, vcx, dir.join("main.rs"));
        assert!(complete_at(&editor, vcx, "\nrem").iter().any(|r| r.label == "remote_member"));
        ws.update_in(vcx, |ws, window, cx| match change {
            "syntax" => ws.set_syntax(&syntax("Python"), window, cx),
            "roots" => {
                ws.roots.clear();
                ws.index_roots(cx);
            }
            "settings" => {
                let mut settings = cx.global::<Settings>().clone();
                settings.index_files = false;
                cx.set_global(settings);
            }
            _ => unreachable!(),
        });
        vcx.run_until_parked();
        assert!(editor.read_with(vcx, |e, _| e.completion_waiting().is_none()), "stale {change} popup survived");
    }

    #[gpui::test]
    fn populated_completion_invalidates_on_syntax_change(cx: &mut TestAppContext) {
        invalidate_completion_context(cx, "syntax");
    }

    #[gpui::test]
    fn populated_completion_invalidates_on_root_change(cx: &mut TestAppContext) {
        invalidate_completion_context(cx, "roots");
    }

    #[gpui::test]
    fn populated_completion_invalidates_on_index_setting_change(cx: &mut TestAppContext) {
        invalidate_completion_context(cx, "settings");
    }

    #[gpui::test]
    fn syntax_family_variants_contribute_indexed_candidates(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "completion-family",
            &[
                ("main.js", "function main() { const remoteLocal = 1;\nrem\n}\n"),
                ("closed.ts", "function remoteTyped() {}\n"),
                ("foreign.rs", "fn remoteForeign() {}\n"),
            ],
        );
        let editor = open(&ws, vcx, dir.join("main.js"));
        let rows = complete_at(&editor, vcx, "\nrem");
        assert!(rows.iter().any(|r| r.label == "remoteTyped"), "{rows:?}");
        assert!(!rows.iter().any(|r| r.label == "remoteForeign"));
    }

    fn complete_at(
        editor: &Entity<Editor>,
        vcx: &mut VisualTestContext,
        needle: &str,
    ) -> Vec<crate::languages::complete::Candidate> {
        editor.update_in(vcx, |e, window, cx| {
            e.close_completion(cx);
            let at = e.text(cx).rfind(needle).unwrap() + needle.len();
            e.sel(cx).set_cursor(at);
            window.activate_window();
            e.focus(window);
            cx.notify();
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("ctrl-space");
        settle_until(vcx, |vcx| editor.read_with(vcx, |e, _| !e.completion_rows().is_empty()));
        editor.read_with(vcx, |e, _| e.completion_rows())
    }

    #[gpui::test]
    fn project_receiver_and_frequency_metadata_reach_the_popup(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "completion-receiver",
            &[
                ("main.rs", "let len = 0;\nobj.le\n"),
                ("closed.rs", "fn length() {}\nfn len() {}\nfn uses() { obj.length(); obj.length(); len; len; }\n"),
            ],
        );
        let editor = open(&ws, vcx, dir.join("main.rs"));
        let rows = complete_at(&editor, vcx, "obj.le");
        assert_eq!(rows[0].label, "length", "{rows:?}");
        editor.update(vcx, |e, cx| e.set_text("let len = 0;\nle\n", cx));
        let rows = complete_at(&editor, vcx, "\nle");
        assert_eq!(rows[0].label, "len", "{rows:?}");
    }

    #[gpui::test]
    fn index_off_keeps_local_contextual_completion_working(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "completion-index-off",
            &[("main.rs", "obj.length(); let len = 0;\nobj.le\n"), ("closed.rs", "fn length_external() {}\n")],
        );
        vcx.update(|_, cx| {
            let mut settings = cx.global::<Settings>().clone();
            settings.index_files = false;
            cx.set_global(settings);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| ws.index.read(cx).disk_entry(&dir.join("closed.rs")).is_none())
        });
        let editor = open(&ws, vcx, dir.join("main.rs"));
        let rows = complete_at(&editor, vcx, "obj.le");
        assert_eq!(rows[0].label, "length", "{rows:?}");
        assert!(!rows.iter().any(|r| r.label == "length_external"));
    }

    #[gpui::test]
    fn commits_are_shared_by_views_in_one_window_and_reset_when_moved(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let first = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        first.update(vcx, |e, cx| e.set_text("alpha bravo\nbr", cx));
        let rows = complete_at(&first, vcx, "\nbr");
        assert_eq!(rows[0].label, "bravo");
        vcx.simulate_keystrokes("tab");
        assert_eq!(first.read_with(vcx, |e, _| e.completion_recent.borrow().front().cloned()), Some("bravo".into()));
        let second = ws.update_in(vcx, |ws, window, cx| {
            ws.new_file(&NewFile, window, cx);
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| e.set_text("alpha bravo\n", cx));
            e
        });
        let rows = complete_at(&second, vcx, "\n");
        assert_eq!(rows[0].label, "bravo", "new view sees window history");
        let (other, _) = vcx.cx.add_window_view(Workspace::new);
        let other_window = other.read_with(vcx, |ws, _| ws.window);
        let mut other_vcx = VisualTestContext::from_window(other_window, &vcx.cx);
        let released = ws.update(vcx, |ws, cx| ws.release_views(std::slice::from_ref(&second), cx));
        other.update_in(&mut other_vcx, |ws, window, cx| {
            ws.adopt_views(released, 0, 0, Some(second.clone()), window, cx)
        });
        let rows = complete_at(&second, &mut other_vcx, "\n");
        assert_eq!(rows[0].label, "alpha", "another window has its own history");
    }

    /// Replace the active view's text and have the index parse it at once.
    fn retype(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, text: &str) {
        ws.update(vcx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert(text);
                })
            });
            ws.index.update(cx, |i, cx| i.ensure_fresh(&e, cx)).detach();
        });
    }

    struct HeldWorker(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl HeldWorker {
        fn new() -> Self {
            use std::sync::{atomic::Ordering, Arc};
            let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (stop, running) = (release.clone(), started.clone());
            let _job = crate::languages::highlight::on_worker_urgent(move || {
                running.store(true, Ordering::Relaxed);
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            while !started.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Self(release)
        }
    }

    impl Drop for HeldWorker {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn ready(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| {
                ws.quick_panel.as_ref().is_some_and(|p| p.loaded && p.rows_task.is_none() && p.filter_task.is_none())
            })
        });
    }

    /// Run the app until the quick panel has loaded `n` rows and filtered them.
    fn ready_rows(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, n: usize) {
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| {
                ws.quick_panel
                    .as_ref()
                    .is_some_and(|p| p.loaded && p.items.len() == n && p.rows_task.is_none() && p.filter_task.is_none())
            })
        });
    }

    fn picker<'a>(
        cx: &'a mut TestAppContext,
        name: &str,
        text: &str,
        keys: &str,
    ) -> (Entity<Workspace>, &'a mut VisualTestContext, Entity<Editor>) {
        let (ws, vcx, dir) = project(cx, name, &[("x.rs", text)]);
        let sibling = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("x.rs"), window, cx);
            let e = ws.active_editor().unwrap();
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            e
        });
        vcx.simulate_keystrokes(keys);
        ready(&ws, vcx);
        (ws, vcx, sibling)
    }

    fn insert_top(e: &Entity<Editor>, vcx: &mut VisualTestContext, text: &str) {
        e.update(vcx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert(text);
            })
        });
        vcx.run_until_parked();
    }

    fn closed(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
    }

    fn selected(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> (String, usize, String) {
        ws.read_with(vcx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            let s = e.read(cx).read_state(cx);
            let line = s.doc.rope.char_to_line(s.cursor());
            (
                s.doc.rope.slice(s.view.selections[s.primary()].range()).to_string(),
                line,
                s.doc.rope.line(line).to_string(),
            )
        })
    }

    fn refreshed_picker(ws: &Entity<Workspace>, target: &Entity<Editor>, vcx: &mut VisualTestContext) {
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                ws.index.read(cx).entry_for(target, cx).is_some_and(|entry| {
                    matches!(entry.key, crate::languages::index::EntryKey::Buffer(_, v) if v == target.read(cx).doc(cx).version)
                        && entry.syntax == target.read(cx).highlighter.syntax_name()
                }) && ws.quick_panel.as_ref().is_none_or(|p| {
                    p.loaded && p.rows_task.is_none() && p.filter_task.is_none()
                })
            })
        });
    }

    fn cross_window_symbol_confirmation(cx: &mut TestAppContext, keys: &str, name: &str) {
        let (ws, vcx, dir) = project(cx, name, &[("x.rs", "fn beta() {} // first\nfn beta() {} // chosen\n")]);
        let (source, target) = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("x.rs"), window, cx);
            let source = ws.active_editor().unwrap();
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            (source, ws.active_editor().unwrap())
        });
        let (other, _) = vcx.cx.add_window_view(Workspace::new);
        let other_window = other.read_with(vcx, |ws, _| ws.window);
        let mut other_vcx = VisualTestContext::from_window(other_window, &vcx.cx);
        let released = ws.update(vcx, |ws, cx| ws.release_views(std::slice::from_ref(&source), cx));
        other.update_in(&mut other_vcx, |ws, window, cx| {
            ws.adopt_views(released, 0, 0, Some(source.clone()), window, cx)
        });
        vcx.simulate_keystrokes(keys);
        ready(&ws, vcx);
        vcx.simulate_keystrokes("down");
        assert!(selected(&ws, vcx).2.contains("chosen"));
        let hold = HeldWorker::new();
        insert_top(&source, &mut other_vcx, "fn beta() {} // inserted\n");
        vcx.run_until_parked();
        target.read_with(vcx, |e, cx| assert_eq!(e.doc(cx).version, source.read(cx).doc(cx).version));
        vcx.simulate_keystrokes("enter");
        drop(hold);
        refreshed_picker(&ws, &target, vcx);
        assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
        let actual = selected(&ws, vcx);
        assert_eq!(actual.0, "beta");
        assert_eq!(actual.1, 2, "wrong definition: {actual:?}");
        assert!(actual.2.contains("chosen"));
    }

    #[gpui::test]
    fn project_symbol_confirmation_follows_edits_in_another_window(cx: &mut TestAppContext) {
        cross_window_symbol_confirmation(cx, "cmd-shift-r", "goto-cross-window-project");
    }

    #[gpui::test]
    fn current_symbol_confirmation_follows_edits_in_another_window(cx: &mut TestAppContext) {
        cross_window_symbol_confirmation(cx, "cmd-r", "goto-cross-window-current");
    }

    fn merged_line_symbol_selection(cx: &mut TestAppContext, keys: &str, name: &str) {
        let (ws, vcx, source) = picker(cx, name, "fn beta() {}\nfn beta() { /* chosen */ }\n", keys);
        vcx.simulate_keystrokes("down");
        assert!(selected(&ws, vcx).2.contains("chosen"));
        let hold = HeldWorker::new();
        source.update(vcx, |e, cx| {
            e.edit(cx, |s| {
                let newline = s.doc.rope.line_to_char(1) - 1;
                s.set_selection(newline, newline + 1);
                s.insert(" ");
            })
        });
        vcx.run_until_parked();
        drop(hold);
        vcx.executor().advance_clock(Duration::from_millis(300));
        refreshed_picker(&ws, &source, vcx);
        ws.read_with(vcx, |ws, _| {
            assert_eq!(ws.quick_panel.as_ref().unwrap().selected, 1);
        });
        assert_eq!(
            ws.read_with(vcx, |ws, cx| {
                let view = ws.active_editor().unwrap();
                let state = view.read(cx).read_state(cx);
                state.view.selections[state.primary()].range().start
            }),
            16,
            "preview must select the second definition"
        );
        vcx.simulate_keystrokes("enter");
        closed(&ws, vcx);
        ws.read_with(vcx, |ws, cx| {
            let view = ws.active_editor().unwrap();
            let state = view.read(cx).read_state(cx);
            let symbols = &ws.index.read(cx).entry_for(&view, cx).unwrap().symbols;
            assert_eq!(symbols.len(), 2);
            let actual = state.view.selections[state.primary()].range();
            assert_eq!(actual, 16..20, "confirmation must select the second definition");
        });
    }

    #[gpui::test]
    fn current_symbol_recovery_keeps_same_line_duplicates(cx: &mut TestAppContext) {
        merged_line_symbol_selection(cx, "cmd-r", "goto-merged-current");
    }

    #[gpui::test]
    fn project_symbol_recovery_keeps_same_line_duplicates(cx: &mut TestAppContext) {
        merged_line_symbol_selection(cx, "cmd-shift-r", "goto-merged-project");
    }

    #[gpui::test]
    fn closing_an_edited_buffer_unsaved_brings_back_the_files_symbols(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "close", &[("x.rs", "fn on_disk() {}\n")]);
        let x = dir.join("x.rs");
        ws.update_in(vcx, |ws, window, cx| ws.open_path(x.clone(), window, cx));
        retype(&ws, vcx, "fn edited() {}\n");
        settle_until(vcx, |vcx| names(&ws, vcx) == ["edited"]);
        ws.update_in(vcx, |ws, window, cx| ws.close_tab(&CloseTab, window, cx));
        vcx.simulate_prompt_answer("Don't Save");
        settle_until(vcx, |vcx| names(&ws, vcx) == ["on_disk"]);
        // no folders, no folder files: the index starts again with none
        ws.update_in(vcx, |ws, window, cx| ws.remove_folders(&RemoveFolders, window, cx));
        assert_eq!(names(&ws, vcx), Vec::<String>::new());
    }

    #[gpui::test]
    fn goto_symbol_in_project_lists_previews_and_opens(cx: &mut TestAppContext) {
        let (ws, vcx, dir) =
            project(cx, "goto-symbol", &[("a.rs", "\nfn alpha() {}\n"), ("b.rs", "\n\nstruct Beta;\n")]);
        let origin = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        vcx.simulate_keystrokes("cmd-shift-r");
        settle_rows(&ws, vcx, 2);
        ws.read_with(vcx, |ws, cx| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert!(p.kind == Kind::ProjectSymbols);
            let rows: Vec<_> =
                p.items.iter().map(|i| (i.label.as_str(), i.detail.as_str(), i.badge.map(|b| b.0))).collect();
            assert_eq!(rows, [("alpha", "a.rs:2", Some('f')), ("Beta", "b.rs:3", Some('s'))]);
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).doc(cx).path.as_ref(), Some(&dir.join("a.rs")), "the initial row previews too");
        });
        vcx.simulate_keystrokes("down");
        ws.read_with(vcx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).doc(cx).path.as_ref(), Some(&dir.join("b.rs")));
            assert_eq!(e.read(cx).doc(cx).rope.char_to_line(e.read(cx).state.cursor()), 2);
            assert!(ws.views.is_preview(&e));
        });
        vcx.simulate_keystrokes("escape");
        ws.read_with(vcx, |ws, _| {
            assert!(ws.quick_panel.is_none());
            assert_eq!(ws.active_editor(), Some(origin.clone()));
            assert_eq!(ws.views.tabs().cloned().collect::<Vec<_>>(), [origin]);
        });
        vcx.simulate_keystrokes("cmd-shift-r");
        settle_rows(&ws, vcx, 2);
        vcx.simulate_input("Beta");
        vcx.simulate_keystrokes("enter");
        ws.read_with(vcx, |ws, cx| {
            assert!(ws.quick_panel.is_none());
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).doc(cx).path.as_ref(), Some(&dir.join("b.rs")));
            assert_eq!(e.read(cx).doc(cx).rope.char_to_line(e.read(cx).state.cursor()), 2);
            assert!(!ws.views.is_preview(&e), "Enter keeps the tab");
        });
    }

    #[gpui::test]
    fn project_symbol_escape_restores_existing_views_preview_and_side_by_side_tabs(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "goto-restore",
            &[("a.rs", "\nfn alpha() {}\n"), ("b.rs", "\nfn beta() {}\n"), ("c.rs", "fn gamma() {}\n")],
        );
        let (a, b, preview, selected) = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            let a = ws.active_editor().unwrap();
            a.update(cx, |e, cx| e.sel(cx).set_selection(1, 3));
            ws.open_path(dir.join("b.rs"), window, cx);
            let b = ws.active_editor().unwrap();
            b.update(cx, |e, cx| e.sel(cx).set_selection(2, 4));
            ws.open_preview(dir.join("c.rs"), window, cx);
            let preview = ws.active_editor().unwrap();
            ws.select_tab(&SelectTab { index: 0 }, window, cx);
            ws.add_tab_to_selection(&AddTabToSelection { forward: true }, window, cx);
            let selected = ws.views.groups[0].selected.clone();
            (a, b, preview, selected)
        });
        vcx.simulate_keystrokes("cmd-shift-r");
        settle_rows(&ws, vcx, 3);
        for (name, target) in [("alpha", &a), ("gamma", &preview), ("beta", &b)] {
            ws.update_in(vcx, |ws, window, cx| {
                let input = ws.quick_panel.as_ref().unwrap().input.clone();
                input.update(cx, |e, cx| e.set_text(name, cx));
                ws.quick_panel_input_changed(window, cx);
            });
            settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.active_editor().as_ref() == Some(target)));
        }
        vcx.simulate_keystrokes("escape");
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(ws.active_editor(), Some(b.clone()));
            assert_eq!(ws.views.groups[0].tabs, [a.clone(), b.clone(), preview.clone()]);
            assert_eq!(ws.views.groups[0].selected, selected);
            assert_eq!(ws.views.groups[0].preview, Some(preview));
            assert_eq!(a.read(cx).state.selections, [crate::editing::text::Selection::new(1, 3)]);
            assert_eq!(b.read(cx).state.selections, [crate::editing::text::Selection::new(2, 4)]);
        });
    }

    #[gpui::test]
    fn project_symbols_fill_while_the_first_index_build_finishes(cx: &mut TestAppContext) {
        let dir = temp("orlo-project-symbol-building");
        for n in 0..10 {
            std::fs::write(dir.join(format!("f{n}.rs")), format!("fn f{n}() {{}}\n")).unwrap();
        }
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir, window, cx);
            ws.goto_symbol_in_project(&GotoSymbolInProject, window, cx);
        });
        // A preview's buffer parse can outlive the folder build, then queue the final rows.
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                ws.index.read(cx).progress().is_none()
                    && ws
                        .quick_panel
                        .as_ref()
                        .is_some_and(|p| p.items.len() == 10 && p.rows_task.is_none() && p.filter_task.is_none())
            })
        });
        ws.read_with(vcx, |ws, _| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert_eq!(p.items.len(), 10);
            assert_eq!(
                p.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
                ["f0", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9"]
            );
        });
    }

    #[gpui::test]
    fn an_unsaved_project_symbol_preview_survives_dismissal(cx: &mut TestAppContext) {
        let (ws, vcx, _) = project(cx, "goto-dirty-preview", &[("x.rs", "fn before() {}\n")]);
        vcx.simulate_keystrokes("cmd-shift-r");
        settle_rows(&ws, vcx, 1);
        ws.update_in(vcx, |ws, window, cx| {
            let preview = ws.active_editor().unwrap();
            preview.update(cx, |e, cx| e.edit(cx, |s| s.insert("edited")));
            ws.quick_panel_dismiss(&quick_panel::Dismiss, window, cx);
            assert!(ws.views.find(&preview).is_some(), "unsaved text must never be discarded with a preview");
            assert!(preview.read(cx).doc(cx).dirty);
            assert!(!ws.views.is_preview(&preview));
        });
    }

    #[gpui::test]
    fn clicking_a_project_symbol_preview_keeps_the_tab(cx: &mut TestAppContext) {
        let (ws, vcx, _) = project(cx, "goto-preview-click", &[("x.rs", "fn shown() {}\n")]);
        vcx.simulate_keystrokes("cmd-shift-r");
        settle_rows(&ws, vcx, 1);
        ws.update_in(vcx, |ws, window, cx| {
            let preview = ws.active_editor().unwrap();
            ws.show_view(&preview, window, cx);
            assert!(ws.quick_panel.is_none());
            assert_eq!(ws.active_editor(), Some(preview));
        });
    }

    #[gpui::test]
    fn project_symbol_previews_keep_blank_sibling_views_synchronized(cx: &mut TestAppContext) {
        let (ws, vcx, _) = project(cx, "goto-blank-siblings", &[("x.rs", "fn disk() {}\n")]);
        let (first, second) = ws.update_in(vcx, |ws, window, cx| {
            ws.set_syntax(&syntax("Rust"), window, cx);
            let first = ws.active_editor().unwrap();
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            (first, ws.active_editor().unwrap())
        });
        vcx.simulate_keystrokes("cmd-shift-r");
        settle_rows(&ws, vcx, 1);
        first.update(vcx, |e, cx| e.edit(cx, |s| s.insert("fn draft() {}\n")));
        vcx.run_until_parked();
        vcx.simulate_keystrokes("escape");
        ws.read_with(vcx, |ws, cx| {
            for e in [&first, &second] {
                assert!(ws.views.find(e).is_some());
                assert_eq!(e.read(cx).doc(cx).text(), "fn draft() {}\n");
                assert!(e.read(cx).doc(cx).dirty);
            }
        });
    }

    #[gpui::test]
    fn project_symbol_enter_waits_for_the_latest_query(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "goto-fast-enter", &[("a.rs", "fn alpha() {}\n"), ("b.rs", "fn beta() {}\n")]);
        vcx.simulate_keystrokes("cmd-shift-r");
        settle_rows(&ws, vcx, 2);
        ws.update_in(vcx, |ws, window, cx| {
            let input = ws.quick_panel.as_ref().unwrap().input.clone();
            input.update(cx, |e, cx| e.set_text("beta", cx));
            ws.quick_panel_input_changed(window, cx);
            ws.confirm_quick_panel(window, cx);
        });
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref(), Some(&dir.join("b.rs")))
        });
    }

    #[gpui::test]
    fn project_symbols_include_untitled_buffers_and_restore_the_original_group(cx: &mut TestAppContext) {
        let (ws, vcx) = window(cx);
        let (draft, origin) = ws.update_in(vcx, |ws, window, cx| {
            let draft = ws.active_editor().unwrap();
            draft.update(cx, |e, cx| e.edit(cx, |s| s.insert("\nfn draft() {}\n")));
            ws.set_syntax(&syntax("Rust"), window, cx);
            ws.set_layout(&SetLayout { preset: Preset::Columns(2) }, window, cx);
            ws.new_file(&NewFile, window, cx);
            (draft, ws.active_editor().unwrap())
        });
        for confirm in [false, true] {
            vcx.simulate_keystrokes("cmd-shift-r");
            settle_rows(&ws, vcx, 1);
            ws.read_with(vcx, |ws, _| {
                let p = ws.quick_panel.as_ref().unwrap();
                assert_eq!(p.items[0].label, "draft");
                assert!(matches!(&p.items[0].action, ItemAction::Location(quick_panel::Place::View(id), 1, _) if *id == draft.entity_id()));
                assert_eq!(ws.active_editor(), Some(draft.clone()));
            });
            vcx.simulate_keystrokes(if confirm { "enter" } else { "escape" });
            ws.read_with(vcx, |ws, _| {
                assert_eq!(ws.views.active, usize::from(!confirm));
                assert_eq!(ws.active_editor(), Some(if confirm { draft.clone() } else { origin.clone() }));
            });
        }
    }

    #[gpui::test]
    fn an_orlo_save_gives_the_file_the_saved_texts_symbols_at_once(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "save", &[("x.rs", "fn before() {}\n")]);
        let x = dir.join("x.rs");
        ws.update_in(vcx, |ws, window, cx| ws.open_path(x.clone(), window, cx));
        retype(&ws, vcx, "fn after() {}\n");
        settle_until(vcx, |vcx| names(&ws, vcx) == ["after"]);
        ws.update_in(vcx, |ws, window, cx| {
            ws.save(&Save, window, cx);
            // clean: closed without a prompt, and the file's entry shows
            ws.close_tab(&CloseTab, window, cx);
        });
        assert_eq!(names(&ws, vcx), ["after"], "no wait for a rescan or a parse");
        // saved before the edit was parsed: the file is read again first
        ws.update_in(vcx, |ws, window, cx| ws.open_path(x.clone(), window, cx));
        ws.update_in(vcx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("fn again() {}\n");
                })
            });
            ws.save(&Save, window, cx);
            ws.close_tab(&CloseTab, window, cx);
        });
        settle_until(vcx, |vcx| names(&ws, vcx) == ["again"]);
    }

    #[gpui::test]
    fn a_save_as_onto_another_folder_file_gives_it_the_buffers_symbols(cx: &mut TestAppContext) {
        let files = [("x.rs", "fn first() {}\n"), ("z.rs", "fn zed() {}\n")];
        let (ws, vcx, dir) = project(cx, "save-as", &files);
        let z = dir.join("z.rs");
        ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.join("x.rs"), window, cx));
        retype(&ws, vcx, "fn moved() {}\n");
        settle_until(vcx, |vcx| names(&ws, vcx) == ["moved", "zed"]);
        ws.update_in(vcx, |ws, window, cx| ws.save_as(&SaveAs, window, cx));
        vcx.simulate_new_path_selection(|_| Some(z.clone()));
        vcx.run_until_parked();
        ws.update_in(vcx, |ws, window, cx| ws.close_tab(&CloseTab, window, cx));
        ws.read_with(vcx, |ws, cx| {
            let index = ws.index.read(cx);
            let (entry, _) = index.definitions("moved")[0];
            assert_eq!(entry.path.as_deref(), Some(z.as_path()));
            assert!(index.definitions("zed").is_empty());
            assert_eq!(index.definitions("first").len(), 1, "x.rs is as it was");
        });
    }

    #[gpui::test]
    fn the_window_becoming_active_rescans_its_folders(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "activate", &[("a.rs", "fn a() {}\n")]);
        std::fs::create_dir_all(dir.join("never/shown")).unwrap();
        std::fs::write(dir.join("never/shown/b.rs"), "fn b() {}\n").unwrap();
        vcx.update(|window, _| window.activate_window());
        settle_until(vcx, |vcx| names(&ws, vcx) == ["a", "b"]);
    }

    #[gpui::test]
    fn an_active_window_rescans_every_10_seconds(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "tick", &[("a.rs", "fn a() {}\n")]);
        vcx.update(|window, _| window.activate_window());
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, cx| ws.index.read(cx).progress().is_none()));
        std::fs::write(dir.join("b.rs"), "fn b() {}\n").unwrap();
        // the poll comes every 500 ms; the rescan only once 10 s have passed since the last
        for _ in 0..18 {
            vcx.executor().advance_clock(Duration::from_millis(500));
            vcx.run_until_parked();
        }
        assert_eq!(names(&ws, vcx), ["a"]);
        for _ in 0..4 {
            vcx.executor().advance_clock(Duration::from_millis(500));
            vcx.run_until_parked();
        }
        settle_until(vcx, |vcx| names(&ws, vcx) == ["a", "b"]);
    }

    #[gpui::test]
    fn the_status_bar_counts_the_first_build_and_names_the_limit(cx: &mut TestAppContext) {
        let dir = temp("orlo-index-status").canonicalize().unwrap();
        for f in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(dir.join(f), "fn x() {}\n").unwrap();
        }
        let (ws, vcx) = window(cx);
        ws.update(vcx, |ws, cx| ws.index.update(cx, |i, _| i.set_limit(2)));
        ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.clone(), window, cx));
        let status =
            |ws: &Entity<Workspace>, vcx: &mut VisualTestContext| ws.read_with(vcx, |ws, cx| ws.index_status(cx));
        assert_eq!(status(&ws, vcx).as_deref(), Some("Indexing 0/0"), "walking");
        settle_until(vcx, |vcx| status(&ws, vcx).as_deref() == Some("Index limited to 2 files"));
    }

    #[gpui::test]
    fn project_no_match_enter_restores_editor_focus(cx: &mut TestAppContext) {
        let (ws, vcx, _) = project(cx, "goto-no-match", &[("x.rs", "fn alpha() {}\n")]);
        let origin = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        vcx.simulate_keystrokes("cmd-shift-r");
        ready_rows(&ws, vcx, 1);
        vcx.simulate_input("no_such_symbol");
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().unwrap().filter_task.is_none()));
        vcx.simulate_keystrokes("enter");
        ws.update_in(vcx, |ws, window, cx| {
            assert!(ws.quick_panel.is_none());
            assert_eq!(ws.active_editor(), Some(origin.clone()));
            assert_eq!(
                window.focused(cx),
                Some(origin.read(cx).focus_handle(cx)),
                "Enter must return keys to the restored origin"
            );
        });
    }

    #[gpui::test]
    fn project_enter_keeps_explicit_blank_sibling_tabs(cx: &mut TestAppContext) {
        let (ws, vcx, _) = project(cx, "goto-blank-confirm", &[("x.rs", "fn alpha() {}\n")]);
        let (first, second) = ws.update_in(vcx, |ws, window, cx| {
            let first = ws.active_editor().unwrap();
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            (first, ws.active_editor().unwrap())
        });
        vcx.simulate_keystrokes("cmd-shift-r");
        ready_rows(&ws, vcx, 1);
        vcx.simulate_keystrokes("enter");
        ws.read_with(vcx, |ws, _| {
            assert!(ws.quick_panel.is_none());
            assert!(ws.views.find(&first).is_some());
            assert!(ws.views.find(&second).is_some(), "opening a target must not close an explicit sibling view");
        });
    }

    #[gpui::test]
    fn current_symbol_selection_survives_other_buffer_index_updates(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "goto-unrelated-index",
            &[("a.rs", "fn alpha() {}\nfn beta() {}\n"), ("b.rs", "fn other() {}\n")],
        );
        let other = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("b.rs"), window, cx);
            let other = ws.active_editor().unwrap();
            ws.open_path(dir.join("a.rs"), window, cx);
            other
        });
        vcx.simulate_keystrokes("cmd-r");
        ready_rows(&ws, vcx, 2);
        vcx.simulate_keystrokes("down");
        assert_eq!(ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().unwrap().selected), 1);
        other.update(vcx, |e, cx| e.edit(cx, |s| s.insert("\n")));
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(300));
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                let fresh = ws.index.read(cx).entry_for(&other, cx).is_some_and(|e| {
                    matches!(e.key, crate::languages::index::EntryKey::Buffer(_, v) if v == other.read(cx).doc(cx).version)
                });
                fresh && ws.quick_panel.as_ref().is_some_and(|p| {
                    p.rows_task.is_none() && p.filter_task.is_none()
                })
            })
        });
        assert_eq!(
            ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().unwrap().selected),
            1,
            "unrelated index work must not change the highlighted symbol"
        );
        vcx.simulate_keystrokes("enter");
        ws.read_with(vcx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            let s = e.read(cx).read_state(cx);
            assert_eq!(s.doc.rope.slice(s.view.selections[s.primary()].range()), "beta");
        });
    }

    #[gpui::test]
    fn project_enter_refreshes_a_target_edited_through_a_sibling(cx: &mut TestAppContext) {
        for first in ["alpha", "beta"] {
            let text = format!("fn {first}() {{}}\nfn beta() {{}}\n");
            let (ws, vcx, dir) = project(cx, &format!("goto-stale-target-{first}"), &[("x.rs", &text)]);
            let (sibling, target) = ws.update_in(vcx, |ws, window, cx| {
                ws.open_path(dir.join("x.rs"), window, cx);
                let sibling = ws.active_editor().unwrap();
                ws.new_view_into_file(&NewViewIntoFile, window, cx);
                (sibling, ws.active_editor().unwrap())
            });
            vcx.simulate_keystrokes("cmd-shift-r");
            ready_rows(&ws, vcx, 2);
            vcx.simulate_keystrokes("down");
            sibling.update(vcx, |e, cx| {
                e.edit(cx, |s| {
                    s.set_cursor(0);
                    s.insert("\n\n\n\n\n\n\n\n\n\n");
                })
            });
            vcx.run_until_parked();
            assert_eq!(
                sibling.read_with(vcx, |e, cx| e.doc(cx).version),
                target.read_with(vcx, |e, cx| e.doc(cx).version)
            );
            vcx.simulate_keystrokes("enter");
            settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
            ws.read_with(vcx, |ws, cx| {
                let target = ws.active_editor().unwrap();
                let s = target.read(cx).read_state(cx);
                let selected = s.doc.rope.slice(s.view.selections[s.primary()].range()).to_string();
                assert_eq!(selected, "beta", "Enter must navigate using the edited buffer's symbol position");
                assert_eq!(s.doc.rope.char_to_line(s.cursor()), 11, "the chosen occurrence must stay selected");
            });
        }
    }

    #[gpui::test]
    fn symbol_mode_reentry_waits_for_the_new_parse(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "goto-symbol-reentry", &[("x.rs", "fn alpha() {}\n")]);
        ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.join("x.rs"), window, cx));
        vcx.simulate_keystrokes("cmd-r");
        ready_rows(&ws, vcx, 1);
        ws.update_in(vcx, |ws, window, cx| {
            let input = ws.quick_panel.as_ref().unwrap().input.clone();
            input.update(cx, |e, cx| e.set_text("", cx));
            ws.quick_panel_input_changed(window, cx);
        });
        let e = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        let hold = HeldWorker::new();
        e.update(vcx, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(0);
                s.insert("fn beta() {}\n");
            })
        });
        vcx.run_until_parked();
        ws.update_in(vcx, |ws, window, cx| {
            let input = ws.quick_panel.as_ref().unwrap().input.clone();
            input.update(cx, |e, cx| e.set_text("@beta", cx));
            ws.quick_panel_input_changed(window, cx);
            ws.confirm_quick_panel(window, cx);
        });
        vcx.run_until_parked();
        let waiting = ws.read_with(vcx, |ws, _| ws.quick_panel.is_some());
        drop(hold);
        assert!(waiting, "the picker closed before the new symbol parse could run");
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
        let selected = e.read_with(vcx, |e, cx| {
            let s = e.read_state(cx);
            s.doc.rope.slice(s.view.selections[s.primary()].range()).to_string()
        });
        assert_eq!(selected, "beta", "reentering symbol mode must wait for the new rows");
    }

    fn duplicate_insertion(cx: &mut TestAppContext, keys: &str, name: &str) {
        let (ws, vcx, sibling) = picker(cx, name, "fn beta() {} // first\nfn beta() {} // chosen\n", keys);
        vcx.simulate_keystrokes("down");
        assert!(selected(&ws, vcx).2.contains("chosen"));
        let hold = HeldWorker::new();
        insert_top(&sibling, vcx, "fn beta() {} // inserted\n");
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_some()));
        drop(hold);
        closed(&ws, vcx);
        let actual = selected(&ws, vcx);
        assert_eq!(actual.1, 2, "wrong definition: {actual:?}");
        assert!(actual.2.contains("chosen"), "wrong definition: {actual:?}");
    }

    #[gpui::test]
    fn project_symbol_selection_survives_a_rebuild_before_its_buffer_is_parsed(cx: &mut TestAppContext) {
        for pending in [false, true] {
            let (ws, vcx, sibling) = picker(
                cx,
                &format!("goto-stale-rebuild-{pending}"),
                "fn beta() {} // first\nfn beta() {} // chosen\n",
                "cmd-shift-r",
            );
            vcx.simulate_keystrokes("down");
            let hold = HeldWorker::new();
            insert_top(&sibling, vcx, "fn beta() {} // inserted\n");
            ws.update_in(vcx, |ws, window, cx| ws.load_symbol_rows(window, cx));
            if !pending {
                ready(&ws, vcx);
            }
            vcx.simulate_keystrokes("enter");
            drop(hold);
            closed(&ws, vcx);
            let actual = selected(&ws, vcx);
            assert_eq!(actual.1, 2, "wrong definition: {actual:?}");
            assert!(actual.2.contains("chosen"), "wrong definition: {actual:?}");
        }
    }

    #[gpui::test]
    fn project_duplicate_insertion_preserves_definition(cx: &mut TestAppContext) {
        duplicate_insertion(cx, "cmd-shift-r", "goto-project-duplicate");
    }

    #[gpui::test]
    fn current_duplicate_insertion_preserves_definition(cx: &mut TestAppContext) {
        duplicate_insertion(cx, "cmd-r", "goto-current-duplicate");
    }

    #[gpui::test]
    fn removing_selected_definition_does_not_confirm_an_unrelated_row(cx: &mut TestAppContext) {
        let (ws, vcx, sibling) = picker(cx, "goto-deleted-symbol", "fn alpha() {}\nfn beta() {}\n", "cmd-shift-r");
        vcx.simulate_keystrokes("down");
        let hold = HeldWorker::new();
        insert_top(&sibling, vcx, "\n");
        vcx.simulate_keystrokes("enter");
        sibling.update(vcx, |e, cx| {
            e.edit(cx, |s| {
                let start = s.doc.rope.line_to_char(2);
                let end = s.doc.rope.line_to_char(3);
                s.set_selection(start, end);
                s.insert("");
            })
        });
        vcx.run_until_parked();
        drop(hold);
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                ws.index.read(cx).entry_for(&sibling, cx).is_some_and(|e| {
                    matches!(e.key, crate::languages::index::EntryKey::Buffer(_, v) if v == sibling.read(cx).doc(cx).version)
                })
            })
        });
        vcx.run_until_parked();
        assert!(
            ws.read_with(vcx, |ws, _| ws.quick_panel.is_some()),
            "pending Enter must not navigate to alpha after beta disappears: {:?}",
            selected(&ws, vcx)
        );
        ready(&ws, vcx);
        ws.update_in(vcx, |ws, window, cx| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert!(!p.confirming);
            assert_eq!(window.focused(cx), Some(p.input.read(cx).focus_handle(cx)));
        });
        vcx.simulate_keystrokes("enter");
        closed(&ws, vcx);
        assert_eq!(selected(&ws, vcx).0, "alpha");
    }

    #[gpui::test]
    fn a_second_edit_during_refresh_retries_and_confirms_the_latest_version(cx: &mut TestAppContext) {
        for keys in ["cmd-r", "cmd-shift-r"] {
            let (ws, vcx, sibling) = picker(cx, &format!("goto-reedit-{keys}"), "fn alpha() {}\nfn beta() {}\n", keys);
            vcx.simulate_keystrokes("down");
            let hold = HeldWorker::new();
            insert_top(&sibling, vcx, "\n");
            vcx.simulate_keystrokes("enter");
            insert_top(&sibling, vcx, "\n\n");
            drop(hold);
            closed(&ws, vcx);
            assert_eq!(selected(&ws, vcx).0, "beta");
            assert_eq!(selected(&ws, vcx).1, 4);
        }
    }

    #[gpui::test]
    fn deleting_the_only_selected_symbol_cancels_pending_confirmation(cx: &mut TestAppContext) {
        for keys in ["cmd-r", "cmd-shift-r"] {
            let (ws, vcx, sibling) = picker(cx, &format!("goto-remove-only-{keys}"), "fn alpha() {}\n", keys);
            let hold = HeldWorker::new();
            sibling.update(vcx, |e, cx| {
                e.edit(cx, |s| {
                    s.select_all();
                    s.insert("");
                })
            });
            vcx.run_until_parked();
            vcx.simulate_keystrokes("enter");
            drop(hold);
            ready(&ws, vcx);
            ws.update_in(vcx, |ws, window, cx| {
                let p = ws.quick_panel.as_ref().unwrap();
                assert!(p.filtered.is_empty());
                assert!(!p.confirming);
                assert_eq!(window.focused(cx), Some(p.input.read(cx).focus_handle(cx)));
            });
            vcx.simulate_keystrokes("escape");
            assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
        }
    }

    #[gpui::test]
    fn typing_cancels_auto_confirmation_and_enter_again_uses_new_query(cx: &mut TestAppContext) {
        for again in [false, true] {
            let (ws, vcx, sibling) =
                picker(cx, &format!("goto-typing-{again}"), "fn alpha() {}\nfn beta() {}\n", "cmd-shift-r");
            let hold = HeldWorker::new();
            insert_top(&sibling, vcx, "\n");
            vcx.simulate_keystrokes("enter enter");
            vcx.simulate_input("beta");
            vcx.run_until_parked();
            if again {
                vcx.simulate_keystrokes("enter enter");
            }
            assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_some()));
            drop(hold);
            if again {
                closed(&ws, vcx);
            } else {
                ready(&ws, vcx);
                assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_some()));
                vcx.simulate_keystrokes("enter");
                closed(&ws, vcx);
            }
            assert_eq!(selected(&ws, vcx).0, "beta");
            assert_eq!(selected(&ws, vcx).1, 2);
        }
    }

    #[gpui::test]
    fn a_stalled_refresh_remains_dismissible(cx: &mut TestAppContext) {
        let (ws, vcx, sibling) = picker(cx, "goto-stalled", "fn alpha() {}\n", "cmd-shift-r");
        let hold = HeldWorker::new();
        insert_top(&sibling, vcx, "\n");
        vcx.simulate_keystrokes("enter");
        vcx.executor().advance_clock(Duration::from_secs(60));
        vcx.run_until_parked();
        assert!(ws.read_with(vcx, |ws, _| { ws.quick_panel.as_ref().is_some_and(|p| p.confirming) }));
        vcx.simulate_keystrokes("escape");
        assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
        ws.update_in(vcx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            assert_eq!(window.focused(cx), Some(editor.read(cx).focus_handle(cx)));
        });
        drop(hold);
    }

    #[test]
    fn the_index_status_wording() {
        assert_eq!(status(None, None), None);
        assert_eq!(status(Some((3, 40)), None).as_deref(), Some("Indexing 3/40"));
        assert_eq!(
            status(None, Some(crate::languages::index::MAX_FILES)).as_deref(),
            Some("Index limited to 100,000 files")
        );
        assert_eq!(status(Some((0, 7)), Some(2)).as_deref(), Some("Indexing 0/7    Index limited to 2 files"));
    }
    #[gpui::test]
    fn single_project_definition_and_missing_status(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "definition-one",
            &[("use.rs", "fn caller() { target(); missing(); }\n"), ("def.rs", "/// Target doc.\nfn target() {}\n")],
        );
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("use.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(16));
            ws.goto_definition(&GotoDefinition, window, cx);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref() == Some(&dir.join("def.rs"))
            })
        });
        assert_eq!(selected(&ws, vcx).0, "target");
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("use.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(26));
            ws.goto_definition(&GotoDefinition, window, cx);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| {
                ws.status.as_ref().is_some_and(|(s, _)| s == "No definition named missing found in the index")
            })
        });
    }

    #[gpui::test]
    fn several_definitions_preview_and_escape(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "definition-many",
            &[("a.rs", "fn target() {}\nfn caller() { target(); }\n"), ("b.rs", "fn target() {}\n")],
        );
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(29));
            ws.goto_definition(&GotoDefinition, window, cx);
        });
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| assert_eq!(ws.quick_panel.as_ref().unwrap().items.len(), 2));
        vcx.simulate_keystrokes("down");
        assert_eq!(
            ws.read_with(vcx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).path.clone()),
            Some(dir.join("b.rs"))
        );
        vcx.simulate_keystrokes("escape");
        assert_eq!(selected(&ws, vcx).1, 1);
        assert_eq!(ws.read_with(vcx, |ws, _| ws.views.tabs().count()), 1);
    }

    #[gpui::test]
    fn expanded_f01_references_refuse_malformed_disk_and_keep_known_and_live_sources(cx: &mut TestAppContext) {
        use crate::chrome::quick_panel::Place;
        let dir = temp("orlo-f01-reference-codecs").canonicalize().unwrap();
        std::fs::write(dir.join("main.rs"), "fn target() {}\n").unwrap();
        std::fs::write(dir.join("valid.rs"), "// target 日本語\n").unwrap();
        let legacy_bytes = b"// target caf\xe9\n";
        for name in ["bad.rs", "known.rs", "live.rs"] {
            std::fs::write(dir.join(name), legacy_bytes).unwrap();
        }
        let (ws, vcx) = window(cx);
        ws.update_in(vcx, |ws, window, cx| ws.open_path(dir.clone(), window, cx));
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, cx| ws.index.read(cx).progress().is_none()));
        ws.update_in(vcx, |ws, window, cx| {
            let live = dir.join("live.rs");
            ws.add_tab(EditorState::from_loaded(&live, Loaded::read(&live).unwrap()), window, cx);
            ws.closed.push(session::ViewState {
                path: dir.join("known.rs"),
                disk_encoding_hint: Some(crate::editing::encoding::Encoding::Windows1252),
                ..Default::default()
            });
            ws.open_path(dir.join("main.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            ws.goto_reference(&GotoReference, window, cx);
        });
        references_ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| {
            let panel = ws.quick_panel.as_ref().unwrap();
            let row = |name: &str| {
                panel.items.iter().find(|item| {
                    matches!(&item.action,
                    ItemAction::Location(Place::File(path), ..) if path == &dir.join(name))
                })
            };
            assert!(row("bad.rs").is_none(), "malformed closed bytes must not produce substituted locations");
            assert_eq!(panel.items.len(), 4);
            assert!(row("main.rs").is_some());
            assert!(row("valid.rs").unwrap().detail.contains("日本語"));
            assert!(row("known.rs").unwrap().detail.contains("café"));
            assert!(row("live.rs").unwrap().detail.contains('�'), "explicit open source text remains searchable");
            let status = &ws.status.as_ref().unwrap().0;
            assert!(status.contains("bad.rs") && status.contains("malformed text"), "{status}");
        });
        for name in ["bad.rs", "known.rs", "live.rs"] {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), legacy_bytes);
        }
    }

    #[gpui::test]
    fn references_are_lexical_whole_words_and_mark_definitions(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "references",
            &[
                ("a.rs", "fn target() {}\nfn caller() { target(); Target(); targeted(); }\n// target\n"),
                ("b.rs", "fn other() { target(); }\n"),
            ],
        );
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            ws.goto_reference(&GotoReference, window, cx);
        });
        settle_until(vcx, |vcx| {
            vcx.executor().advance_clock(Duration::from_millis(50));
            ws.read_with(vcx, |ws, _| {
                ws.quick_panel
                    .as_ref()
                    .is_some_and(|p| p.loaded && p.rows_task.is_none() && p.reference_search.is_none())
            })
        });
        ws.read_with(vcx, |ws, _| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert_eq!(p.title.as_deref(), Some("Text matches for target — lexical, not semantic"));
            assert_eq!(p.items.len(), 4);
            assert_eq!(p.items.iter().filter(|i| i.hint == "def").count(), 1);
            assert_eq!(p.progress.as_deref(), Some("4 matches"));
        });
        vcx.simulate_keystrokes("down down down");
        assert_eq!(selected(&ws, vcx).0, "target");
        vcx.simulate_keystrokes("escape");
        assert_eq!(ws.read_with(vcx, |ws, _| ws.views.tabs().count()), 1);
    }

    #[gpui::test]
    fn definition_to_side_creates_columns_and_uses_next_group(cx: &mut TestAppContext) {
        let (ws, vcx, dir) =
            project(cx, "definition-side", &[("a.rs", "fn caller() { target(); }\n"), ("b.rs", "fn target() {}\n")]);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(16));
            ws.goto_definition_to_side(&GotoDefinitionToSide, window, cx);
        });
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.views.active == 1));
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(ws.views.layout, Layout::preset(Preset::Columns(2)));
            assert_eq!(ws.views.groups[0].tabs[0].read(cx).doc(cx).path, Some(dir.join("a.rs")));
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).path, Some(dir.join("b.rs")));
        });
        assert_eq!(selected(&ws, vcx).0, "target");
    }

    #[gpui::test]
    fn hover_waits_500ms_and_closes_on_keys_and_leave(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (ws, vcx, dir) = project(
            cx,
            "definition-hover",
            &[("a.rs", "fn target() {}\nfn caller() { target(); }\n// target\nconst S: &str = \"target\";\n")],
        );
        let e = open_active(&ws, vcx, dir.join("a.rs"));
        vcx.run_until_parked();
        settle_until(vcx, |vcx| e.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 29).is_some()));
        let point = e.update_in(vcx, |e, window, cx| {
            e.bounds_for_range(29..30, gpui::Bounds::default(), window, cx).unwrap().center()
        });
        vcx.simulate_mouse_move(point, None, gpui::Modifiers::none());
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(499));
        vcx.run_until_parked();
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
        vcx.executor().advance_clock(Duration::from_millis(1));
        settle_until(vcx, |vcx| e.read_with(vcx, |e, _| e.definition_text().is_some()));
        assert!(e.read_with(vcx, |e, _| e.definition_text().unwrap().contains("No documentation comment found")));
        vcx.simulate_keystrokes("left");
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
        for at in [44, 68] {
            let point = e.update_in(vcx, |e, window, cx| {
                e.bounds_for_range(at..at + 1, gpui::Bounds::default(), window, cx).unwrap().center()
            });
            vcx.simulate_mouse_move(point, None, gpui::Modifiers::none());
            vcx.executor().advance_clock(Duration::from_millis(500));
            vcx.run_until_parked();
            assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
        }
    }
    fn references_ready(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
        settle_until(vcx, |vcx| {
            vcx.executor().advance_clock(Duration::from_millis(50));
            ws.read_with(vcx, |ws, _| {
                ws.quick_panel.as_ref().is_some_and(|p| {
                    p.loaded && p.rows_task.is_none() && p.filter_task.is_none() && p.reference_search.is_none()
                })
            })
        });
    }

    #[gpui::test]
    fn missing_definition_reports_incomplete_index(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "definition-incomplete", &[("a.rs", "fn caller() { missing(); }\n")]);
        let e = open(&ws, vcx, dir.join("a.rs"));
        ws.update(vcx, |ws, cx| ws.index.update(cx, |i, cx| i.ensure_fresh(&e, cx)).detach());
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, cx| ws.index.read(cx).entry_for(&e, cx).is_some()));
        let hold = HeldWorker::new();
        vcx.update(|_, cx| {
            let mut s = cx.global::<Settings>().clone();
            s.index_files = false;
            cx.set_global(s);
        });
        vcx.run_until_parked();
        vcx.update(|_, cx| {
            let mut s = cx.global::<Settings>().clone();
            s.index_files = true;
            cx.set_global(s);
        });
        vcx.run_until_parked();
        ws.update_in(vcx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(17));
            ws.goto_definition(&GotoDefinition, window, cx);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| {
                ws.status
                    .as_ref()
                    .is_some_and(|(s, _)| s == "No definition named missing found in the index (index incomplete)")
            })
        });
        drop(hold);
    }

    #[gpui::test]
    fn references_follow_shared_edits_before_confirmation(cx: &mut TestAppContext) {
        let (ws, vcx, dir) =
            project(cx, "references-edited", &[("a.rs", "fn target() {}\nfn caller() { target(); }\n")]);
        let sibling = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            let e = ws.active_editor().unwrap();
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.goto_reference(&GotoReference, window, cx);
            e
        });
        references_ready(&ws, vcx);
        vcx.simulate_keystrokes("down");
        assert_eq!(selected(&ws, vcx).1, 1);
        insert_top(&sibling, vcx, "// inserted\n");
        vcx.executor().advance_clock(Duration::from_millis(300));
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| ws.index.read(cx).entry_for(&sibling, cx).is_some_and(|entry| matches!(entry.key, crate::languages::index::EntryKey::Buffer(_, v) if v == sibling.read(cx).doc(cx).version)))
        });
        vcx.simulate_keystrokes("enter");
        settle_until(vcx, |vcx| {
            vcx.executor().advance_clock(Duration::from_millis(50));
            ws.read_with(vcx, |ws, _| ws.quick_panel.is_none())
        });
        assert_eq!(selected(&ws, vcx).0, "target");
        assert_eq!(selected(&ws, vcx).1, 2);
    }

    #[gpui::test]
    fn search_uses_unsaved_buffers_exclusions_and_escape_cancels(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "references-scope",
            &[("a.rs", "fn target() {}\n"), ("skip.rs", "target\n"), ("ignore.rs", "target\n")],
        );
        std::fs::write(dir.join(".gitignore"), "ignore.rs\n").unwrap();
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
        });
        retype(&ws, vcx, "fn target() {}\n// target unsaved\n");
        vcx.update(|_, cx| {
            let mut s = cx.global::<Settings>().clone();
            s.index_exclude_patterns = vec!["skip.rs".into()];
            cx.set_global(s);
        });
        ws.update_in(vcx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            ws.goto_reference(&GotoReference, window, cx);
        });
        references_ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert_eq!(p.items.len(), 2);
            assert!(p.items.iter().any(|i| i.detail.contains("unsaved")));
        });
        vcx.simulate_keystrokes("escape");
        ws.update_in(vcx, |ws, window, cx| {
            ws.goto_reference(&GotoReference, window, cx);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.reference_search.is_some()))
        });
        let flag =
            ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().unwrap().reference_search.as_ref().unwrap().0 .0.clone());
        vcx.simulate_keystrokes("escape");
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
        assert!(ws.read_with(vcx, |ws, _| ws.quick_panel.is_none()));
    }

    fn hover_at(e: &Entity<Editor>, vcx: &mut VisualTestContext, at: usize) {
        use gpui::EntityInputHandler;
        vcx.run_until_parked();
        let point = e.update_in(vcx, |e, window, cx| {
            e.bounds_for_range(at..at + 1, gpui::Bounds::default(), window, cx).unwrap().center()
        });
        vcx.simulate_mouse_move(point, None, gpui::Modifiers::none());
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_millis(500));
    }

    #[gpui::test]
    fn hover_lists_docs_survives_popup_entry_and_cmd_clicks_to_side(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "hover-click",
            &[("a.rs", "/// Rust docs.\nfn target() {}\nfn caller() { target(); }\n"), ("b.rs", "fn target() {}\n")],
        );
        let e = open_active(&ws, vcx, dir.join("a.rs"));
        settle_until(vcx, |vcx| e.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 44).is_some()));
        hover_at(&e, vcx, 44);
        settle_until(vcx, |vcx| e.read_with(vcx, |e, _| e.definition_text().is_some()));
        let text = e.read_with(vcx, |e, _| e.definition_text().unwrap());
        assert!(text.contains("Rust docs.") && text.contains("No documentation comment found"));
        let popup = vcx.debug_bounds("popup").unwrap();
        let point = gpui::point(popup.left() + gpui::px(30.), popup.top() + gpui::px(48.));
        vcx.simulate_mouse_move(point, None, gpui::Modifiers::none());
        vcx.executor().advance_clock(Duration::from_millis(20));
        vcx.run_until_parked();
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_some()));
        vcx.simulate_click(point, gpui::Modifiers { platform: true, ..Default::default() });
        settle_until(vcx, |vcx| ws.read_with(vcx, |ws, _| ws.views.active == 1));
        assert_eq!(selected(&ws, vcx).0, "target");
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
    }

    #[gpui::test]
    fn hover_requires_definitions_code_scope_and_setting(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "hover-rules", &[("a.rs", "fn target() {}\nfn caller() { missing(); }\n")]);
        let e = open_active(&ws, vcx, dir.join("a.rs"));
        settle_until(vcx, |vcx| e.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 29).is_some()));
        hover_at(&e, vcx, 29);
        vcx.run_until_parked();
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
        vcx.update(|_, cx| {
            let mut s = cx.global::<Settings>().clone();
            s.show_definitions = false;
            cx.set_global(s);
        });
        hover_at(&e, vcx, 4);
        vcx.run_until_parked();
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
        vcx.update(|_, cx| {
            let mut s = cx.global::<Settings>().clone();
            s.show_definitions = true;
            cx.set_global(s);
        });
        e.update(vcx, |e, _| e.highlighter = crate::languages::highlight::Highlighter::by_name("Plain Text"));
        hover_at(&e, vcx, 5);
        vcx.run_until_parked();
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
    }
    #[gpui::test]
    fn definition_picker_refreshes_shared_target_before_enter(cx: &mut TestAppContext) {
        let (ws, vcx, dir) =
            project(cx, "definitions-edited", &[("a.rs", "fn target() {} // first\nfn target() {} // chosen\n")]);
        let sibling = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            let e = ws.active_editor().unwrap();
            ws.new_view_into_file(&NewViewIntoFile, window, cx);
            ws.goto_definition(&GotoDefinition, window, cx);
            e
        });
        ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| {
            assert_eq!(ws.quick_panel.as_ref().unwrap().title.as_deref(), Some("Definitions named target (by name)"))
        });
        vcx.simulate_keystrokes("down");
        let hold = HeldWorker::new();
        insert_top(&sibling, vcx, "// inserted\n");
        vcx.simulate_keystrokes("enter");
        drop(hold);
        closed(&ws, vcx);
        let actual = selected(&ws, vcx);
        assert_eq!(actual.0, "target");
        assert_eq!(actual.1, 2);
        assert!(actual.2.contains("chosen"));
    }

    #[gpui::test]
    fn hover_closes_on_pointer_leave_and_scroll(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "hover-close", &[("a.rs", "fn target() {}\n")]);
        let e = open_active(&ws, vcx, dir.join("a.rs"));
        settle_until(vcx, |vcx| e.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 4).is_some()));
        hover_at(&e, vcx, 4);
        settle_until(vcx, |vcx| e.read_with(vcx, |e, _| e.definition_text().is_some()));
        vcx.simulate_mouse_move(gpui::point(gpui::px(0.), gpui::px(0.)), None, gpui::Modifiers::none());
        vcx.executor().advance_clock(Duration::from_millis(20));
        vcx.run_until_parked();
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
        hover_at(&e, vcx, 4);
        settle_until(vcx, |vcx| e.read_with(vcx, |e, _| e.definition_text().is_some()));
        let popup = vcx.debug_bounds("popup").unwrap().center();
        vcx.simulate_mouse_move(popup, None, gpui::Modifiers::none());
        vcx.executor().advance_clock(Duration::from_millis(20));
        vcx.run_until_parked();
        vcx.simulate_event(gpui::ScrollWheelEvent {
            position: popup,
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(30.))),
            modifiers: gpui::Modifiers::none(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
    }
    #[gpui::test]
    fn disk_reference_preview_tracks_edits_after_the_index_catches_up(cx: &mut TestAppContext) {
        let (ws, vcx, dir) =
            project(cx, "reference-disk-edit", &[("a.rs", "fn target() {}\n"), ("b.rs", "target();\n")]);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            ws.goto_reference(&GotoReference, window, cx);
        });
        references_ready(&ws, vcx);
        vcx.simulate_keystrokes("down");
        let target = ws.read_with(vcx, |ws, _| ws.active_editor().unwrap());
        assert_eq!(target.read_with(vcx, |e, cx| e.doc(cx).path.clone()), Some(dir.join("b.rs")));
        insert_top(&target, vcx, "// inserted\n");
        vcx.executor().advance_clock(Duration::from_millis(300));
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| ws.index.read(cx).entry_for(&target, cx).is_some_and(|entry| matches!(entry.key, crate::languages::index::EntryKey::Buffer(_, v) if v == target.read(cx).doc(cx).version)))
        });
        vcx.simulate_keystrokes("enter");
        settle_until(vcx, |vcx| {
            vcx.executor().advance_clock(Duration::from_millis(50));
            ws.read_with(vcx, |ws, _| ws.quick_panel.is_none())
        });
        assert_eq!(selected(&ws, vcx).0, "target");
        assert_eq!(selected(&ws, vcx).1, 1);
    }

    #[gpui::test]
    fn untitled_reference_in_another_window_previews_and_restores(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "reference-other-window", &[("a.rs", "fn target() {}\n")]);
        let (other, _) = vcx.cx.add_window_view(Workspace::new);
        let other_window = other.read_with(vcx, |ws, _| ws.window);
        let mut other_vcx = VisualTestContext::from_window(other_window, &vcx.cx);
        crate::workspace::ui_tests::type_in(&other, &mut other_vcx, "target();\n");
        let source = other.read_with(&other_vcx, |ws, _| ws.active_editor().unwrap());
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            ws.goto_reference(&GotoReference, window, cx);
        });
        references_ready(&ws, vcx);
        ws.update_in(vcx, |ws, window, cx| {
            let p = ws.quick_panel.as_mut().unwrap();
            p.selected = p.filtered.iter().position(|&i| matches!(&p.items[i].action, ItemAction::Location(quick_panel::Place::View(id), ..) if *id == source.entity_id())).unwrap();
            ws.preview_place(window, cx);
        });
        assert_eq!(selected(&ws, vcx).0, "target");
        assert_eq!(
            ws.read_with(vcx, |ws, cx| ws.active_editor().unwrap().read(cx).read_state(cx).buffer_id()),
            source.read_with(&other_vcx, |e, cx| e.doc(cx).id())
        );
        vcx.simulate_keystrokes("escape");
        assert_eq!(ws.read_with(vcx, |ws, _| ws.views.tabs().count()), 1);
        assert_eq!(source.read_with(&other_vcx, |e, cx| e.doc(cx).text()), "target();\n");
    }
    #[gpui::test]
    fn many_definitions_render_only_visible_rows_and_resize_closes(cx: &mut TestAppContext) {
        let source = "fn target() {}\n".repeat(5_000);
        let (ws, vcx, dir) = project(cx, "hover-many", &[("a.rs", &source)]);
        let e = open_active(&ws, vcx, dir.join("a.rs"));
        settle_until(vcx, |vcx| e.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 4).is_some()));
        hover_at(&e, vcx, 4);
        settle_until(vcx, |vcx| e.read_with(vcx, |e, _| e.definition_text().is_some()));
        assert!(vcx.debug_bounds("popup").is_some());
        e.read_with(vcx, |e, _| assert!((1..100).contains(&e.definition_rendered_rows())));
        vcx.simulate_resize(gpui::size(gpui::px(600.), gpui::px(300.)));
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
    }
    #[gpui::test]
    fn late_hover_result_cannot_skip_a_new_500ms_wait(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (ws, vcx, dir) = project(cx, "hover-late", &[("a.rs", "fn target() {}\n")]);
        let e = open_active(&ws, vcx, dir.join("a.rs"));
        settle_until(vcx, |vcx| e.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 4).is_some()));
        let point = e.update_in(vcx, |e, window, cx| {
            e.bounds_for_range(4..5, gpui::Bounds::default(), window, cx).unwrap().center()
        });
        vcx.simulate_mouse_move(point, None, gpui::Modifiers::none());
        vcx.run_until_parked();
        e.update(vcx, |e, cx| {
            let definition = editor::DefinitionPreview {
                place: quick_panel::Place::File(dir.join("a.rs")),
                path: "a.rs".into(),
                key: crate::languages::index::EntryKey::Buffer(cx.entity_id(), e.doc(cx).version),
                symbol: crate::languages::index::IndexedSymbol {
                    name: "target".into(),
                    kind: "function".into(),
                    line: 0,
                    col: 3,
                    signature: "fn target() {}".into(),
                    doc: None,
                },
            };
            let lines = editor::definition_lines(std::slice::from_ref(&definition));
            e.show_definitions(3..9, e.doc(cx).version, vec![definition], lines, cx);
        });
        assert!(e.read_with(vcx, |e, _| e.definition_text().is_none()));
        vcx.executor().advance_clock(Duration::from_millis(500));
        settle_until(vcx, |vcx| e.read_with(vcx, |e, _| e.definition_text().is_some()));
    }
    #[gpui::test]
    fn hover_click_refreshes_an_edited_target(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "review-hover-stale-target",
            &[("a.rs", "fn caller() { target(); }\n"), ("b.rs", "fn target() {}\n")],
        );
        let (target, source) = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("b.rs"), window, cx);
            let target = ws.active_editor().unwrap();
            ws.open_path(dir.join("a.rs"), window, cx);
            window.activate_window();
            (target, ws.active_editor().unwrap())
        });
        settle_until(vcx, |vcx| {
            source.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 16).is_some())
        });
        hover_at(&source, vcx, 16);
        settle_until(vcx, |vcx| source.read_with(vcx, |e, _| e.definition_text().is_some()));
        insert_top(&target, vcx, "// inserted\n");
        ws.update(vcx, |ws, cx| ws.index.update(cx, |i, cx| i.ensure_fresh(&target, cx)).detach());
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, cx| {
                ws.index.read(cx).entry_for(&target, cx).is_some_and(|entry| {
                    matches!(entry.key, crate::languages::index::EntryKey::Buffer(_, v) if v == target.read(cx).doc(cx).version)
                })
            })
        });
        let popup = vcx.debug_bounds("popup").expect("hover still shown");
        let point = gpui::point(popup.left() + gpui::px(30.), popup.top() + gpui::px(30.));
        vcx.simulate_click(point, gpui::Modifiers::none());
        settle_until(vcx, |vcx| selected(&ws, vcx).0 == "target");
        assert_eq!(selected(&ws, vcx).0, "target", "click must resolve the current target position");
        assert_eq!(selected(&ws, vcx).1, 1);
    }

    #[gpui::test]
    fn hover_click_refreshes_an_edited_disk_target(cx: &mut TestAppContext) {
        let (ws, vcx, dir) =
            project(cx, "hover-stale-disk", &[("a.rs", "fn caller() { target(); }\n"), ("b.rs", "fn target() {}\n")]);
        let source = open_active(&ws, vcx, dir.join("a.rs"));
        settle_until(vcx, |vcx| {
            source.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 16).is_some())
        });
        hover_at(&source, vcx, 16);
        settle_until(vcx, |vcx| source.read_with(vcx, |e, _| e.definition_text().is_some()));
        std::fs::write(dir.join("b.rs"), "// inserted\nfn target() {}\n").unwrap();
        let popup = vcx.debug_bounds("popup").unwrap();
        let point = gpui::point(popup.left() + gpui::px(30.), popup.top() + gpui::px(30.));
        vcx.simulate_click(point, gpui::Modifiers::none());
        settle_until(vcx, |vcx| selected(&ws, vcx).0 == "target");
        assert_eq!(selected(&ws, vcx).1, 1);
    }

    #[gpui::test]
    fn reference_search_discards_removed_roots(cx: &mut TestAppContext) {
        let (ws, vcx, dir) =
            project(cx, "review-reference-roots", &[("a.rs", "fn target() {}\n"), ("b.rs", "target();\n")]);
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(4));
            ws.goto_reference(&GotoReference, window, cx);
        });
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| ws.quick_panel.as_ref().is_some_and(|p| p.reference_search.is_some()))
        });
        ws.update_in(vcx, |ws, window, cx| ws.remove_folders(&RemoveFolders, window, cx));
        references_ready(&ws, vcx);
        ws.read_with(vcx, |ws, _| {
            assert!(ws.roots.is_empty());
            let p = ws.quick_panel.as_ref().unwrap();
            assert!(
                !p.items.iter().any(|i| matches!(&i.action,
            ItemAction::Location(quick_panel::Place::File(path), ..) if path == &dir.join("b.rs"))),
                "removed root still contributes a closed file to the reference search"
            );
        });
    }

    #[gpui::test]
    fn hover_requires_pointer_on_identifier(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(cx, "review-hover-punctuation", &[("a.rs", "fn target() {}\n")]);
        let e = open_active(&ws, vcx, dir.join("a.rs"));
        settle_until(vcx, |vcx| e.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 9).is_some()));
        hover_at(&e, vcx, 9); // the '(' after target
        for _ in 0..40 {
            vcx.run_until_parked();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            e.read_with(vcx, |e, _| e.definition_text().is_none()),
            "punctuation after an identifier must not open its hover"
        );
    }

    #[gpui::test]
    fn definition_to_side_restores_origin_active_tab(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "review-side-origin",
            &[("a.rs", "fn caller() { target(); }\n"), ("b.rs", "fn target() {}\n"), ("c.rs", "fn target() {}\n")],
        );
        ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("b.rs"), window, cx);
            ws.open_path(dir.join("a.rs"), window, cx);
            ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(16));
            ws.goto_definition_to_side(&GotoDefinitionToSide, window, cx);
        });
        ready(&ws, vcx);
        vcx.simulate_keystrokes("enter");
        closed(&ws, vcx);
        ws.read_with(vcx, |ws, cx| {
            assert_eq!(ws.views.active, 1);
            let origin = &ws.views.groups[0];
            assert_eq!(
                origin.tabs[origin.active].read(cx).doc(cx).path,
                Some(dir.join("a.rs")),
                "the source group must keep the caller tab after opening the result to its side"
            );
        });
    }

    #[gpui::test]
    fn hover_click_deleted_disk_target(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "review-hover-deleted-disk",
            &[("a.rs", "fn caller() { target(); }\n"), ("b.rs", "fn target() {}\n")],
        );
        let source = open_active(&ws, vcx, dir.join("a.rs"));
        settle_until(vcx, |vcx| {
            source.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 16).is_some())
        });
        hover_at(&source, vcx, 16);
        settle_until(vcx, |vcx| source.read_with(vcx, |e, _| e.definition_text().is_some()));
        std::fs::remove_file(dir.join("b.rs")).unwrap();
        let popup = vcx.debug_bounds("popup").unwrap();
        let point = gpui::point(popup.left() + gpui::px(30.), popup.top() + gpui::px(30.));
        vcx.simulate_click(point, gpui::Modifiers::none());
        settle_until(vcx, |vcx| {
            ws.read_with(vcx, |ws, _| {
                ws.quick_panel.is_none()
                    && ws.status.as_ref().is_some_and(|s| s.0.contains("No definition named target"))
            })
        });
        assert_ne!(selected(&ws, vcx).0, "target");
    }

    #[gpui::test]
    fn hover_click_refreshes_closed_target(cx: &mut TestAppContext) {
        let (ws, vcx, dir) = project(
            cx,
            "review-hover-closed-target",
            &[("a.rs", "fn caller() { target(); }\n"), ("b.rs", "fn target() {}\n")],
        );
        let (target, source) = ws.update_in(vcx, |ws, window, cx| {
            ws.open_path(dir.join("b.rs"), window, cx);
            let target = ws.active_editor().unwrap();
            ws.open_path(dir.join("a.rs"), window, cx);
            window.activate_window();
            (target, ws.active_editor().unwrap())
        });
        settle_until(vcx, |vcx| {
            source.read_with(vcx, |e, cx| e.highlighter.cached_scope_id(&e.doc(cx).rope, 16).is_some())
        });
        hover_at(&source, vcx, 16);
        settle_until(vcx, |vcx| source.read_with(vcx, |e, _| e.definition_text().is_some()));
        ws.update(vcx, |ws, cx| ws.remove_view(&target, cx));
        std::fs::write(dir.join("b.rs"), "// inserted\nfn target() {}\n").unwrap();
        let popup = vcx.debug_bounds("popup").expect("hover still shown");
        let point = gpui::point(popup.left() + gpui::px(30.), popup.top() + gpui::px(30.));
        vcx.simulate_click(point, gpui::Modifiers::none());
        settle_until(vcx, |vcx| selected(&ws, vcx).0 == "target");
        assert_eq!(selected(&ws, vcx).0, "target", "click must resolve the current target position");
        assert_eq!(selected(&ws, vcx).1, 1);
    }
}
