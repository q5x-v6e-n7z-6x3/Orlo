//! Acting on a search: Find > Extract, Bookmark Matching Lines and Highlight Matches act on
//! the matches the Find panel's query has in the active view (its selection only, with In
//! Selection on), or on the last Find in Files results.

use crate::searching::extract::{self, What};
use crate::searching::results::Source;
use crate::workspace::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExtractFrom {
    /// The active view, or its selections with In Selection on.
    Document,
    /// The last Find in Files results.
    Results,
}

/// Find > Extract: matches or matching lines, with or without their locations, to the
/// clipboard or a new document. The sources are only read.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct Extract {
    pub what: What,
    pub from: ExtractFrom,
    pub to_clipboard: bool,
    pub locations: bool,
}

/// Find > Highlight Matches: the matches (or the selected text's occurrences) in a style.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub struct HighlightMatches {
    pub style: usize,
}

impl Workspace {
    /// The Find query's matches in the active view, or why there are none to use.
    pub(crate) fn search_matches(&mut self, cx: &mut Context<Self>) -> Result<Vec<std::ops::Range<usize>>, String> {
        let query = self.current_query(cx);
        if query.is_empty() {
            return Err("Enter a query in Find first".into());
        }
        let opts = self.find_opts();
        if opts.regex {
            crate::searching::pattern::Pattern::new(&query, &opts)?;
        }
        let all = if self.find_panel().is_some_and(|p| p.syntax != crate::editing::text::SyntaxMask::Any) {
            self.syntax_matches(&query, &opts, cx)?
        } else {
            self.all_matches(&query, &opts, cx)
        };
        let failed = (self.active_editor())
            .and_then(|e| e.read(cx).read_state(cx).search_error(&query, &opts, self.find_scope()));
        match failed {
            Some(e) => Err(e.message()),
            None => Ok(all),
        }
    }

    /// Goto > Bookmarks > Bookmark Matching Lines: added to the bookmarks there are.
    pub(crate) fn bookmark_matches(&mut self, _: &BookmarkMatches, _: &mut Window, cx: &mut Context<Self>) {
        let all = match self.search_matches(cx) {
            Ok(all) => all,
            Err(e) => return self.status_message(e, cx),
        };
        let n = self.with_active(cx, |e, cx| {
            let n = e.sel(cx).bookmark_matches(&all);
            cx.notify();
            n
        });
        let n = n.unwrap_or(0);
        self.status_message(format!("{n} more line{} bookmarked", if n == 1 { "" } else { "s" }), cx);
    }

    /// The Find query's matches with the panel open, else every occurrence of the selected text
    /// (or of the word at the caret), whole words from a caret, case as typed.
    pub(crate) fn highlight_matches(&mut self, a: &HighlightMatches, _: &mut Window, cx: &mut Context<Self>) {
        let ranges = match self.find_panel().is_some() {
            true => self.search_matches(cx),
            false => self.occurrences(cx),
        };
        let ranges = match ranges {
            Ok(r) if !r.is_empty() => r,
            Ok(_) => return self.status_message("No matches to highlight", cx),
            Err(e) => return self.status_message(e, cx),
        };
        let style = a.style;
        self.with_active(cx, |e, cx| {
            e.sel(cx).add_highlights(style, ranges);
            cx.notify();
        });
    }

    fn occurrences(&self, cx: &App) -> Result<Vec<std::ops::Range<usize>>, String> {
        let e = self.active_editor().ok_or("Open a tab first")?;
        let s = e.read(cx).read_state(cx);
        let sel = s.view.selections.last().copied().ok_or("Nothing selected")?;
        let (range, whole_word) = match sel.is_empty() {
            false => (sel.range(), false),
            true => {
                let w = s.word_range_at(sel.head);
                if w.is_empty() {
                    return Err("Select text, or put the caret in a word".into());
                }
                (w, true)
            }
        };
        let text = s.doc.rope.slice(range).to_string();
        let o = FindOpts { case_sensitive: true, whole_word, ..FindOpts::default() };
        Ok(s.find_all(&text, &o))
    }

