use super::*;

/// One Sublime-style window, fresh or as a session saved it; `Orlo` opens them at start and
/// File > New Window adds more.
pub fn open_window(state: Option<WindowState>, cx: &mut App) -> WindowHandle<Workspace> {
    let saved = state.as_ref().and_then(|s| Some((s.bounds?, s.maximized)));
    // gpui's bounds are relative to a display: back onto the saved one, or onto the main one
    // when it is gone
    let uuid = state.as_ref().and_then(|s| s.display.clone());
    let display = cx.displays().into_iter().find(|d| d.uuid().is_ok_and(|u| Some(u.to_string()) == uuid));
    let window_bounds = match saved {
        Some((mut b, maximized)) => {
            // onto the display it comes back on, which may be smaller than when it was saved
            if let Some(screen) = display.clone().or_else(|| cx.primary_display()) {
                // the whole screen, menu bar and Dock included, counts as visible
                let s = screen.bounds().size;
                b = session::clamp_onto(b, [f32::from(s.width), f32::from(s.height)]);
            }
            let [x, y, w, h] = b;
            let b = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
            if maximized {
                WindowBounds::Maximized(b)
            } else {
                WindowBounds::Windowed(b)
            }
        }
        None => WindowBounds::Windowed(Bounds::centered(None, size(px(1200.), px(800.)), cx)),
    };
    cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions { title: Some("Orlo".into()), ..Default::default() }),
            window_bounds: Some(window_bounds),
            display_id: display.map(|d| d.id()),
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                let mut ws = Workspace::new(window, cx);
                if let Some(st) = state {
                    ws.restore(st, window, cx);
                }
                ws
            })
        },
    )
    .expect("open main window")
}

/// Paths handed over from outside (Finder, the Dock, another `Orlo` launch): opened in the
/// active window, or the first, or a new one, which comes to the front.
pub fn open_paths(paths: Vec<PathBuf>, cx: &mut App) -> bool {
    front_window(cx)
        .update(cx, |ws, window, cx| {
            window.activate_window();
            for p in paths {
                ws.open_path_or_project(p, window, cx);
            }
        })
        .is_ok()
}

/// The active window, or the first, or a new one; Orlo comes to the front.
pub(crate) fn front_window(cx: &mut App) -> WindowHandle<Workspace> {
    let window = (cx.active_window().and_then(|w| w.downcast::<Workspace>()))
        .or_else(|| workspace_windows(cx).next())
        .unwrap_or_else(|| open_window(None, cx));
    cx.activate(true);
    window
}

/// Every Orlo window.
pub(crate) fn workspace_windows(cx: &App) -> impl Iterator<Item = WindowHandle<Workspace>> {
    cx.windows().into_iter().filter_map(|w| w.downcast())
}

