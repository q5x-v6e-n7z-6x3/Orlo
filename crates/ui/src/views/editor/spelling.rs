//! Per-view spelling jobs; painting requests work, typing only invalidates it.

use super::*;
use crate::languages::spell::{self, Region, Stamp};

impl Editor {
    pub(crate) fn spell_check(&self) -> bool {
        !self.single_line && self.spell_check_override.unwrap_or(self.view.spell_check)
    }

    pub(crate) fn toggle_spell_check(&mut self, _: &ToggleSpellCheck, _: &mut Window, cx: &mut Context<Self>) {
        if self.single_line {
            return;
        }
        self.spell_check_override = Some(!self.spell_check());
        self.invalidate_spelling();
        cx.notify();
    }

    pub(crate) fn cancel_spelling_action(&mut self) {
        if let Some((stop, _)) = self.spell_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn start_spelling_action(&mut self, stop: Arc<AtomicBool>, cx: &App) {
        self.cancel_spelling_action();
        self.spell_stop = Some((stop, self.spelling_source(cx)));
    }

    pub(crate) fn spelling_source(&self, cx: &App) -> SpellingSource {
        SpellingSource {
            stamp: self.spell_stamp(cx),
            revision: self.spell_revision,
            buffer: self.doc(cx).id(),
            path: self.doc(cx).path.clone(),
            selections: self.state.selections.clone(),
            primary: self.state.primary(),
        }
    }

    pub(crate) fn next_misspelling(&mut self, _: &NextMisspelling, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            cx.emit(EditorEvent::ScanSpelling(true));
        }
    }

    pub(crate) fn prev_misspelling(&mut self, _: &PrevMisspelling, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            cx.emit(EditorEvent::ScanSpelling(false));
        }
    }

    pub(crate) fn spelling_suggestions(&mut self, _: &SpellingSuggestions, _: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            cx.emit(EditorEvent::SpellingSuggestions(self.state.cursor()));
        }
    }

    pub(crate) fn invalidate_spelling(&mut self) {
        self.cancel_spelling_action();
        self.spell_revision += 1;
        self.spell_job = None;
        self.spell_cache.clear();
    }

    fn spell_stamp(&self, cx: &App) -> Stamp {
        Stamp {
            version: self.doc(cx).version,
            syntax: self.highlighter.syntax_name().into(),
            selector: self.view.spelling_selector.clone(),
            generation: spell::DICTIONARY_GENERATION.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn sync_spelling(&mut self, cx: &App) -> bool {
        if self.spell_stop.as_ref().is_some_and(|(_, source)| *source != self.spelling_source(cx)) {
            self.cancel_spelling_action();
        }
        if !self.spell_check() {
            self.spell_job = None;
            self.spell_cache.clear();
            return false;
        }
        if self.spell_cache.sync(self.spell_stamp(cx)) {
            self.spell_job = None;
        }
        true
    }

    pub(crate) fn spelling_in_background(&mut self, visible: Vec<Region>, cx: &mut Context<Self>) {
        if !self.spell_check() || self.spell_job.is_some() {
            return;
        }
        let regions = self.spell_cache.missing(&visible);
        if regions.is_empty() {
            return;
        }
        let (rope, mut highlighter) = (self.doc(cx).rope.clone(), self.highlighter.clone());
        let stamp = self.spell_stamp(cx);
        let stop = Arc::new(AtomicBool::new(false));
        highlighter.cancel_on(stop.clone());
        let flag = stop.clone();
        let selector = stamp.selector.clone();
        let generation = stamp.generation;
        #[cfg(test)]
        let checker = self.spell_checker.clone();
        let task = cx.spawn(async move |this, cx| {
            let preparing = flag.clone();
            let prepared = highlight::on_worker(move || {
                spell::prepare(&rope, highlighter, regions, &selector, generation, &preparing)
            })
            .await;
            let results = match prepared {
                Some(prepared) if !flag.load(Ordering::Relaxed) => {
                    #[cfg(not(test))]
                    let result = spell::on_worker(prepared, flag).await;
                    #[cfg(test)]
                    let result = spell::on_worker_with_checker(prepared, flag, checker).await;
                    result
                }
                _ => return,
            };
            this.update(cx, |e, cx| {
                e.spell_job = None;
                if !e.spell_check() || e.spell_stamp(cx) != stamp {
                    return;
                }
                match results {
                    Some(entries) => e.spell_cache.adopt(&stamp, entries),
                    None => {
                        e.spell_check_override = Some(false);
                        e.status("System spell checking is unavailable", cx);
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.spell_job = Some(HtmlJob { _task: task, _stop: Cancel(stop) });
    }

    #[cfg(test)]
    pub(crate) fn spelling_revision(&self) -> u64 {
        self.spell_revision
    }

    #[cfg(test)]
    pub(crate) fn spelling_ranges(&self, line: usize) -> Vec<Range<usize>> {
        self.spell_cache.ranges(line).cloned().collect()
    }

    #[cfg(test)]
    pub(crate) fn spelling_drawn(&self) -> Vec<Range<usize>> {
        self.layout.as_ref().map_or_else(Vec::new, |l| l.drawn.spelling.clone())
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct SpellingSource {
    stamp: Stamp,
    revision: u64,
    buffer: u64,
    path: Option<PathBuf>,
    selections: Vec<Selection>,
    primary: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    struct WrongWordChecker;
    impl spell::Checker for WrongWordChecker {
        fn misspelled(&self, text: &str, _: &AtomicBool) -> Vec<spell::Misspelling> {
            text.find("wrng")
                .map(|at| spell::Misspelling { range: at..at + 4, source: at..at + 4 })
                .into_iter()
                .collect()
        }
    }

    #[gpui::test]
    fn spelling_long_lines_never_prepare_synchronously_during_typing_or_scheduling(cx: &mut TestAppContext) {
        for body in ["hello ".repeat(200_000), "a".repeat(1_200_000)] {
            let e = cx.new(|cx| Editor::new(EditorState::from_text(&format!("{body} wrng")), false, cx));
            e.update(cx, |e, cx| {
                e.spell_checker = Some(Arc::new(WrongWordChecker));
                e.spell_check_override = Some(true);
                spell::without_preparation(|| {
                    e.sel(cx).set_cursor(0);
                    e.edit(cx, |s| s.insert(" "));
                    assert!(e.sync_spelling(cx));
                    assert!(e.spelling_ranges(0).is_empty());
                    let end = e.doc(cx).len();
                    e.spelling_in_background(vec![Region { line: 0, chars: end - 4..end }], cx);
                    assert!(e.spelling_ranges(0).is_empty());
                });
            });
            let started = Instant::now();
            loop {
                cx.run_until_parked();
                if e.read_with(cx, |e, _| e.spell_job.is_none()) {
                    break;
                }
                assert!(started.elapsed() < Duration::from_secs(15));
                std::thread::yield_now();
            }
            e.read_with(cx, |e, cx| {
                let end = e.doc(cx).len();
                assert_eq!(e.spelling_ranges(0), vec![end - 4..end]);
            });
        }
    }
}
