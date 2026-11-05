//! The search history pickers, the Where chooser and Clear Search History.

use super::find::History;
use super::*;
use crate::chrome::quick_panel::Item;
use crate::searching::saved;

parameter_actions! { workspace, plain;
    /// Where chooser rows that ask for paths.
    WhereAddPaths { folders: bool, }
}

impl Workspace {
    /// Run a path chooser's command only when it returns paths and this window still exists.
    pub(crate) fn prompt_paths(
        &mut self,
        options: PathPromptOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
        chosen: impl FnOnce(&mut Self, Vec<PathBuf>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let rx = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update_in(cx, |ws, window, cx| chosen(ws, paths, window, cx)).ok();
            }
        })
        .detach();
    }

    /// `alt-down` in a Find, Replace or Where field: its history to pick from (the Where field's
    /// is the Where chooser, history included).
    pub(crate) fn show_field_history(&mut self, _: &ShowFieldHistory, window: &mut Window, cx: &mut Context<Self>) {
        let Some((field, which)) = self.history_field(window, cx) else {
            return;
        };
        if which == History::Where {
            return self.choose_where(&ChooseWhere, window, cx);
        }
        let weak = field.downgrade();
        let mut items: Vec<Item> = (self.history_mut(which).entries().iter())
            .map(|e| choice(&crate::searching::results::escape(e), "", ItemAction::SetField(e.clone(), weak.clone())))
            .collect();
        if items.is_empty() {
            items.push(choice(
                "No search history yet",
                "Find, Replace and Find in Files remember what they run",
                ItemAction::Dispatch(Box::new(ClosePanel)),
            ));
        }
        items.push(choice(
            "Clear Search History",
            "Find, Replace and Where, in this window",
            ItemAction::Dispatch(Box::new(ClearSearchHistory)),
        ));
        self.open_quick_panel(Kind::Choices, items, window, cx);
    }

    /// The Where field's "…": folders and files to add, the scope tokens, recent Where values
    /// and saved searches' scopes.
    pub(crate) fn choose_where(&mut self, _: &ChooseWhere, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.fif_panel().map(|p| p.where_.downgrade()) else {
            return;
        };
        let add = |token: &str| ItemAction::AppendField(token.to_string(), field.clone());
        let ask = |folders| ItemAction::Dispatch(Box::new(WhereAddPaths { folders }));
        let open = |label, token| choice(label, token, add(token));
        let mut items = vec![
            choice("Add Folder…", "Choose folders to search", ask(true)),
            choice("Add File…", "Choose files to search", ask(false)),
            open("Add Open Folders", search::OPEN_FOLDERS),
            open("Add Open Files", search::OPEN_FILES),
        ];
        if self.result_set.is_some() {
            items.push(choice(
                "Add Previous Results",
                "The files the last Find in Files found matches in",
                add(search::PREVIOUS_RESULTS),
            ));
            items.push(choice(
                "Previous Matching Lines",
                "Only the physical hit lines of these results; retain their exact source",
                ItemAction::SetField(search::PREVIOUS_MATCHING_LINES.into(), field.clone()),
            ));
        }
        for p in self.sidebar_marked() {
            let shown = tilde(&p);
            items.push(choice(
                &format!("Add Side Bar Selection: {shown}"),
                "Marked with ⌘-click in the side bar",
                add(&shown),
            ));
        }
        for w in (self.where_history.entries().iter()).filter(|w| !w.is_empty()) {
            items.push(choice(w, "Recent Where", ItemAction::SetField(w.clone(), field.clone())));
        }
        for s in saved::list(&saved::path()).unwrap_or_default() {
            if !s.where_.is_empty() {
                let label = format!("Saved: {}", s.name);
                items.push(choice(&label, &s.where_, ItemAction::SetField(s.where_.clone(), field.clone())));
            }
        }
        self.open_quick_panel(Kind::Choices, items, window, cx);
    }

    /// Add Folder… / Add File…: the paths chosen go into Where, after a comma each.
    pub(crate) fn where_add_paths(&mut self, a: &WhereAddPaths, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_paths(
            PathPromptOptions { files: !a.folders, directories: a.folders, multiple: true, prompt: None },
            window,
            cx,
            |ws, paths, window, cx| {
                let Some(field) = ws.fif_panel().map(|p| p.where_.clone()) else {
                    return;
                };
                field.update(cx, |e, cx| {
                    let text = (paths.iter()).fold(e.text(cx), |w, p| search::append_token(&w, &tilde(p)));
                    e.set_text(&text, cx);
                    sel_of(&e.document, &mut e.state, cx).set_cursor(doc_of(&e.document, cx).len());
                });
                field.read(cx).focus(window);
            },
        );
    }

    /// The side bar rows marked with ⌘-click that still exist.
    pub(crate) fn sidebar_marked(&self) -> Vec<PathBuf> {
        (self.sidebar.marked.iter()).filter(|p| p.exists()).cloned().collect()
    }

    /// Find > Find in Side Bar Selection…: Find in Files with Where set to the marked rows.
    pub(crate) fn find_in_sidebar_selection(
        &mut self,
        _: &FindInSidebarSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let marked = self.sidebar_marked();
        if marked.is_empty() {
            return self.status_message("⌘-click files or folders in the side bar first", cx);
        }
        let where_ = (marked.iter()).fold(String::new(), |w, p| search::append_token(&w, &tilde(p)));
        self.find_in_files(&FindInFiles, window, cx);
        if let Some(field) = self.fif_panel().map(|p| p.where_.clone()) {
            field.update(cx, |e, cx| e.set_text(&where_, cx));
        }
    }

    /// Find > Clear Search History: Find, Replace and Where, this window's.
    pub(crate) fn clear_search_history(&mut self, _: &ClearSearchHistory, _: &mut Window, cx: &mut Context<Self>) {
        self.query_history.clear();
        self.replace_history.clear();
        self.where_history.clear();
        self.status_message("Search history cleared", cx);
    }
}

pub(crate) fn choice(label: &str, detail: &str, action: ItemAction) -> Item {
    Item::new(label, detail, action)
}
