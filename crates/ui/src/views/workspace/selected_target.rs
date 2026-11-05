//! Explicit activation of one physical document selection; ordinary mouse links stay unchanged.

use super::*;
use crate::editing::text::SelState;

struct Source {
    view: Entity<Editor>,
    document: u64,
    version: u64,
    selection: SelState,
    selection_generation: u64,
    path: Option<PathBuf>,
    roots: Vec<PathBuf>,
    generation: (u64, u64),
    focus: Option<FocusHandle>,
}

#[derive(Debug, PartialEq, Eq)]
struct Address {
    path: PathBuf,
    line: usize,
    column: usize,
}

fn unquote(text: &str) -> Result<&str, String> {
    if let Some(quote) = text.chars().next().filter(|c| matches!(c, '\'' | '"')) {
        return text
            .strip_prefix(quote)
            .and_then(|t| t.strip_suffix(quote))
            .ok_or_else(|| "Match the selected target's quotes".into());
    }
    if text.ends_with(['\'', '"']) {
        return Err("Match the selected target's quotes".into());
    }
    Ok(text)
}

fn address(text: &str, base: Option<&Path>) -> Result<Address, String> {
    let invalid = || "Select a path with an optional positive :line[:column]".to_string();
    if text.is_empty() || text.chars().any(char::is_control) {
        return Err(invalid());
    }
    let text = if text.ends_with(['\'', '"']) { unquote(text)? } else { text };
    // A file URL's scheme is not a location separator; the existing decoder validates its host/escapes.
    let prefix = if text.starts_with("file://") {
        7
    } else if text.starts_with(['\'', '"']) && text[1..].starts_with("file://") {
        8
    } else {
        0
    };
    let mut path = text;
    let mut numbers = Vec::new();
    while let Some((head, tail)) = path[prefix.min(path.len())..].rsplit_once(':') {
        if numbers.len() == 2 || tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        numbers.push(tail.parse::<usize>().ok().filter(|n| *n > 0).ok_or_else(invalid)?);
        path = &path[..prefix + head.len()];
    }
    let path = unquote(path)?;
    if path.is_empty() {
        return Err(invalid());
    }
    let path = if path.starts_with("file://") {
        orlo_terminal::links::local_file_url(path).ok_or_else(invalid)?
    } else {
        if path.contains(':') {
            return Err(invalid());
        }
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            base.ok_or_else(|| "A relative path needs a named document or one project folder".to_string())?.join(path)
        }
    };
    let (line, column) = match numbers.as_slice() {
        [] => (1, 1),
        [line] => (*line, 1),
        [column, line] => (*line, *column),
        _ => unreachable!(),
    };
    Ok(Address { path, line, column })
}

fn web_url(text: &str) -> Result<&str, String> {
    let text = unquote(text)?;
    let parsed = gpui::http_client::Url::parse(text).ok();
    if !text
        .split_once("://")
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || text.chars().any(|c| c.is_whitespace() || c.is_control())
        || parsed.as_ref().is_none_or(|url| !matches!(url.scheme(), "http" | "https") || url.host_str().is_none())
    {
        return Err("Select one complete HTTP(S) URL".into());
    }
    Ok(text)
}

