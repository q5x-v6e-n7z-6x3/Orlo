//! Spell checking over the whole document, and the quick panel of choices for a word, acted on
//! only while the view and the word are still the ones it was opened for.

use super::*;
use crate::languages::spell;
use gpui::WeakEntity;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(crate) struct Job {
    _task: Task<()>,
    _stop: editor::Cancel,
}

impl Workspace {
    pub(crate) fn scan_spelling(
        &mut self,
        editor: Entity<Editor>,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_navigation();
        let e = editor.read(cx);
        let source = e.spelling_source(cx);
        let selection = e.state.selections[e.state.primary()].range();
        let anchor = if forward { selection.end } else { selection.start };
        let (rope, highlighter, selector) =
            (e.doc(cx).rope.clone(), e.highlighter.clone(), e.view.spelling_selector.clone());
        #[cfg(test)]
        let checker = e.spell_checker.clone();
        let stop = Arc::new(AtomicBool::new(false));
        editor.update(cx, |e, cx| e.start_spelling_action(stop.clone(), cx));
        let flag = stop.clone();
        let task = cx.spawn_in(window, async move |ws, cx| {
            #[cfg(not(test))]
            let result = spell::scan(rope, highlighter, anchor, forward, selector, flag.clone()).await;
            #[cfg(test)]
            let result =
                spell::scan_with_checker(rope, highlighter, anchor, forward, selector, flag.clone(), checker).await;
            ws.update_in(cx, |ws, window, cx| {
                if flag.load(Ordering::Relaxed)
                    || ws.active_editor().as_ref() != Some(&editor)
                    || editor.read(cx).spelling_source(cx) != source
                    || !editor.read(cx).is_focused(window)
                {
                    return;
                }
                ws.spelling_job = None;
                match result {
                    Ok(Some(range)) => editor.update(cx, |e, cx| e.select_range(range, cx)),
                    Ok(None) => ws.status_message("No misspellings", cx),
                    Err(_) => ws.status_message("System spell checking is unavailable", cx),
                }
            })
            .ok();
        });
        self.spelling_job = Some(Job { _task: task, _stop: editor::Cancel(stop) });
    }
}

pub(crate) struct Choice {
    editor: WeakEntity<Editor>,
    source: editor::SpellingSource,
    range: std::ops::Range<usize>,
    word: String,
    navigation: u64,
}

