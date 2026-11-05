//! Goto Anything, Command Palette, Goto Line and Goto Symbol through the quick panel.

use super::*;
use crate::editing::clipboard_history::ClipboardHistory;
use crate::languages::index::EntryKey;
use quick_panel::Place;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

impl Workspace {
    pub(crate) fn goto_symbol(&mut self, _: &GotoSymbol, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_editor().is_none() {
            return;
        }
        // Sublime's Goto Symbol is Goto Anything with "@" typed: deleting it lists files (made
        // then: walking the folder now would only slow ⌘R down)
        self.open_quick_panel(Kind::Files, Vec::new(), window, cx);
        if let Some(p) = &self.quick_panel {
            p.input.update(cx, |e, cx| {
                e.set_text("@", cx);
                e.sel(cx).set_cursor(1);
            });
        }
    }

    forward_actions! {
    pub(crate) goto_definition(GotoDefinition, window, cx) => navigate_definition(false, window, cx);
    pub(crate) goto_definition_to_side(GotoDefinitionToSide, window, cx) => navigate_definition(true, window, cx);
    pub(crate) goto_reference(GotoReference, window, cx) => lexical_references(window, cx);
    pub(crate) goto_line(GotoLine, window, cx) => open_quick_panel(Kind::GotoLine, Vec::new(), window, cx);
    pub(crate) quick_panel_next(quick_panel::SelectNext, window, cx) => quick_panel_select(1, window, cx);
    pub(crate) quick_panel_prev(quick_panel::SelectPrev, window, cx) => quick_panel_select(-1, window, cx);
    pub(crate) quick_panel_confirm(quick_panel::Confirm, window, cx) => confirm_quick_panel(window, cx);
    }

    fn navigate_definition(&mut self, side: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_navigation();
        if self.can_navigate_with_server(cx) {
            self.lsp_definition(side, window, cx);
        } else {
            self.local_definition(side, window, cx);
        }
    }

    fn can_navigate_with_server(&self, cx: &App) -> bool {
        self.has_language_server(cx)
    }

    /// The active view and the word at its caret, to navigate from.
    fn navigation_word(&self, cx: &App) -> Option<(Entity<Editor>, String)> {
        let editor = self.active_editor()?;
        let e = editor.read(cx);
        let (_, name) = e.navigation_word(e.state.cursor(), cx)?;
        Some((editor, name))
    }

    pub(crate) fn local_definition(&mut self, side: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((editor, name)) = self.navigation_word(cx) {
            self.indexed_definitions(editor, name, side, window, cx);
        }
    }

    fn indexed_definitions(
        &mut self,
        editor: Entity<Editor>,
        name: String,
        side: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let e = editor.read(cx);
        let place = place_of(&editor, cx);
        let syntax = e.highlighter.syntax_name().to_string();
        let source = (e.definition_source(cx), e.state.cursor());
        self.open_quick_panel(Kind::Definitions, Vec::new(), window, cx);
        let p = self.quick_panel.as_mut().unwrap();
        p.title = Some(format!("Definitions named {name} (by name)"));
        p.lookup = Some((name, place, syntax));
        p.to_side = side;
        p.initial = true;
        p.definition_source = Some(source);
        p.origin = Some(snapshot_view(&editor, cx));
        self.list_symbols(editor, window, cx);
    }

