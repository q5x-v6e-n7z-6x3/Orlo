//! Transient explorer locations and active-file breadcrumbs over the existing sidebar/picker.

use super::pickers::choice;
use super::*;
use orlo_project::explorer::Target;
use std::sync::Arc;

#[derive(Clone)]
struct Source {
    view: Option<(Entity<Editor>, u64, u64, Option<PathBuf>)>,
    roots: Vec<crate::config::project::Root>,
    generation: (u64, u64),
}

impl Source {
    fn capture(ws: &Workspace, cx: &App) -> Self {
        let view = ws.active_editor().map(|view| {
            let d = view.read(cx).doc(cx);
            (view.clone(), d.id(), d.version, d.path.clone())
        });
        Self { view, roots: ws.roots.clone(), generation: (ws.restore_gen, ws.language.navigation_generation) }
    }

    fn current(&self, ws: &Workspace, cx: &App) -> bool {
        if self.roots != ws.roots || self.generation != (ws.restore_gen, ws.language.navigation_generation) {
            return false;
        }
        match (&self.view, ws.active_editor()) {
            (None, None) => true,
            (Some((view, id, version, path)), Some(active)) if view == &active => {
                let d = view.read(cx).doc(cx);
                d.id() == *id && d.version == *version && d.path == *path
            }
            _ => false,
        }
    }
}

pub(crate) struct BreadcrumbChoice {
    source: Arc<Source>,
    target: Target,
    children: bool,
}

/// The ordinary opener retains the initiating surface until it can admit this exact file.
#[derive(Clone)]
pub(super) struct BreadcrumbOpen {
    choice: Arc<BreadcrumbChoice>,
    request: u64,
    surface: gpui::EntityId,
    focus: Option<FocusHandle>,
}

impl BreadcrumbOpen {
    pub(super) fn target(&self) -> &Target {
        &self.choice.target
    }

    pub(super) fn current(&self, ws: &Workspace, window: &Window, cx: &App) -> bool {
        self.request == ws.sidebar.location_request
            && self.choice.source.current(ws, cx)
            && self.focus == window.focused(cx)
            && ws.quick_panel.as_ref().map(|p| p.input.entity_id()) == Some(self.surface)
    }

    pub(super) fn finish(&self, ws: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
        ws.close_quick_panel(false, cx);
        ws.focus_editor(window, cx);
    }
}

enum Navigation {
    Visit(PathBuf, Option<Target>),
    Step(bool),
    Parent(PathBuf),
}

impl Workspace {
    pub(crate) fn explorer_back(&mut self, _: &ExplorerBack, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer_navigation(Navigation::Step(false), window, cx);
    }

    pub(crate) fn explorer_forward(&mut self, _: &ExplorerForward, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer_navigation(Navigation::Step(true), window, cx);
    }

    pub(crate) fn explorer_parent(&mut self, _: &ExplorerParent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.explorer_folder() else {
            return self.status_message("Select a project folder first", cx);
        };
        let Some(parent) = path.parent().filter(|p| self.roots.iter().any(|r| p.starts_with(&r.path))) else {
            return self.status_message("At the project root; Parent keeps project folders unchanged", cx);
        };
        self.explorer_navigation(Navigation::Parent(parent.to_path_buf()), window, cx);
    }

    fn explorer_folder(&self) -> Option<PathBuf> {
        self.sidebar
            .locations
            .current()
            .map(|t| t.path.clone())
            .or_else(|| {
                self.sidebar.selected.as_ref().map(|path| {
                    // This is cached tree data, with no stat in keyboard admission.
                    match self.sidebar.is_directory(path) {
                        Some(false) => path.parent().unwrap_or(path).to_path_buf(),
                        _ => path.clone(),
                    }
                })
            })
            .or_else(|| self.roots.first().map(|r| r.path.clone()))
    }

    pub(crate) fn navigate_explorer(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer_navigation(Navigation::Visit(path, None), window, cx);
    }