impl Workspace {
    pub(crate) fn spelling_suggestions(
        &mut self,
        editor: Entity<Editor>,
        at: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_panel(Kind::Spelling, Vec::new(), window, cx);
        let panel = self.quick_panel.as_mut().unwrap();
        panel.title = Some("Spelling Suggestions…".into());
        panel.loaded = false;
        let panel_id = panel.input.entity_id();
        let e = editor.read(cx);
        let source = e.spelling_source(cx);
        let (rope, mut highlighter, selector) =
            (e.doc(cx).rope.clone(), e.highlighter.clone(), e.view.spelling_selector.clone());
        let at = at.min(rope.len_chars());
        let line = rope.char_to_line(at);
        let base = rope.line_to_char(line);
        let col = at - base;
        let from = col.saturating_sub(spell::CHUNK_CHARS / 2);
        let region = spell::Region { line, chars: from..(from + spell::CHUNK_CHARS).min(rope.line(line).len_chars()) };
        let navigation = self.language.navigation_generation;
        let generation = spell::DICTIONARY_GENERATION.load(Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        highlighter.cancel_on(stop.clone());
        #[cfg(test)]
        let checker = e.spell_checker.clone();
        editor.update(cx, |e, cx| e.start_spelling_action(stop.clone(), cx));
        let flag = stop.clone();
        let task = cx.spawn_in(window, async move |ws, cx| {
            let prepared =
                spell::prepare_complete(rope, highlighter, vec![region], selector, generation, flag.clone()).await;
            if flag.load(Ordering::Relaxed) {
                return;
            }
            let result = match prepared {
                Some((_, prepared)) => {
                    let work = spell::WordWork::Inspect { prepared, line_base: base, at: col };
                    #[cfg(not(test))]
                    let result = spell::word_on_worker(work, flag.clone()).await;
                    #[cfg(test)]
                    let result = spell::word_on_worker_with_checker(work, flag.clone(), checker).await;
                    result
                }
                None => None,
            };
            ws.update_in(cx, |ws, window, cx| {
                if flag.load(Ordering::Relaxed)
                    || ws.language.navigation_generation != navigation
                    || ws.active_editor().as_ref() != Some(&editor)
                    || editor.read(cx).spelling_source(cx) != source
                    || !ws.quick_panel.as_ref().is_some_and(|p| p.input.entity_id() == panel_id)
                {
                    return;
                }
                ws.spelling_job = None;
                let Some(spell::WordAnswer::Suggestions(Some(suggestions))) = result else {
                    ws.status_message(
                        if result.is_none() {
                            "System spell checking is unavailable"
                        } else {
                            "No spelling suggestions"
                        },
                        cx,
                    );
                    ws.cancel_quick_panel(cx);
                    editor.read(cx).focus(window);
                    return;
                };
                let choice = Arc::new(Choice {
                    editor: editor.downgrade(),
                    source,
                    range: suggestions.range,
                    word: suggestions.word,
                    navigation,
                });
                let mut actions: Vec<_> = suggestions
                    .guesses
                    .into_iter()
                    .map(|guess| (guess.clone(), spell::WordAction::Replace(guess)))
                    .collect();
                actions.extend([
                    ("Ignore Word".into(), spell::WordAction::Ignore),
                    ("Learn Word".into(), spell::WordAction::Learn),
                ]);
                if suggestions.learned {
                    actions.push(("Unlearn Word".into(), spell::WordAction::Unlearn));
                }
                let items = actions
                    .into_iter()
                    .map(|(label, action)| {
                        quick_panel::Item::new(label, "", ItemAction::Spelling(choice.clone(), action))
                    })
                    .collect();
                let panel = ws.quick_panel.as_mut().unwrap();
                panel.set_items(Kind::Spelling, items);
                panel.loaded = true;
                let query = panel.query.clone();
                panel.refilter(&query);
                cx.notify();
            })
            .ok();
        });
        self.spelling_job = Some(Job { _task: task, _stop: editor::Cancel(stop) });
    }

    pub(crate) fn confirm_spelling(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.quick_panel.as_ref().filter(|p| p.loaded) else {
            return;
        };
        let Some(item) = p.filtered.get(p.selected).and_then(|i| p.items.get(*i)) else {
            return;
        };
        let ItemAction::Spelling(choice, action) = &item.action else {
            return;
        };
        let (choice, action) = (choice.clone(), action.clone());
        let Some(editor) = choice.editor.upgrade() else {
            return;
        };
        let e = editor.read(cx);
        if self.language.navigation_generation != choice.navigation
            || self.active_editor().as_ref() != Some(&editor)
            || e.spelling_source(cx) != choice.source
            || (e.doc(cx).rope).get_slice(choice.range.clone()).is_none_or(|r| r != choice.word.as_str())
        {
            self.cancel_quick_panel(cx);
            self.focus_editor(window, cx);
            return self.status_message("The text changed; ask for suggestions again", cx);
        }
        self.quick_panel = None;
        if let spell::WordAction::Replace(replacement) = action {
            editor.update(cx, |e, cx| {
                e.edit(cx, |s| {
                    s.replace_ranges(vec![(choice.range.clone(), replacement)]);
                })
            });
            editor.read(cx).focus(window);
            cx.notify();
            return;
        }
        #[cfg(test)]
        let checker = e.spell_checker.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let views: Vec<_> = self.open_editors(cx).into_iter().map(|e| e.downgrade()).collect();
        // past the checks above: the dictionary write runs to its end, and every view still open
        // is checked again
        cx.spawn(async move |_, cx| {
            let work = spell::WordWork::Change { word: choice.word.clone(), action };
            #[cfg(not(test))]
            let result = spell::word_on_worker(work, stop).await;
            #[cfg(test)]
            let result = spell::word_on_worker_with_checker(work, stop, checker).await;
            if matches!(result, Some(spell::WordAnswer::Changed)) {
                cx.update(|cx| {
                    for view in views {
                        view.update(cx, |e, cx| {
                            e.invalidate_spelling();
                            cx.notify();
                        })
                        .ok();
                    }
                })
                .ok();
            }
        })
        .detach();
        editor.read(cx).focus(window);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::{text, type_in, window};
    use super::*;

    #[gpui::test]
    fn a_choice_for_a_word_since_changed_closes_the_panel_and_says_so(cx: &mut gpui::TestAppContext) {
        let (ws, vcx) = window(cx);
        type_in(&ws, vcx, "teh");
        ws.update_in(vcx, |ws, window, cx| {
            let editor = ws.active_editor().unwrap();
            let choice = Arc::new(Choice {
                editor: editor.downgrade(),
                source: editor.read(cx).spelling_source(cx),
                range: 0..3,
                word: "teh".into(),
                navigation: ws.language.navigation_generation,
            });
            let item = quick_panel::Item::new(
                "the",
                "",
                ItemAction::Spelling(choice, spell::WordAction::Replace("the".into())),
            );
            ws.open_quick_panel(Kind::Spelling, vec![item], window, cx);
            ws.quick_panel.as_mut().unwrap().loaded = true;
            editor.update(cx, |e, cx| e.edit(cx, |s| s.insert("x")));
            ws.status = None;
            ws.confirm_spelling(window, cx);
            assert!(ws.quick_panel.is_none());
            assert!(ws.status.is_some());
        });
        assert_eq!(text(&ws, vcx), "tehx");
    }
}