    pub(crate) fn lexical_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_navigation();
        let Some((editor, name)) = self.navigation_word(cx) else {
            return;
        };
        self.open_quick_panel(Kind::References, Vec::new(), window, cx);
        let p = self.quick_panel.as_mut().unwrap();
        p.title = Some(format!("Text matches for {name} — lexical, not semantic"));
        p.progress = Some("0 matches — searching…".into());
        p.lookup = Some((name, Place::View(editor.entity_id()), String::new()));
        p.origin = Some(snapshot_view(&editor, cx));
        self.list_symbols(editor, window, cx);
    }

    fn start_reference_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.quick_panel.as_ref() else {
            return;
        };
        let Some((name, ..)) = p.lookup.clone() else {
            return;
        };
        let panel = p.input.entity_id();
        let generation = self.index.read(cx).generation;
        self.quick_panel.as_mut().unwrap().reference_generation = Some(generation);
        let mut seen = HashSet::new();
        let mut versions = HashMap::new();
        let buffers: Vec<_> = self
            .open_editors(cx)
            .into_iter()
            .filter_map(|e| {
                let state = e.read(cx).read_state(cx);
                seen.insert(state.buffer_id()).then(|| {
                    versions.insert(state.buffer_id(), state.doc.version);
                    (place_of(&e, cx), e.read(cx).title(cx), state.doc.rope.clone())
                })
            })
            .collect();
        self.quick_panel.as_mut().unwrap().reference_versions = versions;
        let hints: HashMap<_, _> =
            self.closed.iter().filter_map(|view| Some((view.path.clone(), view.disk_encoding_hint?))).collect();
        let roots: Vec<_> = crate::config::project::present(&self.roots);
        let mut scope = search::resolve(&search::parse_where(search::OPEN_FOLDERS), &roots, &[], &[]);
        scope.exclude = cx.global::<Settings>().index_exclude_patterns.clone();
        scope.root_excludes = self
            .roots
            .iter()
            .map(|r| (r.path.clone(), r.folder_exclude_patterns.clone(), r.file_exclude_patterns.clone()))
            .collect();
        let index = self.index.read(cx).snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let stop = cancel.clone();
        let work = cx.background_spawn(async move {
            let definitions = index.definitions(&name);
            let opts = crate::editing::text::FindOpts { case_sensitive: true, whole_word: true, ..Default::default() };
            let matcher = crate::searching::pattern::Matcher::new(&name, &opts).expect("literal identifier");
            let cancelled = || stop.load(Ordering::Relaxed);
            let files = search::collect(&scope, &stop, &AtomicUsize::new(0));
            let mut sources: Vec<_> = buffers.into_iter().map(|(p, title, rope)| (p, title, Some(rope))).collect();
            let open: HashSet<_> = sources
                .iter()
                .filter_map(|(p, ..)| match p {
                    Place::File(p) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            sources
                .extend(files.into_iter().filter(|p| !open.contains(p)).map(|p| (Place::File(p), String::new(), None)));
            let mut items = Vec::new();
            let mut last = Instant::now();
            let mut errors = Vec::new();
            for (place, title, rope) in sources {
                if cancelled() {
                    return;
                }
                let found = match rope {
                    Some(rope) => search::search_source(&rope.to_string(), false, &matcher, 0, None, &cancelled),
                    None => match &place {
                        Place::File(path) => {
                            let hint = hints.get(path).copied().or_else(|| {
                                (!hints.is_empty())
                                    .then(|| path.canonicalize().ok().and_then(|p| hints.get(&p).copied()))
                                    .flatten()
                            });
                            match search::read_file_with_hint(path, hint) {
                                Ok(source) => source.search(&matcher, 0, None, &cancelled),
                                Err(why) => {
                                    errors.push(format!(
                                        "{}: {}",
                                        path.display(),
                                        why.unwrap_or_else(|| "file too large".into())
                                    ));
                                    continue;
                                }
                            }
                        }
                        Place::View(_) => unreachable!(),
                    },
                };
                match found {
                    Some(search::Found::Lines { lines, matches, .. }) => {
                        let path = match &place {
                            Place::File(p) => tilde(p),
                            Place::View(_) => title,
                        };
                        let def_lines: HashSet<_> = definitions
                            .iter()
                            .filter(|(entry, _)| match (&entry.path, &entry.key, &place) {
                                (Some(a), _, Place::File(b)) => a == b,
                                (None, EntryKey::Buffer(a, _), Place::View(b)) => a == b,
                                _ => false,
                            })
                            .map(|(_, s)| s.line)
                            .collect();
                        for hit in matches {
                            let (line, col) = hit.start;
                            let text = lines
                                .binary_search_by_key(&line, |(l, _)| *l)
                                .ok()
                                .map(|i| lines[i].1.as_str())
                                .unwrap_or_default();
                            let cols = col..hit.end.1;
                            let (text, _) = search::cut(text, std::slice::from_ref(&cols));
                            items.push(quick_panel::Item {
                                label: format!("{path}:{}", line + 1),
                                detail: text,
                                hint: if def_lines.contains(&line) { "def" } else { "" }.into(),
                                badge: None,
                                action: ItemAction::Location(place.clone(), line, col..hit.end.1),
                            });
                        }
                    }
                    Some(search::Found::Failed(why)) => {
                        let source = match &place {
                            Place::File(path) => path.display().to_string(),
                            Place::View(_) => title,
                        };
                        errors.push(format!("{source}: {why}"));
                    }
                    _ => {}
                }
                if last.elapsed() >= Duration::from_millis(100) {
                    let batch = clone_location_items(&items);
                    if tx.send((quick_panel::Rows::new(Kind::References, batch), false, Vec::new())).is_err() {
                        return;
                    }
                    last = Instant::now();
                }
            }
            let _ = tx.send((quick_panel::Rows::new(Kind::References, items), true, errors));
        });
        let task = cx.spawn_in(window, async move |ws, cx| {
            let _work = work;
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                let (rows, done, errors) = match rx.try_recv() {
                    Ok(batch) => batch,
                    Err(mpsc::TryRecvError::Empty) => continue,
                    // the worker ended without its last batch: nothing more is coming
                    Err(mpsc::TryRecvError::Disconnected) => {
                        ws.update(cx, |ws, cx| {
                            if let Some(p) = (ws.quick_panel.as_mut()).filter(|p| p.input.entity_id() == panel) {
                                p.reference_search = None;
                                p.progress = Some("Reference search stopped".into());
                                cx.notify();
                            }
                        })
                        .ok();
                        return;
                    }
                };
                let result = ws.update_in(cx, |ws, window, cx| {
                    if ws.index.read(cx).generation != generation {
                        ws.load_symbol_rows(window, cx);
                        return false;
                    }
                    let Some(p) =
                        ws.quick_panel.as_mut().filter(|p| p.input.entity_id() == panel && p.kind == Kind::References)
                    else {
                        return false;
                    };
                    p.progress =
                        Some(format!("{} matches{}", rows.items.len(), if done { "" } else { " — searching…" }));
                    p.reference_rows = Some(rows);
                    p.loaded = true;
                    ws.load_symbol_rows(window, cx);
                    if !errors.is_empty() {
                        ws.status_message(format!("Reference search: {}", errors.join("; ")), cx);
                    }
                    true
                });
                if !result.unwrap_or(false) {
                    return;
                }
                if done {
                    ws.update(cx, |ws, _| {
                        if let Some(p) = ws.quick_panel.as_mut().filter(|p| p.input.entity_id() == panel) {
                            p.reference_search = None;
                        }
                    })
                    .ok();
                    return;
                }
            }
        });
        self.quick_panel.as_mut().unwrap().reference_search = Some((editor::Cancel(cancel), task));
    }

    pub(crate) fn hover_definitions(
        &mut self,
        editor: Entity<Editor>,
        word: std::ops::Range<usize>,
        name: String,
        version: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request_source = editor.read(cx).definition_source(cx);
        let fresh = self.index.update(cx, |i, cx| i.ensure_fresh(&editor, cx));
        cx.spawn_in(window, async move |ws, cx| {
            fresh.await;
            let Ok(Some(task)) = ws.update(cx, |ws, cx| {
                if editor.read(cx).definition_source(cx) != request_source {
                    return None;
                }
                let source = place_of(&editor, cx);
                let syntax = editor.read(cx).highlighter.syntax_name().to_string();
                let index = ws.index.read(cx).snapshot();
                Some(cx.background_spawn(async move {
                    let mut definitions = Vec::new();
                    for (entry, symbol) in index.definitions(&name) {
                        let (place, path) = match (entry.path, &entry.key) {
                            (Some(p), _) => (Place::File(p.clone()), tilde(&p)),
                            (None, EntryKey::Buffer(id, _)) => (Place::View(*id), "untitled".into()),
                            _ => continue,
                        };
                        definitions.push((
                            definition_rank(&source, &syntax, &place, &entry.syntax),
                            editor::DefinitionPreview { place, path, key: entry.key, symbol },
                        ));
                    }
                    definitions.sort_by(|a, b| {
                        (a.0, &a.1.path, a.1.symbol.line, a.1.symbol.col).cmp(&(
                            b.0,
                            &b.1.path,
                            b.1.symbol.line,
                            b.1.symbol.col,
                        ))
                    });
                    let definitions = definitions.into_iter().map(|(_, d)| d).collect::<Vec<_>>();
                    let lines = editor::definition_lines(&definitions);
                    (definitions, lines)
                }))
            }) else {
                return;
            };
            let (definitions, lines) = task.await;
            editor
                .update(cx, |e, cx| {
                    if e.definition_source(cx) == request_source {
                        e.show_definitions(word, version, definitions, lines, cx);
                    }
                })
                .ok();
        })
        .detach();
    }

    pub(crate) fn open_definition(
        &mut self,
        target: editor::DefinitionPreview,
        side: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_navigation();
        if side {
            self.definition_side(window, cx);
        }
        if let Some(editor) = self.show_place(&target.place, false, window, cx) {
            if !self.indexed_target_current(&editor, Some(&target.key), cx) {
                self.indexed_definitions(editor, target.symbol.name, false, window, cx);
                return;
            }
            let s = target.symbol;
            editor.update(cx, |e, cx| select_cols(e, s.line, &(s.col..s.col + s.name.chars().count()), cx));
        }
        self.focus_editor(window, cx);
    }

    pub(crate) fn definition_side(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.views.active + 1;
        if self.views.groups.len() == 1 {
            self.views.set_layout(Layout::preset(Preset::Columns(2)));
        }
        self.views.focus(next % self.views.groups.len());
        self.groups_changed(window, cx);
    }

    pub(crate) fn open_quick_panel(
        &mut self,
        kind: Kind,
        items: Vec<quick_panel::Item>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_navigation();
        // Restore an existing preview before replacing the panel and its saved origin.
        self.close_quick_panel(true, cx);
        self.quick_panel = Some(QuickPanel::new(kind, items, window, cx));
        cx.notify();
    }

    pub(crate) fn goto_anything(&mut self, _: &GotoAnything, window: &mut Window, cx: &mut Context<Self>) {
        // the files as the last walk found them, at once; a walk behind the panel brings in
        // what changed since
        let items = self.file_items(cx);
        self.open_quick_panel(Kind::Files, items, window, cx);
        self.walk_files(cx);
    }

    /// Walk the roots for Goto Anything on another thread, keeping the list for this window;
    /// a Goto Anything showing files takes the new list when it lands.
    pub(crate) fn walk_files(&mut self, cx: &mut Context<Self>) {
        if self.roots.is_empty() {
            self.file_list = None;
            return;
        }
        // one at a time; one asked for meanwhile runs when it lands (the roots may have changed)
        if self.file_walk.is_some() {
            self.walk_again = true;
            return;
        }
        let roots = self.roots.clone();
        let walked: Vec<PathBuf> = roots.iter().map(|r| r.path.clone()).collect();
        let walk = cx.background_spawn(async move { quick_panel::root_file_paths(&roots) });
        self.file_walk = Some(cx.spawn(async move |ws, cx| {
            let paths = walk.await;
            ws.update(cx, |ws, cx| {
                ws.file_walk = None;
                let now: Vec<PathBuf> = ws.roots.iter().map(|r| r.path.clone()).collect();
                if std::mem::take(&mut ws.walk_again) || now != walked {
                    ws.walk_files(cx);
                }
                if now != walked {
                    return;
                }
                let same = (ws.file_list.as_ref()).is_some_and(|(f, p)| *f == walked && **p == paths);
                ws.file_list = Some((walked, std::sync::Arc::new(paths)));
                let showing = ws.quick_panel.as_ref().is_some_and(|p| p.kind == Kind::Files);
                if same || !showing {
                    return;
                }
                let items = ws.file_items(cx);
                if let Some(p) = ws.quick_panel.as_mut() {
                    // the highlighted file stays highlighted wherever it lands in the new list
                    let path = |p: &QuickPanel, i: usize| match &p.items[i].action {
                        ItemAction::OpenFile(f) => Some(f.clone()),
                        _ => None,
                    };
                    let was = p.filtered.get(p.selected).and_then(|&i| path(p, i));
                    p.set_items(Kind::Files, items);
                    let q = p.query.clone();
                    p.refilter(&q);
                    if let Some(at) =
                        was.and_then(|w| (0..p.filtered.len()).find(|&k| path(p, p.filtered[k]).as_ref() == Some(&w)))
                    {
                        p.selected = at;
                    }
                }
                ws.quick_panel_preview(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Goto Anything's rows: open files first, then the roots' files from the last walk; each
    /// real file once.
    pub(crate) fn file_items(&self, cx: &App) -> Vec<quick_panel::Item> {
        let open: Vec<PathBuf> = self.views.tabs().filter_map(|t| t.read(cx).doc(cx).path.clone()).collect();
        let walked: Vec<PathBuf> = self.roots.iter().map(|r| r.path.clone()).collect();
        let cached = match &self.file_list {
            Some((f, paths)) if *f == walked => paths.as_slice(),
            _ => &[],
        };
        let several = self.roots.len() > 1;
        let labels: Vec<String> = self.roots.iter().map(|r| r.label()).collect();
        // open files and the walk's files are real paths: relative to each root's real path
        let real: Vec<PathBuf> = (self.roots.iter()).map(|r| crate::editing::text::resolve(&r.path)).collect();
        let item = |root: Option<usize>, p: &PathBuf| match root {
            Some(i) => {
                let label = several.then(|| labels[i].as_str());
                quick_panel::file_item(&real[i], label, p.clone())
            }
            None => quick_panel::file_item(Path::new("/"), None, p.clone()),
        };
        let root_of = |p: &PathBuf| (0..real.len()).find(|&i| p.starts_with(&real[i]));
        let mut seen = HashSet::new();
        (open.iter().map(|p| (root_of(p), p)))
            .chain(cached.iter().map(|(i, p)| (Some(*i), p)))
            .filter(|(_, p)| seen.insert(*p))
            .map(|(i, p)| item(i, p))
            .collect()
    }

    pub(crate) fn command_palette(&mut self, _: &CommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        let git_origin = self.git_palette_focus(window, cx).is_some();
        let mut items = quick_panel::command_items();
        let navigation = self.navigation_capabilities(cx);
        items.retain(|item| match &item.action {
            ItemAction::Dispatch(action)
                if action.as_any().is::<LspTypeDefinition>() || action.as_any().is::<LspTypeDefinitionToSide>() =>
            {
                navigation.type_definition
            }
            ItemAction::Dispatch(action)
                if action.as_any().is::<LspImplementation>() || action.as_any().is::<LspImplementationToSide>() =>
            {
                navigation.implementation
            }
            _ => true,
        });
        if !self.has_range_formatting(cx) {
            items.retain(|item| {
                !matches!(&item.action, ItemAction::Dispatch(action)
                if action.as_any().is::<LspFormatSelection>())
            });
        }
        match self.automation_command_items() {
            Ok(extra) => items.extend(extra),
            Err(e) => self.status_message(e, cx),
        }
        self.open_quick_panel(Kind::Commands, items, window, cx);
        self.quick_panel.as_mut().unwrap().git_origin = git_origin;
    }

    pub(crate) fn paste_from_history(&mut self, _: &PasteFromHistory, window: &mut Window, cx: &mut Context<Self>) {
        // a Find, Find in Files or form field with the keys takes the paste, as cmd-v would
        let input = self.focused_input(window, cx).map(|e| e.downgrade());
        let entries = cx.default_global::<ClipboardHistory>().entries();
        let items = quick_panel::history_items(entries, input);
        self.open_quick_panel(Kind::ClipboardHistory, items, window, cx);
    }

    pub(crate) fn clear_clipboard_history(
        &mut self,
        _: &ClearClipboardHistory,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.default_global::<ClipboardHistory>().clear();
    }

    pub fn quick_panel_input_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_panel.as_ref().is_some_and(|p| p.kind == Kind::Bookmarks) {
            self.refresh_bookmark_panel(cx);
            return;
        }
        let Some(p) = &self.quick_panel else {
            return;
        };
        let query = p.input.read(cx).text(cx);
        // Goto Anything: an `@` prefix turns it into Goto Symbol, like Sublime
        let want = match (p.base, query.starts_with('@')) {
            (Kind::Files, true) => Kind::Symbols,
            (Kind::Files, false) => Kind::Files,
            (k, _) => k,
        };
        let swapped = want != p.kind;
        if swapped {
            let items = match want {
                Kind::Symbols => Vec::new(),
                _ => self.file_items(cx),
            };
            let p = self.quick_panel.as_mut().unwrap();
            p.loaded = false;
            p.confirming = false;
            p.symbol_preview = None;
            p.rows_task = None;
            p.filter_task = None;
            p.set_items(want, items);
            if want == Kind::Symbols {
                if let Some(editor) = self.active_editor() {
                    self.list_symbols(editor, window, cx);
                }
            }
        }
        let p = self.quick_panel.as_mut().unwrap();
        let q = match (p.kind, query.strip_prefix('@')) {
            (Kind::Symbols, Some(rest)) => rest.to_string(),
            _ => query,
        };
        if symbol_kind(p.kind) {
            if !swapped && p.query == q {
                return;
            }
            p.query = q;
            p.confirming = false;
            p.symbol_preview = None;
            return self.filter_symbol_panel(window, cx);
        }
        p.refilter(&q);
        self.quick_panel_preview(cx);
        self.preview_place(window, cx);
        cx.notify();
    }

    /// Refresh a symbol picker's buffer before filling its rows.
    fn list_symbols(&mut self, editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((panel, kind)) = self.quick_panel.as_ref().map(|p| (p.input.entity_id(), p.kind)) else {
            return;
        };
        let fresh = self.index.update(cx, |i, cx| i.ensure_fresh(&editor, cx));
        let task = cx.spawn_in(window, async move |ws, cx| {
            fresh.await;
            ws.update_in(cx, |ws, window, cx| {
                if kind == Kind::Symbols && ws.active_editor().as_ref() != Some(&editor) {
                    return;
                }
                let Some(p) = ws.quick_panel.as_mut().filter(|p| p.input.entity_id() == panel && p.kind == kind) else {
                    return;
                };
                let e = editor.read(cx);
                if !ws.index.read(cx).entry_for(&editor, cx).is_some_and(|entry| {
                    matches!(entry.key, EntryKey::Buffer(_, v) if v == e.doc(cx).version)
                        && entry.syntax == e.highlighter.syntax_name()
                }) {
                    ws.list_symbols(editor.clone(), window, cx);
                    return;
                }
                p.loaded = true;
                if kind == Kind::References {
                    ws.start_reference_search(window, cx);
                } else {
                    ws.load_symbol_rows(window, cx);
                }
            })
            .ok();
        });
        self.quick_panel.as_mut().unwrap().rows_task = Some(task);
    }

    /// Goto Symbol in Project: refresh each buffer once, in the active view's syntax.
    pub(crate) fn goto_symbol_in_project(
        &mut self,
        _: &GotoSymbolInProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_panel(Kind::ProjectSymbols, Vec::new(), window, cx);
        let panel = self.quick_panel.as_ref().unwrap().input.entity_id();
        let (mut seen, mut fresh) = (HashSet::new(), Vec::new());
        for e in self.active_editor().into_iter().chain(self.views.tabs().cloned()) {
            if seen.insert(e.read(cx).doc(cx).id()) {
                fresh.push(self.index.update(cx, |i, cx| i.ensure_fresh(&e, cx)));
            }
        }
        cx.spawn_in(window, async move |ws, cx| {
            for f in fresh {
                f.await;
            }
            ws.update_in(cx, |ws, window, cx| {
                let Some(p) = ws.quick_panel.as_mut().filter(|p| p.input.entity_id() == panel) else {
                    return;
                };
                p.loaded = true;
                ws.load_symbol_rows(window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Index notifications also fill a picker opened before the first build finishes.
    pub(crate) fn load_symbol_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.index.read(cx).generation;
        if let Some(p) = self
            .quick_panel
            .as_mut()
            .filter(|p| p.kind == Kind::References && p.reference_generation.is_some_and(|old| old != generation))
        {
            p.reference_search = None;
            p.rows_task = None;
            p.filter_task = None;
            let old = (
                p.replace_rows(Kind::References, quick_panel::Rows::new(Kind::References, Vec::new())),
                p.reference_rows.take(),
            );
            cx.background_spawn(async move { drop(old) }).detach();
            p.filtered.clear();
            p.selected = 0;
            p.loaded = false;
            p.confirming = false;
            self.start_reference_search(window, cx);
            return;
        }
        let refresh = self.quick_panel.as_ref().filter(|p| p.kind == Kind::References && p.loaded).and_then(|p| {
            let (view, _) = p.symbol_preview.as_ref()?;
            let e = view.read(cx);
            (p.reference_versions.get(&e.doc(cx).id()).copied().unwrap_or(0) != e.doc(cx).version).then(|| view.clone())
        });
        if let Some(view) = refresh {
            self.quick_panel.as_mut().unwrap().loaded = false;
            self.list_symbols(view, window, cx);
            return;
        }
        let reference_rows = self.quick_panel.as_mut().and_then(|p| p.reference_rows.take());
        let Some(p) = self.quick_panel.as_ref().filter(|p| p.loaded) else {
            return;
        };
        let (kind, panel) = (p.kind, p.input.entity_id());
        let (rows, buffer) = match kind {
            Kind::Symbols => {
                let Some(e) = self.active_editor() else {
                    return;
                };
                let Some(entry) = self.index.read(cx).entry_for(&e, cx) else {
                    return;
                };
                let (symbols, syntax) = (entry.symbols.clone(), entry.syntax.clone());
                let view = e.entity_id();
                let expected = (e, symbols.clone());
                let rows = cx.background_spawn(async move {
                    quick_panel::Rows::new(Kind::Symbols, quick_panel::symbol_items(&symbols, &syntax, view))
                });
                (rows, Some(expected))
            }
            Kind::ProjectSymbols | Kind::Definitions => {
                let buffer = match &p.symbol_preview {
                    Some((e, _)) => {
                        let Some(entry) = self.index.read(cx).entry_for(e, cx) else {
                            return;
                        };
                        Some((e.clone(), entry.symbols.clone()))
                    }
                    None => None,
                };
                (self.project_symbol_rows(cx), buffer)
            }
            Kind::References => {
                let Some(rows) = reference_rows else { return };
                (cx.background_spawn(async move { rows }), None)
            }
            _ => return,
        };
        let generation = self.index.read(cx).generation;
        let task = cx.spawn_in(window, async move |ws, cx| {
            let mut rows = Some(rows.await);
            loop {
                let Ok(Some(query)) = ws.update(cx, |ws, _| {
                    ws.quick_panel
                        .as_ref()
                        .filter(|p| p.input.entity_id() == panel && p.kind == kind)
                        .map(|p| p.query.clone())
                }) else {
                    return;
                };
                let keys = rows.as_ref().unwrap().keys.clone();
                let q = query.clone();
                let filtered = cx.background_spawn(async move { crate::chrome::fuzzy::filter(&q, &keys) }).await;
                let finished = ws
                    .update_in(cx, |ws, window, cx| {
                        if ws.index.read(cx).generation != generation {
                            return true;
                        }
                        let current = buffer.as_ref().is_none_or(|(e, expected)| {
                            let index = ws.index.read(cx);
                            index.entry_for(e, cx).is_some_and(|entry| {
                                (kind != Kind::Symbols || ws.active_editor().as_ref() == Some(e))
                                    && std::sync::Arc::ptr_eq(&entry.symbols, expected)
                                    && matches!(entry.key, EntryKey::Buffer(_, v) if v == e.read(cx).doc(cx).version)
                                    && entry.syntax == e.read(cx).highlighter.syntax_name()
                            })
                        });
                        if !current {
                            if let Some(p) =
                                ws.quick_panel.as_mut().filter(|p| p.input.entity_id() == panel && p.kind == kind)
                            {
                                p.rows_task = None;
                                if p.confirming {
                                    ws.confirm_quick_panel(window, cx);
                                }
                            }
                            return true;
                        }
                        let Some(p) =
                            ws.quick_panel.as_mut().filter(|p| p.input.entity_id() == panel && p.kind == kind)
                        else {
                            return true;
                        };
                        // Typing during preparation retries the filter off the window thread.
                        if p.query != query {
                            return false;
                        }
                        // The saved preview range follows edits even when that view makes the edit.
                        let had_selection = p.symbol_preview.is_some();
                        let selected = selected_location(p);
                        let preview = p
                            .symbol_preview
                            .as_ref()
                            .and_then(|(view, ranges)| Some((view.clone(), ranges.first()?.clone())));
                        let old = p.replace_rows(kind, rows.take().unwrap());
                        p.rows_task = None;
                        p.filter_task = None;
                        p.filtered = filtered;
                        let recovered = selected.and_then(|(name, place)| {
                            let (view, range) = preview.as_ref()?;
                            let e = view.read(cx);
                            if match &place {
                                Place::File(path) => e.doc(cx).path.as_ref() != Some(path),
                                Place::View(id) => {
                                    view.entity_id() != *id && !(kind == Kind::References && e.doc(cx).path.is_none())
                                }
                            } {
                                return None;
                            }
                            let (line, col) = e.doc(cx).point(range.start);
                            let cols = col..col + range.len();
                            p.filtered.iter().position(|&i| {
                                let item = &p.items[i];
                                (kind == Kind::References || item.label == name)
                                    && matches!(&item.action, ItemAction::Location(p, l, c)
                                    if *p == place && *l == line && *c == cols)
                            })
                        });
                        p.selected = recovered.unwrap_or(0);
                        if had_selection && recovered.is_none() {
                            p.confirming = false;
                        }
                        cx.background_spawn(async move { drop(old) }).detach();
                        ws.symbol_panel_ready(window, cx);
                        true
                    })
                    .unwrap_or(true);
                if finished {
                    break;
                }
            }
        });
        self.quick_panel.as_mut().unwrap().rows_task = Some(task);
    }

    fn filter_symbol_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.quick_panel.as_mut() else {
            return;
        };
        let (panel, kind, query, keys) = (p.input.entity_id(), p.kind, p.query.clone(), p.keys.clone());
        p.filtered.clear();
        p.selected = 0;
        let filter = {
            let (query, keys) = (query.clone(), keys.clone());
            cx.background_spawn(async move { crate::chrome::fuzzy::filter(&query, &keys) })
        };
        p.filter_task = Some(cx.spawn_in(window, async move |ws, cx| {
            let filtered = filter.await;
            ws.update_in(cx, |ws, window, cx| {
                let Some(p) = ws.quick_panel.as_mut().filter(|p| {
                    p.input.entity_id() == panel
                        && p.kind == kind
                        && p.query == query
                        && std::sync::Arc::ptr_eq(&p.keys, &keys)
                }) else {
                    return;
                };
                p.filter_task = None;
                p.filtered = filtered;
                ws.symbol_panel_ready(window, cx);
            })
            .ok();
        }));
        cx.notify();
    }

    fn symbol_panel_ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.quick_panel.as_ref() else {
            return;
        };
        if !p.loaded || p.rows_task.is_some() || p.filter_task.is_some() {
            cx.notify();
            return;
        }
        if p.kind == Kind::Definitions && p.initial {
            let current = p.origin.as_ref().is_some_and(|(editor, ..)| {
                p.definition_source.as_ref().is_some_and(|(source, caret)| {
                    let e = editor.read(cx);
                    self.active_editor().as_ref() == Some(editor)
                        && e.definition_source(cx) == *source
                        && e.state.cursor() == *caret
                })
            });
            if !current {
                self.quick_panel.as_mut().unwrap().origin = None;
                self.close_quick_panel(false, cx);
                self.focus_editor(window, cx);
                cx.notify();
                return;
            }
            let count = p.items.len();
            self.quick_panel.as_mut().unwrap().initial = false;
            if count == 0 {
                let (name, ..) = (self.quick_panel.as_ref()).and_then(|p| p.lookup.as_ref()).unwrap();
                let suffix = if self.index.read(cx).progress().is_some() { " (index incomplete)" } else { "" };
                let message = format!("No definition named {name} found in the index{suffix}");
                self.close_quick_panel(true, cx);
                self.focus_editor(window, cx);
                self.status_message(message, cx);
                return;
            }
            if count == 1 {
                self.confirm_quick_panel(window, cx);
                return;
            }
        }
        if self.quick_panel.as_ref().unwrap().confirming {
            self.confirm_quick_panel(window, cx);
        } else {
            self.quick_panel_preview(cx);
            self.preview_place(window, cx);
        }
        cx.notify();
    }

    /// Cheap entry snapshots on the window thread; sorting and row strings on the pool.
    fn project_symbol_rows(&self, cx: &mut Context<Self>) -> Task<quick_panel::Rows> {
        let kind = self.quick_panel.as_ref().map_or(Kind::ProjectSymbols, |p| p.kind);
        let lookup = self.quick_panel.as_ref().and_then(|p| p.lookup.clone());
        let several = self.roots.len() > 1;
        let roots: Vec<(PathBuf, String)> =
            self.roots.iter().map(|r| (crate::editing::text::resolve(&r.path), r.label())).collect();
        let index = self.index.read(cx).snapshot();
        let titles: HashMap<_, _> = self.views.tabs().map(|e| (e.entity_id(), e.read(cx).title(cx))).collect();
        cx.background_spawn(async move {
            let mut found = Vec::new();
            for entry in index.entries() {
                let (symbols, syntax) = (entry.symbols, entry.syntax);
                let (place, mut name) = match (entry.path, entry.key) {
                    (Some(path), _) => (Place::File(path), String::new()),
                    (None, EntryKey::Buffer(id, _)) => {
                        let Some(title) = titles.get(&id) else {
                            continue;
                        };
                        (Place::View(id), title.clone())
                    }
                    _ => continue,
                };
                if let Place::File(path) = &place {
                    name = match roots.iter().find(|(r, _)| path.starts_with(r)) {
                        Some((r, label)) => {
                            quick_panel::file_item(r, several.then_some(label.as_str()), path.clone()).detail
                        }
                        None => quick_panel::file_item(Path::new("/"), None, path.clone()).detail,
                    };
                }
                found.extend(symbols.iter().filter(|s| lookup.as_ref().is_none_or(|(name, ..)| s.name == *name)).map(
                    |s| {
                        let rank = lookup
                            .as_ref()
                            .map_or(0, |(_, source, language)| definition_rank(source, language, &place, &syntax));
                        let mut item = quick_panel::project_symbol_item(s, &place, &name);
                        if kind == Kind::Definitions {
                            item.detail = format!("{} — {}", item.detail, s.signature);
                        }
                        ((rank, s.name.to_lowercase(), name.clone(), s.line, s.col), item)
                    },
                ));
            }
            found.sort_by(|a, b| a.0.cmp(&b.0));
            quick_panel::Rows::new(kind, found.into_iter().map(|(_, item)| item).collect())
        })
    }

    /// Sublime previews the target line of `:N` while you browse.
    fn quick_panel_preview(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        let Some(p) = &mut self.quick_panel else {
            return;
        };
        if let Some(n) = p.line_query() {
            if p.origin.is_none() {
                p.origin = Some(snapshot_view(&editor, cx));
            }
            editor.update(cx, |e, cx| e.goto_line(n, cx));
        }
    }

    /// A row's place shown as the row is highlighted: its file as the active
    /// group's preview tab, which the next one replaces; a file open in the group, or an
    /// untitled buffer's view, as it is. The caret goes on the target and the panel keeps the
    /// keys; Escape brings back the view it started from, Enter keeps the tab.
    pub(crate) fn preview_place(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = &self.quick_panel else {
            return;
        };
        let Some(ItemAction::Location(place, line, cols)) = (p.filtered.get(p.selected)).map(|&i| &p.items[i].action)
        else {
            return;
        };
        let (place, line, cols) = (place.clone(), *line, cols.clone());
        // opening and closing tabs would end the panel: it stands aside meanwhile
        let mut p = self.quick_panel.take().expect("checked above");
        if p.origin.is_none() {
            p.origin = self.active_editor().map(|e| snapshot_view(&e, cx));
        }
        let before = p.views.get_or_insert_with(|| self.views.clone());
        self.views.groups[self.views.active].preview = None;
        self.keep_blank_for_preview = true;
        let shown = self.show_place(&place, true, window, cx);
        self.keep_blank_for_preview = false;
        if let Some(view) = shown {
            if before.find(&view).is_some() && !p.moved.iter().any(|(e, ..)| *e == view) {
                p.moved.push(snapshot_view(&view, cx));
            }
            view.update(cx, |e, cx| select_cols(e, line, &cols, cx));
            if symbol_kind(p.kind) {
                let e = view.read(cx);
                p.symbol_preview = Some((view.clone(), vec![e.state.selections[e.state.primary()].range()]));
            }
            if let Some(old) = p.opened.take().filter(|e| *e != view) {
                if self.preview_can_close(&old, cx) {
                    self.remove_view(&old, cx);
                }
            }
            if before.find(&view).is_none() {
                self.views.groups[self.views.active].preview = Some(view.clone());
                p.opened = Some(view);
            }
        } else if let Some((g, _)) = p.opened.as_ref().and_then(|e| self.views.find(e)) {
            self.views.groups[g].preview = p.opened.clone();
        }
        p.input.read(cx).focus(window);
        self.quick_panel = Some(p);
        cx.notify();
    }

    /// `place`'s view in the active group: its file opened there (as the preview tab when
    /// `preview`), or the untitled buffer's view selected. `None` while the file is read in
    /// the background, or when it is gone.
    fn show_place(
        &mut self,
        place: &Place,
        preview: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Editor>> {
        match place {
            Place::File(path) => {
                match preview {
                    true => self.open_preview(path.clone(), window, cx),
                    false => self.open_path_here(path.clone(), window, cx),
                }
                let shown =
                    (self.active_editor()).filter(|e| e.read(cx).doc(cx).path.as_deref() == Some(path.as_path()));
                // a file read in the background (over 4 MB, which the index leaves
                // out, so one that grew since) is not previewed; Enter opens it
                if shown.is_none() && preview {
                    if let Some(kind) = self.previewing.get_mut(path) {
                        *kind = PendingOpen::Cancelled;
                    }
                }
                shown
            }
            Place::View(id) => {
                let source = self.open_editors(cx).into_iter().find(|e| e.entity_id() == *id)?;
                let local = self
                    .views
                    .tabs()
                    .find(|e| **e == source)
                    .cloned()
                    .or_else(|| self.views.tabs().find(|e| e.read(cx).same_document(source.read(cx))).cloned());
                if let Some(view) = local {
                    let (g, i) = self.views.find(&view)?;
                    if g == self.views.active || preview {
                        self.views.select(g, i);
                        return Some(view);
                    }
                }
                let clone = source.update(cx, |e, cx| cx.new(|cx| e.another_view(cx)));
                self.attach(clone.clone(), true, window, cx);
                Some(clone)
            }
        }
    }

    fn quick_panel_select(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.quick_panel {
            let len = p.filtered.len();
            if len > 0 {
                p.selected = (p.selected as isize + step).rem_euclid(len as isize) as usize;
            }
            self.quick_panel_preview(cx);
            self.preview_place(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn quick_panel_dismiss(
        &mut self,
        _: &quick_panel::Dismiss,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_quick_panel(true, cx);
        self.focus_editor(window, cx);
        cx.notify();
    }

    /// Other actions keep the tabs they chose while restoring previewed selections and scroll.
    pub(crate) fn cancel_quick_panel(&mut self, cx: &mut Context<Self>) {
        self.invalidate_navigation();
        self.close_quick_panel(false, cx);
    }

    fn preview_can_close(&self, view: &Entity<Editor>, cx: &App) -> bool {
        !view.read(cx).doc(cx).unsaved()
            || self.open_editors(cx).iter().any(|other| other != view && other.read(cx).same_document(view.read(cx)))
    }

    pub(super) fn close_quick_panel(&mut self, restore_tabs: bool, cx: &mut Context<Self>) {
        if let Some(mut p) = self.quick_panel.take() {
            if p.kind == Kind::Spelling {
                self.spelling_job = None;
            }
            self.restore_panel_views(&p, None, cx);
            if let Some(opened) = p.opened.as_ref().filter(|e| self.views.is_preview(e)) {
                if !self.preview_can_close(opened, cx) {
                    self.views.promote(opened);
                } else if restore_tabs {
                    self.remove_view(opened, cx);
                }
            }
            if let Some(before) = p.views.take() {
                for (g, old) in before.groups.into_iter().enumerate() {
                    if g >= self.views.groups.len() {
                        continue;
                    }
                    let group = &mut self.views.groups[g];
                    if restore_tabs {
                        group.active = old
                            .tabs
                            .get(old.active)
                            .and_then(|e| group.tabs.iter().position(|t| t == e))
                            .unwrap_or(group.active);
                        group.selected = old.selected.into_iter().filter(|e| group.tabs.contains(e)).collect();
                    }
                    if restore_tabs || group.preview.is_none() {
                        group.preview = old.preview.filter(|e| group.tabs.contains(e) && !e.read(cx).doc(cx).unsaved());
                    }
                }
                if restore_tabs {
                    self.views.active = before.active.min(self.views.groups.len() - 1);
                }
            }
            let rows = (std::mem::take(&mut p.items), std::mem::take(&mut p.keys));
            cx.background_spawn(async move { drop(rows) }).detach();
        }
        cx.notify();
    }

    fn restore_panel_views(&mut self, p: &QuickPanel, keep: Option<&Entity<Editor>>, cx: &mut Context<Self>) {
        for (editor, sels, (line, into, left)) in p.origin.iter().chain(&p.moved) {
            if Some(editor) != keep {
                editor.update(cx, |e, cx| {
                    e.restore_view(sels.clone(), (*line, *into), cx);
                    e.follow_scroll(None, Some(*left), cx);
                });
            }
        }
    }

    fn indexed_target_current(&self, view: &Entity<Editor>, expected: Option<&EntryKey>, cx: &App) -> bool {
        let (index, editor) = (self.index.read(cx), view.read(cx));
        if let Some(key) = expected {
            let unchanged = match key {
                EntryKey::Buffer(_, version) => {
                    editor.doc(cx).version == *version
                        && index.entry_for(view, cx).is_some_and(|entry| entry.key == *key)
                }
                EntryKey::File(path, stamp) => {
                    editor.doc(cx).path.as_ref() == Some(path)
                        && !editor.doc(cx).dirty
                        && editor.doc(cx).disk.as_ref() == Some(stamp)
                }
            };
            if !unchanged {
                return false;
            }
        }
        let syntax = editor.highlighter.syntax_name();
        match index.entry_for(view, cx) {
            Some(entry) => {
                matches!(entry.key, EntryKey::Buffer(_, v) if v == editor.doc(cx).version) && entry.syntax == syntax
            }
            // A clean preview can use its disk entry while its first buffer parse runs.
            None => {
                !editor.doc(cx).dirty
                    && editor.doc(cx).path.as_deref().and_then(|path| index.disk_entry(path)).is_some_and(|entry| {
                        matches!(&entry.key, EntryKey::File(path, stamp)
                                if editor.doc(cx).path.as_ref() == Some(path)
                                    && editor.doc(cx).disk.as_ref() == Some(stamp))
                            && entry.syntax == syntax
                    })
            }
        }
    }

    pub fn confirm_quick_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_template(window, cx) {
            return;
        }
        if self.quick_panel.as_ref().is_some_and(|p| {
            p.language_navigation.as_ref().is_some_and(|request| {
                !window.is_window_active()
                    || !p.input.read(cx).is_focused(window)
                    || !self.language_request_current(request, cx)
            })
        }) {
            self.status_message("Language navigation discarded: source or server changed", cx);
            return;
        }
        if self.confirm_breadcrumb(window, cx) {
            return;
        }
        if self.quick_panel.as_ref().is_some_and(|p| p.kind == Kind::Bookmarks) {
            self.confirm_bookmark_panel(window, cx);
            return;
        }
        if self.quick_panel.as_ref().is_some_and(|p| p.kind == Kind::Spelling) {
            self.confirm_spelling(window, cx);
            return;
        }
        if let Some(p) = self.quick_panel.as_mut().filter(|p| symbol_kind(p.kind)) {
            if !p.loaded || p.rows_task.is_some() || p.filter_task.is_some() {
                p.confirming = true;
                return;
            }
        }
        let stale = self
            .quick_panel
            .as_ref()
            .filter(|p| symbol_kind(p.kind))
            .and_then(|p| match &p.items.get(*p.filtered.get(p.selected)?)?.action {
                ItemAction::Location(place, ..) => {
                    self.active_editor().into_iter().chain(self.views.tabs().cloned()).find(|e| match place {
                        Place::File(path) => e.read(cx).doc(cx).path.as_ref() == Some(path),
                        Place::View(id) => {
                            e.entity_id() == *id
                                || (p.kind == Kind::References
                                    && p.symbol_preview.as_ref().is_some_and(|(view, _)| view == e))
                        }
                    })
                }
                _ => None,
            })
            .filter(|e| {
                if self.quick_panel.as_ref().is_some_and(|p| {
                    p.kind == Kind::References
                        && p.reference_versions.get(&e.read(cx).doc(cx).id()).copied().unwrap_or(0)
                            != e.read(cx).doc(cx).version
                }) {
                    return true;
                }
                !self.indexed_target_current(e, None, cx)
            });
        if let Some(editor) = stale {
            let p = self.quick_panel.as_mut().unwrap();
            p.loaded = false;
            p.confirming = true;
            self.list_symbols(editor, window, cx);
            return;
        }
        let Some(mut p) = self.quick_panel.take() else {
            return;
        };
        let git_command = p.filtered.get(p.selected).and_then(|idx| p.items.get(*idx)).is_some_and(
            |item| matches!(&item.action, ItemAction::Dispatch(action) if action.name().starts_with("workspace::Git")),
        );
        if let Some(focus) = self.git_palette_focus(window, cx).filter(|_| git_command && p.git_origin) {
            window.focus(&focus);
            self.git.selection_active = true;
        } else {
            self.focus_editor(window, cx);
        }
        cx.notify();
        let origin = p.origin.as_ref().and_then(|(_, sels, _)| Some(sels.last()?.head));
        let jump = |e: &mut Editor, line: usize, cx: &mut Context<Editor>| {
            if let Some(o) = origin {
                e.record_jump(o);
            }
            e.goto_line(line, cx);
        };
        if let Some(n) = p.line_query() {
            self.with_active(cx, |e, cx| jump(e, n, cx));
            return;
        }
        let Some(&idx) = p.filtered.get(p.selected) else {
            self.quick_panel = Some(p);
            self.close_quick_panel(true, cx);
            self.focus_editor(window, cx);
            return;
        };
        match &p.items[idx].action {
            ItemAction::Template(..) => unreachable!("templates validate before focus changes"),
            ItemAction::Explorer(..) => unreachable!("breadcrumb choices validate before focus changes"),
            ItemAction::Bookmark(..) => unreachable!("bookmarks validate before focus changes"),
            ItemAction::Spelling(..) => {
                unreachable!("spelling choices are source-guarded before focus changes")
            }
            ItemAction::Language(choice) => self.choose_language(choice.clone(), window, cx),
            ItemAction::OpenFile(path) => self.open_path_here(path.clone(), window, cx),
            ItemAction::Dispatch(action) => window.dispatch_action(action.boxed_clone(), cx),
            ItemAction::PasteHistory(entry, input) => {
                // the empty-history row
                if entry.text.is_empty() {
                    return;
                }
                // recorded again, it moves to the top
                (cx.default_global::<ClipboardHistory>()).record(&entry.text, entry.whole_lines);
                let item = editor::clipboard_item((entry.text.clone(), entry.whole_lines));
                cx.write_to_clipboard(item);
                match input.as_ref().and_then(|e| e.upgrade()) {
                    Some(input) => {
                        input.update(cx, |e, cx| e.paste_entry(entry, cx));
                        input.read(cx).focus(window);
                    }
                    None => {
                        self.with_active(cx, |e, cx| e.paste_entry(entry, cx));
                    }
                }
            }
            ItemAction::SetField(text, field) | ItemAction::AppendField(text, field) => {
                let append = matches!(&p.items[idx].action, ItemAction::AppendField(..));
                if text == crate::searching::search::PREVIOUS_MATCHING_LINES {
                    let scope = self
                        .result_set
                        .as_ref()
                        .ok_or_else(|| "No source results".to_string())
                        .and_then(crate::searching::results::ResultSet::line_scope);
                    match scope {
                        Ok(scope) => self.previous_lines = Some(scope),
                        Err(error) => {
                            self.status_message(error, cx);
                            return;
                        }
                    }
                }
                if let Some(field) = field.upgrade() {
                    field.update(cx, |e, cx| {
                        let text = match append {
                            true => crate::searching::search::append_token(&e.text(cx), text),
                            false => text.clone(),
                        };
                        e.set_text(&text, cx);
                        sel_of(&e.document, &mut e.state, cx).set_cursor(doc_of(&e.document, cx).len());
                    });
                    field.read(cx).focus(window);
                }
            }
            ItemAction::View(id) => self.activate_view(*id, window, cx),
            ItemAction::Location(place, line, cols) => {
                if p.to_side {
                    self.restore_panel_views(&p, None, cx);
                    if let Some(before) = &p.views {
                        for (group, old) in self.views.groups.iter_mut().zip(&before.groups) {
                            if let Some(active) = old.tabs.get(old.active) {
                                if let Some(i) = group.tabs.iter().position(|e| e == active) {
                                    group.active = i;
                                }
                            }
                        }
                        self.views.active = before.active;
                    }
                    self.definition_side(window, cx);
                }
                let target = self.show_place(place, false, window, cx);
                // the panel's preview of another file goes; the one chosen is kept as a tab
                let other = |t: &&Entity<Editor>| Some(*t) != target.as_ref();
                if let Some(t) = (p.opened.as_ref().filter(other)).filter(|t| self.views.is_preview(t)) {
                    if !self.preview_can_close(t, cx) {
                        self.views.promote(t);
                    } else {
                        self.remove_view(t, cx);
                    }
                }
                self.restore_panel_views(&p, target.as_ref(), cx);
                if matches!(place, Place::File(_)) {
                    if let (Some((blank, ..)), Some(view)) = (&p.origin, &target) {
                        if Self::is_blank_untitled(blank, cx)
                            && p.views.as_ref().is_some_and(|before| {
                                before.find(blank).is_some_and(|(g, _)| before.groups[g].tabs.len() == 1)
                            })
                            && self.views.find(blank).map(|(g, _)| g) == self.views.find(view).map(|(g, _)| g)
                        {
                            self.remove_view(blank, cx);
                        }
                    }
                }
                if let Some(before) = &p.views {
                    for old in &before.groups {
                        if let Some((g, _)) = old.preview.as_ref().and_then(|e| self.views.find(e)) {
                            self.views.groups[g].preview = old.preview.clone();
                        }
                    }
                }
                match target {
                    Some(view) => {
                        self.views.promote(&view);
                        let from = (p.origin.as_ref()).filter(|(o, ..)| *o == view).and(origin);
                        view.update(cx, |e, cx| {
                            from.inspect(|&o| e.record_jump(o));
                            select_cols(e, *line, cols, cx);
                        });
                    }
                    // read in the background: the caret goes there when it is in
                    None => {
                        if let Place::File(path) = place {
                            let jump = PendingJump {
                                line: *line,
                                hit: Some((cols.start, None)),
                                selection: None,
                                browser: None,
                            };
                            self.pending_jumps.insert(path.clone(), jump);
                        }
                    }
                }
                self.focus_editor(window, cx);
            }
        }
        let rows = (std::mem::take(&mut p.items), std::mem::take(&mut p.keys));
        cx.background_spawn(async move { drop(rows) }).detach();
    }
}

/// Whether `kind` lists symbols, filtered and prepared off the window thread.
fn symbol_kind(kind: Kind) -> bool {
    matches!(kind, Kind::Symbols | Kind::ProjectSymbols | Kind::Definitions | Kind::References)
}

/// The file `editor` shows, else the view itself for an untitled buffer.
fn place_of(editor: &Entity<Editor>, cx: &App) -> Place {
    (editor.read(cx).doc(cx).path.clone()).map_or(Place::View(editor.entity_id()), Place::File)
}

fn selected_location(p: &QuickPanel) -> Option<(String, Place)> {
    let idx = *p.filtered.get(p.selected)?;
    let item = p.items.get(idx)?;
    match &item.action {
        ItemAction::Location(place, ..) => Some((item.label.clone(), place.clone())),
        _ => None,
    }
}

fn snapshot_view(editor: &Entity<Editor>, cx: &App) -> quick_panel::Origin {
    let e = editor.read(cx);
    let (top, into) = e.top(cx);
    (editor.clone(), e.state.selections_primary_last(), (top, into, e.scroll_position(cx).2))
}

/// Chars `cols` of line `line` selected (each clamped to the line).
fn select_cols(e: &mut Editor, line: usize, cols: &std::ops::Range<usize>, cx: &mut Context<Editor>) {
    let (start, end) = (e.doc(cx).offset(line, cols.start), e.doc(cx).offset(line, cols.end));
    e.select_range(start..end, cx);
}

fn definition_rank(source: &Place, syntax: &str, place: &Place, language: &str) -> u8 {
    if source == place {
        return 0;
    }
    if matches!((source, place), (Place::File(a), Place::File(b)) if a.parent() == b.parent()) {
        return 1;
    }
    if syntax == language {
        2
    } else {
        3
    }
}

fn clone_location_items(items: &[quick_panel::Item]) -> Vec<quick_panel::Item> {
    items
        .iter()
        .filter_map(|i| match &i.action {
            ItemAction::Location(place, line, cols) => Some(quick_panel::Item {
                label: i.label.clone(),
                detail: i.detail.clone(),
                hint: i.hint.clone(),
                badge: i.badge,
                action: ItemAction::Location(place.clone(), *line, cols.clone()),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{settle_until, type_in, window};
    use gpui::TestAppContext;

    #[test]
    fn definitions_sort_current_file_directory_syntax_then_others() {
        let source = Place::File("/project/src/current.rs".into());
        let mut places = [
            (Place::File("/project/other/a.py".into()), "Python"),
            (Place::File("/project/other/a.rs".into()), "Rust"),
            (Place::File("/project/src/b.py".into()), "Python"),
            (source.clone(), "Rust"),
        ];
        places.sort_by_key(|(place, syntax)| definition_rank(&source, "Rust", place, syntax));
        assert_eq!(
            places.iter().map(|(place, _)| place.clone()).collect::<Vec<_>>(),
            [
                source,
                Place::File("/project/src/b.py".into()),
                Place::File("/project/other/a.rs".into()),
                Place::File("/project/other/a.py".into()),
            ]
        );
    }

    #[gpui::test]
    fn replacing_a_quick_panel_restores_the_previous_preview(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        type_in(&ws, cx, "one\ntwo\nthree");
        ws.update_in(cx, |ws, window, cx| {
            ws.with_active(cx, |e, cx| e.select_range(1..2, cx));
            ws.goto_line(&GotoLine, window, cx);
            ws.quick_panel.as_mut().unwrap().refilter("3");
            ws.quick_panel_preview(cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).state.cursor(), 8);
            ws.command_palette(&CommandPalette, window, cx);
            assert_eq!(
                ws.active_editor().unwrap().read(cx).state.selections,
                vec![crate::editing::text::Selection::new(1, 2)]
            );
            ws.quick_panel_dismiss(&quick_panel::Dismiss, window, cx);
            assert_eq!(
                ws.active_editor().unwrap().read(cx).state.selections,
                vec![crate::editing::text::Selection::new(1, 2)]
            );
        });
    }

    #[gpui::test]
    fn a_local_definition_does_not_jump_after_its_caret_or_buffer_changes(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let text = "fn target() {}\n\nfn caller() { target(); }\n";
        type_in(&ws, cx, text);
        ws.update_in(cx, |ws, window, cx| {
            ws.set_syntax(&SetSyntax { name: "Rust".into() }, window, cx);
        });
        let editor = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
        for replace in [false, true] {
            ws.update(cx, |ws, cx| {
                ws.index.update(cx, |i, cx| i.ensure_fresh(&editor, cx)).detach();
            });
            settle_until(cx, |cx| {
                ws.read_with(cx, |ws, cx| {
                    ws.index.read(cx).entry_for(&editor, cx).is_some_and(
                        |entry| matches!(entry.key, EntryKey::Buffer(_, v) if v == editor.read(cx).doc(cx).version),
                    )
                })
            });
            ws.update_in(cx, |ws, window, cx| {
                ws.with_active(cx, |e, cx| e.sel(cx).set_cursor(34));
                ws.local_definition(false, window, cx);
                ws.with_active(cx, |e, cx| {
                    if replace {
                        e.set_text(text, cx);
                    }
                    e.sel(cx).set_cursor(if replace { 34 } else { 15 });
                });
            });
            settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.quick_panel.is_none()));
            assert_eq!(editor.read_with(cx, |e, _| e.state.cursor()), if replace { 34 } else { 15 });
        }
    }

    #[gpui::test]
    fn goto_symbol_in_pony_lists_its_tags_scm_definitions_with_their_badges(cx: &mut TestAppContext) {
        let dir = crate::workspace::ui_tests::temp("orlo-goto-pony");
        let sample = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/languages/pony/samples");
        let path = dir.join("sample.pony");
        std::fs::copy(sample.join("sample.pony"), &path).unwrap();
        let (ws, cx) = window(cx);
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(path.clone(), window, cx);
            assert_eq!(ws.active_editor().unwrap().read(cx).highlighter.syntax_name(), "Pony");
            ws.goto_symbol(&GotoSymbol, window, cx);
        });
        crate::workspace::ui_tests::settle_rows(&ws, cx, 21);
        ws.read_with(cx, |ws, _| {
            let items = &ws.quick_panel.as_ref().unwrap().items;
            let rows: Vec<_> = items.iter().take(5).map(|i| (i.label.as_str(), i.badge.map(|b| b.0))).collect();
            assert_eq!(
                rows,
                [
                    ("Number", Some('c')),
                    ("Main", Some('c')),
                    ("create", Some('f')),
                    ("tick", Some('f')),
                    ("_report", Some('f'))
                ]
            );
            assert_eq!((items[12].label.as_str(), items[12].badge.map(|b| b.0)), ("Named", Some('i')));
            assert!(matches!(&items[1].action, ItemAction::Location(_, 9, name) if *name == (6..10)));
        });
    }
}
