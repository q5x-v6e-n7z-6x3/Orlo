//! Find > Saved Searches: save the open search panel's query, replacement, toggles and
//! Where under a name, use one again, rename, delete, import and export them.

use crate::chrome::quick_panel::Item;
use crate::searching::saved::{self, SavedSearch};
use crate::workspace::*;

/// What a row of the saved searches list does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SavedUse {
    Use,
    Rename,
    Delete,
}

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct UseSavedSearch {
    pub name: String,
    pub how: SavedUse,
}

impl Workspace {
    /// The open Find or Find in Files panel's search, as a saved search would keep it.
    fn panel_search(&self, cx: &App) -> Option<SavedSearch> {
        let s = |opts: FindOpts, query: String, replacement: String, where_: String, flags| SavedSearch {
            name: String::new(),
            query,
            replacement,
            regex: opts.regex,
            case_sensitive: opts.case_sensitive,
            whole_word: opts.whole_word,
            where_,
            flags,
        };
        let preserving = |preserve_case| crate::searching::results::SearchFlags { preserve_case, ..Default::default() };
        match &self.panel {
            Some(Panel::Find(p)) => {
                let (q, r) = (p.query.read(cx).text(cx), p.replace.read(cx).text(cx));
                Some(s(p.opts, q, r, String::new(), preserving(p.preserve_case)))
            }
            Some(Panel::FindInFiles(p)) => Some(s(
                p.opts,
                p.query.read(cx).text(cx),
                p.replace.read(cx).text(cx),
                p.where_.read(cx).text(cx),
                p.flags,
            )),
            Some(Panel::Playground(_)) => {
                let (q, r, o, preserve) = self.playground_search_values(cx)?;
                Some(s(o, q, r, String::new(), preserving(preserve)))
            }
            _ => None,
        }
    }