    /// Find > Extract: see `Extract`.
    pub(crate) fn extract(&mut self, a: &Extract, window: &mut Window, cx: &mut Context<Self>) {
        let (what, locations) = (a.what, a.locations);
        let title = match what {
            What::Matches => "Extracted Matches",
            What::Lines => "Extracted Lines",
        };
        let finish = move |ws: &mut Workspace,
                           out: Option<String>,
                           to_clipboard: bool,
                           window: &mut Window,
                           cx: &mut Context<Workspace>| {
            let Some(out) = out else {
                return ws.status_message("Nothing to extract", cx);
            };
            match to_clipboard {
                true => {
                    let n = out.lines().count();
                    editor::copy_to_clipboard(cx, (out, false));
                    ws.status_message(format!("Copied {n} line{}", if n == 1 { "" } else { "s" }), cx);
                }
                false => ws.scratch_tab(&out, title.into(), window, cx),
            }
        };
        match a.from {
            ExtractFrom::Document => {
                let all = match self.search_matches(cx) {
                    Ok(all) => all,
                    Err(e) => return self.status_message(e, cx),
                };
                let Some(e) = self.active_editor() else {
                    return;
                };
                let e = e.read(cx);
                let text = e.doc(cx).text();
                let name = (e.doc(cx).path.as_deref()).map_or_else(|| e.title(cx), tilde);
                let matches = crate::editing::text::char_to_byte_ranges(&text, &all);
                let out = extract::extract(&[extract::Source { name, text: &text, matches }], what, locations);
                finish(self, out, a.to_clipboard, window, cx);
            }
            ExtractFrom::Results => {
                let Some(set) = &self.result_set else {
                    return self.status_message("No Find in Files results in this window yet", cx);
                };
                // each file's text as the results were found in: a buffer at that version, or
                // the file on disk with the same bytes
                let buffers: Vec<(PathBuf, (u64, u64), ropey::Rope)> = (self.open_editors(cx).into_iter())
                    .filter_map(|e| {
                        let s = e.read(cx).read_state(cx);
                        Some((s.doc.path.clone()?, (s.buffer_id(), s.doc.version), s.doc.rope.clone()))
                    })
                    .collect();
                let files: Vec<_> = (set.files.iter())
                    .filter(|f| !f.entries().is_empty())
                    .map(|f| {
                        let hits: Vec<_> = f.entries().iter().map(|e| e.hit.bytes.clone()).collect();
                        (f.path.clone(), tilde(&f.shown), f.source.clone(), hits)
                    })
                    .collect();
                let to_clipboard = a.to_clipboard;
                let work = cx.background_spawn(async move {
                    let mut texts = Vec::new();
                    let mut stale = Vec::new();
                    for (path, name, source, hits) in files {
                        let text = match source {
                            Source::Buffer { id, version } => (buffers.iter())
                                .find(|(p, v, _)| *p == path && *v == (id, version))
                                .map(|(_, _, r)| r.to_string()),
                            Source::Disk { hash, encoding, .. } => (search::read_file_with_hint(&path, Some(encoding))
                                .ok())
                            .filter(|t| t.hash == hash)
                            .map(|t| t.text),
                        };
                        match text {
                            Some(t) => texts.push((name, t, hits)),
                            None => stale.push(name),
                        }
                    }
                    if !stale.is_empty() {
                        return Err(stale);
                    }
                    let sources: Vec<_> = (texts.iter())
                        .map(|(name, text, hits)| extract::Source { name: name.clone(), text, matches: hits.clone() })
                        .collect();
                    Ok(extract::extract(&sources, what, locations))
                });
                cx.spawn_in(window, async move |this, cx| {
                    let out = work.await;
                    this.update_in(cx, |ws, window, cx| match out {
                        Ok(out) => finish(ws, out, to_clipboard, window, cx),
                        Err(stale) => ws.status_message(
                            format!(
                                "{} changed since the search ({}…): Refresh the results first",
                                stale.len(),
                                stale[0]
                            ),
                            cx,
                        ),
                    })
                    .ok();
                })
                .detach();
            }
        }
    }
}