impl Workspace {
    fn selected_source(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(Source, String), String> {
        self.invalidate_navigation();
        self.focus_editor(window, cx);
        let view = self.active_editor().ok_or("Select document text first")?;
        self.selection_set_admission(&view, cx)?;
        let (document, version, selection, selection_generation, path, text) = view.update(cx, |e, cx| {
            e.catch_up(cx);
            e.sel(cx).sel_checkpoint();
            let s = e.read_state(cx);
            let text = match s.view.selections.as_slice() {
                [selection] if !selection.range().is_empty() && !selection.has_virtual() => {
                    Some(s.doc.rope.slice(selection.range()).to_string())
                }
                _ => None,
            };
            (s.buffer_id(), s.doc.version, s.selection_state(), s.selection_generation(), s.doc.path.clone(), text)
        });
        let text = text.ok_or("Select exactly one nonempty physical target")?;
        Ok((
            Source {
                view,
                document,
                version,
                selection,
                selection_generation,
                path,
                roots: self.roots.iter().map(|r| r.path.clone()).collect(),
                generation: (self.restore_gen, self.language.navigation_generation),
                focus: window.focused(cx),
            },
            text,
        ))
    }

    fn selected_source_current(&self, source: &Source, window: &Window, cx: &App) -> bool {
        if self.active_editor().as_ref() != Some(&source.view)
            || !self.views.tabs().any(|v| v == &source.view)
            || source.generation != (self.restore_gen, self.language.navigation_generation)
            || source.roots != self.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>()
            || source.focus != window.focused(cx)
            || self.selection_set_admission(&source.view, cx).is_err()
        {
            return false;
        }
        let e = source.view.read(cx);
        let s = e.read_state(cx);
        s.buffer_id() == source.document
            && s.doc.version == source.version
            && s.doc.path == source.path
            && s.selection_state() == source.selection
            && s.selection_generation() == source.selection_generation
    }

    pub(crate) fn open_selected_url(&mut self, _: &OpenSelectedUrl, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.selected_source(window, cx).and_then(|(_, text)| web_url(&text).map(str::to_string));
        match result {
            Ok(url) => cx.open_url(&url),
            Err(error) => self.status_message(error, cx),
        }
    }

    pub(crate) fn open_selected_path(&mut self, _: &OpenSelectedPath, window: &mut Window, cx: &mut Context<Self>) {
        let (source, text) = match self.selected_source(window, cx) {
            Ok(value) => value,
            Err(error) => return self.status_message(error, cx),
        };
        let base = source.path.as_deref().and_then(Path::parent).or(match source.roots.as_slice() {
            [root] => Some(root.as_path()),
            _ => None,
        });
        let target = match address(&text, base) {
            Ok(value) => value,
            Err(error) => return self.status_message(error, cx),
        };
        let preflight = cx.background_spawn(async move {
            let path = target.path.canonicalize()?;
            let metadata = std::fs::metadata(&path)?;
            if !metadata.is_file() {
                return Err(std::io::Error::other("Not a regular file"));
            }
            Ok::<_, std::io::Error>((Address { path, ..target }, metadata.len()))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = preflight.await;
            let prepared = this
                .update_in(cx, |ws, window, cx| {
                    if !ws.selected_source_current(&source, window, cx) {
                        return None;
                    }
                    let (target, size) = match result {
                        Ok(value) => value,
                        Err(error) => {
                            ws.status_message(format!("Could not open selected path: {error}"), cx);
                            return None;
                        }
                    };
                    if ws.show_selected_existing(&target, window, cx) {
                        return None;
                    }
                    if pipelines::reading_blocked(&target.path, cx) {
                        ws.status_message("File is being written; try navigation again", cx);
                        return None;
                    }
                    let prompt = (size > ASK_ABOVE).then(|| {
                        window.prompt(
                            PromptLevel::Warning,
                            "Open a very large file?",
                            Some(&format!("{} is {:.1} GB.", target.path.display(), size as f64 / (1u64 << 30) as f64)),
                            &["Open", "Cancel"],
                            cx,
                        )
                    });
                    let encoding = ws.remembered_disk_encoding(&target.path);
                    let observed = pipelines::observe_read(&target.path, cx);
                    Some((target, encoding, observed, prompt))
                })
                .ok()
                .flatten();
            let Some((target, encoding, observed, prompt)) = prepared else { return };
            if let Some(prompt) = prompt {
                if prompt.await != Ok(0) {
                    return;
                }
            }
            let path = target.path.clone();
            let loaded = cx.background_spawn(async move { Loaded::read_with_hint(&path, encoding) }).await;
            #[cfg(test)]
            crate::testing::gate::wait("selected-read", &target.path).await;
            this.update_in(cx, |ws, window, cx| {
                if !ws.selected_source_current(&source, window, cx) {
                    return;
                }
                let loaded = match loaded {
                    Ok(value) => value,
                    Err(error) => return ws.status_message(format!("Could not open selected path: {error}"), cx),
                };
                if pipelines::reading_blocked(&target.path, cx)
                    || observed.changed()
                    || loaded.disk.is_none()
                    || loaded.disk != crate::files::watch::stamp(&target.path)
                {
                    return ws.status_message("File changed or is being written; try navigation again", cx);
                }
                // A target may have been opened/edited in any window during the read.
                if ws.show_selected_existing(&target, window, cx) {
                    return;
                }
                ws.opened(target.path.clone(), Ok(EditorState::from_loaded(&target.path, loaded)), window, cx);
                ws.reveal_selected_address(&target, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn show_selected_existing(&mut self, target: &Address, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Opening prefers this group's view, then another local view, then the registry owner.
        // Save As can leave distinct live documents sharing a path: admit the one we will adopt.
        let local = self.editor_for_path(&target.path, cx);
        let Some(document) =
            local.as_ref().map(|view| view.read(cx).document.clone()).or_else(|| Documents::for_path(&target.path, cx))
        else {
            return false;
        };
        if let Some(view) = self.open_editors(cx).into_iter().find(|v| v.read(cx).document == document) {
            if let Err(error) = self.selection_set_admission(&view, cx) {
                self.status_message(error, cx);
                return true;
            }
        }
        // Registry adoption through open_path can otherwise wait for a writer and choose a
        // different owner later. A local view needs no read; a remote owner can be retried.
        if local.is_none() && pipelines::reading_blocked(&target.path, cx) {
            self.status_message("File is being written; try navigation again", cx);
            return true;
        }
        self.open_path_here(target.path.clone(), window, cx);
        self.reveal_selected_address(target, window, cx);
        true
    }

    fn reveal_selected_address(&mut self, target: &Address, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor_for_path(&target.path, cx) {
            if let Err(error) = self.selection_set_admission(&editor, cx) {
                return self.status_message(error, cx);
            }
            editor.update(cx, |e, cx| e.goto_address(target.line, target.column, cx));
            self.focus_editor(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editing::text::Selection;
    use crate::workspace::ui_tests::{new_view, settle_until, temp, two_windows, type_in, window, write_in};
    use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};

    fn select(ws: &Entity<Workspace>, cx: &mut VisualTestContext, text: &str) -> Entity<Editor> {
        ws.update(cx, |ws, cx| {
            let view = ws.active_editor().unwrap();
            view.update(cx, |e, cx| {
                e.set_text(text, cx);
                e.sel(cx).select_all();
            });
            view
        })
    }
    fn open(ws: &Entity<Workspace>, cx: &mut VisualTestContext) {
        ws.update_in(cx, |ws, w, cx| ws.open_selected_path(&OpenSelectedPath, w, cx));
    }
    fn opened(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &Path) {
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).path.as_deref() == Some(path))
        });
    }

    #[test]
    fn n21_address_grammar_preserves_full_names_and_validates_locations() {
        for (text, path, line, column) in [
            ("雪 notes!", "/base/雪 notes!", 1, 1),
            ("name;,.?!", "/base/name;,.?!", 1, 1),
            ("  literal  ", "/base/  literal  ", 1, 1),
            ("'a b':2:3", "/base/a b", 2, 3),
            ("\"a b:2:3\"", "/base/a b", 2, 3),
            ("/absolute 雪:4", "/absolute 雪", 4, 1),
            ("file:///absolute%20%E9%9B%AA:2:7", "/absolute 雪", 2, 7),
        ] {
            assert_eq!(address(text, Some(Path::new("/base"))).unwrap(), Address { path: path.into(), line, column });
        }
        for text in [
            "",
            "a:0",
            "a:1:0",
            "a:-1",
            "a:x",
            "a:1:x",
            "a:1:2:3",
            "a:",
            "a:184467440737095516160",
            "a\nb",
            "'a",
            "a'",
            "\"a'",
            "file://foreign.invalid/a",
            "file:///bad%xx",
            "file:///bad%00",
            "http://example.com",
            "https://example.com",
            "ssh://host/file",
        ] {
            assert!(address(text, Some(Path::new("/base"))).is_err(), "{text}");
        }
        assert!(address("relative", None).is_err());
        assert!(address("/absolute", None).is_ok());
    }

    #[gpui::test]
    fn n21_named_relative_parent_and_source_selection_copy_are_preserved(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-relative").canonicalize().unwrap();
        let target = write_in(&dir, "雪 draft!", "a\nbc\ndef");
        let source_path = write_in(&dir, "source.txt", "'雪 draft!':2:99");
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, w, cx| ws.open_path(source_path.clone(), w, cx));
        let source = ws.update(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| e.sel(cx).select_all());
            e
        });
        let before = source.read_with(cx, |e, cx| {
            (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.read_state(cx).selection_state())
        });
        source.update_in(cx, |e, w, cx| e.perform_action(&editor::Copy, w, cx));
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "'雪 draft!':2:99");
        open(&ws, cx);
        opened(&ws, cx, &target);
        ws.read_with(cx, |ws, cx| assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 4));
        assert!(
            source.read_with(cx, |e, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state()
            )) == before
        );
    }

    #[gpui::test]
    fn n21_untitled_root_no_base_multiple_roots_and_missing_never_create_tabs(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-roots").canonicalize().unwrap();
        let target = write_in(&dir, "README", "target");
        let (ws, cx) = window(cx);
        let source = select(&ws, cx, "README");
        for roots in [
            vec![],
            vec![crate::config::project::Root::new(dir.clone()), crate::config::project::Root::new(dir.join("other"))],
        ] {
            ws.update(cx, |ws, _| ws.roots = roots);
            open(&ws, cx);
            cx.run_until_parked();
            ws.read_with(cx, |ws, _| {
                assert_eq!(ws.active_editor().as_ref(), Some(&source));
                assert_eq!(ws.views.tabs().count(), 1);
                assert!(ws.status.is_some());
            });
        }
        ws.update(cx, |ws, _| ws.roots = vec![crate::config::project::Root::new(dir)]);
        open(&ws, cx);
        opened(&ws, cx, &target);
        ws.update_in(cx, |ws, w, cx| {
            ws.activate_view(source.entity_id(), w, cx);
        });
        select(&ws, cx, "missing.txt");
        open(&ws, cx);
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| ws.status.as_ref().is_some_and(|s| s.0.contains("Could not open selected")))
        });
        ws.read_with(cx, |ws, _| assert_eq!(ws.views.tabs().count(), 2));
    }

    #[gpui::test]
    fn n21_empty_virtual_multiple_and_directory_targets_refuse_without_mutation(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-invalid").canonicalize().unwrap();
        let (ws, cx) = window(cx);
        type_in(&ws, cx, &dir.to_string_lossy());
        let source = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        for selections in [
            vec![Selection::caret(0)],
            vec![Selection::new(0, 1), Selection::new(2, 3)],
            vec![Selection::from_points((0, 1), (0, 3))],
        ] {
            source.update(cx, |e, cx| e.sel(cx).set_selections(selections));
            let before = source.read_with(cx, |e, cx| {
                (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.read_state(cx).selection_state())
            });
            open(&ws, cx);
            cx.run_until_parked();
            assert!(
                source.read_with(cx, |e, cx| (
                    e.text(cx),
                    e.doc(cx).version,
                    e.doc(cx).save_point(),
                    e.read_state(cx).selection_state()
                )) == before
            );
        }
        source.update(cx, |e, cx| e.sel(cx).select_all());
        open(&ws, cx);
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| ws.status.as_ref().is_some_and(|s| s.0.contains("Not a regular file")))
        });
        ws.read_with(cx, |ws, _| assert_eq!(ws.views.tabs().count(), 1));
    }

    #[gpui::test]
    fn n21_only_explicit_url_action_records_http_and_https(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        for url in ["https://example.com/雪?q=one#two", "http://example.com/a"] {
            select(&ws, cx, url);
            let before = cx.opened_url();
            open(&ws, cx);
            assert_eq!(cx.opened_url(), before);
            ws.update_in(cx, |ws, w, cx| ws.open_selected_url(&OpenSelectedUrl, w, cx));
            assert_eq!(cx.opened_url().as_deref(), Some(url));
        }
        let before = cx.opened_url();
        for invalid in
            ["file:///tmp/a", "javascript:alert(1)", "mailto:a@b", "https://", "https://a b", "https://:", "a.txt"]
        {
            select(&ws, cx, invalid);
            ws.update_in(cx, |ws, w, cx| ws.open_selected_url(&OpenSelectedUrl, w, cx));
            assert_eq!(cx.opened_url(), before, "{invalid}");
        }
    }

    #[gpui::test]
    fn n21_source_local_and_shared_composition_refuse_before_editor_borrow(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        select(&ws, cx, "https://example.com");
        new_view(&ws, cx);
        let views: Vec<_> = ws.read_with(cx, |ws, _| ws.views.tabs().cloned().collect());
        for composing in &views {
            composing.update_in(cx, |e, w, cx| e.replace_and_mark_text_in_range(Some(0..1), "h", Some(0..1), w, cx));
            let active = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
            let before =
                active.read_with(cx, |e, cx| (e.text(cx), e.read_state(cx).selection_state(), e.marked.clone()));
            open(&ws, cx);
            ws.update_in(cx, |ws, w, cx| ws.open_selected_url(&OpenSelectedUrl, w, cx));
            assert!(cx.opened_url().is_none());
            assert!(
                active.read_with(cx, |e, cx| (e.text(cx), e.read_state(cx).selection_state(), e.marked.clone()))
                    == before
            );
            composing.update_in(cx, |e, w, cx| e.unmark_text(w, cx));
        }
    }

    #[gpui::test]
    fn n21_stale_reads_reject_changed_selection_text_tab_base_and_newer_action(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-stale").canonicalize().unwrap();
        let target = write_in(&dir, "target", "loaded");
        let (ws, cx) = window(cx);
        for mode in 0..9 {
            ws.update_in(cx, |ws, w, cx| {
                ws.new_file(&NewFile, w, cx);
            });
            let source = select(&ws, cx, &target.to_string_lossy());
            let gate = crate::testing::gate::pause("selected-read", &target);
            open(&ws, cx);
            settle_until(cx, |_| gate.arrived() == 1);
            ws.update_in(cx, |ws, w, cx| match mode {
                0 => source.update(cx, |e, cx| {
                    e.sel(cx).set_cursor(0);
                    e.sel(cx).select_all();
                }),
                1 => source.update(cx, |e, cx| e.edit(cx, |s| s.insert("changed"))),
                2 => ws.new_file(&NewFile, w, cx),
                3 => {
                    ws.roots.push(crate::config::project::Root::new(dir.clone()));
                }
                4 => ws.open_selected_url(&OpenSelectedUrl, w, cx),
                5 => source.update(cx, |e, cx| e.update_doc(cx, |d| d.path = Some(dir.join("rebound")))),
                6 => ws.remove_view(&source, cx),
                7 => w.focus(&ws.focus_handle),
                _ => source.update(cx, |e, cx| e.replace_and_mark_text_in_range(Some(0..1), "X", Some(0..1), w, cx)),
            });
            gate.release();
            cx.run_until_parked();
            ws.read_with(cx, |ws, cx| {
                assert!(ws.editor_for_path(&target, cx).is_none(), "mode {mode} must not attach a stale target")
            });
        }
    }

    #[gpui::test]
    fn n21_deleted_or_changed_file_after_read_never_attaches(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-disk-races").canonicalize().unwrap();
        let (ws, cx) = window(cx);
        for remove in [false, true] {
            let target = write_in(&dir, "target", "original");
            select(&ws, cx, &target.to_string_lossy());
            let gate = crate::testing::gate::pause("selected-read", &target);
            open(&ws, cx);
            settle_until(cx, |_| gate.arrived() == 1);
            if remove {
                std::fs::remove_file(&target).unwrap();
            } else {
                std::fs::write(&target, "different file bytes").unwrap();
            }
            gate.release();
            cx.run_until_parked();
            ws.read_with(cx, |ws, cx| {
                assert!(ws.editor_for_path(&target, cx).is_none());
                assert_eq!(ws.views.tabs().count(), 1);
            });
        }
    }

    #[gpui::test]
    fn n21_alias_reuses_dirty_live_document_opened_during_read(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-alias").canonicalize().unwrap();
        let target = write_in(&dir, "target", "disk");
        let alias = dir.join("alias");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let (one, two, mut a, mut b) = two_windows(cx);
        select(&one, &mut a, &format!("{}:99:99", alias.display()));
        let gate = crate::testing::gate::pause("selected-read", &target);
        open(&one, &mut a);
        settle_until(&mut a, |_| gate.arrived() == 1);
        two.update_in(&mut b, |ws, w, cx| ws.open_path(target.clone(), w, cx));
        let peer = two.read_with(&b, |ws, _| ws.active_editor().unwrap());
        peer.update(&mut b, |e, cx| {
            e.edit(cx, |s| {
                s.set_cursor(s.len());
                s.insert(" live 雪");
            })
        });
        let document = peer.read_with(&b, |e, cx| e.doc(cx).id());
        gate.release();
        opened(&one, &mut a, &target);
        one.read_with(&a, |ws, cx| {
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            assert_eq!(e.doc(cx).id(), document);
            assert_eq!(e.text(cx), "disk live 雪");
            assert_eq!(e.state.cursor(), e.doc(cx).len());
            assert!(e.doc(cx).dirty);
        });
        assert_eq!(std::fs::read_to_string(target).unwrap(), "disk");
    }

    #[gpui::test]
    fn n21_destination_composition_during_read_refuses_and_origin_keeps_typing(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-target-ime").canonicalize().unwrap();
        let target = write_in(&dir, "target", "disk");
        let (one, two, mut a, mut b) = two_windows(cx);
        let source = select(&one, &mut a, &target.to_string_lossy());
        let gate = crate::testing::gate::pause("selected-read", &target);
        open(&one, &mut a);
        settle_until(&mut a, |_| gate.arrived() == 1);
        two.update_in(&mut b, |ws, w, cx| ws.open_path(target.clone(), w, cx));
        let peer = two.read_with(&b, |ws, _| ws.active_editor().unwrap());
        peer.update_in(&mut b, |e, w, cx| e.replace_and_mark_text_in_range(Some(0..1), "D", Some(0..1), w, cx));
        gate.release();
        a.run_until_parked();
        one.read_with(&a, |ws, _| assert_eq!(ws.active_editor().as_ref(), Some(&source)));
        one.update_in(&mut a, |_, w, _| w.activate_window());
        a.simulate_keystrokes("x");
        assert!(source.read_with(&a, |e, cx| e.text(cx)) == "x");
        peer.read_with(&b, |e, _| assert!(!e.marked.is_empty()));
    }

    #[gpui::test]
    fn n21_palette_native_menu_and_custom_json_share_both_actions(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let items = quick_panel::command_items();
        for name in ["Goto: Open Selected Path/Address", "Goto: Open Selected URL"] {
            assert!(items.iter().any(|i| i.label == name));
        }
        std::fs::write(crate::config::customization::keymap_path(), r#"[{"keys":["cmd-alt-o"],"command":"workspace::OpenSelectedPath"},{"keys":["cmd-alt-u"],"command":"workspace::OpenSelectedUrl"}]"#).unwrap();
        ws.update_in(cx, |ws, w, cx| ws.reload_customizations(&ReloadCustomizations, w, cx));
        select(&ws, cx, "https://example.com/custom");
        cx.simulate_keystrokes("cmd-alt-u");
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/custom"));
        cx.simulate_keystrokes("cmd-alt-o");
        ws.read_with(cx, |ws, _| assert!(ws.status.is_some()));
        ws.update_in(cx, |ws, w, cx| {
            fn actions(items: Vec<MenuItem>, found: &mut Vec<Box<dyn Action>>) {
                for item in items {
                    match item {
                        MenuItem::Action { action, .. }
                            if action.as_any().is::<OpenSelectedPath>() || action.as_any().is::<OpenSelectedUrl>() =>
                        {
                            found.push(action)
                        }
                        MenuItem::Submenu(submenu) => actions(submenu.items, found),
                        _ => {}
                    }
                }
            }
            let mut found = Vec::new();
            for menu in ws.menus(cx) {
                actions(menu.items, &mut found);
            }
            assert_eq!(found.len(), 2);
            for action in found {
                w.dispatch_action(action, cx);
            }
        });
    }

    #[gpui::test]
    fn n21_remembered_codec_uses_existing_closed_file_hint(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-codec").canonicalize().unwrap();
        let target = write_in(&dir, "legacy.txt", b"caf\xe9");
        let (ws, cx) = window(cx);
        select(&ws, cx, &format!("file://{}:1:99", target.display()));
        ws.update(cx, |ws, _| {
            ws.closed.push(ViewState {
                path: target.clone(),
                disk_encoding_hint: Some(crate::editing::encoding::Encoding::Windows1252),
                ..ViewState::default()
            })
        });
        open(&ws, cx);
        opened(&ws, cx, &target);
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            let e = e.read(cx);
            assert_eq!(e.text(cx), "café");
            assert_eq!(e.doc(cx).disk_encoding, crate::editing::encoding::Encoding::Windows1252);
            assert_eq!(e.state.cursor(), 4);
            assert!(!e.doc(cx).read_only);
        });
        assert_eq!(std::fs::read(target).unwrap(), b"caf\xe9");
    }

    #[gpui::test]
    fn n21_source_address_precedes_narrowing_restore_and_preserves_target_redo(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-reveal").canonicalize().unwrap();
        let target = write_in(&dir, "target", "head\n  inner\n    雪body\ntail\n");
        let (ws, cx) = window(cx);
        let destination = ws.update_in(cx, |ws, w, cx| {
            ws.open_path(target.clone(), w, cx);
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.set_cursor(s.len());
                    s.insert("!");
                });
            });
            e.update(cx, |e, cx| {
                e.perform_action(&editor::Undo, w, cx);
                e.sel(cx).set_selection(0, 4);
                e.perform_action(&editor::NarrowToSelection, w, cx);
                assert!(!e.folds().is_empty());
                e.input_enabled = false;
                e.update_doc(cx, |d| d.read_only = true);
            });
            ws.new_file(&NewFile, w, cx);
            e
        });
        select(&ws, cx, &format!("{}:3:2", target.display()));
        let before = destination.read_with(cx, |e, cx| (e.text(cx), e.doc(cx).version, e.doc(cx).save_point()));
        open(&ws, cx);
        opened(&ws, cx, &target);
        destination.update_in(cx, |e, w, cx| {
            assert_eq!(e.state.cursor(), 14);
            assert!(e.folds().is_empty());
            assert_eq!((e.text(cx), e.doc(cx).version, e.doc(cx).save_point()), before);
            e.input_enabled = true;
            e.update_doc(cx, |d| d.read_only = false);
            e.perform_action(&editor::Redo, w, cx);
            assert!(e.text(cx).ends_with('!'));
        });
    }

    #[gpui::test]
    fn n21_large_file_cancel_preserves_source_focus_selection_and_history(cx: &mut TestAppContext) {
        let dir = temp("orlo-n21-large-cancel").canonicalize().unwrap();
        let target = dir.join("large");
        let file = std::fs::File::create(&target).unwrap();
        file.set_len(ASK_ABOVE + 1).unwrap();
        let (ws, cx) = window(cx);
        type_in(&ws, cx, &target.to_string_lossy());
        let source = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        source.update(cx, |e, cx| e.sel(cx).select_all());
        let before = source.read_with(cx, |e, cx| {
            (e.text(cx), e.doc(cx).version, e.doc(cx).save_point(), e.read_state(cx).selection_state())
        });
        open(&ws, cx);
        settle_until(cx, |cx| cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert!(
            source.read_with(cx, |e, cx| (
                e.text(cx),
                e.doc(cx).version,
                e.doc(cx).save_point(),
                e.read_state(cx).selection_state()
            )) == before
        );
        ws.read_with(cx, |ws, _| assert_eq!(ws.views.tabs().count(), 1));
        cx.simulate_keystrokes("x");
        assert_eq!(source.read_with(cx, |e, cx| e.text(cx)), "x");
        source.update_in(cx, |e, w, cx| e.perform_action(&editor::Undo, w, cx));
        assert_eq!(source.read_with(cx, |e, cx| e.text(cx)), before.0);
    }

    mod admission_tests {
        use super::*;

        fn save_as_existing(
            ws: &Entity<Workspace>,
            cx: &mut VisualTestContext,
            from: PathBuf,
            to: &Path,
        ) -> Entity<Editor> {
            let view = ws.update_in(cx, |ws, w, cx| {
                ws.open_path(from, w, cx);
                let view = ws.active_editor().unwrap();
                ws.save_as(&SaveAs, w, cx);
                view
            });
            cx.simulate_new_path_selection(|_| Some(to.to_path_buf()));
            settle_until(cx, |cx| {
                view.read_with(cx, |e, cx| e.doc(cx).path.as_deref() == Some(to) && !e.doc(cx).dirty)
            });
            view
        }

        #[gpui::test]
        fn n21_same_path_registry_owner_composition_refuses_before_attachment(cx: &mut TestAppContext) {
            let dir = temp("orlo-n21-registry-ime").canonicalize().unwrap();
            let target = write_in(&dir, "target", "older\n");
            let from = write_in(&dir, "from", "latest\n");
            let (origin, older_window, mut a, mut b) = two_windows(cx);
            let (latest_window, _) = cx.add_window_view(Workspace::new);
            let mut c = VisualTestContext::from_window(latest_window.read_with(cx, |ws, _| ws.window), cx);
            older_window.update_in(&mut b, |ws, w, cx| ws.open_path(target.clone(), w, cx));
            let older = older_window.read_with(&b, |ws, _| ws.active_editor().unwrap());
            let latest = save_as_existing(&latest_window, &mut c, from, &target);
            a.read(|cx| {
                assert_ne!(older.read(cx).document, latest.read(cx).document);
                assert_eq!(Documents::for_path(&target, cx), Some(latest.read(cx).document.clone()));
            });
            latest.update_in(&mut c, |e, w, cx| e.replace_and_mark_text_in_range(Some(0..1), "雪", Some(0..1), w, cx));
            let source = select(&origin, &mut a, &format!("{}:2:1", target.display()));
            origin.update_in(&mut a, |ws, w, cx| {
                w.activate_window();
                ws.focus_editor(w, cx);
                // This is the older path-matched view, not the document normal opening adopts.
                let first = ws.open_editors(cx).into_iter().find(|e| e.read(cx).doc(cx).path.as_ref() == Some(&target));
                assert_eq!(first, Some(older.clone()));
            });
            // Window activation invalidates pending navigation; finish it before invoking the command.
            a.run_until_parked();
            let before = latest.read_with(&c, |e, cx| (e.text(cx), e.doc(cx).version, e.marked.clone()));
            open(&origin, &mut a);
            settle_until(&mut a, |cx| {
                origin.read_with(cx, |ws, _| ws.status.is_some() || ws.active_editor().as_ref() != Some(&source))
            });
            origin.read_with(&a, |ws, _| {
                assert_eq!(ws.active_editor().as_ref(), Some(&source));
                assert_eq!(ws.views.tabs().count(), 1);
            });
            assert_eq!(older.read_with(&b, |e, cx| e.text(cx)), "older\n");
            assert_eq!(latest.read_with(&c, |e, cx| (e.text(cx), e.doc(cx).version, e.marked.clone())), before);
            latest.update_in(&mut c, |e, w, cx| e.unmark_text(w, cx));
            let lease = origin.update(&mut a, |ws, cx| {
                ws.status = None;
                pipelines::lease(&target, cx)
            });
            open(&origin, &mut a);
            settle_until(&mut a, |cx| {
                origin.read_with(cx, |ws, _| {
                    ws.status.as_ref().is_some_and(|s| s.0 == "File is being written; try navigation again")
                })
            });
            drop(lease);
            a.run_until_parked();
            origin.read_with(&a, |ws, _| {
                assert_eq!(ws.active_editor().as_ref(), Some(&source));
                assert_eq!(ws.views.tabs().count(), 1, "a busy refusal must not queue later attachment");
            });
            a.simulate_keystrokes("x");
            assert_eq!(source.read_with(&a, |e, cx| e.text(cx)), "x");
            source.update_in(&mut a, |e, w, cx| e.perform_action(&editor::Undo, w, cx));
            assert_eq!(source.read_with(&a, |e, cx| e.text(cx)), format!("{}:2:1", target.display()));
        }

        #[gpui::test]
        fn n21_same_path_local_view_has_priority_over_other_composing_document(cx: &mut TestAppContext) {
            let dir = temp("orlo-n21-local-ime").canonicalize().unwrap();
            let target = write_in(&dir, "target", "older\n");
            let from = write_in(&dir, "from", "latest\n");
            let (origin, peer, mut a, mut b) = two_windows(cx);
            origin.update_in(&mut a, |ws, w, cx| ws.open_path(target.clone(), w, cx));
            let local = origin.read_with(&a, |ws, _| ws.active_editor().unwrap());
            let other = save_as_existing(&peer, &mut b, from, &target);
            other.update_in(&mut b, |e, w, cx| e.replace_and_mark_text_in_range(Some(0..1), "雪", Some(0..1), w, cx));
            origin.update_in(&mut a, |ws, w, cx| ws.new_file(&NewFile, w, cx));
            select(&origin, &mut a, &format!("{}:2:1", target.display()));
            let before = other.read_with(&b, |e, cx| (e.text(cx), e.doc(cx).version, e.marked.clone()));
            let lease = origin.update(&mut a, |_, cx| pipelines::lease(&target, cx));
            open(&origin, &mut a);
            opened(&origin, &mut a, &target);
            drop(lease);
            origin.read_with(&a, |ws, cx| {
                assert_eq!(ws.active_editor().as_ref(), Some(&local));
                assert_eq!(local.read(cx).state.cursor(), 6);
                assert_eq!(local.read(cx).text(cx), "older\n");
                assert!(ws.status.is_none());
            });
            assert_eq!(other.read_with(&b, |e, cx| (e.text(cx), e.doc(cx).version, e.marked.clone())), before);
        }
    }
}