/// Quit, New Window and Open with no window to take them (every window minimised): the menu
/// bar and the keys still work, as in Sublime.
pub fn global_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| quit(cx));
    cx.on_action(|_: &NewWindow, cx| {
        open_window(None, cx);
    });
    cx.on_action(|_: &Open, cx| {
        let rx =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: true, multiple: true, prompt: None });
        cx.spawn(async move |cx| {
            let Ok(Ok(Some(paths))) = rx.await else {
                return;
            };
            cx.update(|cx| {
                open_window(None, cx)
                    .update(cx, |ws, window, cx| {
                        for p in paths {
                            ws.open_path(p, window, cx);
                        }
                    })
                    .ok();
            })
            .ok();
        })
        .detach();
    });
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut ws = Self {
            cli: Default::default(),
            json_generation: 0,
            spelling_job: None,
            language: language::State::new(cx),
            index: cx.new(crate::languages::index::ProjectIndex::new),
            outline: Default::default(),
            completion_recent: Default::default(),
            automation: Default::default(),
            terminal: None,
            terminal_views: Groups::new(),
            git: Default::default(),
            views: Groups::new(),
            roots: Vec::new(),
            project: None,
            workspace_file: None,
            sidebar: SidebarState::default(),
            quick_panel: None,
            keep_blank_for_preview: false,
            panel: None,
            branches: HashMap::new(),
            real_paths: Default::default(),
            last_query: String::new(),
            query_history: Default::default(),
            last_replace: String::new(),
            last_find_opts: FindOpts::default(),
            last_preserve_case: false,
            view: ViewSettings::default(),
            overrides: Overrides::default(),
            show_tabs: true,
            show_status_bar: true,
            closed: Vec::new(),
            focus_handle: cx.focus_handle(),
            focus_subs: HashMap::new(),
            doc_subs: HashMap::new(),
            poller: None,
            polling: false,
            checking: HashSet::new(),
            asking: false,
            reloads: Vec::new(),
            next_reload: 0,
            save_sheets: Vec::new(),
            saves: Default::default(),
            closing: false,
            search: None,
            pipeline_run: None,
            last_results: None,
            last_where: String::new(),
            last_fif_replace: String::new(),
            fif_flags: crate::searching::results::SearchFlags::default(),
            result_set: None,
            result_pages: Default::default(),
            previous_lines: None,
            applying: None,
            replace_history: search::QueryHistory::default(),
            where_history: search::QueryHistory::default(),
            last_saved_name: String::new(),
            last_fif_query: String::new(),
            fif_opts: FindOpts::default(),
            loading: HashSet::new(),
            opening_after_write: HashSet::new(),
            previewing: HashMap::new(),
            tab_scroll: Vec::new(),
            find_origin: None,
            find_quiet: false,
            status: None,
            window: window.window_handle(),
            menus_due: false,
            restoring: Vec::new(),
            restore_gen: 0,
            comparison: Default::default(),
            pending_jumps: HashMap::new(),
            menus_built: 0,
            menu_delim: None,
            menu_range_formatting: false,
            menu_language_navigation: language::NavigationCapabilities::default(),
            unavailable: Vec::new(),
            last_dir: None,
            searching: None,
            find_pending: None,
            find_debounce: None,
            file_list: None,
            file_walk: None,
            walk_again: false,
            sidebar_anim: None,
            groups_bounds: Default::default(),
            sync_scroll: (false, false),
            tab_drop: None,
            context_tab: None,
            close_queue: Vec::new(),
            context_menu: None,
            sidebar_focus: cx.focus_handle(),
            panel_hidden: false,
            panel_collapsed: false,
            panel_places: Default::default(),
            distraction_free: None,
            shown_title: String::new(),
            escape_keys: None,
        };
        ws.sidebar.visible = true;
        ws.sidebar.width = theme::SIDEBAR_W;
        // the status bar's count and limit
        cx.observe_in(&ws.index, window, |ws, _, window, cx| {
            ws.load_symbol_rows(window, cx);
            cx.notify();
        })
        .detach();
        view::refresh_reduce_motion(cx);
        ws.view = ws.effective_view(cx);
        ws.add_tab(EditorState::new(), window, cx);
        sync_theme(window, cx);
        cx.observe_window_appearance(window, |_, window, cx| {
            sync_theme(window, cx);
            cx.notify();
        })
        .detach();
        // a saved settings file applies to every window and drops their menu overrides
        let settings = cx.global::<Settings>();
        let mut server_settings = (settings.lsp_enabled, settings.language_servers.clone());
        cx.observe_global_in::<Settings>(window, move |ws, window, cx| {
            ws.invalidate_language_requests(cx);
            let settings = cx.global::<Settings>();
            ws.sidebar.configure(settings.explorer_sort, settings.explorer_sticky_ancestors);
            if server_settings.0 != settings.lsp_enabled || server_settings.1 != settings.language_servers {
                server_settings = (settings.lsp_enabled, settings.language_servers.clone());
                ws.invalidate_navigation();
                ws.language.invalidate_caches();
                for editor in ws.open_editors(cx) {
                    editor.update(cx, |e, cx| {
                        e.close_completion(cx);
                        e.signature_triggers = Default::default();
                        cx.notify();
                    });
                }
            }
            sync_theme(window, cx);
            ws.overrides = Overrides::default();
            ws.apply_view(cx);
        })
        .detach();
        // Escape cancels a tab drag or closes a context menu before any key binding sees
        // it: the editor, the side bar and the panels bind Escape, and bindings run before
        // key-down listeners
        let this = cx.weak_entity();
        ws.escape_keys = Some(cx.intercept_keystrokes(move |e, window, cx| {
            if e.keystroke.key != "escape" {
                return;
            }
            this.update(cx, |ws, cx| {
                if ws.window != window.window_handle() {
                    return;
                }
                if cx.try_global::<TabDrag>().is_some_and(|d| d.0.is_some()) {
                    cx.stop_active_drag(window);
                    ws.end_tab_drag(cx);
                    cx.stop_propagation();
                } else if ws.context_menu.is_some() {
                    ws.context_tab = None;
                    ws.close_context_menu(cx);
                    cx.stop_propagation();
                }
            })
            .ok();
        }));
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |ws, cx| {
                if ws.has_dirty(cx) || ws.saving_before_close(cx) {
                    ws.confirm_close(window, cx);
                    false
                } else {
                    ws.save_if_last(window, cx);
                    ws.close_terminals(cx);
                    true
                }
            })
            .unwrap_or(true)
        });
        // another window's open or background load adds to Open Recent: the active window's
        // menu bar shows it
        cx.observe_global::<Recent>(|ws, cx| ws.refresh_menus(cx)).detach();
        // File > Session Could Not Be Read… comes and goes
        cx.observe_global::<SessionBlocked>(|ws, cx| ws.refresh_menus(cx)).detach();
        // files may have changed in other apps while the window was in the background, and the
        // menus list this window's groups and recent files
        cx.observe_window_activation(window, |ws, window, cx| {
            ws.invalidate_navigation();
            if window.is_window_active() {
                // Reduce Motion may have been turned on or off meanwhile
                if view::refresh_reduce_motion(cx) {
                    ws.apply_view(cx);
                }
                ws.refresh_menus(cx);
                ws.poll_disk(window, cx);
                ws.index_rescan(false, cx);
                cx.notify();
            }
        })
        .detach();
        ws.start_polling(window, cx);
        // a closed window's tabs no longer show their documents: a path opened again elsewhere
        // gets a new one unless another window still shows it
        cx.on_release(|ws: &mut Workspace, cx| {
            let shown: Vec<_> = (ws.views.tabs()).map(|t| t.read(cx).document.clone()).collect();
            for document in shown {
                Documents::hidden(&document, cx);
            }
            cx.defer(file_autosave::refresh_all);
        })
        .detach();
        ws.start_language_polling(window, cx);
        ws.refresh_menus(cx);
        ws
    }

    pub fn alert(&self, window: &mut Window, cx: &mut Context<Self>, message: &str, detail: &str) {
        // the alert only informs; its answer is not needed
        drop(window.prompt(PromptLevel::Critical, message, Some(detail), &["OK"], cx));
    }
}
