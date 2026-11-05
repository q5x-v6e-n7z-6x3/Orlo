//! Up, Down and added carets keep to an x, as a view's `Measure` draws its rows, not a column.

use super::*;
use crate::viewport::expand_tabs;

/// A proportional font in miniature, over unwrapped lines: CJK two units wide, `i` and `l`
/// half a unit, anything else (a tab's columns too) one.
struct Widths;

impl Widths {
    /// The x of each display column of `row`'s line, and past its last.
    fn xs(s: &Document, row: DisplayRow) -> Vec<f32> {
        let line = s.line_start(row.0)..s.line_end(row.0);
        let text = s.rope.slice(line).to_string();
        let (shown, _) = expand_tabs(&text, s.tab_size);
        let mut xs = vec![0.];
        for c in shown.chars() {
            let w = match c {
                'i' | 'l' => 0.5,
                c if c as u32 >= 0x2E80 => 2.,
                _ => 1.,
            };
            xs.push(xs.last().unwrap() + w);
        }
        xs
    }
}

impl Measure for Widths {
    fn x_of(&self, s: &Document, at: DisplayPoint) -> f32 {
        let xs = Self::xs(s, at.row);
        xs[at.col.0.min(xs.len() - 1)]
    }

    fn col_at(&self, s: &Document, row: DisplayRow, x: f32) -> DisplayCol {
        let xs = Self::xs(s, row);
        let near = (0..xs.len()).min_by(|&a, &b| (xs[a] - x).abs().total_cmp(&(xs[b] - x).abs()));
        DisplayCol(near.unwrap_or(0))
    }
}

fn go(s: &mut EditorState, motion: Motion, select: bool) -> usize {
    s.move_cursors_in(motion, select, &Lines, &Widths);
    s.cursor()
}

#[test]
fn up_and_down_land_at_the_x_not_the_column() {
    // after 日本 is x 4: the fifth column of the line below, not the third
    let mut s = st("日本語\nabcdef\nl日日", 2);
    assert_eq!(go(&mut s, Motion::Down, false), 4 + 4);
    // l is half a unit: x 4 is nearest the end of l日日 (4.5), not after l日 (2.5)
    assert_eq!(go(&mut s, Motion::Down, false), 11 + 3);
    assert_eq!(go(&mut s, Motion::Up, false), 4 + 4);
    assert_eq!(go(&mut s, Motion::Up, false), 2);
}

#[test]
fn the_x_survives_shorter_rows_between() {
    // after 日本語日 is x 8; "ab" ends at 2
    let mut s = st("日本語日本\nab\nabcdefghij", 4);
    assert_eq!(go(&mut s, Motion::Down, false), 6 + 2);
    assert_eq!(go(&mut s, Motion::Down, false), 9 + 8);
    assert_eq!(go(&mut s, Motion::Up, false), 6 + 2);
    assert_eq!(go(&mut s, Motion::Up, false), 4);
}

#[test]
fn sideways_moves_clicks_and_edits_reset_the_x() {
    let text = "日本語\niiiiiiiiiiii";
    let mut s = st(text, 2);
    // x 4 is after eight i's
    assert_eq!(go(&mut s, Motion::Down, false), 4 + 8);
    for _ in 0..3 {
        go(&mut s, Motion::Left, false);
    }
    // now at x 2.5: up lands after 日 (2), where the kept x 4 would give 4
    assert_eq!(go(&mut s, Motion::Up, false), 1);

    let mut s = st(text, 2);
    go(&mut s, Motion::Down, false);
    s.view.selections = vec![Selection::caret(4 + 5)];
    assert_eq!(go(&mut s, Motion::Up, false), 1);

    let mut s = st(text, 2);
    go(&mut s, Motion::Down, false);
    s.insert("iiii");
    // after twelve i's is x 6: after 日本語, where the kept x 4 would give 2
    assert_eq!(go(&mut s, Motion::Up, false), 3);
}

#[test]
fn a_tab_is_its_columns_wide() {
    // after the tab is column 4, x 4: after 日本 below, not after 日本語日
    let mut s = st("\tab\n日本語日本", 1);
    assert_eq!(go(&mut s, Motion::Down, false), 4 + 2);
    assert_eq!(go(&mut s, Motion::Up, false), 1);
}

#[test]
fn each_caret_keeps_its_own_x() {
    // x 2 and x 8; "abc" ends at 3
    let mut s = carets("日本語日本\nabc\nabcdefghij", &[1, 4]);
    go(&mut s, Motion::Down, false);
    go(&mut s, Motion::Down, false);
    assert_eq!(carets_of(&s), [10 + 2, 10 + 8]);
}

#[test]
fn extending_a_selection_and_page_down_keep_the_x() {
    let mut s = st("日本語\nabcdef\nab\nabcdef", 2);
    go(&mut s, Motion::Down, true);
    assert_eq!(s.view.selections[0].range(), 2..4 + 4);
    let mut s = st("日本語\nabcdef\nab\nabcdef", 2);
    assert_eq!(go(&mut s, Motion::PageDown(3), false), 14 + 4);
    assert_eq!(go(&mut s, Motion::PageUp(3), false), 2);
}

#[test]
fn line_ends_stay_line_ends() {
    let mut s = st("日本語\nabcdef\nab", 0);
    go(&mut s, Motion::LineEnd, false);
    assert_eq!(go(&mut s, Motion::Down, false), 4 + 6);
    assert_eq!(go(&mut s, Motion::Down, false), 11 + 2);
}

#[test]
fn added_carets_land_at_the_x() {
    let mut s = st("abcdef\n日本語\nabcdef", 7 + 2);
    s.select_lines(true, &Lines, &Widths);
    assert_eq!(carets_of(&s), [9, 11 + 4]);
    s.view.selections = vec![Selection::new(7, 7 + 2)];
    s.select_lines(false, &Lines, &Widths);
    let ranges: Vec<_> = s.view.selections.iter().map(|s| s.range()).collect();
    assert_eq!(ranges, [0..4, 7..9]);
}

#[test]
fn cells_measure_a_column_a_unit() {
    // `Cells` is how a view without shaped rows moves: as columns always did
    let mut s = st("日本語\nabcdef", 2);
    s.move_cursors_in(Motion::Down, false, &Lines, &Cells);
    assert_eq!(s.cursor(), 4 + 2);
    assert_eq!(Cells.x_of(&s.doc, DisplayPoint::new(0, 3)), 3.);
    assert_eq!(Cells.col_at(&s.doc, DisplayRow(0), 2.6), DisplayCol(3));
    assert_eq!(Cells.col_at(&s.doc, DisplayRow(0), -1.), DisplayCol(0));
}
