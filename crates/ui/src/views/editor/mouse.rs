//! Mouse, scroll wheel and minimap input, and scroll commands.

use super::*;
use gpui::{
    px, Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, Window,
};

impl Editor {
    pub(crate) fn on_spelling_right_click(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.spell_check() {
            return;
        }
        let Some(at) = self.hit(event.position, true, cx) else {
            return;
        };
        let line = self.doc(cx).rope.char_to_line(at);
        let col = at - self.doc(cx).rope.line_to_char(line);
        if self.spell_cache.ranges(line).any(|r| r.contains(&col)) {
            self.focus(window);
            cx.emit(EditorEvent::SpellingSuggestions(at));
        }
    }

    pub(crate) fn hover_definition(&mut self, at: Option<Point<Pixels>>, window: &Window, cx: &mut Context<Self>) {
        let word = at.and_then(|pos| {
            if self.single_line
                || self.results
                || self.drag.is_some()
                || !self.highlighter.scope().starts_with("source.")
                || !self.definitions_allowed(window, cx)
            {
                return None;
            }
            let offset = self.hit(pos, true, cx)?;
            if !(self.doc(cx).rope).get_char(offset).is_some_and(EditorState::is_word_char) {
                return None;
            }
            let scope = self.highlighter.cached_scope_id(&self.doc(cx).rope, offset)?;
            if highlight::class(scope) != highlight::Class::Code {
                return None;
            }
            self.navigation_word(offset, cx)
        });
        if let Some(h) = &mut self.definition_hover {
            h.over_word = word.as_ref().is_some_and(|(range, _)| *range == h.word);
            if h.over_word {
                return;
            }
            if word.is_none() && !h.definitions.is_empty() {
                h.task = Some(cx.spawn(async move |e, cx| {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    e.update(cx, |e, cx| {
                        if e.definition_hover.as_ref().is_some_and(|h| !h.over_word && !h.over_popup) {
                            e.close_definition_hover(cx);
                        }
                    })
                    .ok();
                }));
                return;
            }
        }
        self.close_definition_hover(cx);
        let Some((word, name)) = word else { return };
        let version = self.doc(cx).version;
        let (weak, here) = (cx.weak_entity(), window.window_handle());
        let keys = cx.intercept_keystrokes(move |_, window, cx| {
            if window.window_handle() == here {
                weak.update(cx, |e, cx| e.close_definition_hover(cx)).ok();
            }
        });
        let range = word.clone();
        let task = cx.spawn(async move |e, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            e.update(cx, |e, cx| {
                if e.definition_hover.as_ref().is_some_and(|h| {
                    h.word == range && h.version == e.doc(cx).version && h.source == e.definition_source(cx)
                }) {
                    e.definition_hover.as_mut().unwrap().requested = true;
                    cx.emit(EditorEvent::HoverDefinition(range, name, version));
                }
            })
            .ok();
        });
        self.definition_hover = Some(popup::DefinitionHover {
            source: self.definition_source(cx),
            word,
            version,
            requested: false,
            definitions: Arc::default(),
            lines: Arc::default(),
            viewport: window.viewport_size(),
            #[cfg(test)]
            rendered_rows: 0,
            over_word: true,
            over_popup: false,
            task: Some(task),
            _keys: keys,
        });
    }

    pub(crate) fn offset_for_position(&self, pos: Point<Pixels>, cx: &App) -> Option<usize> {
        self.hit(pos, false, cx)
    }

    /// The offset under `pos`: the char boundary nearest it, or with `glyph` the char the
    /// pointer is on (none past a row's text, left of it or below the last row). A
    /// tab's right half is taken for the char after it (columns inside a tab go to the nearer
    /// edge); floor the column to be exact.
    pub(crate) fn hit(&self, pos: Point<Pixels>, glyph: bool, cx: &App) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let lh = self.line_height() as f64;
        let y = f32::from(pos.y - layout.bounds.top()) as f64 + layout.scroll_top;
        let rows = self.rows();
        let line = (y / lh).floor().max(0.) as usize;
        let last = rows.rows(self.doc(cx)) - 1;
        let row = DisplayRow(line.min(last));
        let drawn = (row.0.checked_sub(layout.first_row)).and_then(|i| layout.rows.get(i));
        let Some(Some((x0, shaped))) = drawn else {
            if glyph {
                return None;
            }
            let doc = self.doc(cx);
            return Some(match (drawn, rows.row_bounds(doc, row)) {
                // a row of text not drawn (a drag above the view, or rows below it): the
                // column under the pointer, in cells
                (None, Some(_)) => {
                    let col = (f32::from(pos.x - layout.text_left) / f32::from(layout.space_w)).round();
                    rows.offset_at(doc, DisplayPoint::new(row.0, col.max(0.) as usize))
                }
                // a filler row has no char under the pointer; a press there lands at the end
                // of the line above it
                (_, Some(b)) => b.end,
                (_, None) => rows.offset_at(doc, DisplayPoint::new(row.0, 0)),
            });
        };
        if glyph && line > last {
            return None;
        }
        let left = layout.text_left + layout.space_w * *x0 as f32;
        let byte = match glyph {
            false => shaped.closest_index_for_x(pos.x - left),
            true => shaped.index_for_x(pos.x - left).filter(|_| pos.x >= left)?,
        };
        let shown_col = shaped.text[..byte.min(shaped.text.len())].chars().count();
        // the painted row gives the column, the text as it is now the offset: the frame can be
        // older than an edit (a click at a continuation row's start gives its break offset)
        Some(rows.offset_at(self.doc(cx), DisplayPoint::new(row.0, x0 + shown_col)))
    }

    /// The display point under `pos`, for an option-drag's box.
    fn row_col_for_position(&self, pos: Point<Pixels>, cx: &App) -> Option<DisplayPoint> {
        let layout = self.layout.as_ref()?;
        let y = f32::from(pos.y - layout.bounds.top()) as f64 + layout.scroll_top;
        let row = ((y / self.line_height() as f64).floor().max(0.) as usize).min(self.rows().rows(self.doc(cx)) - 1);
        if self.state.virtual_space.enabled() {
            if let Some(Some((x0, shaped))) = row.checked_sub(layout.first_row).and_then(|i| layout.rows.get(i)) {
                let x = pos.x - layout.text_left - layout.space_w * *x0 as f32;
                let byte = shaped.closest_index_for_x(x).min(shaped.text.len());
                let extra = (f32::from(x - shaped.width) / f32::from(layout.space_w)).round().max(0.) as usize;
                return Some(DisplayPoint::new(row, x0 + shaped.text[..byte].chars().count() + extra));
            }
        }
        let col = (f32::from(pos.x - layout.text_left) / f32::from(layout.space_w)).round();
        Some(DisplayPoint::new(row, col.max(0.) as usize))
    }

    fn selection_for_position(&self, pos: Point<Pixels>, cx: &App) -> Option<Selection> {
        let offset = self.offset_for_position(pos, cx)?;
        if self.state.virtual_space.allows(false) {
            let at = self.row_col_for_position(pos, cx)?;
            Some(self.read_state(cx).selection_at(at, &self.rows()))
        } else {
            Some(Selection::caret(offset))
        }
    }

    pub(crate) fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.button != MouseButton::Left {
            return;
        }
        window.focus(&self.focus_handle);
        self.sel(cx).sel_checkpoint();
        if let Some((mm_left, mm_first)) = self.layout.as_ref().and_then(|l| l.minimap) {
            if e.position.x >= mm_left {
                self.scroll_to_minimap(e.position.y, mm_first, cx);
                self.scrolled(cx);
                cx.notify();
                return;
            }
        }
        let Some(offset) = self.offset_for_position(e.position, cx) else {
            return;
        };
        self.dismiss_signature();
        if e.click_count == 2 && self.results {
            let line = self.doc(cx).point(offset).0;
            match crate::searching::search::result_target(self.doc(cx), line) {
                Some((path, to)) => {
                    let s = self.doc(cx);
                    let hit = crate::searching::search::result_match(s, &s.outlines, line, offset);
                    cx.emit(EditorEvent::OpenResult(path, to, hit))
                }
                None => self.sel(cx).select_word_at(offset),
            }
            cx.notify();
            return;
        }
        // a click ends a composition where it was
        self.marked.clear();
        let m = e.modifiers;
        let point = self.selection_for_position(e.position, cx).map_or((offset, 0), |s| s.head_point());
        let from = (self.read_state(cx)).drag_from_point(point, e.click_count, m.platform, m.shift);
        // in a document, a plain press on selected text leaves the selections be: the
        // release makes it a click, the pointer going 4 px a drag of the text
        if self.view.drag_text
            && !self.single_line
            && e.click_count == 1
            && !m.modified()
            && self.on_selection(e.position, cx)
        {
            let (this, here) = (cx.weak_entity(), window.window_handle());
            let _escape = cx.intercept_keystrokes(move |k, w, cx| {
                if k.keystroke.key != "escape" || w.window_handle() != here {
                    return;
                }
                if this.update(cx, |ed, cx| ed.cancel_text_drag(cx)).unwrap_or(false) {
                    cx.stop_propagation();
                }
            });
            let text = TextDrag { press: e.position, moved: false, _escape };
            self.drag = Some(Drag { from, corner: None, pos: e.position, text: Some(text) });
            cx.notify();
            return;
        }
        // option-drag selects a box from the press
        let column = (m.alt && e.click_count == 1)
            .then(|| self.row_col_for_position(e.position, cx))
            .flatten()
            .map(|at| (at, f32::from(e.position.x - self.layout.as_ref().unwrap().text_left)));
        if let Some((a, x)) = column {
            self.with_rows(window, cx, |s, rows, measure| {
                s.drag_box_measured(&from.others, from.subtract, (a.row, x), (a.row, x), rows, measure);
            });
        } else {
            self.sel(cx).drag_to_point(&from, point);
        }
        if from.unit == crate::editing::text::DragUnit::Line {
            self.lines_over_folds(cx);
        }
        self.drag = Some(Drag { from, corner: column, pos: e.position, text: None });
        self.cancel_spelling_action();
        cx.notify();
    }

    /// Whether the char under `pos` is selected: a press there may start dragging the text.
    fn on_selection(&self, pos: Point<Pixels>, cx: &App) -> bool {
        let sels = &self.state.selections;
        self.hit(pos, true, cx).is_some_and(|o| sels.iter().any(|s| s.range().contains(&o)))
    }

    /// Where text dragged to `pos` would land: at the offset under the pointer, if that
    /// is in the text and not in or against the selections being dragged.
    pub(crate) fn drop_offset(&self, pos: Point<Pixels>, cx: &App) -> Option<usize> {
        let in_text = self.layout.as_ref()?.text_area.contains(&pos);
        let at = self.offset_for_position(pos, cx).filter(|_| in_text)?;
        self.read_state(cx).can_drop(at).then_some(at)
    }

    /// Escape in a text drag: nothing is dropped and the selections stay. False when there
    /// is no text drag.
    fn cancel_text_drag(&mut self, cx: &mut Context<Self>) -> bool {
        let dragging = self.drag.as_ref().is_some_and(|d| d.text.is_some());
        if dragging {
            self.drag = None;
            cx.notify();
        }
        dragging
    }

    pub(crate) fn on_mouse_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.pressed_button != Some(MouseButton::Left) {
            // The button went up unheard over another window: either kind of drag is over.
            if self.drag.take().is_some() {
                self.sel(cx).sel_checkpoint();
                cx.notify();
            }
            return;
        }
        let Some(drag) = &mut self.drag else { return };
        drag.pos = e.position;
        if let Some(t) = &mut drag.text {
            // a pointer within 4 px is still a click
            t.moved |= (e.position - t.press).magnitude() > 4.;
            if !t.moved {
                return;
            }
        }
        self.drag_to_mouse(window, cx);
        if self.outside_rows(e.position) != 0. && !self.autoscrolling {
            self.autoscrolling = true;
            cx.spawn_in(window, async move |this, cx| {
                let mut last = std::time::Instant::now();
                loop {
                    cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
                    let dt = std::mem::replace(&mut last, std::time::Instant::now()).elapsed();
                    let go = this.update_in(cx, |e, window, cx| e.autoscroll(dt.as_secs_f32(), window, cx));
                    if !go.unwrap_or(false) {
                        break;
                    }
                }
            })
            .detach();
        }
        cx.notify();
    }

    /// Select to where the mouse is now, from the press.
    fn drag_to_mouse(&mut self, window: &Window, cx: &App) {
        // a text drag selects nothing: the text lands where the button comes up
        let Some(Drag { from, corner, pos, text: None }) = &self.drag else {
            return;
        };
        let (from, corner, pos) = (from.clone(), *corner, *pos);
        match corner {
            Some((a, x)) => {
                // measured as the press was: columns count characters, and an emoji or CJK
                // character is two cells wide
                if let Some(b) = self.row_col_for_position(pos, cx) {
                    let to_x = f32::from(pos.x - self.layout.as_ref().unwrap().text_left);
                    self.with_rows(window, cx, |s, rows, measure| {
                        s.drag_box_measured(&from.others, from.subtract, (a.row, x), (b.row, to_x), rows, measure);
                    });
                }
            }
            None => {
                if let Some(selection) = self.selection_for_position(pos, cx) {
                    self.sel(cx).drag_to_point(&from, selection.head_point());
                    if from.unit == crate::editing::text::DragUnit::Line {
                        self.lines_over_folds(cx);
                    }
                }
            }
        }
        self.cancel_spelling_action();
    }

    /// How far `pos` is above (negative) or below the text rows; 0 inside.
    fn outside_rows(&self, pos: Point<Pixels>) -> f32 {
        let Some(b) = self.layout.as_ref().map(|l| l.bounds) else {
            return 0.;
        };
        if pos.y < b.top() {
            f32::from(pos.y - b.top())
        } else if pos.y > b.bottom() {
            f32::from(pos.y - b.bottom())
        } else {
            0.
        }
    }

    /// `dt` seconds of the scroll while a drag is held past the top or bottom edge, faster
    /// the further out (Sublime's runs about 80 lines a second just past the edge); false to
    /// stop.
    fn autoscroll(&mut self, dt: f32, window: &Window, cx: &mut Context<Self>) -> bool {
        let over = match &self.drag {
            Some(d) if !self.single_line => self.outside_rows(d.pos),
            _ => 0.,
        };
        if over == 0. {
            self.autoscrolling = false;
            return false;
        }
        let step = self.line_height() * (50. + over.abs() * 4.) * dt.min(0.1);
        let step = (step * over.signum()) as f64;
        self.stop_scroll_anim();
        self.scroll_top = (self.scroll_top + step).clamp(0., self.max_scroll(cx));
        self.drag_to_mouse(window, cx);
        cx.notify();
        true
    }

    pub(crate) fn on_mouse_up(&mut self, e: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.end_drag(Some(e), cx);
    }

    /// Let go off the view, or over something drawn on it.
    pub(crate) fn on_mouse_up_out(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.end_drag(None, cx);
    }

    /// The button is up (`up`: over this view). A text drag was a click if the pointer
    /// never went 4 px; else its text moves to where the pointer is, or with Option is copied
    /// there, and nothing happens when that is no place to drop it.
    fn end_drag(&mut self, up: Option<&MouseUpEvent>, cx: &mut Context<Self>) {
        if let Some(Drag { from, text: Some(text), .. }) = self.drag.take() {
            match (text.moved, up) {
                (false, _) => self.sel(cx).drag_to(&from, from.anchor.start),
                (true, Some(up)) => {
                    if let Some(at) = self.drop_offset(up.position, cx) {
                        self.edit_text(cx, |s| {
                            s.move_text(at, up.modifiers.alt);
                        });
                    }
                }
                (true, None) => {}
            }
            cx.notify();
        }
        // the drag's end: the state the press replaced is pushed, once
        self.sel(cx).sel_checkpoint();
        self.cancel_spelling_action();
    }

    pub(crate) fn on_scroll(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.single_line {
            return;
        }
        let delta = e.delta.pixel_delta(px(self.line_height()));
        let dy = f32::from(delta.y) as f64;
        self.stop_scroll_anim();
        self.scroll_top = (self.scroll_top - dy).clamp(0., self.max_scroll(cx));
        // sideways room is the widest line on screen, so the view can snap left
        // after scrolling down to shorter lines; track the widest line of the file to fix
        let max_left = match &self.layout {
            Some(l) if self.wrap.is_none() => {
                let widest = (l.rows.iter().flatten())
                    .map(|(x0, s)| f32::from(s.width + l.space_w * *x0 as f32))
                    .fold(0., f32::max);
                (widest + 60. - f32::from(l.text_width)).max(0.)
            }
            // wrapped rows never run past the right edge
            _ => 0.,
        };
        self.scroll_left = (self.scroll_left - f32::from(delta.x)).clamp(0., max_left);
        self.scrolled(cx);
        self.scroll_bar = Some(cx.spawn(async move |this, cx| {
            let shown = std::time::Duration::from_millis(theme::SCROLL_PUCK_SHOWN_MS);
            cx.background_executor().timer(shown).await;
            this.update(cx, |e, cx| {
                e.scroll_bar = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The rows fully in view, less one: a Page Up or Down (before the first paint, 40).
    pub fn page_lines(&self) -> usize {
        self.layout.as_ref().map_or(PAGE_LINES, |l| {
            let rows = (f32::from(l.bounds.size.height) / self.line_height()).floor() as usize;
            rows.saturating_sub(1).max(1)
        })
    }

    /// Sublime on macOS does not scroll past the end.
    pub(crate) fn max_scroll(&self, cx: &App) -> f64 {
        let height = self.layout.as_ref().map_or(0., |l| f32::from(l.bounds.size.height));
        let total = self.rows().rows(self.doc(cx)) as f64 * self.line_height() as f64;
        (total - height as f64).max(0.)
    }

    pub(crate) fn scroll_lines(&mut self, a: &ScrollLines, _: &mut Window, cx: &mut Context<Self>) {
        let step = if a.up { -1. } else { 1. } * self.line_height();
        self.scroll_top = (self.scroll_top + step as f64).clamp(0., self.max_scroll(cx));
        self.scrolled(cx);
        cx.notify();
    }

    /// Goto > Scroll > Scroll to Selection.
    pub(crate) fn show_at_center(&mut self, _: &ShowAtCenter, _: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = &self.layout else { return };
        let height = f32::from(layout.bounds.size.height);
        let row = self.rows().row_of(self.doc(cx), self.state.cursor()).row;
        let y = row.0 as f64 * self.line_height() as f64;
        self.scroll_top = (y - height as f64 / 2.).clamp(0., self.max_scroll(cx));
        self.scrolled(cx);
        cx.notify();
    }

    /// A click on the minimap centres the editor on that row.
    fn scroll_to_minimap(&mut self, y: Pixels, mm_first: usize, cx: &App) {
        let Some(layout) = &self.layout else { return };
        let height = f32::from(layout.bounds.size.height);
        let lh = self.line_height() as f64;
        let into = f32::from(y - layout.bounds.top()) / self.minimap_line_height();
        let row = mm_first as f64 + into as f64;
        self.scroll_anim = None;
        self.scroll_top = (row * lh - height as f64 / 2.).clamp(0., self.max_scroll(cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editing::text::{EditorState, Motion};
    use gpui::{point, Entity, Modifiers, TestAppContext, VisualTestContext};

    #[gpui::test]
    fn option_drag_without_virtual_space_keeps_measured_columns(cx: &mut TestAppContext) {
        // the test text system draws an emoji two cells wide: columns must still count chars
        let (e, cx) = super::super::focused_editor("😀😀ab\nxxxxxx", None, cx);
        let p = e.read_with(cx, |e, cx| {
            let b = e.char_bounds(2, cx).unwrap();
            point(b.left() + gpui::px(1.), b.center().y)
        });
        let alt = Modifiers { alt: true, ..Modifiers::none() };
        cx.simulate_mouse_down(p, MouseButton::Left, alt);
        cx.simulate_mouse_move(p, Some(MouseButton::Left), alt);
        cx.simulate_mouse_up(p, MouseButton::Left, alt);
        e.read_with(cx, |e, _| assert_eq!(e.state.selections[0].range(), 2..2));
    }

    /// An editor alone in a test window; gpui's test text system lays text out monospace.
    fn editor<'a>(text: &str, cx: &'a mut TestAppContext) -> (Entity<Editor>, &'a mut VisualTestContext) {
        let text = text.to_string();
        cx.add_window_view(move |_, cx| {
            let mut e = Editor::new(EditorState::from_text(&text), false, cx);
            e.wrap_override = Some(false);
            e
        })
    }

    #[gpui::test]
    fn a_drag_past_the_view_keeps_the_column_under_the_pointer(cx: &mut TestAppContext) {
        let (e, cx) = editor(&"abcdefgh\n".repeat(200), cx);
        cx.run_until_parked();
        let (a, below) = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            let below = point(l.text_left + l.space_w * 3.1, l.bounds.bottom() + gpui::px(e.line_height() * 3.));
            (cell(e, 1.1, 0.), below)
        });
        cx.simulate_mouse_down(a, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(below, Some(MouseButton::Left), Modifiers::none());
        e.read_with(cx, |e, cx| assert_eq!(e.doc(cx).point(e.state.selections[0].head).1, 3));
        cx.simulate_mouse_up(below, MouseButton::Left, Modifiers::none());
    }

    #[gpui::test]
    fn a_click_on_a_long_line_scrolled_sideways_lands_on_the_char_under_it(cx: &mut TestAppContext) {
        let (e, cx) = editor(&format!("{}END\nnext\n", "x".repeat(6_000)), cx);
        e.update(cx, |e, cx| {
            e.sel(cx).move_cursors(Motion::LineEnd, false);
            e.scroll_to_cursor(cx);
            cx.notify();
        });
        cx.run_until_parked();
        // the middle of the "N" of END: char 6,001
        let (at, scrolled) = e.read_with(cx, |e, _| (cell(e, 6_001.5, 0.), e.scroll_left));
        assert!(scrolled > 0., "the view went to the line's end");
        cx.simulate_click(at, Modifiers::none());
        let caret = e.read_with(cx, |e, _| e.state.cursor());
        assert!((6_001..=6_002).contains(&caret), "{caret}");
    }

    #[gpui::test]
    fn clicks_and_ime_bounds_below_a_fold_go_by_the_rows_drawn(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler;
        let (e, cx) = editor("zero\none\ntwo\nthree\nfour\nfive\n", cx);
        e.update(cx, |e, cx| {
            e.fold_lines_now(0, 3, cx);
            cx.notify();
        });
        cx.run_until_parked();
        let (at, layout_bounds, lh) = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            let lh = e.line_height();
            // the second row shows "four"; the first, "zero" and its fold's placeholder, whose
            // blank columns its marker is drawn over
            let drawn: Vec<&str> = (l.rows.iter().flatten()).map(|r| r.1.text.as_ref()).collect();
            assert_eq!(drawn, ["zero   ", "four", "five", ""]);
            let x = l.text_left + l.space_w * 2.2;
            (point(x, l.bounds.top() + px(lh * 1.5)), l.bounds, lh)
        });
        cx.simulate_click(at, Modifiers::none());
        let (caret, four) = e.read_with(cx, |e, cx| (e.state.cursor(), e.doc(cx).line_start(4)));
        assert_eq!(caret, four + 2);
        // the input method's window goes where that caret is drawn
        let ime = e.update_in(cx, |e, window, cx| {
            let at = e.doc(cx).char_to_utf16(caret);
            e.bounds_for_range(at..at, layout_bounds, window, cx)
        });
        assert_eq!(ime.map(|b| b.origin.y), Some(layout_bounds.top() + px(lh)));
        // the view scrolls over the rows it shows: zero, four, five and the empty last line
        e.read_with(cx, |e, cx| assert_eq!(e.rows().rows(e.doc(cx)), 4));
    }

    #[gpui::test]
    fn a_drag_selects_from_the_press_to_the_release(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha beta gamma\n", cx);
        cx.run_until_parked();
        let (a, b) = e.read_with(cx, |e, _| (cell(e, 6.1, 0.), cell(e, 10.1, 0.)));
        cx.simulate_mouse_down(a, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(b, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(b, MouseButton::Left, Modifiers::none());
        let sel = e.read_with(cx, |e, _| e.state.selections.clone());
        assert_eq!(sel, vec![crate::editing::text::Selection::new(6, 10)]);
    }

    #[gpui::test]
    fn a_drag_is_one_step_of_selection_history(cx: &mut TestAppContext) {
        use crate::editing::text::Selection;
        let (e, cx) = editor("alpha beta gamma\n", cx);
        cx.run_until_parked();
        let (a, b, c) = (at(&e, cx, 6.1), at(&e, cx, 8.1), at(&e, cx, 10.1));
        cx.simulate_mouse_down(a, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(b, Some(MouseButton::Left), Modifiers::none());
        // a frame mid-drag must not record the in-between selection
        cx.run_until_parked();
        e.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.simulate_mouse_move(c, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(c, MouseButton::Left, Modifiers::none());
        e.update(cx, |e, cx| {
            assert_eq!(e.state.selections, vec![Selection::new(6, 10)]);
            assert!(e.sel(cx).undo_selection());
            assert_eq!(e.state.selections, vec![Selection::caret(0)]);
            assert!(!e.sel(cx).undo_selection());
        });
    }

    #[gpui::test]
    fn an_alt_cmd_shift_drag_takes_its_box_out_of_the_selections(cx: &mut TestAppContext) {
        use crate::editing::text::Selection;
        let (e, cx) = editor("abcdef\nabcdef\nabcdef\n", cx);
        e.update(cx, |e, cx| {
            e.sel(cx).set_selections(vec![Selection::new(0, 6), Selection::new(14, 20)]);
        });
        cx.run_until_parked();
        let (a, b) = e.read_with(cx, |e, _| (cell(e, 2., 0.), cell(e, 4., 2.)));
        let m = Modifiers { alt: true, platform: true, shift: true, ..Modifiers::none() };
        cx.simulate_mouse_down(a, MouseButton::Left, m);
        cx.simulate_mouse_move(b, Some(MouseButton::Left), m);
        cx.simulate_mouse_up(b, MouseButton::Left, m);
        let sel = e.read_with(cx, |e, _| e.state.selections.clone());
        assert_eq!(sel, [Selection::new(0, 2), Selection::new(4, 6), Selection::new(14, 16), Selection::new(18, 20),]);
    }

    /// Where display column `col` of row `row` is in the drawn frame; a fraction is how far
    /// into the cell.
    fn cell(e: &Editor, col: f32, row: f32) -> Point<Pixels> {
        let l = e.layout.as_ref().unwrap();
        let y = l.bounds.top() + px(e.line_height() * (row + 0.5));
        point(l.text_left + l.space_w * col, y)
    }

    fn at(e: &Entity<Editor>, cx: &mut VisualTestContext, col: f32) -> Point<Pixels> {
        e.read_with(cx, |e, _| cell(e, col, 0.))
    }

    fn state(e: &Entity<Editor>, cx: &mut VisualTestContext) -> (String, Vec<Selection>) {
        e.read_with(cx, |e, cx| (e.doc(cx).text(), e.state.selections.clone()))
    }

    /// A press at `from`, the pointer to `to`, and the release there with `up` held.
    fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>, up: Modifiers) {
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(to, MouseButton::Left, up);
        cx.run_until_parked();
    }

    fn option() -> Modifiers {
        Modifiers { alt: true, ..Modifiers::none() }
    }

    /// "alpha beta gamma" with "beta" selected, drawn.
    fn beta(cx: &mut TestAppContext) -> (Entity<Editor>, &mut VisualTestContext) {
        let (e, cx) = editor("alpha beta gamma\n", cx);
        e.update(cx, |e, cx| {
            e.sel(cx).set_selection(6, 10);
            cx.notify();
        });
        cx.run_until_parked();
        (e, cx)
    }

    const PLAIN: &str = "alpha beta gamma\n";

    #[gpui::test]
    fn a_drag_moves_the_selected_text_forward_and_back(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        // "beta" onto the front of "gamma"
        let (press, drop) = (at(&e, cx, 7.4), at(&e, cx, 11.1));
        drag(cx, press, drop, Modifiers::none());
        let moved = ("alpha  betagamma\n".to_string(), vec![Selection::new(7, 11)]);
        assert_eq!(state(&e, cx), moved);
        // and back to the front of the line
        let (press, drop) = (at(&e, cx, 8.4), at(&e, cx, 0.1));
        drag(cx, press, drop, Modifiers::none());
        let back = ("betaalpha  gamma\n".to_string(), vec![Selection::new(0, 4)]);
        assert_eq!(state(&e, cx), back);
    }

    #[gpui::test]
    fn option_at_the_release_copies(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        // past the line's end: the end of the text
        let (press, drop) = (at(&e, cx, 7.4), at(&e, cx, 16.4));
        drag(cx, press, drop, option());
        let copied = ("alpha beta gammabeta\n".to_string(), vec![Selection::new(16, 20)]);
        assert_eq!(state(&e, cx), copied);
    }

    #[gpui::test]
    fn a_drop_in_or_against_the_source_cancels(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        let kept = (PLAIN.to_string(), vec![Selection::new(6, 10)]);
        // inside it, against its start, against its end; moved or copied
        for (col, up) in [(8.4, Modifiers::none()), (6.1, Modifiers::none()), (10.1, option())] {
            let (press, drop) = (at(&e, cx, 7.4), at(&e, cx, col));
            drag(cx, press, drop, up);
            assert_eq!(state(&e, cx), kept, "{col}");
        }
    }

    #[gpui::test]
    fn escape_cancels_the_drag_and_only_a_drag(cx: &mut TestAppContext) {
        cx.update(|cx| cx.bind_keys(key_bindings()));
        let (e, cx) = editor(PLAIN, cx);
        let both = vec![Selection::new(0, 5), Selection::new(6, 10)];
        e.update(cx, |e, cx| e.sel(cx).set_selections(both.clone()));
        cx.run_until_parked();
        // on its way: the drop that would have followed does nothing, and the key did not also
        // collapse the selections
        let (press, far) = (at(&e, cx, 7.4), at(&e, cx, 14.1));
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(far, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_keystrokes("escape");
        assert!(e.read_with(cx, |e, _| e.drag.is_none()));
        cx.simulate_mouse_move(far, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(far, MouseButton::Left, Modifiers::none());
        assert_eq!(state(&e, cx), (PLAIN.to_string(), both.clone()));
        // before it moved: the press is no click either
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        cx.simulate_keystrokes("escape");
        cx.simulate_mouse_up(press, MouseButton::Left, Modifiers::none());
        assert_eq!(state(&e, cx), (PLAIN.to_string(), both));
        // with no drag, Escape is what it was
        cx.simulate_keystrokes("escape");
        assert_eq!(e.read_with(cx, |e, _| e.state.selections.len()), 1);
    }

    #[gpui::test]
    fn escape_in_another_window_leaves_a_text_drag_alone(cx: &mut TestAppContext) {
        let mut other = cx.clone();
        let (e, cx) = beta(cx);
        let (_, other) = other.add_window_view(|_, cx| Editor::new(EditorState::new(), false, cx));
        let (press, far) = (at(&e, cx, 7.4), at(&e, cx, 14.1));
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(far, Some(MouseButton::Left), Modifiers::none());
        other.simulate_keystrokes("escape");
        assert!(e.read_with(cx, |e, _| e.drag.is_some()));
        cx.simulate_keystrokes("escape");
        assert!(e.read_with(cx, |e, _| e.drag.is_none()));
    }

    #[gpui::test]
    fn a_text_drag_ends_when_the_pointer_returns_without_the_button(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        // the button went up over another window, which this one heard nothing of
        let (press, far) = (at(&e, cx, 7.4), at(&e, cx, 14.1));
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(far, Some(MouseButton::Left), Modifiers::none());
        assert!(e.read_with(cx, |e, _| e.drag.is_some()));
        cx.simulate_mouse_move(far, None, Modifiers::none());
        assert!(e.read_with(cx, |e, _| e.drag.is_none()));
        assert_eq!(state(&e, cx), (PLAIN.to_string(), vec![Selection::new(6, 10)]));
    }

    #[gpui::test]
    fn a_selection_drag_ends_when_the_pointer_returns_without_the_button(cx: &mut TestAppContext) {
        let (e, cx) = editor(&"line\n".repeat(200), cx);
        cx.run_until_parked();
        let (press, below) = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            (cell(e, 1.4, 0.), point(l.text_left, l.bounds.bottom() + px(30.)))
        });
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(below, Some(MouseButton::Left), Modifiers::none());
        assert!(e.read_with(cx, |e, _| e.drag.is_some()));
        cx.simulate_mouse_move(press, None, Modifiers::none());
        e.update_in(cx, |e, window, cx| {
            assert!(e.drag.is_none(), "the release over another window ended the drag");
            assert!(!e.autoscroll(0.05, window, cx));
        });
    }

    #[gpui::test]
    fn a_press_on_selected_text_collapses_only_when_it_is_a_click(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        let press = at(&e, cx, 7.4);
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        // the selection waits for the release
        assert_eq!(state(&e, cx).1, vec![Selection::new(6, 10)]);
        // a pointer within 4 px is still a click
        let near = press + point(px(2.), px(1.));
        cx.simulate_mouse_move(near, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(near, MouseButton::Left, Modifiers::none());
        assert_eq!(state(&e, cx), (PLAIN.to_string(), vec![Selection::caret(7)]));
        // the press was one step of selection history
        e.update(cx, |e, cx| {
            assert!(e.sel(cx).undo_selection());
            assert_eq!(e.state.selections, vec![Selection::new(6, 10)]);
        });
    }

    #[gpui::test]
    fn several_selections_move_together_joined_by_line_breaks(cx: &mut TestAppContext) {
        let (e, cx) = editor("one two three four five\n", cx);
        let both = vec![Selection::new(0, 3), Selection::new(8, 13)];
        e.update(cx, |e, cx| e.sel(cx).set_selections(both));
        cx.run_until_parked();
        // dragged by "three" to the end of "four"
        let (press, drop) = (at(&e, cx, 9.4), at(&e, cx, 18.1));
        drag(cx, press, drop, Modifiers::none());
        let joined = (" two  fourone\nthree five\n".to_string(), vec![Selection::new(10, 19)]);
        assert_eq!(state(&e, cx), joined);
    }

    #[gpui::test]
    fn undo_and_redo_take_the_drop_back_and_forth(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        let (press, drop) = (at(&e, cx, 7.4), at(&e, cx, 11.1));
        drag(cx, press, drop, Modifiers::none());
        let moved = state(&e, cx);
        let run = |e: &Entity<Editor>, cx: &mut VisualTestContext, a: &dyn gpui::Action| {
            e.update_in(cx, |e, window, cx| assert!(e.perform_action(a, window, cx)));
        };
        run(&e, cx, &Undo);
        assert_eq!(state(&e, cx), (PLAIN.to_string(), vec![Selection::new(6, 10)]));
        run(&e, cx, &Redo);
        assert_eq!(state(&e, cx), moved);
        // a copy is one step too
        run(&e, cx, &Undo);
        let (press, drop) = (at(&e, cx, 7.4), at(&e, cx, 16.4));
        drag(cx, press, drop, option());
        run(&e, cx, &Undo);
        assert_eq!(state(&e, cx), (PLAIN.to_string(), vec![Selection::new(6, 10)]));
    }

    #[gpui::test]
    fn another_view_of_the_buffer_follows_the_drop(cx: &mut TestAppContext) {
        let (a, cx) = beta(cx);
        let b = a.update(cx, |a, cx| cx.new(|cx| a.another_view(cx)));
        // a caret in "gamma", between the source and the drop
        b.update(cx, |b, cx| b.sel(cx).set_selection(12, 12));
        let (press, drop) = (at(&a, cx, 7.4), at(&a, cx, 14.1));
        drag(cx, press, drop, Modifiers::none());
        assert_eq!(state(&a, cx).0, "alpha  gambetama\n");
        // the other view applies the document's change as it is delivered
        cx.run_until_parked();
        let followed = ("alpha  gambetama\n".to_string(), vec![Selection::caret(8)]);
        assert_eq!(state(&b, cx), followed);
    }

    #[gpui::test]
    fn drag_text_off_keeps_the_old_press(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        e.update(cx, |e, _| e.view.drag_text = false);
        let (press, to) = (at(&e, cx, 7.4), at(&e, cx, 12.4));
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        // the press collapsed the selection at once, and the drag selects from it
        assert_eq!(state(&e, cx).1, vec![Selection::caret(7)]);
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        assert_eq!(state(&e, cx), (PLAIN.to_string(), vec![Selection::new(7, 12)]));
    }

    #[gpui::test]
    fn a_modified_press_on_selected_text_keeps_its_meaning(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        let shift = Modifiers { shift: true, ..Modifiers::none() };
        let (press, to) = (at(&e, cx, 7.4), at(&e, cx, 12.4));
        // what the press selects at once and what the drag to column 12 leaves: shift extends
        // from the anchor, cmd adds a selection to the one it is in, option makes a box
        let held = [
            (shift, Selection::new(6, 7), Selection::new(6, 12)),
            (Modifiers::command(), Selection::new(6, 10), Selection::new(6, 12)),
            (option(), Selection::caret(7).in_rectangle(), Selection::new(7, 12).in_rectangle()),
        ];
        for (m, at_once, dragged) in held {
            e.update(cx, |e, cx| e.sel(cx).set_selection(6, 10));
            cx.simulate_mouse_down(press, MouseButton::Left, m);
            assert_eq!(state(&e, cx).1, vec![at_once], "{m:?}");
            cx.simulate_mouse_move(to, Some(MouseButton::Left), m);
            cx.simulate_mouse_up(to, MouseButton::Left, m);
            assert_eq!(state(&e, cx), (PLAIN.to_string(), vec![dragged]), "{m:?}");
        }
    }

    #[gpui::test]
    fn a_press_waits_only_when_the_char_under_the_pointer_is_selected(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        let press = |cx: &mut VisualTestContext, p| {
            cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::none());
            let sels = e.read_with(cx, |e, _| e.state.selections.clone());
            cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::none());
            sels
        };
        // the space before "beta" and the one after it: the nearest boundary is the
        // selection's edge, but the pointer is on a char that is not selected
        let kept = vec![Selection::new(6, 10)];
        for (col, caret) in [(5.8, 6), (10.2, 10)] {
            e.update(cx, |e, cx| e.sel(cx).set_selection(6, 10));
            let p = at(&e, cx, col);
            assert_eq!(press(cx, p), vec![Selection::caret(caret)], "{col}");
        }
        // the first and the last selected char, on their outer halves
        for col in [6.1, 9.9] {
            e.update(cx, |e, cx| e.sel(cx).set_selection(6, 10));
            let p = at(&e, cx, col);
            assert_eq!(press(cx, p), kept, "{col}");
        }
        // left of the text, in its margin: no char either
        e.update(cx, |e, cx| e.sel(cx).set_selection(0, 5));
        let p = at(&e, cx, -0.3);
        assert_eq!(press(cx, p), vec![Selection::caret(0)]);
        // one selected char
        e.update(cx, |e, cx| e.sel(cx).set_selection(7, 8));
        let p = at(&e, cx, 7.1);
        assert_eq!(press(cx, p), vec![Selection::new(7, 8)]);
        // a selection that takes in the line break: past the line's text is no char
        e.update(cx, |e, cx| e.sel(cx).set_selection(11, 17));
        let p = at(&e, cx, 30.);
        assert_eq!(press(cx, p), vec![Selection::caret(16)]);
    }

    #[gpui::test]
    fn a_press_below_the_last_row_is_on_no_char(cx: &mut TestAppContext) {
        let (e, cx) = editor("alpha beta gamma", cx);
        e.update(cx, |e, cx| {
            e.sel(cx).set_selection(6, 10);
            cx.notify();
        });
        cx.run_until_parked();
        let (on, below) = e.read_with(cx, |e, _| (cell(e, 7.4, 0.), cell(e, 7.4, 3.)));
        cx.simulate_mouse_down(below, MouseButton::Left, Modifiers::none());
        assert_eq!(state(&e, cx).1, vec![Selection::caret(7)]);
        cx.simulate_mouse_up(below, MouseButton::Left, Modifiers::none());
        // the same column on the row itself waits
        e.update(cx, |e, cx| e.sel(cx).set_selection(6, 10));
        cx.simulate_mouse_down(on, MouseButton::Left, Modifiers::none());
        assert_eq!(state(&e, cx).1, vec![Selection::new(6, 10)]);
    }

    #[gpui::test]
    fn the_drop_caret_is_drawn_where_the_text_would_land(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        let carets = |e: &Entity<Editor>, cx: &mut VisualTestContext| {
            e.read_with(cx, |e, _| e.layout.as_ref().unwrap().drawn.carets.clone())
        };
        assert!(carets(&e, cx).is_empty(), "a selection draws no caret");
        let (press, drop) = (at(&e, cx, 7.4), at(&e, cx, 11.1));
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        assert!(carets(&e, cx).is_empty(), "not until the pointer has gone 4 px");
        cx.simulate_mouse_move(drop, Some(MouseButton::Left), Modifiers::none());
        cx.run_until_parked();
        let (x, drawn) = (at(&e, cx, 11.).x, carets(&e, cx));
        assert_eq!(drawn.len(), 1, "{drawn:?}");
        assert!((drawn[0].left() - x).abs() < px(0.5), "{:?} vs {x:?}", drawn[0]);
        // over the source, in the gutter or on the minimap a drop does nothing: no caret
        let (inside, gutter, minimap) = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            let (y, mm) = (cell(e, 0., 0.).y, l.minimap.expect("a minimap").0);
            (cell(e, 8.4, 0.), point(l.bounds.left() + px(3.), y), point(mm + px(5.), y))
        });
        for p in [inside, gutter, minimap] {
            cx.simulate_mouse_move(p, Some(MouseButton::Left), Modifiers::none());
            cx.run_until_parked();
            assert!(carets(&e, cx).is_empty(), "{p:?}");
        }
        cx.simulate_mouse_move(drop, Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(drop, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        assert!(carets(&e, cx).is_empty(), "the drop selected the text it moved");
    }

    #[gpui::test]
    fn a_drop_outside_the_text_or_the_view_cancels(cx: &mut TestAppContext) {
        let (e, cx) = beta(cx);
        let kept = (PLAIN.to_string(), vec![Selection::new(6, 10)]);
        let (gutter, minimap, outside) = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            let y = cell(e, 0., 0.).y;
            let mm = l.minimap.expect("a minimap").0;
            (point(l.bounds.left() + px(3.), y), point(mm + px(5.), y), point(l.bounds.right() + px(40.), y))
        });
        for p in [gutter, minimap, outside] {
            let press = at(&e, cx, 7.4);
            drag(cx, press, p, Modifiers::none());
            assert_eq!(state(&e, cx), kept, "{p:?}");
            assert!(e.read_with(cx, |e, _| e.drag.is_none()));
        }
    }

    /// An editor with a panel over its middle, as a palette or a popup is.
    struct Covered(Entity<Editor>);

    impl Render for Covered {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let panel = div().absolute().top(px(60.)).left(px(300.)).size(px(200.));
            div().size_full().relative().child(self.0.clone()).child(panel.occlude())
        }
    }

    #[gpui::test]
    fn a_drop_over_a_panel_drawn_on_the_view_cancels(cx: &mut TestAppContext) {
        let (cover, cx) = cx.add_window_view(|_, cx| {
            let mut state = EditorState::from_text(PLAIN);
            state.set_selection(6, 10);
            Covered(cx.new(|cx| Editor::new(state, false, cx)))
        });
        cx.run_until_parked();
        let e = cover.read_with(cx, |c, _| c.0.clone());
        let press = at(&e, cx, 7.4);
        // inside the text area, so the drop would have gone through but for the panel
        let under_panel = point(px(350.), px(100.));
        assert!(e.read_with(cx, |e, cx| e.drop_offset(under_panel, cx).is_some()));
        drag(cx, press, under_panel, Modifiers::none());
        let kept = (PLAIN.to_string(), vec![Selection::new(6, 10)]);
        assert_eq!(state(&e, cx), kept);
        // the same release with the panel out of the way drops
        let elsewhere = point(px(350.), px(300.));
        drag(cx, press, elsewhere, Modifiers::none());
        assert_ne!(state(&e, cx).0, PLAIN);
    }

    #[gpui::test]
    fn a_text_drag_scrolls_past_the_views_edge_and_selects_nothing(cx: &mut TestAppContext) {
        let (e, cx) = editor(&"line\n".repeat(200), cx);
        e.update(cx, |e, cx| e.sel(cx).set_selection(0, 4));
        cx.run_until_parked();
        let (press, below) = e.read_with(cx, |e, _| {
            let l = e.layout.as_ref().unwrap();
            (cell(e, 1.4, 0.), point(l.text_left, l.bounds.bottom() + px(30.)))
        });
        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(below, Some(MouseButton::Left), Modifiers::none());
        e.update_in(cx, |e, window, cx| {
            assert!(e.autoscroll(0.05, window, cx), "goes on while the pointer is out");
            assert!(e.scroll_top > 0.);
            assert_eq!(e.state.selections, vec![Selection::new(0, 4)]);
        });
    }

    #[gpui::test]
    fn a_drop_below_a_fold_goes_by_the_rows_drawn(cx: &mut TestAppContext) {
        let (e, cx) = editor("zero\none\ntwo\nthree\nfour\nfive\n", cx);
        e.update(cx, |e, cx| {
            e.fold_lines_now(0, 3, cx);
            let five = e.doc(cx).line_start(5);
            e.sel(cx).set_selection(five, five + 4);
            cx.notify();
        });
        cx.run_until_parked();
        // the rows drawn are zero, four, five; "five" goes between the "o" and the "u" of "four"
        let (press, drop) = e.read_with(cx, |e, _| (cell(e, 1.4, 2.), cell(e, 2.1, 1.)));
        drag(cx, press, drop, Modifiers::none());
        let four = e.read_with(cx, |e, cx| e.doc(cx).line_start(4));
        let moved = "zero\none\ntwo\nthree\nfofiveur\n\n".to_string();
        assert_eq!(state(&e, cx), (moved, vec![Selection::new(four + 2, four + 6)]));
        let hidden = e.read_with(cx, |e, cx| e.hidden_lines(cx));
        assert_eq!((hidden.len(), hidden[0].clone()), (1, 1..4), "still folded");
    }

    #[gpui::test]
    fn a_drop_on_a_folds_marker_lands_at_its_headers_end(cx: &mut TestAppContext) {
        let (e, cx) = editor("head\n  one\n  two\ntail\n", cx);
        e.update(cx, |e, cx| {
            e.fold_lines_now(0, 2, cx);
            let tail = e.doc(cx).line_start(3);
            e.sel(cx).set_selection(tail, tail + 4);
            cx.notify();
        });
        cx.run_until_parked();
        // the rows drawn are head, tail; the pointer is on the marker, past "head"
        let (press, marker) = e.read_with(cx, |e, _| (cell(e, 1.4, 1.), cell(e, 5., 0.)));
        drag(cx, press, marker, Modifiers::none());
        let text = "headtail\n  one\n  two\n\n".to_string();
        assert_eq!(state(&e, cx), (text, vec![Selection::new(4, 8)]));
        // the text joined the header's own, which stays folded as it does when typed there
        let hidden = e.read_with(cx, |e, cx| e.hidden_lines(cx));
        assert_eq!((hidden.len(), hidden[0].clone()), (1, 1..3));
    }
}