    fn saved_or_alert(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Vec<SavedSearch>> {
        match saved::list(&saved::path()) {
            Ok(l) => Some(l),
            Err(e) => {
                self.alert(window, cx, "The saved searches cannot be read", &e);
                None
            }
        }
    }

    fn write_saved(&mut self, list: &[SavedSearch], done: &str, cx: &mut Context<Self>) -> Result<(), String> {
        saved::write(&saved::path(), list).map_err(|e| format!("Could not save: {e}"))?;
        self.status_message(done.to_string(), cx);
        Ok(())
    }

    /// Asked `question`, and on `yes` the saved searches as they are now changed by `change`,
    /// which says what it did; a problem goes to the status bar.
    fn confirm_saved_change(
        &self,
        question: &str,
        yes: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Vec<SavedSearch>) -> Result<String, String> + 'static,
    ) {
        let answer = window.prompt(PromptLevel::Warning, question, None, &[yes, "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |ws, cx| {
                    let r = saved::list(&saved::path()).and_then(|mut list| {
                        let done = change(&mut list)?;
                        ws.write_saved(&list, &done, cx)
                    });
                    if let Err(e) = r {
                        ws.status_message(e, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Find > Saved Searches > Save Search…: the open panel's search under a name; a name
    /// already used asks before the search of that name is replaced.
    pub(crate) fn save_search(&mut self, _: &SaveSearch, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.panel_search(cx) else {
            let detail = "Open Find, Replace or Find in Files with the search to save.";
            return self.inform(window, cx, "No search to save", detail);
        };
        // checked as it will be saved; its name comes next
        let named = SavedSearch { name: "name".into(), ..draft.clone() };
        if let Some(why) = named.problem() {
            return self.alert(window, cx, "This search cannot be saved", &why);
        }
        if self.active_editor().is_none() {
            return self.inform(window, cx, "No open tab", "Open a tab (File > New File) to name the search.");
        }
        let done: OnDone = Box::new(move |v, _, ws, window, cx| {
            let mut s = draft.clone();
            s.name = v.text("text").trim().to_string();
            let mut list = saved::list(&saved::path())?;
            let taken = list.iter().any(|x| x.name.trim().to_lowercase() == s.name.to_lowercase());
            if !taken {
                saved::upsert(&mut list, s.clone(), false)?;
                return ws.write_saved(&list, &format!("Saved search \"{}\"", s.name), cx);
            }
            // the name is in use: replace it only when told to
            let question = format!("Replace the saved search \"{}\"?", s.name);
            ws.confirm_saved_change(&question, "Replace", window, cx, move |list| {
                saved::upsert(list, s.clone(), true)?;
                Ok(format!("Replaced \"{}\"", s.name))
            });
            Ok(())
        });
        let name = self.last_saved_name.clone();
        self.show_input("Save search as", &name, done, window, cx);
    }

    fn choose_saved(&mut self, how: SavedUse, window: &mut Window, cx: &mut Context<Self>) {
        let Some(list) = self.saved_or_alert(window, cx) else {
            return;
        };
        if list.is_empty() {
            let detail = "Save one with Find > Saved Searches > Save Search…";
            return self.inform(window, cx, "No saved searches", detail);
        }
        let items: Vec<Item> = (list.into_iter())
            .map(|s| {
                let detail = match (s.where_.is_empty(), s.replacement.is_empty()) {
                    (true, true) => crate::searching::results::escape(&s.query),
                    (true, false) => format!(
                        "{} → {}",
                        crate::searching::results::escape(&s.query),
                        crate::searching::results::escape(&s.replacement)
                    ),
                    (false, _) => format!("{} in {}", crate::searching::results::escape(&s.query), s.where_),
                };
                Item::command(s.name.clone(), detail, UseSavedSearch { name: s.name, how })
            })
            .collect();
        self.open_quick_panel(Kind::Choices, items, window, cx);
    }

    forward_actions! {
    pub(crate) open_saved_search(OpenSavedSearch, window, cx) => choose_saved(SavedUse::Use, window, cx);
    pub(crate) rename_saved_search(RenameSavedSearch, window, cx) => choose_saved(SavedUse::Rename, window, cx);
    pub(crate) delete_saved_search(DeleteSavedSearch, window, cx) => choose_saved(SavedUse::Delete, window, cx);
    }

    /// A row of the list: the search into its panel (Find in Files when it has a Where,
    /// Replace when it has a replacement, else Find), renamed, or deleted after asking.
    pub(crate) fn use_saved_search(&mut self, a: &UseSavedSearch, window: &mut Window, cx: &mut Context<Self>) {
        let Some(list) = self.saved_or_alert(window, cx) else {
            return;
        };
        let Some(s) = list.iter().find(|s| s.name == a.name).cloned() else {
            return self.status_message(format!("No saved search is called \"{}\"", a.name), cx);
        };
        match a.how {
            SavedUse::Use => {
                self.last_saved_name = s.name.clone();
                match self.playground_panel().is_some() {
                    // the playground open: the search into it, to try it there
                    true => {
                        let (o, p) = (s.opts(), s.flags.preserve_case);
                        self.load_playground(&s.query, &s.replacement, o, p, cx);
                    }
                    false => self.load_search(&s, window, cx),
                }
            }
            SavedUse::Rename => {
                let from = s.name.clone();
                let done: OnDone = Box::new(move |v, _, ws, _, cx| {
                    let mut list = saved::list(&saved::path())?;
                    let to = v.text("text").trim().to_string();
                    saved::rename(&mut list, &from, &to)?;
                    ws.write_saved(&list, &format!("Renamed to \"{to}\""), cx)
                });
                if self.active_editor().is_none() {
                    return self.inform(window, cx, "No open tab", "Open a tab (File > New File) to rename it.");
                }
                self.show_input("New name", &s.name, done, window, cx);
            }
            SavedUse::Delete => {
                let question = format!("Delete the saved search \"{}\"?", s.name);
                self.confirm_saved_change(&question, "Delete", window, cx, move |list| {
                    saved::delete(list, &s.name);
                    Ok(format!("Deleted \"{}\"", s.name))
                });
            }
        }
    }

    /// `s` into the panel it is for, nothing run yet.
    pub(crate) fn load_search(&mut self, s: &SavedSearch, window: &mut Window, cx: &mut Context<Self>) {
        let opts = s.opts();
        if s.where_.is_empty() {
            let replace = !s.replacement.is_empty();
            match replace {
                true => self.replace(&Replace, window, cx),
                false => self.find(&Find, window, cx),
            }
            if let Some(p) = self.find_panel_mut() {
                (p.opts, p.preserve_case) = (opts, s.flags.preserve_case);
                let (q, r) = (p.query.clone(), p.replace.clone());
                self.find_quiet = true;
                q.update(cx, |e, cx| e.set_text(&s.query, cx));
                r.update(cx, |e, cx| e.set_text(&s.replacement, cx));
            }
            self.refresh_matches(cx);
        } else {
            self.find_in_files(&FindInFiles, window, cx);
            if let Some(p) = self.fif_panel_mut() {
                (p.opts, p.flags) = (opts, s.flags);
                let (q, w, r) = (p.query.clone(), p.where_.clone(), p.replace.clone());
                q.update(cx, |e, cx| e.set_text(&s.query, cx));
                w.update(cx, |e, cx| e.set_text(&s.where_, cx));
                r.update(cx, |e, cx| e.set_text(&s.replacement, cx));
            }
            self.validate_fif(cx);
        }
        cx.notify();
    }

    /// Import Saved Searches…: a file of them, checked whole first; a name in use gets " 2",
    /// " 3" … after it.
    pub(crate) fn import_saved_searches(
        &mut self,
        _: &ImportSavedSearches,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_paths(
            PathPromptOptions { files: true, directories: false, multiple: false, prompt: None },
            window,
            cx,
            |ws, paths, window, cx| {
                if let Some(path) = paths.into_iter().next() {
                    ws.import_from(&path, window, cx);
                }
            },
        );
    }

    pub(crate) fn import_from(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return self.alert(window, cx, "Could not import", &format!("{}: {e}", path.display())),
        };
        let incoming = match saved::parse(&text) {
            Ok(l) => l,
            Err(errors) => {
                let detail = crate::searching::pipeline::problems(&errors);
                return self.alert(window, cx, "Nothing was imported", &detail);
            }
        };
        let Some(mut list) = self.saved_or_alert(window, cx) else {
            return;
        };
        let names = saved::import(&mut list, incoming);
        let done = format!(
            "Imported {} saved search{}: {}",
            names.len(),
            if names.len() == 1 { "" } else { "es" },
            names.join(", ")
        );
        if let Err(e) = self.write_saved(&list, &done, cx) {
            self.alert(window, cx, "Could not import", &e);
        }
    }

    /// Export Saved Searches…: all of them, in the format Import reads.
    pub(crate) fn export_saved_searches(
        &mut self,
        _: &ExportSavedSearches,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(list) = self.saved_or_alert(window, cx) else {
            return;
        };
        let dir = self.last_dir.clone().unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()));
        let rx = cx.prompt_for_new_path(&dir, Some("Saved Searches.json"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            this.update(cx, |ws, cx| {
                let msg = match saved::write(&path, &list) {
                    Ok(()) => format!("Exported {} saved searches to {}", list.len(), tilde(&path)),
                    Err(e) => format!("Could not export: {e}"),
                };
                ws.status_message(msg, cx);
            })
            .ok();
        })
        .detach();
    }
}