    fn explorer_navigation(&mut self, navigation: Navigation, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.location_request += 1;
        let request = self.sidebar.location_request;
        let source = Source::capture(self, cx);
        let selected = self.sidebar.selected.clone();
        let focus = window.focused(cx);
        let picker = self.quick_panel.as_ref().map(|p| p.input.entity_id());
        let mut history = self.sidebar.locations.clone();
        let seed = match &navigation {
            Navigation::Visit(path, _) => self
                .roots
                .iter()
                .filter(|r| path.starts_with(&r.path))
                .max_by_key(|r| r.path.components().count())
                .map(|r| r.path.clone()),
            Navigation::Parent(_) => self.explorer_folder(),
            Navigation::Step(_) => None,
        };
        let roots = self.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>();
        let task = cx.background_spawn(async move {
            if history.current().is_none() {
                if let Some(seed) = seed.and_then(|p| Target::directory(p).ok()) {
                    history.visit(seed);
                }
            }
            let target = match navigation {
                Navigation::Visit(path, expected) => {
                    expected.map_or_else(|| Target::directory(path), Ok).and_then(|t| {
                        t.validate()?;
                        history.visit(t.clone());
                        Ok(Some(t))
                    })
                }
                Navigation::Parent(path) => Target::directory(path).map(|t| {
                    history.visit(t.clone());
                    Some(t)
                }),
                Navigation::Step(forward) => Ok(history.step(forward)),
            };
            let target = target.map_err(|e| e.to_string())?.ok_or("No valid explorer location in that direction")?;
            if !roots.iter().any(|r| target.path.starts_with(r)) {
                return Err("Location is outside the project; add its folder explicitly".to_string());
            }
            target.validate().map_err(|e| e.to_string())?;
            // Populate the same listings cache as the background poll, so reveal has real rows.
            let mut listings = Vec::new();
            for ancestor in target.path.ancestors().filter(|p| roots.iter().any(|r| p.starts_with(r))) {
                listings.push((ancestor.to_path_buf(), sidebar::entries(ancestor)));
            }
            target.validate().map_err(|e| e.to_string())?;
            Ok::<_, String>((history, target, listings))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, window, cx| {
                if request != ws.sidebar.location_request
                    || !source.current(ws, cx)
                    || selected != ws.sidebar.selected
                    || focus != window.focused(cx)
                    || picker != ws.quick_panel.as_ref().map(|p| p.input.entity_id())
                {
                    return;
                }
                match result {
                    Ok((history, target, listings)) => {
                        ws.sidebar.locations = history;
                        ws.reveal_explorer(target.path, listings, window, cx);
                    }
                    Err(error) => ws.status_message(format!("Explorer: {error}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn reveal_explorer(
        &mut self,
        path: PathBuf,
        listings: Vec<(PathBuf, Vec<(PathBuf, bool)>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for ancestor in path.ancestors().filter(|p| self.roots.iter().any(|r| p.starts_with(&r.path))) {
            self.sidebar.expand(ancestor.to_path_buf());
        }
        self.sidebar.cache_listings(listings);
        self.sidebar.selected = Some(path.clone());
        self.sidebar.visible = true;
        self.sidebar_anim = None;
        if let Some(i) = self.sidebar.rows().iter().position(|r| r.path == path) {
            self.reveal_sidebar_row(i);
        }
        window.focus(&self.sidebar_focus);
        cx.notify();
    }

    pub(crate) fn path_breadcrumbs(&mut self, _: &PathBreadcrumbs, window: &mut Window, cx: &mut Context<Self>) {
        let source = Source::capture(self, cx);
        let Some(path) = source.view.as_ref().and_then(|(_, _, _, path)| path.clone()) else {
            return self.status_message("Path Breadcrumbs needs a named active file", cx);
        };
        self.load_breadcrumbs(source, None, path, window, cx);
    }

    fn load_breadcrumbs(
        &mut self,
        mut source: Source,
        directory: Option<Target>,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_panel(Kind::Choices, Vec::new(), window, cx);
        source.generation = (self.restore_gen, self.language.navigation_generation);
        let source = Arc::new(source);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some(format!("Path: {}", tilde(&path)));
        panel.progress = Some("Reading folders…".into());
        let surface = panel.input.entity_id();
        self.sidebar.location_request += 1;
        let request = self.sidebar.location_request;
        let task = cx.background_spawn(async move {
            match directory {
                Some(directory) => {
                    let children = directory.children().map_err(|e| e.to_string())?;
                    let mut choices = vec![("Reveal this folder".to_string(), directory, false)];
                    choices.extend(children.into_iter().map(|t| (file_label(&t), t, false)));
                    Ok(choices)
                }
                None => path
                    .parent()
                    .into_iter()
                    .flat_map(Path::ancestors)
                    .map(|p| Target::directory(p.to_path_buf()).map(|t| (tilde(p), t, true)).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>(),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |ws, _, cx| {
                if request != ws.sidebar.location_request
                    || !source.current(ws, cx)
                    || ws.quick_panel.as_ref().map(|p| p.input.entity_id()) != Some(surface)
                {
                    return;
                }
                let items = match result {
                    Ok(choices) => choices
                        .into_iter()
                        .map(|(label, target, children)| {
                            choice(
                                &label,
                                &tilde(&target.path),
                                ItemAction::Explorer(Arc::new(BreadcrumbChoice {
                                    source: source.clone(),
                                    target,
                                    children,
                                })),
                            )
                        })
                        .collect(),
                    Err(error) => {
                        ws.status_message(format!("Path Breadcrumbs: {error}"), cx);
                        Vec::new()
                    }
                };
                let p = ws.quick_panel.as_mut().unwrap();
                p.progress = None;
                p.set_items(Kind::Choices, items);
                p.refilter(&p.input.read(cx).text(cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Handle these before generic Choices dismisses the picker or changes focus.
    pub(super) fn confirm_breadcrumb(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(choice) =
            self.quick_panel.as_ref().and_then(|p| p.items.get(*p.filtered.get(p.selected)?)).and_then(|i| {
                match &i.action {
                    ItemAction::Explorer(choice) => Some(choice.clone()),
                    _ => None,
                }
            })
        else {
            return false;
        };
        if !choice.source.current(self, cx) {
            self.status_message("Path Breadcrumbs: source changed; open the menu again", cx);
            return true;
        }
        if choice.children {
            self.load_breadcrumbs(
                choice.source.as_ref().clone(),
                Some(choice.target.clone()),
                choice.target.path.clone(),
                window,
                cx,
            );
            return true;
        }
        let surface = self.quick_panel.as_ref().unwrap().input.entity_id();
        let focus = window.focused(cx);
        let target = choice.target.clone();
        let task = cx.background_spawn(async move { target.validate().map_err(|e| e.to_string()) });
        self.sidebar.location_request += 1;
        let request = self.sidebar.location_request;
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            #[cfg(test)]
            crate::testing::gate::wait("breadcrumb-validated", &choice.target.path).await;
            this.update_in(cx, |ws, window, cx| {
                if request != ws.sidebar.location_request
                    || !choice.source.current(ws, cx)
                    || focus != window.focused(cx)
                    || ws.quick_panel.as_ref().map(|p| p.input.entity_id()) != Some(surface)
                {
                    return;
                }
                if let Err(error) = result {
                    return ws.status_message(format!("Path Breadcrumbs: {error}"), cx);
                }
                if choice.target.is_dir {
                    ws.close_quick_panel(false, cx);
                    ws.focus_editor(window, cx);
                    ws.explorer_navigation(
                        Navigation::Visit(choice.target.path.clone(), Some(choice.target.clone())),
                        window,
                        cx,
                    );
                } else {
                    ws.open_breadcrumb_file(
                        BreadcrumbOpen { choice: choice.clone(), request, surface, focus },
                        window,
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
        true
    }
}

fn file_label(target: &Target) -> String {
    format!("{}{}", target.path.file_name().unwrap_or_default().to_string_lossy(), if target.is_dir { "/" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::ui_tests::{settle_until, temp, window};
    use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};

    fn project(ws: &Entity<Workspace>, cx: &mut VisualTestContext, name: &str) -> PathBuf {
        let d = temp(name).canonicalize().unwrap();
        for folder in ["a/nested/deep", "a/other", "b/inside"] {
            std::fs::create_dir_all(d.join(folder)).unwrap();
        }
        std::fs::write(d.join("a/nested/source.txt"), "source\ntext\n").unwrap();
        std::fs::write(d.join("a/child.txt"), "child\n").unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.open_path(d.join("a"), window, cx);
            ws.open_path(d.join("b"), window, cx);
            ws.open_path(d.join("a/nested/source.txt"), window, cx);
        });
        d
    }

    fn arrived(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: &Path) {
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| {
                ws.sidebar.locations.current().is_some_and(|t| t.path == path)
                    && ws.sidebar.selected.as_deref() == Some(path)
            })
        });
    }

    fn visit(ws: &Entity<Workspace>, cx: &mut VisualTestContext, path: PathBuf) {
        ws.update_in(cx, |ws, window, cx| ws.navigate_explorer(path.clone(), window, cx));
        arrived(ws, cx, &path);
    }

    fn choose(ws: &Entity<Workspace>, cx: &mut VisualTestContext, label: &str) {
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| {
                ws.quick_panel
                    .as_ref()
                    .is_some_and(|p| p.progress.is_none() && p.items.iter().any(|i| i.label == label))
            })
        });
        ws.update(cx, |ws, _| {
            let p = ws.quick_panel.as_mut().unwrap();
            p.selected = p.filtered.iter().position(|i| p.items[*i].label == label).unwrap();
        });
        cx.simulate_keystrokes("enter");
    }

    #[gpui::test]
    fn n28_keyboard_history_two_roots_parent_and_editor_jumps_are_independent(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-keyboard");
        let source = ws.update(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            e.update(cx, |e, cx| {
                e.edit(cx, |s| s.insert("dirty "));
                e.sel(cx).set_cursor(2);
                e.jump_to(8, cx);
                e.update_doc(cx, |d| d.set_viewer_mode(true));
            });
            ws.sidebar.selected = Some(d.join("a/nested"));
            e
        });
        cx.simulate_keystrokes("ctrl-0 enter");
        arrived(&ws, cx, &d.join("a/nested"));
        visit(&ws, cx, d.join("a/nested/deep"));
        visit(&ws, cx, d.join("b/inside"));
        cx.simulate_keystrokes("alt-left");
        arrived(&ws, cx, &d.join("a/nested/deep"));
        cx.simulate_keystrokes("alt-right");
        arrived(&ws, cx, &d.join("b/inside"));
        cx.simulate_keystrokes("alt-up");
        arrived(&ws, cx, &d.join("b"));
        cx.simulate_keystrokes("alt-up");
        ws.read_with(cx, |ws, cx| {
            assert!(ws.status.as_ref().unwrap().0.contains("project root"));
            assert_eq!(ws.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>(), [d.join("a"), d.join("b")]);
            for p in ["a", "a/nested", "a/nested/deep", "b", "b/inside"] {
                assert!(ws.sidebar.is_expanded(&d.join(p)), "expansion retained: {p}");
            }
            let e = source.read(cx);
            assert_eq!(e.state.cursor(), 8);
            assert_eq!(e.text(cx), "dirty source\ntext\n");
            assert!(e.doc(cx).dirty && e.doc(cx).viewer_mode());
        });
        cx.simulate_keystrokes("escape ctrl--");
        assert_eq!(source.read_with(cx, |e, _| e.state.cursor()), 2, "explorer actions do not consume editor jumps");
        ws.read_with(cx, |ws, _| assert_eq!(ws.sidebar.locations.current().unwrap().path, d.join("b")));
    }

    #[gpui::test]
    fn n28_history_skips_deleted_and_replaced_locations_and_branches(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-history-ui");
        for path in ["a/nested", "a/nested/deep", "a/other", "b/inside"] {
            visit(&ws, cx, d.join(path));
        }
        std::fs::remove_dir(d.join("a/nested/deep")).unwrap();
        std::fs::rename(d.join("a/other"), d.join("a/replaced")).unwrap();
        std::fs::create_dir(d.join("a/other")).unwrap();
        cx.simulate_keystrokes("alt-left");
        arrived(&ws, cx, &d.join("a/nested"));
        visit(&ws, cx, d.join("a/other"));
        cx.simulate_keystrokes("alt-right");
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| ws.status.as_ref().is_some_and(|s| s.0.contains("No valid explorer location")))
        });
        ws.read_with(cx, |ws, _| assert_eq!(ws.sidebar.selected.as_deref(), Some(d.join("a/other").as_path())));
        std::fs::rename(d.join("a/other"), d.join("a/moved")).unwrap();
        ws.update(cx, |ws, _| ws.sidebar.moved(&d.join("a/other"), &d.join("a/moved")));
        cx.simulate_keystrokes("alt-left");
        arrived(&ws, cx, &d.join("a/nested"));
        cx.simulate_keystrokes("alt-right");
        arrived(&ws, cx, &d.join("a/moved"));
    }

    #[gpui::test]
    fn n28_removed_root_prunes_populated_history_but_keeps_other_locations(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-removed-root");
        for path in ["a/nested", "b/inside", "a/other"] {
            visit(&ws, cx, d.join(path));
        }
        ws.update_in(cx, |ws, window, cx| ws.remove_root(&RemoveRoot { index: 1 }, window, cx));
        cx.simulate_keystrokes("alt-left");
        arrived(&ws, cx, &d.join("a/nested"));
        ws.read_with(cx, |ws, _| {
            assert_eq!(ws.roots.len(), 1);
            assert!(ws.sidebar.is_expanded(&d.join("a/other")));
        });
        cx.simulate_keystrokes("alt-right");
        arrived(&ws, cx, &d.join("a/other"));
        ws.update_in(cx, |ws, window, cx| ws.remove_root(&RemoveRoot { index: 0 }, window, cx));
        ws.read_with(cx, |ws, _| assert!(ws.sidebar.locations.current().is_none()));
    }

    #[gpui::test]
    fn n28_project_reset_discards_old_current_and_seeds_new_project(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-reset-history");
        visit(&ws, cx, d.join("a/nested"));
        ws.update_in(cx, |ws, window, cx| ws.clear_window(window, cx));
        ws.read_with(cx, |ws, _| assert!(ws.sidebar.locations.current().is_none()));
        ws.update_in(cx, |ws, window, cx| ws.open_path(d.join("b"), window, cx));
        visit(&ws, cx, d.join("b/inside"));
        cx.simulate_keystrokes("alt-left");
        arrived(&ws, cx, &d.join("b"));
        cx.simulate_keystrokes("alt-up");
        ws.read_with(cx, |ws, _| {
            assert!(ws.status.as_ref().unwrap().0.contains("project root"));
            assert_eq!(ws.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>(), [d.join("b")]);
        });
    }

    #[gpui::test]
    fn n28_breadcrumb_ancestor_children_open_normally_and_reveal_without_replacing_roots(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-breadcrumb-open");
        cx.dispatch_action(PathBreadcrumbs);
        choose(&ws, cx, &tilde(&d.join("a")));
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| {
                ws.quick_panel
                    .as_ref()
                    .is_some_and(|p| p.progress.is_none() && p.items.iter().any(|i| i.label == "child.txt"))
            })
        });
        cx.simulate_input("child");
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert_eq!(p.filtered.iter().map(|i| p.items[*i].label.as_str()).collect::<Vec<_>>(), ["child.txt"]);
        });
        cx.simulate_keystrokes("enter");
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, cx| {
                ws.active_editor().unwrap().read(cx).doc(cx).path.as_deref() == Some(d.join("a/child.txt").as_path())
            })
        });
        cx.dispatch_action(PathBreadcrumbs);
        choose(&ws, cx, &tilde(&d.join("a")));
        choose(&ws, cx, "nested/");
        arrived(&ws, cx, &d.join("a/nested"));
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.roots.len(), 2);
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "child\n");
        });
        cx.update(|w, cx| assert!(ws.read(cx).sidebar_focus.is_focused(w)));
    }

    #[gpui::test]
    fn n28_stale_breadcrumb_workers_choices_and_folder_navigation_preserve_new_context(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-stale");
        let replacement = ws.update_in(cx, |ws, window, cx| {
            ws.path_breadcrumbs(&PathBreadcrumbs, window, cx);
            ws.open_quick_panel(
                Kind::Choices,
                vec![choice("Replacement", "", ItemAction::Dispatch(Box::new(ClosePanel)))],
                window,
                cx,
            );
            ws.quick_panel.as_ref().unwrap().input.entity_id()
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            let p = ws.quick_panel.as_ref().unwrap();
            assert_eq!(p.input.entity_id(), replacement);
            assert_eq!(p.items[0].label, "Replacement");
        });
        cx.dispatch_action(PathBreadcrumbs);
        choose(&ws, cx, &tilde(&d.join("a")));
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| {
                ws.quick_panel.as_ref().is_some_and(|p| p.items.iter().any(|i| i.label == "child.txt"))
            })
        });
        std::fs::rename(d.join("a/child.txt"), d.join("a/old-child.txt")).unwrap();
        std::fs::write(d.join("a/child.txt"), "replacement").unwrap();
        choose(&ws, cx, "child.txt");
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.status.as_ref().is_some_and(|s| s.0.contains("replaced"))));
        ws.update_in(cx, |ws, window, cx| {
            ws.active_editor().unwrap().update(cx, |e, cx| e.edit(cx, |s| s.insert("new ")));
            assert!(ws.confirm_breadcrumb(window, cx));
            assert!(ws.status.as_ref().unwrap().0.contains("source changed"));
            ws.cancel_quick_panel(cx);
            ws.focus_editor(window, cx);
            ws.navigate_explorer(d.join("a/nested"), window, cx);
            ws.active_editor().unwrap().update(cx, |e, cx| e.edit(cx, |s| s.insert("later ")));
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert!(ws.sidebar.locations.current().is_none(), "changed source rejects pending navigation");
            assert!(ws.active_editor().unwrap().read(cx).text(cx).starts_with("new later "));
        });
    }

    #[gpui::test]
    fn n28_child_activation_does_not_steal_a_new_focus_and_pinned_folder_rejects_replacement(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-child-admission");
        cx.dispatch_action(PathBreadcrumbs);
        choose(&ws, cx, &tilde(&d.join("a")));
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| {
                ws.quick_panel.as_ref().is_some_and(|p| p.items.iter().any(|i| i.label == "child.txt"))
            })
        });
        ws.update_in(cx, |ws, window, cx| {
            let p = ws.quick_panel.as_mut().unwrap();
            p.selected = p.filtered.iter().position(|i| p.items[*i].label == "child.txt").unwrap();
            ws.confirm_quick_panel(window, cx);
            window.focus(&ws.sidebar_focus);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert_eq!(ws.active_editor().unwrap().read(cx).doc(cx).path, Some(d.join("a/nested/source.txt")));
            assert!(ws.quick_panel.is_some());
        });
        let pinned = Target::directory(d.join("a/other")).unwrap();
        std::fs::rename(d.join("a/other"), d.join("a/old-other")).unwrap();
        std::fs::create_dir(d.join("a/other")).unwrap();
        ws.update_in(cx, |ws, window, cx| {
            ws.cancel_quick_panel(cx);
            ws.focus_editor(window, cx);
            ws.explorer_navigation(Navigation::Visit(pinned.path.clone(), Some(pinned)), window, cx);
        });
        settle_until(cx, |cx| ws.read_with(cx, |ws, _| ws.status.as_ref().is_some_and(|s| s.0.contains("replaced"))));
        ws.read_with(cx, |ws, _| assert!(ws.sidebar.locations.current().is_none()));
    }

    #[gpui::test]
    fn n28_history_is_window_local_and_reading_preserves_native_composition(cx: &mut TestAppContext) {
        let (a, b, mut acx, bcx) = crate::workspace::ui_tests::two_windows(cx);
        let d = project(&a, &mut acx, "orlo-n28-window-ime");
        let source = a.read_with(&acx, |ws, _| ws.active_editor().unwrap());
        source.update_in(&mut acx, |e, window, cx| {
            e.replace_and_mark_text_in_range(Some(0..6), "composing", Some(1..3), window, cx);
        });
        let before =
            source.read_with(&acx, |e, cx| (e.text(cx), e.marked.clone(), e.state.cursor(), e.doc(cx).version));
        visit(&a, &mut acx, d.join("a/nested"));
        acx.dispatch_action(PathBreadcrumbs);
        choose(&a, &mut acx, &tilde(&d.join("a")));
        choose(&a, &mut acx, "nested/");
        arrived(&a, &mut acx, &d.join("a/nested"));
        assert_eq!(
            source.read_with(&acx, |e, cx| (e.text(cx), e.marked.clone(), e.state.cursor(), e.doc(cx).version)),
            before
        );
        b.read_with(&bcx, |ws, _| {
            assert!(ws.sidebar.locations.current().is_none());
            assert!(ws.roots.is_empty());
        });
    }

    #[gpui::test]
    fn n28_project_change_and_new_surface_reject_pending_folder_delivery(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-stale-project");
        ws.update_in(cx, |ws, window, cx| {
            ws.navigate_explorer(d.join("a/nested"), window, cx);
            ws.roots.swap(0, 1);
            ws.sidebar.set_roots(&ws.roots);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| assert!(ws.sidebar.locations.current().is_none()));
        ws.update_in(cx, |ws, window, cx| {
            ws.navigate_explorer(d.join("b/inside"), window, cx);
            ws.goto_anything(&GotoAnything, window, cx);
            ws.cancel_quick_panel(cx);
        });
        cx.run_until_parked();
        ws.read_with(cx, |ws, _| {
            assert!(
                ws.sidebar.locations.current().is_none(),
                "opening then closing a replacement surface invalidates the old request"
            )
        });
    }

    fn deferred_child_race(cx: &mut TestAppContext, name: &str, mode: usize) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, name);
        let target = d.join("a/child.txt");
        let gate = crate::testing::gate::pause("breadcrumb-validated", &target);
        cx.dispatch_action(PathBreadcrumbs);
        choose(&ws, cx, &tilde(&d.join("a")));
        choose(&ws, cx, "child.txt");
        settle_until(cx, |_| gate.arrived() == 1);
        let lease = ws.update(cx, |_, cx| pipelines::lease(&target, cx));
        gate.release();
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| {
                ws.opening_after_write.contains(&target)
                    || ws.status.as_ref().is_some_and(|s| s.0.contains("being written"))
            })
        });
        std::fs::rename(&target, d.join("a/original-child.txt")).unwrap();
        match mode {
            1 => std::fs::create_dir(&target).unwrap(),
            2 => std::fs::write(&target, "replacement bytes").unwrap(),
            _ => {}
        }
        drop(lease);
        cx.executor().advance_clock(std::time::Duration::from_millis(20));
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert!(
                ws.editor_for_path(&target, cx).is_none(),
                "a stale child must not become a named empty or replacement buffer"
            );
            assert_eq!(
                ws.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>(),
                [d.join("a"), d.join("b")],
                "a replacement directory must not become a project root"
            );
            assert!(ws.quick_panel.is_some(), "refusal retains the initiating picker");
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "source\ntext\n");
        });
    }

    #[gpui::test]
    fn n28_deferred_child_deletion_does_not_create_named_empty(cx: &mut TestAppContext) {
        deferred_child_race(cx, "orlo-n28-deferred-delete", 0);
    }

    #[gpui::test]
    fn n28_deferred_child_directory_replacement_does_not_add_a_root(cx: &mut TestAppContext) {
        deferred_child_race(cx, "orlo-n28-deferred-directory", 1);
    }

    #[gpui::test]
    fn n28_deferred_child_regular_replacement_does_not_open_new_identity(cx: &mut TestAppContext) {
        deferred_child_race(cx, "orlo-n28-deferred-regular", 2);
    }

    fn child_read_pause(ws: &Entity<Workspace>, cx: &mut VisualTestContext, d: &Path) -> crate::testing::gate::Pause {
        let target = d.join("a/child.txt");
        let gate = crate::testing::gate::pause("open-read", &target);
        cx.dispatch_action(PathBreadcrumbs);
        choose(ws, cx, &tilde(&d.join("a")));
        choose(ws, cx, "child.txt");
        settle_until(cx, |_| gate.arrived() == 1);
        gate
    }

    #[gpui::test]
    fn n28_post_read_child_changes_refuse_deletion_directory_and_regular_replacement(cx: &mut TestAppContext) {
        for mode in 0..3 {
            let (ws, cx) = window(cx);
            let d = project(&ws, cx, &format!("orlo-n28-post-read-{mode}"));
            let target = d.join("a/child.txt");
            let gate = child_read_pause(&ws, cx, &d);
            std::fs::rename(&target, d.join("a/original.txt")).unwrap();
            match mode {
                1 => std::fs::create_dir(&target).unwrap(),
                2 => std::fs::write(&target, "replacement").unwrap(),
                _ => {}
            }
            gate.release();
            settle_until(cx, |cx| {
                ws.read_with(cx, |ws, _| ws.status.as_ref().is_some_and(|s| s.0.starts_with("Path Breadcrumbs:")))
            });
            ws.read_with(cx, |ws, cx| {
                assert!(ws.editor_for_path(&target, cx).is_none());
                assert_eq!(ws.roots.len(), 2);
                assert!(ws.quick_panel.is_some());
                assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "source\ntext\n");
            });
        }
    }

    #[gpui::test]
    fn n28_guarded_read_rechecks_source_focus_surface_ime_and_write_reservations(cx: &mut TestAppContext) {
        for mode in 0..5 {
            let (ws, cx) = window(cx);
            let d = project(&ws, cx, &format!("orlo-n28-read-context-{mode}"));
            let target = d.join("a/child.txt");
            let source = ws.read_with(cx, |ws, _| ws.active_editor().unwrap());
            let gate = child_read_pause(&ws, cx, &d);
            ws.update_in(cx, |ws, window, cx| match mode {
                0 => source.update(cx, |e, cx| e.edit(cx, |s| s.insert("changed "))),
                1 => window.focus(&ws.sidebar_focus),
                2 => ws.goto_anything(&GotoAnything, window, cx),
                3 => source.update(cx, |e, cx| {
                    e.replace_and_mark_text_in_range(Some(0..1), "compose", Some(1..2), window, cx)
                }),
                _ => drop(pipelines::lease(&target, cx)),
            });
            gate.release();
            settle_until(cx, |cx| ws.read_with(cx, |ws, _| !ws.loading.contains(&target)));
            ws.read_with(cx, |ws, cx| {
                assert!(ws.editor_for_path(&target, cx).is_none(), "mode {mode}");
                assert_eq!(ws.active_editor().unwrap(), source);
                if mode == 3 {
                    assert!(!source.read(cx).marked.is_empty());
                }
            });
        }
    }

    #[gpui::test]
    fn n28_guarded_open_reuses_a_dirty_document_that_appeared_during_read(cx: &mut TestAppContext) {
        let (a, b, mut acx, mut bcx) = crate::workspace::ui_tests::two_windows(cx);
        let d = project(&a, &mut acx, "orlo-n28-read-live-owner");
        let target = d.join("a/child.txt");
        let gate = child_read_pause(&a, &mut acx, &d);
        let owner = b.update_in(&mut bcx, |ws, window, cx| {
            ws.open_path(target.clone(), window, cx);
            let owner = ws.active_editor().unwrap();
            owner.update(cx, |e, cx| {
                e.edit(cx, |s| s.insert("dirty "));
                e.replace_and_mark_text_in_range(Some(0..1), "d", Some(0..1), window, cx);
            });
            owner
        });
        gate.release();
        settle_until(&mut acx, |cx| {
            a.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref() == Some(&target))
        });
        a.read_with(&acx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).document, owner.read(cx).document);
            assert_eq!(e.read(cx).text(cx), "dirty child\n");
            assert!(e.read(cx).doc(cx).dirty);
            assert!(!owner.read(cx).marked.is_empty(), "adoption preserves the owner's native composition");
        });
    }

    #[gpui::test]
    fn n28_guarded_open_uses_remembered_codec_and_preserves_source_composition(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-read-codec");
        let target = d.join("a/child.txt");
        std::fs::write(&target, b"caf\xe9\r\nsecond\n").unwrap();
        let source = ws.update_in(cx, |ws, window, cx| {
            ws.closed.push(ViewState {
                path: target.clone(),
                disk_encoding_hint: Some(crate::editing::encoding::Encoding::Windows1252),
                ..Default::default()
            });
            let source = ws.active_editor().unwrap();
            source.update(cx, |e, cx| e.replace_and_mark_text_in_range(Some(0..1), "compose", Some(1..3), window, cx));
            source
        });
        let before = source.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version));
        cx.dispatch_action(PathBreadcrumbs);
        choose(&ws, cx, &tilde(&d.join("a")));
        choose(&ws, cx, "child.txt");
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, cx| ws.active_editor().unwrap().read(cx).doc(cx).path.as_ref() == Some(&target))
        });
        ws.read_with(cx, |ws, cx| {
            let e = ws.active_editor().unwrap();
            assert_eq!(e.read(cx).text(cx), "café\nsecond\n");
            assert_eq!(e.read(cx).doc(cx).disk_encoding, crate::editing::encoding::Encoding::Windows1252);
            assert!(!e.read(cx).doc(cx).read_only);
        });
        assert_eq!(source.read_with(cx, |e, cx| (e.text(cx), e.marked.clone(), e.doc(cx).version)), before);
    }

    #[gpui::test]
    fn n28_large_child_cancel_and_replacement_after_prompt_do_not_admit_stale_open(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-large-child");
        let target = d.join("a/child.txt");
        std::fs::File::create(&target).unwrap().set_len(ASK_ABOVE + 1).unwrap();
        cx.dispatch_action(PathBreadcrumbs);
        choose(&ws, cx, &tilde(&d.join("a")));
        choose(&ws, cx, "child.txt");
        settle_until(cx, |cx| cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert!(ws.quick_panel.is_some());
            assert!(ws.editor_for_path(&target, cx).is_none());
        });
        choose(&ws, cx, "child.txt");
        settle_until(cx, |cx| cx.has_pending_prompt());
        std::fs::rename(&target, d.join("a/large-original.txt")).unwrap();
        std::fs::write(&target, "replacement").unwrap();
        cx.simulate_prompt_answer("Open");
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| ws.status.as_ref().is_some_and(|s| s.0.starts_with("Path Breadcrumbs:")))
        });
        ws.read_with(cx, |ws, cx| {
            assert!(ws.editor_for_path(&target, cx).is_none());
            assert!(ws.quick_panel.is_some());
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "source\ntext\n");
        });
        // Restoring the original inode permits another prompt; a newer source still cancels admission.
        std::fs::remove_file(&target).unwrap();
        std::fs::rename(d.join("a/large-original.txt"), &target).unwrap();
        choose(&ws, cx, "child.txt");
        settle_until(cx, |cx| cx.has_pending_prompt());
        ws.update(cx, |ws, cx| ws.active_editor().unwrap().update(cx, |e, cx| e.edit(cx, |s| s.insert("changed "))));
        cx.simulate_prompt_answer("Open");
        cx.run_until_parked();
        ws.read_with(cx, |ws, cx| {
            assert!(ws.editor_for_path(&target, cx).is_none());
            assert!(ws.quick_panel.is_some());
            assert_eq!(ws.active_editor().unwrap().read(cx).text(cx), "changed source\ntext\n");
        });
    }

    #[gpui::test]
    fn n28_menu_palette_json_custom_keys_and_toolbar_reach_navigation(cx: &mut TestAppContext) {
        let (ws, cx) = window(cx);
        let d = project(&ws, cx, "orlo-n28-reachability");
        let names = ["ExplorerBack", "ExplorerForward", "ExplorerParent", "PathBreadcrumbs"];
        ws.update_in(cx, |ws, _, cx| {
            let palette = quick_panel::command_items();
            fn collect(items: Vec<MenuItem>, names: &mut Vec<String>) {
                for item in items {
                    match item {
                        MenuItem::Action { action, .. } => names.push(action.name().to_string()),
                        MenuItem::Submenu(menu) => collect(menu.items, names),
                        _ => {}
                    }
                }
            }
            let mut menu = Vec::new();
            for m in ws.menus(cx) {
                collect(m.items, &mut menu);
            }
            for name in names {
                let command = format!("workspace::{name}");
                assert!(menu.contains(&command));
                assert!(palette.iter().any(|i| matches!(&i.action, ItemAction::Dispatch(a) if a.name() == command)));
                assert!(cx.build_action(&command, None).is_ok());
            }
        });
        visit(&ws, cx, d.join("a/nested"));
        std::fs::write(crate::config::customization::keymap_path(), r#"[{"keys":["cmd-alt-9"],"command":"workspace::ExplorerParent"},{"keys":["cmd-alt-8"],"command":"workspace::PathBreadcrumbs"}]"#).unwrap();
        ws.update_in(cx, |ws, window, cx| ws.reload_customizations(&ReloadCustomizations, window, cx));
        cx.simulate_keystrokes("cmd-alt-9");
        arrived(&ws, cx, &d.join("a"));
        cx.run_until_parked();
        let back = cx.debug_bounds("explorer-back").expect("visible explorer toolbar");
        cx.simulate_click(back.center(), gpui::Modifiers::default());
        arrived(&ws, cx, &d.join("a/nested"));
        cx.simulate_keystrokes("cmd-alt-8");
        settle_until(cx, |cx| {
            ws.read_with(cx, |ws, _| {
                ws.quick_panel.as_ref().is_some_and(|p| p.title.as_ref().is_some_and(|t| t.starts_with("Path:")))
            })
        });
    }
}
