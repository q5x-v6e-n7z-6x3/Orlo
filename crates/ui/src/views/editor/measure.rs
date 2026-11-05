//! How far across a view's rows each column is drawn, for the motions that keep to an x.

use super::element::{display_len, row, text_run, Layout, RowText, LONG_LINE};
use super::*;
use crate::editing::display::expand_tabs_at;
use crate::editing::text::Measure;
use gpui::ShapedLine;
use std::cell::RefCell;
use std::collections::HashMap;

/// A view's rows as shaped text: x in pixels from the text's left edge, gutter, scroll and
/// window aside. A row the last frame painted and that still reads the same is measured as
/// painted, in its fonts and weights; any other (off screen for Page Up/Down, or edited since
/// the frame) is shaped here in the view's font, once per motion. Before a first paint there is
/// nothing to measure by, and a column is a unit.
pub(crate) struct Shaped<'a> {
    rows: &'a Padded<'a, Folded<'a>>,
    wrapped: bool,
    layout: Option<&'a Layout>,
    window: &'a Window,
    /// Each row asked for: the column it starts at (a continuation row's indent) and its text
    /// as shaped; `None` for a long row, which goes by columns as its clicks do.
    shaped: RefCell<HashMap<usize, Option<(usize, ShapedLine)>>>,
}

impl<'a> Shaped<'a> {
    pub(crate) fn new(
        rows: &'a Padded<'a, Folded<'a>>,
        wrapped: bool,
        layout: Option<&'a Layout>,
        window: &'a Window,
    ) -> Self {
        Self { rows, wrapped, layout, window, shaped: RefCell::default() }
    }

    /// Text row `r` as shaped: the painted one if it shows what the row reads now.
    fn line(&self, s: &Document, layout: &Layout, r: DisplayRow) -> Option<(usize, ShapedLine)> {
        if let Some(line) = self.shaped.borrow().get(&r.0) {
            return line.clone();
        }
        let line = row(s, self.rows, r.0)
            // a long unwrapped row is cut to the view when painted: long for what it shows
            .filter(|row| self.wrapped || display_len(&row.pieces) <= LONG_LINE)
            .map(|row| {
                let text = RowText::new(&s.rope, &row.pieces).text;
                let shown = expand_tabs_at(&text, s.tab_size, row.x0).0;
                let painted = (r.0.checked_sub(layout.first_row))
                    .and_then(|i| layout.rows.get(i)?.as_ref())
                    .filter(|(x0, line)| *x0 == row.x0 && line.text.as_ref() == shown.as_ref());
                match painted {
                    Some(line) => line.clone(),
                    None => {
                        let run = text_run(shown.len(), &layout.font, Hsla::default());
                        let shaped = self.window.text_system().shape_line(
                            SharedString::from(shown.into_owned()),
                            layout.font_size,
                            &[run],
                            None,
                        );
                        (row.x0, shaped)
                    }
                }
            });
        self.shaped.borrow_mut().insert(r.0, line.clone());
        line
    }
}

impl Measure for Shaped<'_> {
    fn x_of(&self, s: &Document, at: DisplayPoint) -> f32 {
        let Some(layout) = self.layout else {
            return at.col.0 as f32;
        };
        let x = match self.line(s, layout, at.row) {
            Some((x0, line)) => {
                let col = at.col.0.saturating_sub(x0);
                let extra = col.saturating_sub(line.text.chars().count());
                layout.space_w * (x0 + extra) as f32 + line.x_for_index(byte_of(&line.text, col))
            }
            None => layout.space_w * at.col.0 as f32,
        };
        f32::from(x)
    }

    fn col_at(&self, s: &Document, row: DisplayRow, x: f32) -> DisplayCol {
        let Some(layout) = self.layout else {
            return DisplayCol(x.round().max(0.) as usize);
        };
        match self.line(s, layout, row) {
            Some((x0, line)) => {
                let x = px(x) - layout.space_w * x0 as f32;
                let byte = line.closest_index_for_x(x);
                let extra = (f32::from(x - line.width) / f32::from(layout.space_w)).round().max(0.) as usize;
                DisplayCol(x0 + line.text[..byte.min(line.text.len())].chars().count() + extra)
            }
            None => DisplayCol((x / f32::from(layout.space_w)).round().max(0.) as usize),
        }
    }
}
